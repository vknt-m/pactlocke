//! Shadow Games and Spectate in a Run with other Players: this Player's input
//! stream goes out through the lobby's network, every other Player's comes
//! in, and each runs as a Shadow Game on its own thread. Watching one pauses
//! this Player's own game and shows the Shadow's picture and sound.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use pactlocke_emu::{CoreAdapter, FrameSlot};
use pactlocke_game::{GameProfile, ProfileGame};
use pactlocke_mgba::MgbaCore;
use pactlocke_run::{
    CheatWatch, GamePoint, InputChunk, Judge, Link, LinkKeeper, Locked, LogFollower, PlayerKey, RunClock, RunCommand,
    RunEvent, RunLog, Seen, ShadowCheckpoint, ShadowGame, ShadowRunner, ShadowState, ShadowStatus, StreamHub,
    StreamMessage, SystemRunClock,
};

use crate::audio::AudioOut;
use crate::lobby_ui::Lobbies;

/// Another Player, and the Shadow Game of their game.
pub struct Shadow {
    /// Their install's key.
    pub key: PlayerKey,
    /// Their name in the roster.
    pub name: String,
    runner: ShadowRunner,
    /// A developer's made-up Run (`--demo-coop`): what to show instead of the runner's status.
    demo: Option<ShadowStatus>,
}

impl Shadow {
    /// Where their Shadow Game stands.
    pub fn status(&self) -> ShadowStatus {
        self.demo.clone().unwrap_or_else(|| self.runner.status())
    }

    /// Where its frames are published while watched.
    pub fn frames(&self) -> Arc<FrameSlot> {
        self.runner.frames()
    }

    /// Why it can't be watched now, if it can't.
    pub fn cant_watch(&self) -> Option<String> {
        match self.status().state {
            ShadowState::Matched => None,
            ShadowState::Diverged { .. } | ShadowState::Restarted { .. } => {
                Some(format!("Can't watch {} right now", self.name))
            }
        }
    }
}

/// This app's side of every input stream, and a Shadow Game of every other Player.
pub struct Spectate {
    me: PlayerKey,
    /// Joins other Players' Run Checkpoints and goes back on a Retry.
    follower: LogFollower,
    /// Kills this Player's member of a broken Link and closes its source.
    keeper: LinkKeeper,
    hub: StreamHub,
    outbox: Arc<Mutex<Vec<InputChunk>>>,
    /// This Player's Run, for keeping the Shadow Games' checkpoints in its file.
    commands: Sender<RunCommand>,
    pub shadows: Vec<Shadow>,
    /// The Shadow being watched.
    pub watching: Option<usize>,
    pub battery_saver: bool,
    /// Who this app has heard from, and when.
    presence: pactlocke_run::Presence,
    /// Players this Player chose to keep waiting for, rather than vote out.
    keep_waiting: Vec<PlayerKey>,
    /// What this app has seen of the others' streams, to flag cheats.
    watch: CheatWatch,
    /// The other Players' Run events from the Run log, for the feed: whose,
    /// and the line, newest first; and how far into the log that is.
    others_feed: Vec<(PlayerKey, pactlocke_run::FeedEntry)>,
    fed: usize,
    /// Shadow Games that stopped matching, as their threads said: whose, and at what frame.
    diverged: Arc<Mutex<Vec<(PlayerKey, u64)>>>,
    /// Partner Avatars: where this Player's game reads them from.
    avatars: Option<Arc<Mutex<Vec<pactlocke_run::PartnerAvatar>>>>,
}

impl Spectate {
    /// A Shadow Game of every other Player in the lock's roster, each on
    /// that Player's game (`rom_of`: the Run's ROM, or in a randomized Run
    /// their randomized game, made here), booted as the Run started, or
    /// from its checkpoint in the Run file (`kept`) on a resume.
    /// `outbox` is this Player's own stream; `commands` reach their Run.
    pub fn new(
        me: PlayerKey,
        locked: &Locked,
        rom_of: &dyn Fn(PlayerKey) -> Arc<[u8]>,
        profile: &Arc<GameProfile>,
        outbox: Arc<Mutex<Vec<InputChunk>>>,
        commands: &Sender<RunCommand>,
        kept: &[(PlayerKey, ShadowCheckpoint)],
    ) -> Result<Self, String> {
        let host = locked.roster.first().map_or(me, |p| p.key);
        let mut hub = StreamHub::new(me, host);
        let mut shadows = Vec::new();
        let mut watch = CheatWatch::new(locked.start_ms);
        let diverged: Arc<Mutex<Vec<(PlayerKey, u64)>>> = Arc::default();
        for player in locked.roster.iter().filter(|p| p.key != me) {
            let mut core = MgbaCore::new();
            let rom = rom_of(player.key);
            core.load_rom(&rom).map_err(|e| e.to_string())?;
            let game = ProfileGame::new(profile.clone(), rom, core);
            let shadow = match kept.iter().find(|(p, _)| *p == player.key) {
                Some((_, checkpoint)) => {
                    hub.starts_at(player.key, checkpoint.at);
                    // Their stream goes on from there, in a boot that began
                    // as its clock says: what arrives at once is their play
                    // while this app was away.
                    watch.resumed(player.key, checkpoint.at, checkpoint.clock_ms, SystemRunClock.now_ms());
                    ShadowGame::from_checkpoint(game, locked.start_ms, checkpoint)
                        .map_err(|e| format!("The Shadow Game of {} couldn't be restored: {e}", player.name))?
                }
                None => ShadowGame::power_on(game, locked.start_ms),
            };
            let commands = commands.clone();
            let name = player.name.clone();
            let (key, diverged_too) = (player.key, diverged.clone());
            let tell = Box::new(move |frame| {
                log::warn!("The Shadow Game of {name} stopped matching at frame {frame}");
                diverged_too
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((key, frame));
                // The Run is closed when nobody is listening.
                let _ = commands.send(RunCommand::OutOfSync { player: name, frame });
            });
            shadows.push(Shadow {
                key: player.key,
                name: player.name.clone(),
                runner: ShadowRunner::spawn(shadow, Box::new(SystemRunClock), tell),
                demo: None,
            });
        }
        Ok(Self {
            me,
            follower: LogFollower::new(me),
            keeper: LinkKeeper::new(me),
            hub,
            outbox,
            commands: commands.clone(),
            shadows,
            watching: None,
            battery_saver: false,
            avatars: None,
            presence: pactlocke_run::Presence::new(SystemRunClock.now_ms()),
            keep_waiting: Vec::new(),
            watch,
            others_feed: Vec::new(),
            fed: 0,
            diverged,
        })
    }

    /// Keeps each Shadow Game's checkpoint in the Run file: those taken
    /// since the last call, or, as this app leaves the Run, one of each now.
    pub fn keep_checkpoints(&self, now: bool) {
        for shadow in &self.shadows {
            let checkpoint = if now {
                shadow.runner.checkpoint_now()
            } else {
                shadow.runner.take_checkpoint()
            };
            if let Some(checkpoint) = checkpoint {
                // The Run is closed when nobody is listening.
                let _ = self.commands.send(RunCommand::KeepShadow {
                    player: shadow.key,
                    checkpoint: Box::new(checkpoint),
                });
            }
        }
    }

    /// The Run was resumed from its Run file: the log as kept there was
    /// followed before, and `own` is this Player's stream from before this
    /// session, to answer the others asking for what they missed.
    pub fn resumed(&mut self, log: &RunLog, own: Vec<InputChunk>) {
        self.follower = LogFollower::resuming(self.me, log);
        self.fed = log.entries().len();
        self.hub.already_sent(own);
    }

    /// This Player's game draws the Partner Avatars from here.
    pub fn draw_partner_avatars(&mut self, avatars: Arc<Mutex<Vec<pactlocke_run::PartnerAvatar>>>) {
        self.avatars = Some(avatars);
    }

    /// Sends this Player's new chunks, takes in everyone else's, and tells
    /// this Player's game where every Partner Avatar is, as their Shadows
    /// show them: only a Shadow that matches and is caught up.
    pub fn pump(&mut self, lobbies: &mut Lobbies) {
        if let Some(avatars) = &self.avatars {
            let now: Vec<pactlocke_run::PartnerAvatar> = self
                .shadows
                .iter()
                .filter_map(|shadow| {
                    let status = shadow.status();
                    let live = status.state == ShadowState::Matched && !status.catching_up;
                    let seen = status.seen.avatar.filter(|_| live)?;
                    Some(pactlocke_run::PartnerAvatar {
                        player: shadow.key,
                        seen,
                    })
                })
                .collect();
            *avatars.lock().unwrap_or_else(|e| e.into_inner()) = now;
        }
        // Streams go through whoever hosts now: after a takeover, the new Host.
        if let Some(host) = lobbies.host_key()
            && host != self.hub.host()
        {
            self.hub.host_is(host);
        }
        self.keep_checkpoints(false);
        let own = std::mem::take(&mut *self.outbox.lock().unwrap_or_else(|e| e.into_inner()));
        for chunk in own {
            for out in self.hub.own(chunk) {
                lobbies.send_stream(out);
            }
        }
        let now = SystemRunClock.now_ms();
        let mut flags = Vec::new();
        for (from, bytes) in lobbies.take_stream() {
            let Some(message) = StreamMessage::from_bytes(&bytes) else {
                continue;
            };
            self.presence.heard(from, now);
            if let StreamMessage::Chunk(chunk) = &message {
                self.presence.heard(chunk.player, now);
            }
            let received = self.hub.receive(from, message);
            for out in received.send {
                lobbies.send_stream(out);
            }
            if let Some(log) = lobbies.log() {
                if let Some(rewrite) = &received.rewritten {
                    flags.extend(self.watch.rewritten(log, rewrite, now));
                }
                for chunk in &received.new {
                    flags.extend(self.watch.arrived(chunk, log, now));
                }
            }
            for chunk in received.new {
                if let Some(shadow) = self.shadows.iter().find(|s| s.key == chunk.player) {
                    shadow.runner.receive(chunk);
                }
            }
        }
        let diverged = std::mem::take(&mut *self.diverged.lock().unwrap_or_else(|e| e.into_inner()));
        if let Some(log) = lobbies.log() {
            for (player, frame) in diverged {
                let Some(shadow) = self.shadows.iter().find(|s| s.key == player) else {
                    continue;
                };
                let boot = shadow.status().boot;
                flags.extend(self.watch.diverged(log, player, GamePoint { boot, frame }, now));
            }
            flags.extend(self.watch.due(log, now));
        }
        // Each flag goes in the Run log for every Player to see.
        if !flags.is_empty() {
            lobbies.share(flags);
        }
    }

    /// This Player's key.
    pub fn me(&self) -> PlayerKey {
        self.me
    }

    /// What this Player's Run should do about the log's new entries and
    /// the Links as they stand now.
    pub fn follow(&mut self, log: &RunLog) -> Vec<RunCommand> {
        // Only the entries new since last time, each read on its own, so one
        // that can't be read never shifts the rest.
        let now = SystemRunClock.now_ms();
        for entry in log.entries().iter().skip(self.fed) {
            let author = entry.submission.author;
            if author != self.me
                && let Ok(pactlocke_run::LogBody::Event(event)) = serde_json::from_slice(&entry.submission.body)
                && let Some(line) = pactlocke_run::FeedEntry::of_others(&event, now)
            {
                self.others_feed.insert(0, (author, line));
            }
        }
        self.fed = log.entries().len();
        let mut commands = self.follower.follow(log);
        if log.locked().is_some_and(|l| l.ruleset.links) {
            let links = self.links(log);
            commands.extend(self.keeper.keep(log, &links));
        }
        commands
    }

    /// The other Players' feed lines, newest first.
    pub fn others_feed(&self) -> &[(PlayerKey, pactlocke_run::FeedEntry)] {
        &self.others_feed
    }

    /// Every Link, with the other Players' catches and Deaths judged by
    /// what their Shadow Games here have shown.
    pub fn links(&self, log: &RunLog) -> Vec<Link> {
        let judge = Seeing(self.shadows.iter().map(|s| (s.key, s.status().seen)).collect());
        pactlocke_run::links(log, &self.me, &judge)
    }

    /// Watches Shadow `i`, playing its sound through `audio`.
    pub fn watch(&mut self, i: usize, audio: Option<&AudioOut>) {
        self.stop_watching();
        if let Some(shadow) = self.shadows.get(i) {
            match audio {
                Some(audio) => shadow.runner.watch(Box::new(audio.sink())),
                None => shadow.runner.watch(Box::new(Silent)),
            }
            self.watching = Some(i);
        }
    }

    pub fn stop_watching(&mut self) {
        if let Some(shadow) = self.watching.take().and_then(|i| self.shadows.get(i)) {
            shadow.runner.unwatch();
        }
    }

    /// The partner view: every Shadow publishes its picture as a tile.
    pub fn set_partner_view(&mut self, on: bool) {
        for shadow in &self.shadows {
            shadow.runner.tiled(on);
        }
    }

    /// Battery saver: the Shadows run only while one is watched.
    pub fn set_battery_saver(&mut self, on: bool) {
        self.battery_saver = on;
        for shadow in &self.shadows {
            shadow.runner.battery_saver(on);
        }
    }

    /// The Shadow being watched, if any.
    pub fn watched(&self) -> Option<&Shadow> {
        self.shadows.get(self.watching?)
    }
}

/// Where a watched Shadow's sound goes when there is no audio device.
struct Silent;

impl pactlocke_emu::AudioSink for Silent {
    fn push(&mut self, _samples: &[i16], _sample_rate: u32) {}
}

/// A vote under way, in a banner that stands out from the rest of the panel.
fn vote_banner(ui: &mut eframe::egui::Ui, contents: impl FnOnce(&mut eframe::egui::Ui)) {
    let colour = ui.visuals().warn_fg_color;
    eframe::egui::Frame::new()
        .fill(colour.gamma_multiply(0.12))
        .stroke(eframe::egui::Stroke::new(1.0, colour))
        .corner_radius(5.0)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            contents(ui);
        });
}

/// Retries in a Run with other Players: what is left of the pool, and the
/// vote. Returns this Player's vote when they cast one.
pub fn votes(ui: &mut eframe::egui::Ui, log: &RunLog, me: PlayerKey, online: bool) -> Option<RunEvent> {
    let pool = log.locked().map_or(0, |l| l.ruleset.retries);
    if pool == 0 {
        return None;
    }
    let left = log.retries_left();
    let players = log.active().len();
    let mut cast = None;
    match log.vote() {
        Some(vote) => vote_banner(ui, |ui| {
            ui.strong(format!(
                "Retry vote: back to Run Checkpoint {} · {} of {players} said Yes",
                vote.call,
                vote.yes.len()
            ));
            if !vote.yes.contains(&me) && online {
                ui.horizontal(|ui| {
                    if ui.button("Yes").clicked() {
                        cast = Some(RunEvent::RetryVote {
                            call: vote.call,
                            yes: true,
                        });
                    }
                    if ui.button("No").clicked() {
                        cast = Some(RunEvent::RetryVote {
                            call: vote.call,
                            yes: false,
                        });
                    }
                });
            }
        }),
        None => {
            let latest = log.latest_complete();
            let start = ui.add_enabled(
                left > 0 && latest.is_some() && online,
                eframe::egui::Button::new(format!("Start a Retry vote · {left} left")),
            );
            let start = start.on_hover_text(
                "Everyone goes back to the latest Run Checkpoint if everyone says Yes; the first No ends the vote",
            );
            if start.clicked()
                && let Some(call) = latest
            {
                cast = Some(RunEvent::RetryVote { call, yes: true });
            }
        }
    }
    cast
}

/// What every Shadow Game here has shown, by Player.
struct Seeing(Vec<(PlayerKey, Seen)>);

impl Judge for Seeing {
    fn has(&self, player: &PlayerKey, pid: u32) -> bool {
        self.0.iter().any(|(p, seen)| p == player && seen.owned.contains(&pid))
    }

    fn fainted(&self, player: &PlayerKey, pid: u32) -> bool {
        self.0
            .iter()
            .any(|(p, seen)| p == player && seen.fainted.contains(&pid))
    }
}

/// What this Player chose about a Player long offline.
pub enum DropoutAsk {
    /// A vote to share in the Run log.
    Vote(RunEvent),
    /// Don't offer the vote for this Player again in this session.
    KeepWaiting(PlayerKey),
}

impl Spectate {
    /// Whether this Player's app has gone quiet long enough to show offline.
    pub fn shows_offline(&self, player: &PlayerKey) -> bool {
        // A made-up Run's Players are never heard from, and never offline.
        let demo = self.shadows.iter().any(|s| s.demo.is_some());
        !demo && self.presence.offline(player, SystemRunClock.now_ms())
    }

    /// Shows a developer's made-up Run: each other Player's Shadow Game
    /// reports this status, whatever its own game is doing.
    pub fn show_demo(&mut self, statuses: Vec<(PlayerKey, ShadowStatus)>) {
        for (key, status) in statuses {
            if let Some(shadow) = self.shadows.iter_mut().find(|s| s.key == key) {
                shadow.demo = Some(status);
            }
        }
    }

    /// Stop offering a Dropout vote for this Player.
    pub fn keep_waiting(&mut self, player: PlayerKey) {
        self.keep_waiting.push(player);
    }
}

/// A Player's name from the Run's roster.
fn roster_name(log: &RunLog, key: &PlayerKey) -> String {
    log.locked()
        .and_then(|l| l.roster.into_iter().find(|p| p.key == *key))
        .map_or_else(|| "A Player".to_owned(), |p| p.name)
}

/// The vote on clearing a cheat flag, for a Player who may vote.
fn flag_vote(flag: &pactlocke_run::Flag, yes: bool) -> RunEvent {
    RunEvent::FlagVote {
        player: flag.player,
        cheat: flag.cheat,
        boot: flag.at.boot,
        yes,
    }
}

/// Votes under way on clearing a cheat flag, with the other votes: what
/// each is about, how many said Yes, and Yes and No for a Player who may vote.
pub fn flag_votes(ui: &mut eframe::egui::Ui, log: &RunLog, me: PlayerKey, online: bool) -> Option<RunEvent> {
    let mut cast = None;
    for vote in log.flag_votes() {
        let voters = log.flag_voters(&vote.flag.player);
        let whom = if vote.flag.player == me {
            "you".to_owned()
        } else {
            roster_name(log, &vote.flag.player)
        };
        vote_banner(ui, |ui| {
            ui.strong(format!(
                "Vote to clear the cheat flag on {whom}, at frame {}: {} · {} of {} said Yes",
                vote.flag.at.frame,
                vote.flag.cheat.describe(),
                vote.yes.len(),
                voters.len()
            ));
            if voters.contains(&me) && !vote.yes.contains(&me) && online {
                ui.horizontal(|ui| {
                    for (text, yes) in [("Yes", true), ("No", false)] {
                        if ui.button(text).clicked() {
                            cast = Some(flag_vote(&vote.flag, yes));
                        }
                    }
                });
            }
        });
    }
    cast
}

/// The Lobby tab's cheat flags that stand, each with a vote to clear it for
/// a Player who may vote and none is under way: a flag that shouldn't stand,
/// such as one after a crash nobody could give the presses back for.
pub fn flags_to_clear(ui: &mut eframe::egui::Ui, log: &RunLog, me: PlayerKey, online: bool) -> Option<RunEvent> {
    let votes = log.flag_votes();
    let mut cast = None;
    for flag in log.flags() {
        let whom = if flag.player == me {
            "you".to_owned()
        } else {
            roster_name(log, &flag.player)
        };
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                crate::theme::CHEAT,
                format!(
                    "Cheat flag on {whom}, at frame {}: {}",
                    flag.at.frame,
                    flag.cheat.describe()
                ),
            );
            let open = votes.iter().any(|v| v.flag == flag);
            if !open
                && log.flag_voters(&flag.player).contains(&me)
                && ui
                    .add_enabled(online, eframe::egui::Button::new("Vote to clear it"))
                    .on_hover_text("Everyone but the flagged Player must say Yes; the first No ends the vote")
                    .clicked()
            {
                cast = Some(flag_vote(&flag, true));
            }
        });
    }
    cast
}

/// The button that starts a Dropout vote on a Player.
fn start_dropout_button(ui: &mut eframe::egui::Ui, text: &str, online: bool) -> bool {
    ui.add_enabled(online, eframe::egui::Button::new(text))
        .on_hover_text("Everyone else must say Yes; their Pokémon leave every Link, and the Run goes on without them")
        .clicked()
}

/// Dropouts: a vote under way, or the offer to start one for a Player
/// offline 15 minutes (or to keep waiting). With two Players there is none.
pub fn dropouts(ui: &mut eframe::egui::Ui, log: &RunLog, spectate: &Spectate, online: bool) -> Option<DropoutAsk> {
    let me = spectate.me;
    let name = |key: &PlayerKey| roster_name(log, key);
    let mut cast = None;
    if let Some((leaving, yes)) = log.dropout_vote() {
        let voters = log.active().len().saturating_sub(1);
        vote_banner(ui, |ui| {
            ui.strong(format!(
                "Dropout vote: {} leaves the Run · {} of {voters} said Yes",
                name(&leaving),
                yes.len()
            ));
            if leaving != me && !yes.contains(&me) && online {
                ui.horizontal(|ui| {
                    for (text, answer) in [("Yes", true), ("No", false)] {
                        if ui.button(text).clicked() {
                            cast = Some(DropoutAsk::Vote(RunEvent::DropoutVote {
                                player: leaving,
                                yes: answer,
                            }));
                        }
                    }
                });
            }
        });
        return cast;
    }
    if !log.dropout_allowed() {
        return None;
    }
    let now = SystemRunClock.now_ms();
    for player in log.active() {
        if player == me || spectate.keep_waiting.contains(&player) || !spectate.presence.dropout_offered(&player, now) {
            continue;
        }
        ui.label(format!(
            "{} has been offline for {} minutes.",
            name(&player),
            pactlocke_run::DROPOUT_OFFER_MS / 60_000
        ));
        ui.horizontal(|ui| {
            if start_dropout_button(ui, "Start a Dropout vote", online) {
                cast = Some(DropoutAsk::Vote(RunEvent::DropoutVote { player, yes: true }));
            }
            if ui.button("Keep waiting").clicked() {
                cast = Some(DropoutAsk::KeepWaiting(player));
            }
        });
    }
    cast
}

/// The Lobby tab's Dropout votes started by hand: one for each other Player
/// offline now, once their offer was dismissed or before it comes. None
/// with two Players, or while a vote is under way.
pub fn dropout_by_hand(
    ui: &mut eframe::egui::Ui,
    log: &RunLog,
    spectate: &Spectate,
    online: bool,
) -> Option<DropoutAsk> {
    if !log.dropout_allowed() || log.dropout_vote().is_some() {
        return None;
    }
    let mut cast = None;
    for player in log.active() {
        if player == spectate.me || !spectate.shows_offline(&player) {
            continue;
        }
        let text = format!("Start a Dropout vote on {}", roster_name(log, &player));
        if start_dropout_button(ui, &text, online) {
            cast = Some(DropoutAsk::Vote(RunEvent::DropoutVote { player, yes: true }));
        }
    }
    cast
}
