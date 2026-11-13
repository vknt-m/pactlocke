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
    InputChunk, Judge, Link, LinkKeeper, Locked, LogFollower, PlayerKey, RunClock, RunCommand, RunEvent, RunLog, Seen,
    ShadowGame, ShadowRunner, ShadowState, ShadowStatus, StreamHub, StreamMessage, SystemRunClock,
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
}

impl Shadow {
    /// Where their Shadow Game stands.
    pub fn status(&self) -> ShadowStatus {
        self.runner.status()
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
    pub shadows: Vec<Shadow>,
    /// The Shadow being watched.
    pub watching: Option<usize>,
    pub battery_saver: bool,
    /// Who this app has heard from, and when.
    presence: pactlocke_run::Presence,
    /// Players this Player chose to keep waiting for, rather than vote out.
    keep_waiting: Vec<PlayerKey>,
    /// Partner Avatars: where this Player's game reads them from.
    avatars: Option<Arc<Mutex<Vec<pactlocke_run::PartnerAvatar>>>>,
}

impl Spectate {
    /// A Shadow Game of every other Player in the lock's roster, each on
    /// this Player's own copy of the ROM, booted as the Run started.
    /// `outbox` is this Player's own stream; `commands` reach their Run.
    pub fn new(
        me: PlayerKey,
        locked: &Locked,
        rom: &Arc<[u8]>,
        profile: &Arc<GameProfile>,
        outbox: Arc<Mutex<Vec<InputChunk>>>,
        commands: &Sender<RunCommand>,
    ) -> Result<Self, String> {
        let host = locked.roster.first().map_or(me, |p| p.key);
        let mut shadows = Vec::new();
        for player in locked.roster.iter().filter(|p| p.key != me) {
            let mut core = MgbaCore::new();
            core.load_rom(rom).map_err(|e| e.to_string())?;
            let game = ProfileGame::new(profile.clone(), rom.clone(), core);
            let shadow = ShadowGame::power_on(game, locked.start_ms);
            let commands = commands.clone();
            let name = player.name.clone();
            let tell = Box::new(move |frame| {
                // The Run is closed when nobody is listening.
                let _ = commands.send(RunCommand::OutOfSync { player: name, frame });
            });
            shadows.push(Shadow {
                key: player.key,
                name: player.name.clone(),
                runner: ShadowRunner::spawn(shadow, Box::new(SystemRunClock), tell),
            });
        }
        Ok(Self {
            me,
            follower: LogFollower::new(me),
            keeper: LinkKeeper::new(me),
            hub: StreamHub::new(me, host),
            outbox,
            shadows,
            watching: None,
            battery_saver: false,
            avatars: None,
            presence: pactlocke_run::Presence::new(SystemRunClock.now_ms()),
            keep_waiting: Vec::new(),
        })
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
        let own = std::mem::take(&mut *self.outbox.lock().unwrap_or_else(|e| e.into_inner()));
        for chunk in own {
            for out in self.hub.own(chunk) {
                lobbies.send_stream(out);
            }
        }
        let now = SystemRunClock.now_ms();
        for (from, bytes) in lobbies.take_stream() {
            let Some(message) = StreamMessage::from_bytes(&bytes) else {
                continue;
            };
            self.presence.heard(from, now);
            if let StreamMessage::Chunk(chunk) = &message {
                self.presence.heard(chunk.player, now);
            }
            let (out, new) = self.hub.receive(from, message);
            for out in out {
                lobbies.send_stream(out);
            }
            for chunk in new {
                if let Some(shadow) = self.shadows.iter().find(|s| s.key == chunk.player) {
                    shadow.runner.receive(chunk);
                }
            }
        }
    }

    /// This Player's key.
    pub fn me(&self) -> PlayerKey {
        self.me
    }

    /// What this Player's Run should do about the log's new entries and
    /// the Links as they stand now.
    pub fn follow(&mut self, log: &RunLog) -> Vec<RunCommand> {
        let mut commands = self.follower.follow(log);
        if log.locked().is_some_and(|l| l.ruleset.links) {
            let links = self.links(log);
            commands.extend(self.keeper.keep(log, &links));
        }
        commands
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
        Some(vote) => {
            ui.label(format!(
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
        }
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
        self.presence.offline(player, SystemRunClock.now_ms())
    }

    /// Stop offering a Dropout vote for this Player.
    pub fn keep_waiting(&mut self, player: PlayerKey) {
        self.keep_waiting.push(player);
    }
}

/// Dropouts: a vote under way, or the offer to start one for a Player
/// offline 15 minutes (or to keep waiting).
pub fn dropouts(ui: &mut eframe::egui::Ui, log: &RunLog, spectate: &Spectate, online: bool) -> Option<DropoutAsk> {
    let me = spectate.me;
    let names: Vec<(PlayerKey, String)> = log
        .locked()
        .map(|l| l.roster.iter().map(|p| (p.key, p.name.clone())).collect())
        .unwrap_or_default();
    let name = |key: &PlayerKey| {
        names
            .iter()
            .find(|(k, _)| k == key)
            .map_or("A Player", |(_, n)| n.as_str())
            .to_owned()
    };
    let mut cast = None;
    if let Some((leaving, yes)) = log.dropout_vote() {
        let voters = log.active().len().saturating_sub(1);
        ui.label(format!(
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
        return cast;
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
            if ui
                .add_enabled(online, eframe::egui::Button::new("Start a Dropout vote"))
                .on_hover_text(
                    "Everyone else must say Yes; their Pokémon leave every Link, and the Run goes on without them",
                )
                .clicked()
            {
                cast = Some(DropoutAsk::Vote(RunEvent::DropoutVote { player, yes: true }));
            }
            if ui.button("Keep waiting").clicked() {
                cast = Some(DropoutAsk::KeepWaiting(player));
            }
        });
    }
    cast
}
