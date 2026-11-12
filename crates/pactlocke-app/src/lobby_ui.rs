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
}

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
        }
    }

    fn relay_choice(&self) -> Relays {
        Relays::parse(&self.relays)
    }

    /// Leaves the lobby this app is in. The network shuts down on its own
    /// thread, so the window never waits for it.
    fn leave(&mut self) {
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

    pub fn open_join_prompt(&mut self) {
        self.prompt = Some(JoinPrompt::default());
    }

    /// The join prompt and the lobby window, when open. Returns the Run to
    /// start when the countdown after the lock is over.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<CoopStart> {
        self.join_prompt(ctx);
        let now_ms = SystemRunClock.now_ms();
        let start = match (&mut self.host, &mut self.join) {
            (Some((node, coop)), _) => coop.pump(node, now_ms),
            (None, Some((node, coop))) => coop.pump(node, now_ms),
            (None, None) => None,
        };
        self.lobby_window(ctx, now_ms);
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
                .and_then(|invite| JoinLobby::new(invite, &name, APP_VERSION, rom))
                .and_then(|lobby| {
                    let key = key?;
                    let coop = JoinCoop::new(key, lobby.invite().host, picked);
                    Ok((JoinNode::start(key, lobby, relays), coop))
                });
            match started {
                Ok(node) => {
                    self.leave();
                    self.join = Some(node);
                    self.prompt = None;
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
        let mut open = true;
        let mut new_invite = false;
        let mut picked = None;
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
            });
        if new_invite && let Some((host, _)) = &self.host {
            host.new_invite();
        }
        if let (Some(rom), Some((join, coop))) = (picked, &mut self.join) {
            join.rom_picked(&rom.sha1);
            coop.rom_picked(rom.path, rom.sha1);
        }
        if !open {
            self.leave();
        }
        // The view comes from the network threads: look again soon.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn invite_section(ui: &mut egui::Ui, view: &LobbyView, new_invite: &mut bool) {
    match &view.invite_link {
        Some(link) => {
            ui.label("Send this link to one friend. Each link lets one Player in, once.");
            let mut shown = link.clone();
            ui.add(egui::TextEdit::singleline(&mut shown).desired_width(420.0));
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
