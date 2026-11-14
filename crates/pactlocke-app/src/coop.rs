//! Starting a Run together: the Host's Run setup (a preset, the rules, every
//! Player's Ready, the lock) and the Run log every Player shares. The lobby's
//! network carries both; this side reads and writes them once a UI frame.

use std::path::PathBuf;

use eframe::egui;
use pactlocke_game::{CapabilityReport, LevelCap, MAX_LEVEL, Rule};
use pactlocke_net::{APP_VERSION, HostNode, JoinNode, Message, PeerKey, RunAdvert, new_key};
use pactlocke_run::{
    Locked, LogBody, LogEntry, LogError, PlayerKey, Presence, Preset, Ruleset, RunEvent, RunLog, RunSetup, SetupCheck,
    SetupMessage, SetupPlayer, SetupView, Signer, Submission,
};

/// How often the Host sends setup again, for a Player who reconnected.
const RESEND_MS: i64 = 2_000;

/// How long a Player's entry may wait for the Host to order it before the
/// Player is told the Host is leaving it out.
const UNORDERED_MS: i64 = 10_000;

/// The Host's ROM for the Run, and what its Game Profile can enforce.
pub struct HostRom {
    pub rom: Vec<u8>,
    pub path: PathBuf,
    pub capabilities: CapabilityReport,
    pub level_caps: Vec<LevelCap>,
    /// The battles its Game Profile can make Sync Gates.
    pub sync_gates: Vec<String>,
}

/// The countdown is over: the Run to start, on this ROM.
pub struct CoopStart {
    pub locked: Locked,
    pub rom: Vec<u8>,
    pub path: PathBuf,
}

/// What this app brings, for the lock to compare.
fn own_check(game_sha1: Option<String>) -> SetupCheck {
    SetupCheck {
        app_build: APP_VERSION.to_owned(),
        game_sha1,
        hack_options: Vec::new(),
        randomizer: None,
    }
}

/// The rules the Host chooses, with their checkbox text.
const RULES: [(Rule, &str); 10] = [
    (Rule::FirstEncounter, "First Encounter"),
    (Rule::DupesClause, "Dupes clause"),
    (Rule::Statics, "Statics"),
    (Rule::Gifts, "Gifts"),
    (Rule::FaintIsDead, "Faint = Dead"),
    (Rule::Links, "Links"),
    (Rule::NoItems, "No items in battle"),
    (Rule::SetMode, "Set mode"),
    (Rule::MustNickname, "Must nickname"),
    (Rule::TypeClause, "Type clause"),
];

fn rule_on(ruleset: &mut Ruleset, rule: Rule) -> Option<&mut bool> {
    Some(match rule {
        Rule::FirstEncounter => &mut ruleset.first_encounter,
        Rule::DupesClause => &mut ruleset.dupes_clause,
        Rule::Statics => &mut ruleset.statics,
        Rule::Gifts => &mut ruleset.gifts,
        Rule::FaintIsDead => &mut ruleset.faint_is_dead,
        Rule::Links => &mut ruleset.links,
        Rule::NoItems => &mut ruleset.no_items,
        Rule::SetMode => &mut ruleset.set_mode,
        Rule::MustNickname => &mut ruleset.must_nickname,
        Rule::TypeClause => &mut ruleset.type_clause,
        _ => return None,
    })
}

/// Why a Host who took over after the lock has no setup to change.
const TAKEN_OVER: &str = "The Run has started: its Ruleset is locked.";

/// The Host's side: setup, and the log they order.
pub struct HostCoop {
    /// Run setup; none for a Player who took over hosting after the lock.
    setup: Option<RunSetup>,
    log: RunLog,
    signer: Signer,
    rom: Vec<u8>,
    path: PathBuf,
    preset: Preset,
    /// The ROM's own level caps, for when the Host turns caps on.
    level_caps: Vec<LevelCap>,
    /// The setup last sent to everyone, and when.
    sent: Option<(SetupView, i64)>,
    /// When the Host last let everyone hear from them, with no setup to send.
    heartbeat_ms: i64,
    started: bool,
    error: Option<String>,
    /// Input stream messages from Players, for Spectate: from whom, and the bytes.
    stream_inbox: Vec<(PlayerKey, Vec<u8>)>,
}

impl HostCoop {
    pub fn new(key: [u8; 32], name: &str, run: &RunAdvert, rom: HostRom) -> Self {
        let signer = Signer::from_secret(key);
        let preset = Preset::ClassicNuzlocke;
        let mut setup = RunSetup::new(
            SetupPlayer {
                key: signer.public(),
                name: name.to_owned(),
                check: own_check(Some(run.game_sha1.clone())),
                ready_for: None,
            },
            &run.name,
            &run.game_name,
            &run.game_sha1,
            rom.capabilities,
            rom.level_caps.clone(),
            preset,
        );
        setup.set_sync_gates(rom.sync_gates);
        let mut coop = Self {
            log: RunLog::new(signer.public()),
            setup: Some(setup),
            signer,
            rom: rom.rom,
            path: rom.path,
            preset,
            level_caps: rom.level_caps,
            sent: None,
            heartbeat_ms: 0,
            started: false,
            error: None,
            stream_inbox: Vec::new(),
        };
        let mut nonce = [0; 16];
        nonce.copy_from_slice(&new_key()[..16]);
        coop.submit(
            None,
            &LogBody::Genesis {
                nonce,
                run_name: run.name.clone(),
                game_name: run.game_name.clone(),
                game_sha1: run.game_sha1.clone(),
            },
        );
        coop
    }

    /// The Player with `key`, next in join order, takes over ordering the
    /// Run `log` from a Host gone silent, then orders the entries they had
    /// waiting for the old Host, signed again after the takeover.
    pub fn take_over(key: [u8; 32], log: RunLog, waiting: Vec<LogBody>, rom_path: PathBuf) -> Self {
        let mut coop = Self {
            setup: None,
            log,
            signer: Signer::from_secret(key),
            rom: Vec::new(),
            path: rom_path,
            preset: Preset::ClassicNuzlocke,
            level_caps: Vec::new(),
            sent: None,
            heartbeat_ms: 0,
            started: true,
            error: None,
            stream_inbox: Vec::new(),
        };
        coop.submit(None, &LogBody::Takeover);
        for body in waiting {
            coop.submit(None, &body);
        }
        coop
    }

    /// The Host hands over to a Player who took over hosting: the Run log
    /// as it is here, and where the Host's ROM is.
    pub fn into_log(self) -> (RunLog, PathBuf) {
        (self.log, self.path)
    }

    fn me(&self) -> [u8; 32] {
        self.signer.public()
    }

    /// The Host's own entry: signed, ordered, and sent to everyone.
    fn submit(&mut self, node: Option<&HostNode>, body: &LogBody) {
        let counter = self.log.next_counter(&self.me());
        let submission = self.log.sign(&self.signer, counter, body);
        self.order(node, submission);
    }

    fn order(&mut self, node: Option<&HostNode>, submission: Submission) {
        match self.log.order(&self.signer, submission) {
            Ok(entry) => {
                if let Some(node) = node {
                    node.send(None, Message::Log(entry.to_bytes()));
                }
            }
            // Players send their waiting entries again whenever they connect,
            // before they have the log: a repeat, or one ahead of a gap they
            // fill once they have it, is no harm.
            Err(e @ (LogError::Replayed { .. } | LogError::Skipped { .. })) => {
                log::debug!("A Run log entry sent again was refused: {e}");
            }
            Err(e) => self.error = Some(format!("A Run log entry was refused: {e}")),
        }
    }

    /// Reads what Players sent, keeps setup in step with the lobby's
    /// roster, and sends setup on when it changed. Returns the Run to start
    /// once the countdown is over.
    pub fn pump(&mut self, node: &HostNode, now_ms: i64) -> Option<CoopStart> {
        let roster = node.view().roster;
        if let Some(setup) = &mut self.setup {
            let gone: Vec<[u8; 32]> = setup.view().players[1..]
                .iter()
                .map(|p| p.key)
                .filter(|key| !roster.iter().any(|(peer, _)| peer.0 == *key))
                .collect();
            for key in gone {
                setup.leave(&key);
            }
        }
        for (from, message) in node.take_inbox() {
            match message {
                Message::Setup(bytes) => {
                    let Some(message) = SetupMessage::from_bytes(&bytes) else {
                        continue;
                    };
                    if let SetupMessage::Check(check) = &message {
                        self.player_checks_in(node, from, &roster, check);
                    }
                    if let Some(setup) = &mut self.setup {
                        setup.receive(&from.0, message);
                    }
                }
                Message::Log(bytes) => {
                    if let Some(submission) = Submission::from_bytes(&bytes) {
                        self.order(Some(node), submission);
                    }
                }
                Message::Input(bytes) => self.stream_inbox.push((from.0, bytes)),
                _ => {}
            }
        }
        if let Some(call) = self.log.retry_due() {
            self.submit(Some(node), &LogBody::Retry { call });
        }
        if let Some(player) = self.log.dropout_due() {
            self.submit(Some(node), &LogBody::Dropout { player });
        }
        match self.setup.as_ref().map(RunSetup::view) {
            Some(view) => {
                let stale = self
                    .sent
                    .as_ref()
                    .is_none_or(|(sent, at)| *sent != view || now_ms - at >= RESEND_MS);
                if stale {
                    node.send(
                        None,
                        Message::Setup(SetupMessage::State(Box::new(view.clone())).to_bytes()),
                    );
                    self.sent = Some((view, now_ms));
                }
            }
            // With no setup to send, the newest entry again, so everyone
            // keeps hearing from the Host even while no game plays.
            None => {
                if now_ms - self.heartbeat_ms >= RESEND_MS
                    && let Some(last) = self.log.entries().last()
                {
                    node.send(None, Message::Log(last.to_bytes()));
                    self.heartbeat_ms = now_ms;
                }
            }
        }
        self.start_when_counted_down(now_ms)
    }

    /// A Player says what they bring: on their first word (or after a
    /// reconnect) they join setup and get the whole log so far.
    fn player_checks_in(&mut self, node: &HostNode, from: PeerKey, roster: &[(PeerKey, String)], check: &SetupCheck) {
        if let Some(setup) = &mut self.setup {
            let known = setup.view().players.iter().any(|p| p.key == from.0);
            if !known {
                let name = roster
                    .iter()
                    .find(|(peer, _)| *peer == from)
                    .map_or_else(|| "Player".to_owned(), |(_, name)| name.clone());
                let joined = setup.join(SetupPlayer {
                    key: from.0,
                    name,
                    check: check.clone(),
                    ready_for: None,
                });
                if joined.is_err() {
                    return;
                }
            }
        }
        for entry in self.log.entries() {
            node.send(Some(from), Message::Log(entry.to_bytes()));
        }
    }

    fn start_when_counted_down(&mut self, now_ms: i64) -> Option<CoopStart> {
        let locked = self.log.locked()?;
        if self.started || now_ms < locked.start_ms {
            return None;
        }
        self.started = true;
        Some(CoopStart {
            locked,
            rom: std::mem::take(&mut self.rom),
            path: self.path.clone(),
        })
    }

    /// Input stream messages since the last call: from whom, and the bytes.
    pub fn take_stream(&mut self) -> Vec<(PlayerKey, Vec<u8>)> {
        std::mem::take(&mut self.stream_inbox)
    }

    /// The Run log as the Host has it.
    pub fn log(&self) -> &RunLog {
        &self.log
    }

    /// What went wrong with the shared Run log, if anything has.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The Host's own Run state, into the log.
    pub fn share(&mut self, node: &HostNode, events: Vec<RunEvent>) {
        for event in events {
            self.submit(Some(node), &LogBody::Event(event));
        }
    }

    /// The setup section of the lobby window.
    pub fn show(&mut self, ui: &mut egui::Ui, node: &HostNode, now_ms: i64) {
        if let Some(locked) = self.log.locked() {
            locked_section(ui, &locked, &self.log, now_ms);
            return;
        }
        let view = self.setup_mut().view();
        ui.horizontal(|ui| {
            ui.label("Preset");
            let mut preset = self.preset;
            egui::ComboBox::from_id_salt("preset")
                .selected_text(preset.name())
                .show_ui(ui, |ui| {
                    for choice in Preset::ALL {
                        ui.selectable_value(&mut preset, choice, choice.name());
                    }
                });
            if preset != self.preset {
                self.preset = preset;
                let _ = self.setup_mut().choose(preset);
            }
        });
        let mut ruleset = self.setup_mut().ruleset().clone();
        for (rule, text) in RULES {
            let unavailable = self.setup_mut().unavailable(rule);
            let on = rule_on(&mut ruleset, rule).expect("a chosen rule");
            let checkbox = ui.add_enabled(unavailable.is_none(), egui::Checkbox::new(on, text));
            if let Some(reason) = unavailable {
                checkbox.on_disabled_hover_text(format!("Unavailable: {reason}"));
            }
        }
        let mut caps = ruleset.level_caps.is_some();
        ui.add_enabled(
            self.setup_mut().unavailable(Rule::LevelCap).is_none(),
            egui::Checkbox::new(&mut caps, "Level caps"),
        );
        if caps != ruleset.level_caps.is_some() {
            ruleset.level_caps = caps.then(|| self.level_caps.clone());
        }
        let gates = self.setup_mut().sync_gates().to_vec();
        if !gates.is_empty() {
            let unavailable = self.setup_mut().unavailable(Rule::SyncGates);
            ui.collapsing(format!("Sync Gates · {}", ruleset.sync_gates.len()), |ui| {
                if let Some(reason) = &unavailable {
                    ui.weak(format!("Unavailable: {reason}"));
                }
                ui.horizontal_wrapped(|ui| {
                    for gate in &gates {
                        let mut on = ruleset.sync_gates.contains(gate);
                        if ui
                            .add_enabled(unavailable.is_none(), egui::Checkbox::new(&mut on, gate))
                            .changed()
                        {
                            // Kept in the profile's order.
                            ruleset.sync_gates = gates
                                .iter()
                                .filter(|g| if *g == gate { on } else { ruleset.sync_gates.contains(g) })
                                .cloned()
                                .collect();
                        }
                    }
                });
            });
        }
        ui.horizontal(|ui| {
            ui.label("Retries");
            ui.add(egui::DragValue::new(&mut ruleset.retries).range(0..=pactlocke_run::MAX_RETRIES))
                .on_hover_text("A shared pool: each Retry sends everyone back to the latest Run Checkpoint, by everyone's vote. 0 is off.");
        });
        if let Some(caps) = ruleset.level_caps.as_mut() {
            ui.indent("level caps", |ui| {
                egui::Grid::new("coop level caps").num_columns(4).show(ui, |ui| {
                    for (i, cap) in caps.iter_mut().enumerate() {
                        ui.label(&cap.name);
                        ui.add(egui::DragValue::new(&mut cap.level).range(1..=MAX_LEVEL));
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
            });
        }
        if ruleset != *self.setup_mut().ruleset() {
            let _ = self.setup_mut().edit(|r| *r = ruleset);
        }
        ui.separator();
        if let Some(ready) = ready_section(ui, &view, &self.me()) {
            // Ready to the Ruleset as shown, so a change this frame clears it.
            self.set_ready(ready, view.version);
        }
        let why = self.setup_mut().not_ready();
        let lock = ui.add_enabled(why.is_empty(), egui::Button::new("Lock and start"));
        if !why.is_empty() {
            lock.on_disabled_hover_text(why.join("\n"));
        } else if lock.clicked() {
            self.lock(node, now_ms);
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }

    /// The Host's own Ready, to Ruleset `version`.
    fn set_ready(&mut self, ready: bool, version: u64) {
        let me = self.me();
        if let Some(setup) = &mut self.setup {
            setup.receive(&me, ready_message(ready, version));
        }
    }

    /// The Host's own Ready, to the Ruleset as it is now.
    pub fn ready(&mut self, ready: bool) {
        if let Some(version) = self.setup_version() {
            self.set_ready(ready, version);
        }
    }

    /// Chooses a preset, which resets the rules to it.
    pub fn choose(&mut self, preset: Preset) {
        self.preset = preset;
        if let Some(setup) = &mut self.setup {
            let _ = setup.choose(preset);
        }
    }

    /// Changes the Ruleset before the lock. `level_caps` on uses the ROM's own caps.
    pub fn edit(&mut self, change: impl FnOnce(&mut Ruleset, &[LevelCap])) -> Result<(), String> {
        let setup = self.setup.as_mut().ok_or(TAKEN_OVER)?;
        let mut ruleset = setup.ruleset().clone();
        change(&mut ruleset, &self.level_caps);
        setup.edit(|r| *r = ruleset).map_err(|e| e.to_string())
    }

    /// Why the Run can't lock yet: nothing when everyone is Ready.
    pub fn not_ready(&self) -> Vec<String> {
        self.setup
            .as_ref()
            .map_or_else(|| vec![TAKEN_OVER.to_owned()], RunSetup::not_ready)
    }

    /// The Ruleset's version now: it changes whenever the Host changes it.
    /// None once taken over after the lock.
    pub fn setup_version(&self) -> Option<u64> {
        self.setup.as_ref().map(|setup| setup.view().version)
    }

    /// How many Players are in setup, the Host included; after a takeover,
    /// how many are still in the Run.
    pub fn players(&self) -> usize {
        self.setup
            .as_ref()
            .map_or_else(|| self.log.active().len(), |setup| setup.view().players.len())
    }

    /// Locks now, or says why it can't.
    pub fn lock_now(&mut self, node: &HostNode, now_ms: i64) -> Result<(), String> {
        self.lock(node, now_ms);
        match self.log.locked() {
            Some(_) => Ok(()),
            None => Err(self.error.clone().unwrap_or_else(|| "The Run didn't lock.".into())),
        }
    }

    /// Locks the Ruleset and roster: the lock goes in the log, and the lobby
    /// takes nobody new.
    fn lock(&mut self, node: &HostNode, now_ms: i64) {
        let Some(setup) = &mut self.setup else {
            return;
        };
        match setup.lock(now_ms) {
            Ok(locked) => {
                self.submit(Some(node), &LogBody::Locked(locked));
                node.lock_roster();
            }
            Err(e) => self.error = Some(format!("The Run can't lock: {e}")),
        }
    }

    /// Run setup, while there is one: always before the lock.
    fn setup_mut(&mut self) -> &mut RunSetup {
        self.setup.as_mut().expect("Run setup until the lock")
    }
}

fn ready_message(ready: bool, version: u64) -> SetupMessage {
    if ready {
        SetupMessage::Ready { version }
    } else {
        SetupMessage::Unready
    }
}

/// A joiner's side: setup as the Host last sent it, and their copy of the log.
pub struct JoinCoop {
    signer: Signer,
    /// The Host this joiner's network connects to: the Run log's, or,
    /// while that Host is silent, another Player who may have taken over.
    host: PlayerKey,
    log: RunLog,
    view: Option<SetupView>,
    rom: Option<PathBuf>,
    check: SetupCheck,
    /// Whether the lobby was connected last frame, and the check last sent.
    connected: bool,
    check_sent: Option<SetupCheck>,
    /// The Ruleset this Player said Ready to: the lock must be exactly it.
    ready_to: Option<Ruleset>,
    next_counter: u64,
    /// This Player's entries sent to the Host and not in the log yet.
    unordered: Vec<Unordered>,
    /// When the Host was last heard from.
    presence: Presence,
    started: bool,
    error: Option<String>,
    /// Input stream messages from the Host, for Spectate.
    stream_inbox: Vec<Vec<u8>>,
}

/// What one of this Player's waiting entries says.
fn body_of(entry: &Unordered) -> Option<LogBody> {
    let submission = Submission::from_bytes(&entry.bytes)?;
    serde_json::from_slice(&submission.body).ok()
}

/// One of this Player's entries the Host hasn't put in the log yet.
struct Unordered {
    counter: u64,
    /// When it was last sent while connected.
    sent_ms: i64,
    /// The signed submission, to send again after a reconnect.
    bytes: Vec<u8>,
}

impl JoinCoop {
    /// Joining the Run `host` hosts, at `now_ms`.
    pub fn new(key: [u8; 32], host: PeerKey, rom: Option<(PathBuf, String)>, now_ms: i64) -> Self {
        let signer = Signer::from_secret(key);
        let me = signer.public();
        let log = RunLog::new(host.0);
        let next_counter = log.next_counter(&me);
        let (rom, sha1) = rom.unzip();
        Self {
            signer,
            host: host.0,
            log,
            view: None,
            rom,
            check: own_check(sha1),
            connected: false,
            check_sent: None,
            ready_to: None,
            next_counter,
            unordered: Vec::new(),
            presence: Presence::new(now_ms),
            started: false,
            error: None,
            stream_inbox: Vec::new(),
        }
    }

    /// A Host who lost every Player carries on as a Player of the Run in
    /// `log`, which they started, dialling `host`: the next Player in join
    /// order, who will have taken over.
    pub fn stepped_down(key: [u8; 32], log: RunLog, host: PlayerKey, rom: PathBuf, now_ms: i64) -> Self {
        let locked = log.locked();
        let sha1 = locked.as_ref().map(|l| l.game_sha1.clone());
        let mut coop = Self::new(key, PeerKey(host), None, now_ms);
        coop.next_counter = log.next_counter(&coop.me());
        coop.log = log;
        coop.rom = Some(rom);
        coop.check.game_sha1 = sha1;
        coop.ready_to = locked.map(|l| l.ruleset);
        coop.started = true;
        coop
    }

    /// The Host this joiner's network goes through, which streams come from.
    pub fn host(&self) -> PlayerKey {
        self.host
    }

    /// From now on this joiner's network goes through `host`.
    pub fn dial(&mut self, host: PlayerKey) {
        self.host = host;
    }

    /// Whether the Run's Host has been silent long enough for the next
    /// Player in join order to take over.
    pub fn host_gone(&self, now_ms: i64) -> bool {
        self.log.locked().is_some() && self.presence.host_gone(&self.log.host(), now_ms)
    }

    /// This Player takes over hosting: their copy of the Run log, the
    /// entries they had waiting for the old Host, and their ROM.
    pub fn into_takeover(self) -> (RunLog, Vec<LogBody>, Option<PathBuf>) {
        let waiting = self.unordered.iter().filter_map(body_of).collect();
        (self.log, waiting, self.rom)
    }

    /// Whether the lobby is connected to the Host now.
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// This Player's copy of the Run log.
    pub fn log(&self) -> &RunLog {
        &self.log
    }

    /// What went wrong with the shared Run log, if anything has.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Input stream messages since the last call: all from the Host, who passes every stream on.
    pub fn take_stream(&mut self) -> Vec<(PlayerKey, Vec<u8>)> {
        let host = self.host;
        std::mem::take(&mut self.stream_inbox)
            .into_iter()
            .map(|bytes| (host, bytes))
            .collect()
    }

    /// The Player picked their ROM in the lobby.
    pub fn rom_picked(&mut self, path: PathBuf, sha1: String) {
        self.rom = Some(path);
        self.check.game_sha1 = Some(sha1);
    }

    fn me(&self) -> [u8; 32] {
        self.signer.public()
    }

    /// Says what this app brings when connected (again), reads the Host's
    /// setup and log entries, and returns the Run to start once the
    /// countdown is over.
    pub fn pump(&mut self, node: &JoinNode, now_ms: i64) -> Option<CoopStart> {
        let connected = !node.view().players.is_empty();
        if connected && (!self.connected || self.check_sent.as_ref() != Some(&self.check)) {
            node.send(Message::Setup(SetupMessage::Check(self.check.clone()).to_bytes()));
            self.check_sent = Some(self.check.clone());
        }
        // Back after a disconnect: every entry the Host hasn't ordered goes
        // again (one it already has is refused as a repeat).
        if connected && !self.connected {
            for entry in &mut self.unordered {
                node.send(Message::Log(entry.bytes.clone()));
                entry.sent_ms = now_ms;
            }
        }
        self.connected = connected;
        let inbox = node.take_inbox();
        let heard = !inbox.is_empty();
        let mut given_back = Vec::new();
        for message in inbox {
            match message {
                Message::Setup(bytes) => {
                    if let Some(SetupMessage::State(view)) = SetupMessage::from_bytes(&bytes) {
                        self.view = Some(*view);
                    }
                }
                Message::Log(bytes) => given_back.extend(self.accept(&bytes)),
                Message::Input(bytes) => self.stream_inbox.push(bytes),
                _ => {}
            }
        }
        // After any takeover in what arrived, so the new Host is the one
        // heard; and only from the Host the log names, not another Player tried.
        if heard && self.host == self.log.host() {
            self.presence.heard(self.log.host(), now_ms);
        }
        // This Player's events the Host's copy of the log doesn't have go to
        // the Host again, before what was waiting, signed again from the
        // Host's count: entries only a Host writes are void with their copy.
        if !given_back.is_empty() {
            let waiting = std::mem::take(&mut self.unordered);
            let bodies: Vec<LogBody> = given_back
                .iter()
                .filter_map(|s| serde_json::from_slice(&s.body).ok())
                .filter(|body| matches!(body, LogBody::Event(_)))
                .chain(waiting.iter().filter_map(body_of))
                .collect();
            self.next_counter = self.log.next_counter(&self.me());
            for body in bodies {
                self.queue(node, &body, now_ms);
            }
        }
        let ordered = self.log.next_counter(&self.me());
        self.unordered.retain(|entry| entry.counter >= ordered);
        // Offline, entries wait for the reconnect: nobody is leaving them out.
        if connected
            && let Some(entry) = self.unordered.first()
            && now_ms - entry.sent_ms >= UNORDERED_MS
        {
            self.error = Some(format!(
                "The Host hasn't put {} of your Run log entries in the log: the Host may be leaving them out.",
                self.unordered.len()
            ));
        }
        self.start_when_counted_down(now_ms)
    }

    /// Why this Player won't start the Run the Host locked, if they won't:
    /// they aren't in its roster, it isn't the Ruleset they said Ready to,
    /// or it is for another ROM.
    fn refuse(&self, locked: &Locked) -> Option<String> {
        if !locked.roster.iter().any(|p| p.key == self.me()) {
            return Some("The Host locked the Run without you in it.".to_owned());
        }
        if self.ready_to.as_ref() != Some(&locked.ruleset) {
            return Some("The Host locked a Ruleset you didn't say Ready to, so your Run won't start.".to_owned());
        }
        let same_rom = self
            .check
            .game_sha1
            .as_ref()
            .is_some_and(|sha1| sha1.eq_ignore_ascii_case(&locked.game_sha1));
        if !same_rom {
            return Some(format!(
                "The Host locked the Run for another ROM ({}).",
                locked.game_name
            ));
        }
        None
    }

    /// An entry of the Host's copy of the log. The whole log comes again
    /// after a reconnect, and from a new Host: what is here already is
    /// skipped, and where this copy split from the Host's, the Host's wins.
    /// Returns this Player's own entries the Host's copy doesn't have.
    fn accept(&mut self, bytes: &[u8]) -> Vec<Submission> {
        let Some(entry) = LogEntry::from_bytes(bytes) else {
            return Vec::new();
        };
        let me = self.me();
        self.log.give_way_to(entry, &me).unwrap_or_else(|e| {
            // A copy without a takeover this one has: this one stays, and
            // this Player looks on for the Host who took over.
            if let LogError::WithoutTakeover { .. } = e {
                log::info!("Kept this copy of the Run log: {e}");
                return Vec::new();
            }
            log::warn!("Refused a Run log entry from the Host: {e}");
            self.error = Some(format!("The Host sent a Run log entry that doesn't check out: {e}"));
            Vec::new()
        })
    }

    fn start_when_counted_down(&mut self, now_ms: i64) -> Option<CoopStart> {
        let locked = self.log.locked()?;
        if self.started || now_ms < locked.start_ms {
            return None;
        }
        self.started = true;
        if let Some(why) = self.refuse(&locked) {
            self.error = Some(why);
            return None;
        }
        let result = self
            .rom
            .clone()
            .ok_or_else(|| "No ROM was chosen for the Run.".to_owned())
            .and_then(|path| {
                std::fs::read(&path)
                    .map(|rom| (rom, path.clone()))
                    .map_err(|e| format!("Couldn't read {}: {e}", path.display()))
            });
        match result {
            Ok((rom, path)) => Some(CoopStart { locked, rom, path }),
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    /// The Ruleset's version as the Host last sent it, once it has.
    pub fn setup_version(&self) -> Option<u64> {
        self.view.as_ref().map(|view| view.version)
    }

    /// This Player's Ready, to the Ruleset as the Host last sent it.
    pub fn ready(&mut self, node: &JoinNode, ready: bool) {
        self.set_ready(node, ready);
    }

    fn set_ready(&mut self, node: &JoinNode, ready: bool) {
        if let Some(view) = &self.view {
            self.ready_to = ready.then(|| view.ruleset.clone());
            node.send(Message::Setup(ready_message(ready, view.version).to_bytes()));
        }
    }

    /// This Player's Run state, signed, to the Host to order.
    pub fn share(&mut self, node: &JoinNode, events: Vec<RunEvent>, now_ms: i64) {
        for event in events {
            self.queue(node, &LogBody::Event(event), now_ms);
        }
    }

    /// One of this Player's entries, signed with their next counter, sent
    /// to the Host and kept until the Host has put it in the log.
    fn queue(&mut self, node: &JoinNode, body: &LogBody, now_ms: i64) {
        let counter = self.next_counter.max(self.log.next_counter(&self.me()));
        let bytes = self.log.sign(&self.signer, counter, body).to_bytes();
        node.send(Message::Log(bytes.clone()));
        self.unordered.push(Unordered {
            counter,
            sent_ms: now_ms,
            bytes,
        });
        self.next_counter = counter + 1;
    }

    /// The setup section of the lobby window: the Ruleset as the Host has
    /// it now, and Ready.
    pub fn show(&mut self, ui: &mut egui::Ui, node: &JoinNode, now_ms: i64) {
        if let Some(locked) = self.log.locked() {
            match self.refuse(&locked) {
                Some(why) => {
                    ui.colored_label(ui.visuals().error_fg_color, why);
                }
                None => locked_section(ui, &locked, &self.log, now_ms),
            }
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            return;
        }
        let Some(view) = self.view.clone() else {
            return;
        };
        ui.label(format!("Ruleset (the Host is choosing) · version {}", view.version));
        let mut ruleset = view.ruleset.clone();
        for (rule, text) in RULES {
            let on = rule_on(&mut ruleset, rule).expect("a chosen rule");
            ui.add_enabled(false, egui::Checkbox::new(on, text));
        }
        let mut caps = ruleset.level_caps.is_some();
        ui.add_enabled(false, egui::Checkbox::new(&mut caps, "Level caps"));
        ui.label(format!("Retries: {}", ruleset.retries));
        if !ruleset.sync_gates.is_empty() {
            ui.label(format!("Sync Gates: {}", ruleset.sync_gates.join(", ")));
        }
        ui.separator();
        if let Some(ready) = ready_section(ui, &view, &self.me()) {
            self.set_ready(node, ready);
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }
}

/// Every Player's Ready and what doesn't match; returns this Player's new
/// Ready when they toggle it.
fn ready_section(ui: &mut egui::Ui, view: &SetupView, me: &[u8; 32]) -> Option<bool> {
    for player in &view.players {
        ui.horizontal(|ui| {
            ui.label(&player.name);
            if view.is_ready(&player.key) {
                ui.label("Ready");
            } else {
                ui.weak("not Ready");
            }
        });
    }
    for mismatch in &view.mismatches {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("{}: {}", mismatch.player, mismatch.what),
        );
    }
    let mut ready = view.is_ready(me);
    ui.checkbox(&mut ready, "Ready to this Ruleset")
        .changed()
        .then_some(ready)
}

/// After the lock: the countdown, then the Run log's tallies.
fn locked_section(ui: &mut egui::Ui, locked: &Locked, log: &RunLog, now_ms: i64) {
    let left = locked.start_ms - now_ms;
    if left > 0 {
        ui.heading(format!("Locked. Starting in {}…", (left + 999) / 1000));
    } else {
        ui.strong("The Run has started.");
    }
    let tallies = log.tallies();
    let flags = log.flags();
    egui::Grid::new("tallies").num_columns(6).show(ui, |ui| {
        for heading in ["Player", "Encounters", "Catches", "Deaths", "Violations", "Cheat flags"] {
            ui.strong(heading);
        }
        ui.end_row();
        for (index, player) in locked.roster.iter().enumerate() {
            let count = |map: &std::collections::BTreeMap<[u8; 32], u32>| map.get(&player.key).copied().unwrap_or(0);
            ui.horizontal(|ui| {
                crate::theme::dot(ui, index);
                ui.label(&player.name);
            });
            ui.label(count(&tallies.encounters).to_string());
            ui.label(count(&tallies.catches).to_string());
            ui.label(count(&tallies.deaths).to_string());
            ui.label(count(&tallies.violations).to_string());
            let flagged = flags.iter().filter(|f| f.player == player.key).count();
            if flagged > 0 {
                ui.colored_label(crate::theme::CHEAT, flagged.to_string());
            } else {
                ui.label("0");
            }
            ui.end_row();
        }
    });
    ui.weak(format!("Run log: {} entries, each checked.", log.entries().len()));
}

#[cfg(test)]
mod tests {
    //! The Host's and a joiner's apps over real iroh endpoints on this
    //! machine: setup, Ready, the lock, the countdown and the shared log.

    use std::time::{Duration, Instant};

    use pactlocke_net::{HostLobby, Invite, JoinLobby, Relays};
    use pactlocke_run::DeathCause;

    use super::*;

    const FIRERED: &str = "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc";

    #[track_caller]
    fn until(what: &str, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(30), "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

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

    #[test]
    fn friends_ready_up_lock_count_down_and_share_one_log() {
        let run = RunAdvert {
            name: "Kanto with friends".into(),
            game_name: "Pokémon FireRed (USA) 1.0".into(),
            game_sha1: FIRERED.into(),
        };
        let host_node = HostNode::start([1; 32], HostLobby::new(run.clone(), "Red", APP_VERSION), Relays::Off);
        let mut host = HostCoop::new(
            [1; 32],
            "Red",
            &run,
            HostRom {
                rom: vec![1, 2, 3],
                path: "red.gba".into(),
                capabilities: CapabilityReport::everything_enforceable(),
                level_caps: Vec::new(),
                sync_gates: Vec::new(),
            },
        );
        until("the invite", || host_node.view().invite_link.is_some());
        until("the Host's addresses", || !host_node.direct_addrs().is_empty());
        let invite = Invite::parse(host_node.view().invite_link.as_deref().unwrap()).unwrap();

        let rom_file = std::env::temp_dir().join(format!("pactlocke-coop-test-{}.gba", std::process::id()));
        std::fs::write(&rom_file, [4, 5, 6]).unwrap();
        let lobby = JoinLobby::new(invite.clone(), "Blue", APP_VERSION, Some(FIRERED.into())).unwrap();
        let blue_node = JoinNode::start_with([2; 32], lobby, Relays::Off, host_node.direct_addrs());
        let mut blue = JoinCoop::new([2; 32], invite.host, Some((rom_file.clone(), FIRERED.into())), 1_000);

        let mut now = 1_000;
        until("Blue in setup, and Blue sees it", || {
            host.pump(&host_node, now);
            blue.pump(&blue_node, now);
            host.setup_mut().view().players.len() == 2 && blue.view.as_ref().is_some_and(|v| v.players.len() == 2)
        });
        assert!(blue.view.as_ref().unwrap().mismatches.is_empty(), "same build and ROM");
        assert_eq!(blue.log.entries().len(), 1, "Blue has the Genesis entry, checked");

        let version = host.setup_mut().view().version;
        host.set_ready(true, version);
        until("Blue's view of the Ruleset", || {
            host.pump(&host_node, now);
            blue.pump(&blue_node, now);
            blue.view.as_ref().is_some_and(|v| v.version == version)
        });
        blue.set_ready(&blue_node, true);
        until("everyone Ready", || {
            host.pump(&host_node, now);
            blue.pump(&blue_node, now);
            host.not_ready().is_empty()
        });

        host.lock(&host_node, now);
        until("Blue has the lock", || {
            host.pump(&host_node, now);
            blue.pump(&blue_node, now);
            blue.log.locked().is_some()
        });
        let locked = host.log.locked().unwrap();
        assert_eq!(locked.start_ms, now + pactlocke_run::COUNTDOWN_MS);
        assert_eq!(blue.log.locked().unwrap(), locked);
        assert!(host.pump(&host_node, now).is_none(), "still counting down");

        now = locked.start_ms;
        let host_start = host.pump(&host_node, now).expect("the Host's Run starts");
        let blue_start = blue.pump(&blue_node, now).expect("Blue's Run starts");
        assert_eq!(host_start.rom, [1, 2, 3]);
        assert_eq!(
            blue_start.rom,
            [4, 5, 6],
            "Blue's own ROM, read from where Blue picked it"
        );
        assert_eq!(blue_start.locked, host_start.locked);
        assert!(host.pump(&host_node, now).is_none(), "a Run starts once");

        host.share(&host_node, vec![died(0xAA)]);
        blue.share(&blue_node, vec![died(0xB1), died(0xB2)], now);
        until("one log for both", || {
            host.pump(&host_node, now);
            blue.pump(&blue_node, now);
            host.log.entries().len() == 5 && blue.log.entries().len() == 5
        });
        assert_eq!(host.log.entries(), blue.log.entries());
        let deaths = blue.log.tallies().deaths;
        assert_eq!(deaths.get(&host.me()), Some(&1));
        assert_eq!(deaths.get(&blue.me()), Some(&2));
        assert!(
            host.error.is_none() && blue.error.is_none(),
            "{:?} {:?}",
            host.error,
            blue.error
        );

        // Ten seconds of play: each game streams four chunks a second, as at
        // 1x, and the Run log grows. Blue stays connected throughout.
        let chunk = vec![0u8; 600];
        let started = Instant::now();
        let mut sent = 0;
        while started.elapsed() < Duration::from_secs(10) {
            let now = locked.start_ms + started.elapsed().as_millis() as i64;
            if started.elapsed().as_millis() as usize / 250 >= sent {
                sent += 1;
                blue_node.send(Message::Input(chunk.clone()));
                host_node.send(None, Message::Input(chunk.clone()));
            }
            host.pump(&host_node, now);
            blue.pump(&blue_node, now);
            let _ = (host.take_stream(), blue.take_stream());
            assert!(
                blue_node.view().error.is_none() && blue.connected(),
                "Blue lost the Host {:?} after {:?}",
                blue_node.view().error,
                started.elapsed()
            );
            std::thread::sleep(Duration::from_millis(16));
        }
        assert!(
            host.error.is_none() && blue.error.is_none(),
            "{:?} {:?}",
            host.error,
            blue.error
        );
        let _ = std::fs::remove_file(rom_file);
    }

    /// A Host's log with its Genesis entry and a lock of `ruleset` for Red and Blue, as bytes.
    fn locked_log(ruleset: Ruleset, blue: [u8; 32]) -> (Vec<Vec<u8>>, i64) {
        let host = Signer::from_secret([1; 32]);
        let mut log = RunLog::new(host.public());
        let genesis = LogBody::Genesis {
            nonce: [7; 16],
            run_name: "Kanto with friends".into(),
            game_name: "Pokémon FireRed (USA) 1.0".into(),
            game_sha1: FIRERED.into(),
        };
        let locked = Locked {
            run_name: "Kanto with friends".into(),
            game_name: "Pokémon FireRed (USA) 1.0".into(),
            game_sha1: FIRERED.into(),
            ruleset,
            roster: vec![
                pactlocke_run::RosterEntry {
                    key: host.public(),
                    name: "Red".into(),
                },
                pactlocke_run::RosterEntry {
                    key: blue,
                    name: "Blue".into(),
                },
            ],
            start_ms: 5_000,
        };
        let mut bytes = Vec::new();
        for (counter, body) in [(1, genesis), (2, LogBody::Locked(locked))] {
            let submission = log.sign(&host, counter, &body);
            bytes.push(log.order(&host, submission).unwrap().to_bytes());
        }
        (bytes, 5_000)
    }

    #[test]
    fn a_player_starts_only_the_ruleset_they_said_ready_to() {
        let rom_file = std::env::temp_dir().join(format!("pactlocke-lock-test-{}.gba", std::process::id()));
        std::fs::write(&rom_file, [4, 5, 6]).unwrap();
        let agreed = Preset::ClassicNuzlocke.ruleset(&[]);
        let blue_key = Signer::from_secret([2; 32]).public();

        for (locked_ruleset, starts) in [(agreed.clone(), true), (Preset::Hardcore.ruleset(&[]), false)] {
            let mut blue = JoinCoop::new(
                [2; 32],
                PeerKey([0; 32]),
                Some((rom_file.clone(), FIRERED.into())),
                1_000,
            );
            blue.log = RunLog::new(Signer::from_secret([1; 32]).public());
            blue.ready_to = Some(agreed.clone());
            let (entries, start_ms) = locked_log(locked_ruleset, blue_key);
            for entry in &entries {
                blue.accept(entry);
            }
            let start = blue.start_when_counted_down(start_ms);
            assert_eq!(start.is_some(), starts, "{:?}", blue.error);
            if !starts {
                assert!(blue.error.as_deref().unwrap().contains("didn't say Ready to"));
            }
        }
        let _ = std::fs::remove_file(rom_file);
    }
}
