//! Inviting friends and joining a friend's Run: the join prompt and the
//! Lobby tab of the side panel, where the Host sets the Ruleset, everyone
//! says Ready, and the lock starts every Player's Run together. The tab
//! stays for the whole Run, with the tallies and connections.

use std::path::{Path, PathBuf};

use eframe::egui;
use pactlocke_game::sha1_hex;
use pactlocke_run::{Outgoing, PlayerKey, RunClock, RunEvent, SystemRunClock};

use crate::coop::{CoopStart, HostCoop, HostRom, JoinCoop};
use pactlocke_net::{
    APP_VERSION, HostLobby, HostNode, Invite, JoinLobby, JoinNode, LobbyView, Message, PeerKey, Relays, RomStatus,
    RunAdvert, load_or_create_key,
};

/// A friend's invite, being filled in.
#[derive(Default)]
struct JoinPrompt {
    link: String,
    rom: Option<PickedRom>,
    error: Option<String>,
}

/// The lobby this app is in, as Host or joiner, and the join prompt.
pub struct Lobbies {
    key: Result<[u8; 32], String>,
    /// The name other Players see, from Settings.
    pub name: String,
    /// The relays to use, from Settings.
    pub relays: Relays,
    /// A folder where each Host writes its direct addresses and joiners
    /// read them, for a test rig with no relay: none in the app.
    pub addresses: Option<PathBuf>,
    host: Option<(HostNode, HostCoop)>,
    join: Option<(JoinNode, JoinCoop)>,
    prompt: Option<JoinPrompt>,
    /// The SHA-1 of this Player's ROM, told to a Host on every reconnect.
    rom_sha1: Option<String>,
    /// A relay to try a Host on beside finding them by key: the one the
    /// Run's first Host gave in their invite.
    relay: Option<String>,
    /// Addresses to try for the Run's first Host beside its relay (the test rig's).
    direct: Vec<std::net::SocketAddr>,
    /// When this joiner's connection to the Host was lost, if it is.
    lost_at: Option<i64>,
    /// When this joiner last started connecting to a Host.
    dialled_at: i64,
    /// Tries at reaching a Host since the Host went silent.
    redials: u32,
    /// How many times this joiner has started connecting to the Host again after losing it.
    reconnects: u32,
    /// Since when this Host has had no Player connected, if so.
    alone_since: Option<i64>,
    /// Whether this Host's direct addresses are written in `addresses`.
    addresses_written: bool,
    /// The lobby as last written to the app's log, to note each change once.
    noted: String,
}

/// How long a joiner waits before reconnecting to the Host.
const RECONNECT_MS: i64 = 3_000;

/// How long each try at reaching a Host lasts once the Host has gone silent,
/// between the next Player in join order and the Host.
const ATTEMPT_MS: i64 = 5_000;

/// How long a Host may have no Player connected before stepping down to
/// dial the next Player in join order: long enough for every Player still
/// online to have tried this Host a few times, between their other tries.
const STEP_DOWN_MS: i64 = 30_000;

impl Lobbies {
    /// `key_file` holds this install's key, its identity on the network.
    pub fn new(key_file: &Path) -> Self {
        Self {
            key: load_or_create_key(key_file).map_err(|e| {
                format!(
                    "This install's network key can't be kept at {}: {e}",
                    key_file.display()
                )
            }),
            name: "Player".into(),
            relays: Relays::N0,
            addresses: None,
            host: None,
            join: None,
            prompt: None,
            rom_sha1: None,
            relay: None,
            direct: Vec::new(),
            lost_at: None,
            dialled_at: 0,
            redials: 0,
            reconnects: 0,
            alone_since: None,
            addresses_written: false,
            noted: String::new(),
        }
    }

    /// Whether this app is in a lobby, as Host or joiner.
    pub fn in_lobby(&self) -> bool {
        self.host.is_some() || self.join.is_some()
    }

    /// Writes the lobby to the app's log when what this app sees of it changes.
    fn note_changes(&mut self) {
        let now = match (&self.host, &self.join) {
            (Some((node, coop)), _) => {
                let view = node.view();
                let players: Vec<String> = view.players.iter().map(|p| p.name.clone()).collect();
                format!(
                    "Host: players [{}]{}{}",
                    players.join(", "),
                    view.error.map(|e| format!(", network error: {e}")).unwrap_or_default(),
                    coop.error().map(|e| format!(", Run log: {e}")).unwrap_or_default(),
                )
            }
            (None, Some((node, coop))) => {
                let view = node.view();
                match view.error {
                    Some(e) => format!("Joiner: lost the Host: {e}"),
                    None if view.players.is_empty() => "Joiner: connecting to the Host".to_owned(),
                    None => format!(
                        "Joiner: connected, players [{}]{}",
                        view.players
                            .iter()
                            .map(|p| p.name.clone())
                            .collect::<Vec<_>>()
                            .join(", "),
                        coop.error().map(|e| format!(", Run log: {e}")).unwrap_or_default(),
                    ),
                }
            }
            (None, None) => "Not in a lobby".to_owned(),
        };
        if now != self.noted {
            log::info!("{now}");
            self.noted = now;
        }
    }

    fn relay_choice(&self) -> Relays {
        self.relays.clone()
    }

    /// Leaves the lobby this app is in. The network shuts down on its own
    /// thread, so the window never waits for it.
    fn leave(&mut self) {
        let (host, join) = (self.host.take(), self.join.take());
        if host.is_some() || join.is_some() {
            drop_in_background((host, join));
        }
    }

    /// This Player leaves the Run with friends they were playing: to the
    /// others it is a lost connection, and Resume brings them back.
    pub fn leave_run(&mut self) {
        log::info!("Left the Run's network");
        self.leave();
    }

    /// Opens a lobby for this Run on the Host's ROM and makes an invite link.
    pub fn host(&mut self, run: RunAdvert, rom: HostRom) {
        self.leave();
        let Ok(key) = self.key.clone() else {
            return;
        };
        let coop = HostCoop::new(key, &self.name, &run, rom);
        let lobby = HostLobby::new(run, &self.name, APP_VERSION);
        self.host = Some((HostNode::start(key, lobby, self.relay_choice()), coop));
        self.relay = None;
        self.addresses_written = false;
        log::info!("Hosting a new lobby");
    }

    /// Opens a Run with friends again with Resume, from this Player's copy
    /// of its Run log (`log`), their entries that were waiting for a Host,
    /// and their ROM: with no invite, by the key of the Player the log names
    /// as Host. If that is this Player, the Run's Lobby opens here and the
    /// others come to it.
    pub fn resume(
        &mut self,
        log: pactlocke_run::RunLog,
        waiting: Vec<pactlocke_run::Submission>,
        rom: PathBuf,
    ) -> Result<(), String> {
        let key = self.key.clone()?;
        let me = self.public_key().ok_or("This install has no network key.")?;
        let locked = log
            .locked()
            .ok_or("This Run's log has no lock, so it can't be resumed.")?;
        if !log.active().contains(&me) {
            return Err("The others dropped you from this Run. You can carry on alone in a solo copy.".to_owned());
        }
        self.leave();
        self.rom_sha1 = Some(locked.game_sha1.clone());
        self.relay = None;
        self.direct = Vec::new();
        self.lost_at = None;
        self.redials = 0;
        self.alone_since = None;
        self.addresses_written = false;
        let now = SystemRunClock.now_ms();
        let host = log.host();
        if host == me {
            let active = log.active();
            let roster = locked
                .roster
                .iter()
                .filter(|p| p.key != me && active.contains(&p.key))
                .map(|p| (PeerKey(p.key), p.name.clone()))
                .collect();
            let run = RunAdvert {
                name: locked.run_name.clone(),
                game_name: locked.game_name.clone(),
                game_sha1: locked.game_sha1.clone(),
            };
            let lobby = HostLobby::taken_over(run, &self.name, APP_VERSION, roster);
            let mut coop = HostCoop::back(key, log, rom);
            coop.order_waiting(waiting);
            self.host = Some((HostNode::start(key, lobby, self.relay_choice()), coop));
            log::info!("Resuming the Run as its Host");
        } else {
            let mut coop = JoinCoop::stepped_down(key, log, host, rom, now);
            coop.queue_again(waiting);
            self.join = Some((self.dial(key, host), coop));
            self.dialled_at = now;
            log::info!("Resuming the Run: connecting to its Host");
        }
        Ok(())
    }

    /// Run state from this Player's game, for the shared Run log.
    pub fn share(&mut self, events: Vec<RunEvent>) {
        if let Some((node, coop)) = &mut self.host {
            coop.share(node, events);
        } else if let Some((node, coop)) = &mut self.join {
            coop.share(node, events, SystemRunClock.now_ms());
        }
    }

    /// This install's key as other Players know it, if it could be kept.
    pub fn public_key(&self) -> Option<PlayerKey> {
        let key = self.key.as_ref().ok()?;
        Some(pactlocke_run::Signer::from_secret(*key).public())
    }

    /// The Player input streams go through: this one while hosting, else
    /// the Host this joiner's network connects to.
    pub fn host_key(&self) -> Option<PlayerKey> {
        match (&self.host, &self.join) {
            (Some(_), _) => self.public_key(),
            (None, Some((_, coop))) => Some(coop.host()),
            (None, None) => None,
        }
    }

    /// Input stream messages from other Players since the last call: from
    /// whom it came (for a joiner, always the Host), and the bytes.
    pub fn take_stream(&mut self) -> Vec<(PlayerKey, Vec<u8>)> {
        match (&mut self.host, &mut self.join) {
            (Some((_, coop)), _) => coop.take_stream(),
            (None, Some((_, coop))) => coop.take_stream(),
            (None, None) => Vec::new(),
        }
    }

    /// Sends an input stream message where the stream hub says. A joiner
    /// sends everything to the Host, who passes it on.
    pub fn send_stream(&self, out: Outgoing) {
        let (to, but, message) = match out {
            Outgoing::To(to, message) => (Some(to), None, message),
            Outgoing::AllBut(but, message) => (None, Some(but), message),
        };
        let bytes = message.to_bytes();
        if let Some((node, _)) = &self.host {
            for (peer, _) in node.view().roster {
                if to.is_none_or(|to| to == peer.0) && but != Some(peer.0) {
                    node.send(Some(peer), Message::Input(bytes.clone()));
                }
            }
        } else if let Some((node, _)) = &self.join {
            node.send(Message::Input(bytes));
        }
    }

    /// Whether this app can reach the other Players now: the Host always,
    /// a joiner while connected to the Host.
    pub fn online(&self) -> bool {
        match (&self.host, &self.join) {
            (Some(_), _) => true,
            (None, Some((_, coop))) => coop.connected(),
            (None, None) => false,
        }
    }

    /// The next Player in join order, once the Run's Host has been silent
    /// long enough, takes over: their app hosts the lobby on its own
    /// node, lets the rest of the roster in by key, and orders the Run log.
    fn take_over_if_due(&mut self, now_ms: i64) {
        let (Some((_, coop)), Some(me), Ok(key)) = (&self.join, self.public_key(), self.key.clone()) else {
            return;
        };
        if !coop.host_gone(now_ms) || coop.log().successor() != Some(me) {
            return;
        }
        let Some(locked) = coop.log().locked() else {
            return;
        };
        let Some((node, coop)) = self.join.take() else {
            return;
        };
        drop_in_background(node);
        // Everyone still in the Run, the old Host among them: not a Player dropped.
        let active = coop.log().active();
        let roster = locked
            .roster
            .iter()
            .filter(|p| p.key != me && active.contains(&p.key))
            .map(|p| (PeerKey(p.key), p.name.clone()))
            .collect();
        let run = RunAdvert {
            name: locked.run_name.clone(),
            game_name: locked.game_name.clone(),
            game_sha1: locked.game_sha1.clone(),
        };
        let (log, waiting, rom) = coop.into_takeover();
        let lobby = HostLobby::taken_over(run, &self.name, APP_VERSION, roster);
        let coop = HostCoop::take_over(key, log, waiting, rom.unwrap_or_default());
        self.host = Some((HostNode::start(key, lobby, self.relay_choice()), coop));
        self.alone_since = None;
        self.addresses_written = false;
        log::info!(
            "Taking over hosting: the Host has been silent for {} seconds",
            pactlocke_run::HOST_SILENT_MS / 1000
        );
    }

    /// The Run's first Host, with no Player connected for a while, may be
    /// the one who vanished, and been taken over from: they carry on as a
    /// Player, dialling the next Player in join order.
    fn step_down_if_alone(&mut self, now_ms: i64) {
        let (Some((node, coop)), Some(me)) = (&self.host, self.public_key()) else {
            return;
        };
        let log = coop.log();
        let alone = node.view().players.len() <= 1;
        let others = log.active().iter().any(|p| *p != me);
        let next = log.successor();
        // A Player who took over hosting stays Host: their copy of the log
        // has the rightful takeover, so the others come to them.
        let took_over = log.first_host() != me;
        if log.locked().is_none() || !alone || !others || took_over {
            self.alone_since = None;
            return;
        }
        let since = *self.alone_since.get_or_insert(now_ms);
        let (Some(next), Ok(key)) = (next, self.key.clone()) else {
            return;
        };
        if now_ms - since < STEP_DOWN_MS {
            return;
        }
        let Some((node, coop)) = self.host.take() else {
            return;
        };
        drop_in_background(node);
        let (log, rom) = coop.into_log();
        if self.rom_sha1.is_none() {
            self.rom_sha1 = log.locked().map(|l| l.game_sha1);
        }
        let coop = JoinCoop::stepped_down(key, log, next, rom, now_ms);
        let node = self.dial(key, next);
        self.join = Some((node, coop));
        self.dialled_at = now_ms;
        self.redials = 1;
        self.alone_since = None;
        log::info!("No Player is connected: carrying on as a Player, dialling the next Host");
    }

    /// A joiner in a Run that has started, whose connection to the Host was
    /// lost, connects again a few seconds later; their Run carries on. Once
    /// the Host has been silent long enough, they try the next Player in
    /// join order, the Host and the rest of the Run in turn, until one of
    /// them answers as Host.
    fn reconnect_if_lost(&mut self, now_ms: i64) {
        let (Some((node, coop)), Some(me), Ok(key)) = (&self.join, self.public_key(), self.key.clone()) else {
            return;
        };
        let log = coop.log();
        let lost = node.view().error.is_some() && log.locked().is_some();
        let silent = coop.host_gone(now_ms);
        if !lost && !silent {
            self.lost_at = None;
            self.redials = 0;
            return;
        }
        let lost_at = *self.lost_at.get_or_insert(now_ms);
        let due = if silent {
            now_ms - self.dialled_at >= ATTEMPT_MS
        } else {
            now_ms - lost_at >= RECONNECT_MS
        };
        if !due {
            return;
        }
        // Silent: the next Player first, then the Host, then anyone else
        // still in the Run, whoever answers as Host.
        let mut hosts = Vec::new();
        if silent {
            hosts.extend(log.successor());
        }
        hosts.push(log.host());
        if silent {
            hosts.extend(log.active());
        }
        let mut seen = Vec::new();
        hosts.retain(|host| {
            *host != me && !seen.contains(host) && {
                seen.push(*host);
                true
            }
        });
        let Some(&target) = hosts.get(self.redials as usize % hosts.len().max(1)) else {
            return;
        };
        if silent {
            self.redials += 1;
        }
        log::info!("Joiner: connecting to the Host again");
        let fresh = self.dial(key, target);
        self.reconnects += 1;
        if let Some((old, coop)) = self.join.as_mut() {
            coop.dial(target);
            let old = std::mem::replace(old, fresh);
            drop_in_background(old);
        }
        self.dialled_at = now_ms;
        self.lost_at = None;
    }

    /// A network node connecting to the Host with key `host`, as a Player
    /// in the Run's roster: by key, with no invite.
    fn dial(&self, key: [u8; 32], host: PlayerKey) -> JoinNode {
        let lobby = JoinLobby::rejoin(
            PeerKey(host),
            self.relay.clone(),
            &self.name,
            APP_VERSION,
            self.rom_sha1.clone(),
        );
        JoinNode::start_with(key, lobby, self.relay_choice(), self.direct_for(host))
    }

    /// The direct addresses known for the Host with key `host`.
    fn direct_for(&self, host: PlayerKey) -> Vec<std::net::SocketAddr> {
        let first = self
            .join
            .as_ref()
            .is_some_and(|(_, coop)| coop.log().first_host() == host);
        let mut addrs = if first { self.direct.clone() } else { Vec::new() };
        if let Some(dir) = &self.addresses
            && let Ok(text) = std::fs::read_to_string(dir.join(addresses_file(host)))
        {
            addrs.extend(
                text.lines()
                    .filter_map(|line| line.trim().parse::<std::net::SocketAddr>().ok()),
            );
        }
        addrs
    }

    /// While hosting, writes this Host's direct addresses where joiners
    /// look for them, if there is such a place; and keeps the relay the
    /// Host's invite gives, to find a later Host on.
    fn note_host_addresses(&mut self) {
        let Some((node, _)) = &self.host else {
            return;
        };
        if self.relay.is_none()
            && let Some(link) = node.view().invite_link
            && let Ok(invite) = Invite::parse(&link)
        {
            self.relay = invite.relay;
        }
        let (Some(dir), Some(me)) = (&self.addresses, self.public_key()) else {
            return;
        };
        let addrs = node.direct_addrs();
        if self.addresses_written || addrs.is_empty() {
            return;
        }
        let text: Vec<String> = addrs.iter().map(ToString::to_string).collect();
        let file = dir.join(addresses_file(me));
        let temp = file.with_extension("tmp");
        if std::fs::write(&temp, text.join("\n"))
            .and_then(|()| std::fs::rename(&temp, &file))
            .is_ok()
        {
            self.addresses_written = true;
        }
    }

    /// This Player's entries waiting for a Host to put them in the Run log,
    /// oldest first: none while hosting.
    pub fn waiting(&self) -> Vec<pactlocke_run::Submission> {
        self.join.as_ref().map(|(_, coop)| coop.waiting()).unwrap_or_default()
    }

    /// The shared Run log, in a Run with other Players.
    pub fn log(&self) -> Option<&pactlocke_run::RunLog> {
        match (&self.host, &self.join) {
            (Some((_, coop)), _) => Some(coop.log()),
            (None, Some((_, coop))) => Some(coop.log()),
            (None, None) => None,
        }
    }

    /// How this app reaches another Player's: straight or through a relay,
    /// with the ping; a joiner reaches the other joiners through the Host.
    pub fn reach(&self, player: PlayerKey) -> Option<crate::player_strip::Reach> {
        use crate::player_strip::Reach;
        let link = match (&self.host, &self.join) {
            (Some((node, _)), _) => node.links().into_iter().find(|(peer, _)| peer.0 == player)?.1,
            (None, Some((node, coop))) if coop.host() == player => node.link()?,
            (None, Some(_)) => return Some(Reach::ViaHost),
            (None, None) => return None,
        };
        Some(if link.direct {
            Reach::Direct { ping_ms: link.ping_ms }
        } else {
            Reach::Relayed { ping_ms: link.ping_ms }
        })
    }

    pub fn open_join_prompt(&mut self) {
        self.prompt = Some(JoinPrompt::default());
    }

    /// The join prompt, when open, and the lobby's own work. Returns the
    /// Run to start when the countdown after the lock is over.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<CoopStart> {
        self.join_prompt(ctx);
        let start = self.pump(SystemRunClock.now_ms());
        if self.in_lobby() {
            // The view comes from the network threads: look again soon.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        start
    }

    /// The lobby's own work, with or without a window: moves the lobby to
    /// the next Player when the Host has gone silent, reconnects a lost
    /// joiner, reads and sends setup and the Run log, and returns the Run
    /// to start when the countdown after the lock is over.
    pub fn pump(&mut self, now_ms: i64) -> Option<CoopStart> {
        self.take_over_if_due(now_ms);
        self.step_down_if_alone(now_ms);
        self.reconnect_if_lost(now_ms);
        self.note_host_addresses();
        let start = match (&mut self.host, &mut self.join) {
            (Some((node, coop)), _) => coop.pump(node, now_ms),
            (None, Some((node, coop))) => coop.pump(node, now_ms),
            (None, None) => None,
        };
        self.note_changes();
        if start.is_some() {
            log::info!("The Run starts");
        }
        start
    }

    /// Joins the Run `link` invites to, on the ROM at `rom` (its path and
    /// SHA-1) if one is chosen, also trying `direct` addresses for the Host.
    pub fn join(
        &mut self,
        link: &str,
        rom: Option<(PathBuf, String)>,
        direct: Vec<std::net::SocketAddr>,
    ) -> Result<(), String> {
        let sha1 = rom.as_ref().map(|(_, sha1)| sha1.clone());
        let invite = Invite::parse(link).map_err(|e| format!("{e}."))?;
        let lobby = JoinLobby::new(invite, &self.name, APP_VERSION, sha1.clone())?;
        let key = self.key.clone()?;
        let coop = JoinCoop::new(key, lobby.invite().host, rom, SystemRunClock.now_ms());
        let relay = lobby.invite().relay.clone();
        let node = JoinNode::start_with(key, lobby, self.relay_choice(), direct.clone());
        self.leave();
        self.join = Some((node, coop));
        self.rom_sha1 = sha1;
        self.relay = relay;
        self.direct = direct;
        self.dialled_at = SystemRunClock.now_ms();
        log::info!("Joining a lobby");
        Ok(())
    }

    /// The Host's invite link now, and where the Host can be reached directly.
    pub fn invite(&self) -> Option<(String, Vec<std::net::SocketAddr>)> {
        let (node, _) = self.host.as_ref()?;
        Some((node.view().invite_link?, node.direct_addrs()))
    }

    /// The Host makes a fresh invite link for another friend.
    pub fn new_invite(&self) {
        if let Some((node, _)) = &self.host {
            node.new_invite();
        }
    }

    /// The lobby as this app's network sees it now.
    pub fn view(&self) -> Option<LobbyView> {
        match (&self.host, &self.join) {
            (Some((node, _)), _) => Some(node.view()),
            (None, Some((node, _))) => Some(node.view()),
            (None, None) => None,
        }
    }

    /// What has gone wrong with the lobby or the Run log, if anything.
    pub fn error(&self) -> Option<String> {
        let coop = match (&self.host, &self.join) {
            (Some((_, coop)), _) => coop.error(),
            (None, Some((_, coop))) => coop.error(),
            (None, None) => None,
        };
        coop.map(str::to_owned).or_else(|| self.view()?.error)
    }

    /// How many times this joiner has started connecting to a Host again,
    /// after losing the connection or while the Host was silent: 0 for a
    /// Host, or a joiner who never lost the Host.
    pub fn reconnects(&self) -> u32 {
        self.reconnects
    }

    /// The Ruleset's version as this app knows it, once it does.
    pub fn setup_version(&self) -> Option<u64> {
        match (&self.host, &self.join) {
            (Some((_, coop)), _) => coop.setup_version(),
            (None, Some((_, coop))) => coop.setup_version(),
            (None, None) => None,
        }
    }

    /// The Host's Run setup, to change before the lock.
    pub fn host_setup(&mut self) -> Option<&mut HostCoop> {
        self.host.as_mut().map(|(_, coop)| coop)
    }

    /// This Player's Ready, to the Ruleset as it is now.
    pub fn ready(&mut self, ready: bool) {
        match (&mut self.host, &mut self.join) {
            (Some((_, coop)), _) => coop.ready(ready),
            (None, Some((node, coop))) => coop.ready(node, ready),
            (None, None) => {}
        }
    }

    /// The Host locks the Ruleset and roster, if everyone is Ready.
    pub fn lock(&mut self, now_ms: i64) -> Result<(), String> {
        let (node, coop) = self.host.as_mut().ok_or("only the Host locks")?;
        coop.lock_now(node, now_ms)
    }

    fn join_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let (mut join, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("join a run")).show(ctx, |ui| {
            ui.heading("Join a friend's Run");
            ui.label("Paste the invite link your friend sent you.");
            ui.add(
                egui::TextEdit::singleline(&mut prompt.link)
                    .hint_text("pactlocke://join?…")
                    .desired_width(420.0),
            );
            ui.horizontal(|ui| {
                if ui.button("Choose your ROM…").clicked() {
                    match pick_rom() {
                        Some(Ok(rom)) => prompt.rom = Some(rom),
                        Some(Err(e)) => prompt.error = Some(e),
                        None => {}
                    }
                }
                match &prompt.rom {
                    Some(rom) => ui.label(&rom.name),
                    None => ui.weak("none yet: you can pick it in the lobby too"),
                };
            });
            ui.weak(format!(
                "Joining as {}. Settings → General changes your name.",
                self.name
            ));
            if let Some(error) = &prompt.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                join = ui
                    .add_enabled(!prompt.link.trim().is_empty(), egui::Button::new("Join"))
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if join {
            let prompt = self.prompt.take().expect("open");
            let picked = prompt.rom.as_ref().map(|rom| (rom.path.clone(), rom.sha1.clone()));
            if let Err(e) = self.join(&prompt.link, picked, Vec::new()) {
                self.prompt = Some(JoinPrompt {
                    error: Some(e),
                    ..prompt
                });
            }
        } else if cancel || modal.should_close() {
            self.prompt = None;
        }
    }

    /// After the lock: how this app reaches each other Player's.
    fn connections(&self, ui: &mut egui::Ui) {
        let Some(locked) = self.log().and_then(|log| log.locked()) else {
            return;
        };
        let me = self.public_key();
        ui.add_space(6.0);
        ui.strong("Connections");
        for (index, player) in locked.roster.iter().enumerate() {
            if Some(player.key) == me {
                continue;
            }
            ui.horizontal(|ui| {
                crate::theme::dot(ui, index);
                ui.label(&player.name);
                ui.weak(crate::player_strip::reach_text(self.reach(player.key), false));
            });
        }
    }

    /// The Lobby tab: before the lock the invite, the Players, the Ruleset,
    /// Ready and the lock; during the Run the tallies and connections.
    pub fn lobby_tab(&mut self, ui: &mut egui::Ui, now_ms: i64) {
        let (view, hosting) = match (&self.host, &self.join) {
            (Some((host, _)), _) => (host.view(), true),
            (None, Some((join, _))) => (join.view(), false),
            (None, None) => {
                ui.weak("Not in a Lobby: the Run's network has been left.");
                return;
            }
        };
        let locked = self.log().is_some_and(|log| log.locked().is_some());
        let mut new_invite = false;
        let mut picked = None;
        let mut leave = false;
        if let Some(run) = &view.run {
            ui.strong(&run.name);
            ui.weak(&run.game_name);
        }
        if let Some(error) = &view.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if hosting {
            invite_section(ui, &view, &mut new_invite);
        } else if view.error.is_none() && view.players.is_empty() {
            ui.label("Connecting to the Host…");
        }
        ui.separator();
        players_section(ui, &view);
        if !hosting && view.rom_matches != Some(true) && view.run.is_some() && ui.button("Choose your ROM…").clicked()
        {
            picked = pick_rom().and_then(Result::ok);
        }
        ui.separator();
        match (&mut self.host, &mut self.join) {
            (Some((node, coop)), _) => coop.show(ui, node, now_ms),
            (None, Some((node, coop))) => coop.show(ui, node, now_ms),
            (None, None) => {}
        }
        if locked {
            self.connections(ui);
        } else {
            ui.separator();
            leave = ui.button("Leave the lobby").clicked();
        }
        if new_invite && let Some((host, _)) = &self.host {
            host.new_invite();
        }
        if let (Some(rom), Some((join, coop))) = (picked, &mut self.join) {
            join.rom_picked(&rom.sha1);
            self.rom_sha1 = Some(rom.sha1.clone());
            coop.rom_picked(rom.path, rom.sha1);
        }
        if leave {
            log::info!("Left the lobby");
            self.leave();
        }
    }
}

fn invite_section(ui: &mut egui::Ui, view: &LobbyView, new_invite: &mut bool) {
    match &view.invite_link {
        Some(link) => {
            ui.label("Send this link to one friend. Each link lets one Player in, once.");
            // Read-only, but selectable, so Ctrl+C works as well as Copy.
            ui.add(egui::TextEdit::singleline(&mut link.as_str()).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(link.clone());
                }
                if ui.button("New link for another friend").clicked() {
                    *new_invite = true;
                }
            });
        }
        None if view.error.is_none() => {
            ui.label("Starting the network…");
        }
        None => {}
    }
}

fn players_section(ui: &mut egui::Ui, view: &LobbyView) {
    ui.label(format!("Players · {}", view.players.len()));
    for (index, player) in view.players.iter().enumerate() {
        ui.horizontal(|ui| {
            crate::theme::dot(ui, index);
            ui.label(&player.name);
            if player.host {
                ui.weak("Host");
            }
            match player.rom {
                RomStatus::Matches => ui.label("ROM matches"),
                RomStatus::Wrong => ui.colored_label(ui.visuals().error_fg_color, "wrong ROM: not the Run's game"),
                RomStatus::NotYet => ui.weak("no ROM yet"),
            };
        });
    }
}

/// A ROM the Player picked: its file name and SHA-1.
struct PickedRom {
    name: String,
    path: PathBuf,
    sha1: String,
}

/// Asks for the ROM to play the Run on; `None` if the Player cancels.
fn pick_rom() -> Option<Result<PickedRom, String>> {
    let path = rfd::FileDialog::new()
        .set_title("Choose the ROM to play this Run on")
        .add_filter("GBA ROM", &["gba", "agb", "bin"])
        .pick_file()?;
    Some(
        std::fs::read(&path)
            .map(|rom| PickedRom {
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path: path.clone(),
                sha1: sha1_hex(&rom),
            })
            .map_err(|e| format!("Couldn't read {}: {e}", path.display())),
    )
}

/// Drops lobbies on their own thread: a network shuts down there, so the
/// window never waits for it.
fn drop_in_background(gone: impl Send + 'static) {
    std::thread::spawn(move || drop(gone));
}

/// The file a Host's direct addresses are written to, in `Lobbies::addresses`.
fn addresses_file(host: PlayerKey) -> String {
    format!("{}.addrs", pactlocke_net::PeerKey(host))
}

#[cfg(test)]
mod tests {
    //! Over real iroh endpoints on this machine: three Players'
    //! lobbies start a Run, the Host's app goes silent, and the lobby moves
    //! to the next Player in join order.

    use std::time::{Duration, Instant};

    use pactlocke_game::CapabilityReport;
    use pactlocke_run::{DeathCause, LogBody};

    use super::*;

    const FIRERED: &str = "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc";

    fn died(pid: u32) -> RunEvent {
        RunEvent::Died {
            pid,
            species: 16,
            species_name: "PIDGEY".into(),
            name: "PIDGEY".into(),
            area: None,
            cause: DeathCause::Poison,
        }
    }

    fn app(dir: &Path, name: &str) -> Lobbies {
        let mut lobbies = Lobbies::new(&dir.join(format!("{name}.key")));
        lobbies.name = name.into();
        lobbies.relays = Relays::Off;
        lobbies.addresses = Some(dir.to_owned());
        lobbies
    }

    /// Pumps every app but the `frozen` ones until `done`, for up to `secs`.
    #[track_caller]
    fn until(apps: &mut [Lobbies], frozen: &[usize], secs: u64, what: &str, done: impl Fn(&[Lobbies]) -> bool) {
        let start = Instant::now();
        while !done(apps) {
            assert!(start.elapsed() < Duration::from_secs(secs), "{what}: {}", state(apps));
            for (i, app) in apps.iter_mut().enumerate() {
                if !frozen.contains(&i) {
                    app.pump(SystemRunClock.now_ms());
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn state(apps: &[Lobbies]) -> String {
        apps.iter()
            .map(|a| {
                let log = a.log();
                format!(
                    "{}: {} entries {:?}, deaths {:?}, online {}, error {:?}",
                    a.name,
                    if a.host.is_some() { "hosts" } else { "joins" },
                    log.map(|l| l.entries().len()),
                    log.map(deaths),
                    a.online(),
                    a.error()
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn deaths(log: &pactlocke_run::RunLog) -> Vec<u32> {
        log.bodies()
            .filter_map(|(_, body)| match body {
                LogBody::Event(RunEvent::Died { pid, .. }) => Some(pid),
                _ => None,
            })
            .collect()
    }

    fn same_log(apps: &[Lobbies]) -> bool {
        let logs: Vec<_> = apps
            .iter()
            .filter_map(|a| a.log().map(|l| l.entries().to_vec()))
            .collect();
        logs.len() == apps.len() && logs.windows(2).all(|w| w[0] == w[1])
    }

    /// The first app hosts, the others join by invite in turn, everyone is
    /// Ready, and the Host locks: the Run starts for everyone.
    fn start_a_run(apps: &mut [Lobbies], rom_file: &Path) {
        // Red hosts; the others join by invite, one after the other.
        apps[0].host(
            RunAdvert {
                name: "Kanto with friends".into(),
                game_name: "Pokémon FireRed (USA) 1.0".into(),
                game_sha1: FIRERED.into(),
            },
            HostRom {
                rom: vec![1, 2, 3],
                path: "red.gba".into(),
                capabilities: CapabilityReport::everything_enforceable(),
                level_caps: Vec::new(),
                sync_gates: Vec::new(),
            },
        );
        for joiner in 1..apps.len() {
            let players = joiner + 1;
            if joiner > 1 {
                apps[0].new_invite();
            }
            let used = apps[0].invite().map(|(link, _)| link);
            until(apps, &[], 30, "an invite", |apps| {
                apps[0]
                    .invite()
                    .is_some_and(|(link, addrs)| Some(&link) != used.as_ref() || joiner == 1 && !addrs.is_empty())
            });
            let (link, addrs) = apps[0].invite().unwrap();
            apps[joiner]
                .join(&link, Some((rom_file.to_owned(), FIRERED.into())), addrs)
                .unwrap();
            until(apps, &[], 30, "the joiner in setup", |apps| {
                apps[0].setup_version().is_some() && apps[0].host.as_ref().unwrap().1.players() == players
            });
        }

        // Everyone Ready to the Ruleset as it stands; the Host locks; the Run starts.
        until(apps, &[], 30, "everyone sees the Ruleset", |apps| {
            apps.iter().all(|a| a.setup_version() == apps[0].setup_version())
        });
        for app in apps.iter_mut() {
            app.ready(true);
        }
        until(apps, &[], 30, "everyone Ready", |apps| {
            apps[0].host.as_ref().unwrap().1.not_ready().is_empty()
        });
        apps[0].lock(SystemRunClock.now_ms()).unwrap();
        let mut started = vec![false; apps.len()];
        let start = Instant::now();
        while started.iter().any(|s| !s) {
            assert!(start.elapsed() < Duration::from_secs(30), "the Run starts for everyone");
            for (i, app) in apps.iter_mut().enumerate() {
                started[i] |= app.pump(SystemRunClock.now_ms()).is_some();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn when_the_host_goes_silent_the_lobby_moves_to_the_next_player_and_the_old_host_joins_as_a_player() {
        let dir = std::env::temp_dir().join(format!("pactlocke-takeover-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rom_file = dir.join("game.gba");
        std::fs::write(&rom_file, [4, 5, 6]).unwrap();
        let mut apps = vec![app(&dir, "Red"), app(&dir, "Blue"), app(&dir, "Green")];
        let [red, blue, green] = [0, 1, 2].map(|i| apps[i].public_key().unwrap());

        start_a_run(&mut apps, &rom_file);
        apps[2].share(vec![died(0xC1)]);
        until(&mut apps, &[], 30, "one log", |apps| {
            same_log(apps) && deaths(apps[0].log().unwrap()) == [0xC1]
        });

        // Red's app goes silent. Green dies meanwhile: it waits for a Host.
        apps[2].share(vec![died(0xC2)]);
        until(&mut apps, &[0], 60, "Blue takes over and Green follows", |apps| {
            apps[1].host.is_some() && same_log(&apps[1..]) && deaths(apps[2].log().unwrap()) == [0xC1, 0xC2]
        });
        let log = apps[2].log().unwrap();
        assert_eq!(log.host(), blue);
        assert_eq!(apps[2].host_key(), Some(blue), "Green's streams go through Blue");
        assert!(log.bodies().any(|(by, body)| by == blue && body == LogBody::Takeover));

        // Red, still silent to the others, dies too; then their app is back.
        apps[0].share(vec![died(0xA1)]);
        until(
            &mut apps,
            &[],
            60,
            "Red joins Blue as a Player, and Red's death is in",
            |apps| apps[0].join.is_some() && same_log(apps) && deaths(apps[1].log().unwrap()).contains(&0xA1),
        );
        assert!(apps[0].host.is_none(), "one Host");
        assert_eq!(apps[0].host_key(), Some(blue));
        let tallies = apps[0].log().unwrap().tallies();
        assert_eq!(tallies.deaths.get(&red), Some(&1));
        assert_eq!(tallies.deaths.get(&green), Some(&2));
        for (i, app) in apps.iter().enumerate() {
            assert_eq!(app.error(), None, "{i}");
        }
        drop(apps);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Players leave a Run with friends by closing their app,
    /// and come back with Resume from their copy of the Run log alone.
    #[test]
    fn players_come_back_with_resume_and_whoever_is_back_first_keeps_the_run_going() {
        let dir = std::env::temp_dir().join(format!("pactlocke-resume-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rom_file = dir.join("game.gba");
        std::fs::write(&rom_file, [4, 5, 6]).unwrap();
        let mut apps = vec![app(&dir, "Red"), app(&dir, "Blue")];
        let [red, blue] = [0, 1].map(|i| apps[i].public_key().unwrap());
        start_a_run(&mut apps, &rom_file);
        apps[1].share(vec![died(0xB1)]);
        until(&mut apps, &[], 30, "one log", |apps| {
            same_log(apps) && deaths(apps[0].log().unwrap()) == [0xB1]
        });

        // Blue closes the app, and opens the Run again: no invite, the Host by key.
        let kept = apps[1].log().unwrap().clone();
        apps[1] = app(&dir, "Blue");
        apps[1].resume(kept, Vec::new(), rom_file.clone()).unwrap();
        apps[1].share(vec![died(0xB2)]);
        until(&mut apps, &[], 30, "Blue back with Red", |apps| {
            apps[1].online() && same_log(apps) && deaths(apps[0].log().unwrap()) == [0xB1, 0xB2]
        });
        assert_eq!(apps[1].host_key(), Some(red));

        // Red, the Host, closes the app; Blue dies meanwhile, and closes the
        // app too before any Host has the death. Red is back first, then
        // Blue, whose death waited in their Run file.
        let red_kept = apps[0].log().unwrap().clone();
        apps[0] = app(&dir, "Red");
        apps[1].share(vec![died(0xB3)]);
        until(&mut apps, &[0], 30, "Blue's death waits", |apps| {
            apps[1].waiting().len() == 1
        });
        let (blue_kept, blue_waiting) = (apps[1].log().unwrap().clone(), apps[1].waiting());
        apps[1] = app(&dir, "Blue");
        apps[0].resume(red_kept, Vec::new(), rom_file.clone()).unwrap();
        until(&mut apps, &[1], 30, "Red hosts again", |apps| apps[0].addresses_written);
        apps[1].resume(blue_kept, blue_waiting, rom_file.clone()).unwrap();
        until(&mut apps, &[], 30, "both back, Blue's death in", |apps| {
            same_log(apps) && deaths(apps[0].log().unwrap()) == [0xB1, 0xB2, 0xB3] && apps[1].waiting().is_empty()
        });
        assert_eq!(apps[1].host_key(), Some(red));

        // Red closes the app again. Blue, back first, keeps the Run going.
        let red_kept = apps[0].log().unwrap().clone();
        let blue_kept = apps[1].log().unwrap().clone();
        apps[0] = app(&dir, "Red");
        apps[1] = app(&dir, "Blue");
        apps[1].resume(blue_kept, Vec::new(), rom_file.clone()).unwrap();
        apps[1].share(vec![died(0xB4)]);
        until(&mut apps, &[0], 60, "Blue takes over", |apps| {
            apps[1].host.is_some() && deaths(apps[1].log().unwrap()) == [0xB1, 0xB2, 0xB3, 0xB4]
        });

        // Red comes back, and joins Blue as a Player; Red's death is in.
        apps[0].resume(red_kept, Vec::new(), rom_file.clone()).unwrap();
        apps[0].share(vec![died(0xA1)]);
        until(&mut apps, &[], 90, "Red joins Blue as a Player", |apps| {
            apps[0].join.is_some() && same_log(apps) && deaths(apps[1].log().unwrap()).contains(&0xA1)
        });
        assert_eq!(apps[0].host_key(), Some(blue));
        assert_eq!(deaths(apps[0].log().unwrap()), [0xB1, 0xB2, 0xB3, 0xB4, 0xA1]);
        for (i, app) in apps.iter().enumerate() {
            assert_eq!(app.error(), None, "{i}");
        }
        drop(apps);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
