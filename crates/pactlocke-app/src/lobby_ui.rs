//! Inviting friends and joining a friend's Run: the join prompt and the
//! lobby window, where the Host sets the Ruleset, everyone says Ready, and
//! the lock starts every Player's Run together.

use std::path::{Path, PathBuf};

use eframe::egui;
use pactlocke_game::sha1_hex;
use pactlocke_run::{Outgoing, PlayerKey, RunClock, RunEvent, SystemRunClock};

use crate::coop::{CoopStart, HostCoop, HostRom, JoinCoop};
use pactlocke_net::{
    APP_VERSION, HostLobby, HostNode, Invite, JoinLobby, JoinNode, LobbyView, Message, Relays, RomStatus, RunAdvert,
    load_or_create_key,
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
    /// The name other Players see.
    pub name: String,
    /// The relays to use: blank for n0's public relays, or URLs.
    pub relays: String,
    host: Option<(HostNode, HostCoop)>,
    join: Option<(JoinNode, JoinCoop)>,
    prompt: Option<JoinPrompt>,
    /// How a joiner reconnects to the Host after losing the connection
    /// mid-Run: the invite and the ROM's SHA-1.
    rejoin: Option<(Invite, Option<String>)>,
    /// When this joiner's connection to the Host was lost, if it is.
    lost_at: Option<i64>,
    /// Whether the Lobby window is showing. Closing it only hides it.
    shown: bool,
    /// The lobby as last written to the network log, to note each change once.
    noted: String,
}

/// How long a joiner waits before reconnecting to the Host.
const RECONNECT_MS: i64 = 3_000;

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
            relays: String::new(),
            host: None,
            join: None,
            prompt: None,
            rejoin: None,
            lost_at: None,
            shown: false,
            noted: String::new(),
        }
    }

    /// Whether this app is in a lobby, as Host or joiner.
    pub fn in_lobby(&self) -> bool {
        self.host.is_some() || self.join.is_some()
    }

    /// Whether the Lobby window is showing.
    pub fn lobby_shown(&self) -> bool {
        self.shown
    }

    /// Shows the Lobby window again after it was closed.
    pub fn show_lobby(&mut self) {
        self.shown = true;
    }

    /// Writes the lobby to the network log when what this app sees of it changes.
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
            crate::netlog::note(&now);
            self.noted = now;
        }
    }

    fn relay_choice(&self) -> Relays {
        Relays::parse(&self.relays)
    }

    /// Leaves the lobby this app is in. The network shuts down on its own
    /// thread, so the window never waits for it.
    fn leave(&mut self) {
        self.shown = false;
        self.rejoin = None;
        let (host, join) = (self.host.take(), self.join.take());
        if host.is_some() || join.is_some() {
            std::thread::spawn(move || drop((host, join)));
        }
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
        self.shown = true;
        crate::netlog::note("Hosting a new lobby");
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

    /// A joiner in a Run that has started, whose connection to the Host was
    /// lost, connects again a few seconds later; their Run carries on.
    fn reconnect_if_lost(&mut self, now_ms: i64) {
        let Some((node, coop)) = &self.join else {
            return;
        };
        let lost = node.view().error.is_some() && coop.log().locked().is_some();
        if !lost {
            self.lost_at = None;
            return;
        }
        let lost_at = *self.lost_at.get_or_insert(now_ms);
        let (Some((invite, sha1)), Ok(key)) = (self.rejoin.clone(), self.key.clone()) else {
            return;
        };
        if now_ms - lost_at < RECONNECT_MS {
            return;
        }
        crate::netlog::note("Joiner: reconnecting to the Host");
        if let Ok(lobby) = JoinLobby::new(invite, &self.name, APP_VERSION, sha1) {
            let fresh = JoinNode::start(key, lobby, self.relay_choice());
            if let Some((old, _)) = self.join.as_mut() {
                let old = std::mem::replace(old, fresh);
                // The old network shuts down on its own thread.
                std::thread::spawn(move || drop(old));
            }
            self.lost_at = None;
        }
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

    /// The join prompt and the lobby window, when open. Returns the Run to
    /// start when the countdown after the lock is over.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<CoopStart> {
        self.join_prompt(ctx);
        let now_ms = SystemRunClock.now_ms();
        self.reconnect_if_lost(now_ms);
        let start = match (&mut self.host, &mut self.join) {
            (Some((node, coop)), _) => coop.pump(node, now_ms),
            (None, Some((node, coop))) => coop.pump(node, now_ms),
            (None, None) => None,
        };
        self.lobby_window(ctx, now_ms);
        self.note_changes();
        if start.is_some() {
            crate::netlog::note("The Run starts");
        }
        start
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
                ui.label("Your name");
                ui.text_edit_singleline(&mut self.name);
            });
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
            ui.collapsing("Relays", |ui| {
                ui.label("Blank uses n0's public relays. Or list your own (a self-hosted iroh-relay), one per line.");
                ui.text_edit_multiline(&mut self.relays);
            });
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
            let (relays, key, name) = (self.relay_choice(), self.key.clone(), self.name.clone());
            let prompt = self.prompt.as_mut().expect("open");
            let rom = prompt.rom.as_ref().map(|rom| rom.sha1.clone());
            let picked = prompt.rom.as_ref().map(|rom| (rom.path.clone(), rom.sha1.clone()));
            let started = Invite::parse(&prompt.link)
                .map_err(|e| format!("{e}."))
                .and_then(|invite| JoinLobby::new(invite, &name, APP_VERSION, rom.clone()))
                .and_then(|lobby| {
                    let key = key?;
                    let coop = JoinCoop::new(key, lobby.invite().host, picked);
                    let rejoin = (lobby.invite().clone(), rom.clone());
                    Ok((JoinNode::start(key, lobby, relays), coop, rejoin))
                });
            match started {
                Ok((node, coop, rejoin)) => {
                    self.leave();
                    self.join = Some((node, coop));
                    self.rejoin = Some(rejoin);
                    self.prompt = None;
                    self.shown = true;
                    crate::netlog::note("Joining a lobby");
                }
                Err(e) => prompt.error = Some(e),
            }
        } else if cancel || modal.should_close() {
            self.prompt = None;
        }
    }

    fn lobby_window(&mut self, ctx: &egui::Context, now_ms: i64) {
        let (view, hosting) = match (&self.host, &self.join) {
            (Some((host, _)), _) => (host.view(), true),
            (None, Some((join, _))) => (join.view(), false),
            (None, None) => return,
        };
        if !self.shown {
            return;
        }
        let locked = self.log().is_some_and(|log| log.locked().is_some());
        let mut open = true;
        let mut new_invite = false;
        let mut picked = None;
        let mut leave = false;
        egui::Window::new("Lobby")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
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
                if !hosting
                    && view.rom_matches != Some(true)
                    && view.run.is_some()
                    && ui.button("Choose your ROM…").clicked()
                {
                    picked = pick_rom().and_then(Result::ok);
                }
                ui.separator();
                match (&mut self.host, &mut self.join) {
                    (Some((node, coop)), _) => coop.show(ui, node, now_ms),
                    (None, Some((node, coop))) => coop.show(ui, node, now_ms),
                    (None, None) => {}
                }
                ui.separator();
                if locked {
                    ui.weak("Closing this window only hides it: the Lobby button brings it back.");
                } else {
                    leave = ui.button("Leave the lobby").clicked();
                }
            });
        if new_invite && let Some((host, _)) = &self.host {
            host.new_invite();
        }
        if let (Some(rom), Some((join, coop))) = (picked, &mut self.join) {
            join.rom_picked(&rom.sha1);
            coop.rom_picked(rom.path, rom.sha1);
        }
        if leave {
            crate::netlog::note("Left the lobby");
            self.leave();
        } else if !open {
            self.shown = false;
        }
        // The view comes from the network threads: look again soon.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn invite_section(ui: &mut egui::Ui, view: &LobbyView, new_invite: &mut bool) {
    match &view.invite_link {
        Some(link) => {
            ui.label("Send this link to one friend. Each link lets one Player in, once.");
            // Read-only, but selectable, so Ctrl+C works as well as Copy.
            ui.add(egui::TextEdit::singleline(&mut link.as_str()).desired_width(420.0));
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
    for player in &view.players {
        ui.horizontal(|ui| {
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
