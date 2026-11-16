//! Run setup, before the lock: the Host edits a Ruleset from a preset while
//! every Player sees it; Players say Ready; when everyone is Ready and every
//! check passes, the Ruleset and roster lock and a countdown starts every game
//! from a new game together. Pure state: the network carries its messages.

use pactlocke_game::{Capability, CapabilityReport, LevelCap, NativeOption, Rule};
use serde::{Deserialize, Serialize};

use crate::{NativeOptionView, Ruleset};

/// How long the 3-2-1 countdown after the lock runs, before every game starts.
pub const COUNTDOWN_MS: i64 = 3000;
/// A Run has one to three Players.
const MAX_PLAYERS: usize = 3;

/// A starting point for the Ruleset; every rule stays optional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    /// First Encounter, faint = Dead, the dupes clause; Gifts kept.
    ClassicNuzlocke,
    /// Classic, with Links between the Players' Pokémon.
    SoulLink,
    /// Classic, with level caps, no items in battle and set mode.
    Hardcore,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::ClassicNuzlocke, Preset::SoulLink, Preset::Hardcore];

    pub fn name(self) -> &'static str {
        match self {
            Preset::ClassicNuzlocke => "Classic Nuzlocke",
            Preset::SoulLink => "Soul Link",
            Preset::Hardcore => "Hardcore",
        }
    }

    /// The preset's Ruleset, with `level_caps` (the game's defaults) where it uses them.
    /// Its Sync Gates are set by the setup: every gate, once there is a second Player.
    pub fn ruleset(self, level_caps: &[LevelCap]) -> Ruleset {
        let classic = Ruleset {
            first_encounter: true,
            faint_is_dead: true,
            dupes_clause: true,
            gifts: true,
            ..Ruleset::default()
        };
        match self {
            Preset::ClassicNuzlocke => classic,
            Preset::SoulLink => Ruleset { links: true, ..classic },
            Preset::Hardcore => Ruleset {
                level_caps: (!level_caps.is_empty()).then(|| level_caps.to_vec()),
                no_items: true,
                set_mode: true,
                ..classic
            },
        }
    }
}

/// Every Ruleset rule a Player can choose.
const CHOOSABLE: [Rule; 12] = [
    Rule::SyncGates,
    Rule::FirstEncounter,
    Rule::DupesClause,
    Rule::Gifts,
    Rule::Statics,
    Rule::FaintIsDead,
    Rule::Links,
    Rule::LevelCap,
    Rule::NoItems,
    Rule::MustNickname,
    Rule::SetMode,
    Rule::TypeClause,
];

/// Why `rule` can't be chosen in this Ruleset on this ROM with this many
/// Players, if it can't: what it depends on, or why the game can't do it.
pub fn unavailable(rule: Rule, ruleset: &Ruleset, capabilities: &CapabilityReport, players: usize) -> Option<String> {
    let needs = match rule {
        Rule::DupesClause | Rule::Statics if !ruleset.first_encounter => Some("it needs First Encounter on"),
        Rule::Links if players < 2 => Some("a solo Run has no Links"),
        Rule::SyncGates if players < 2 => Some("a solo Run has no Sync Gates"),
        Rule::Links if !ruleset.faint_is_dead => Some("it needs Faint = Dead on"),
        _ => None,
    };
    if let Some(needs) = needs {
        return Some(needs.to_owned());
    }
    match capabilities.get(rule) {
        Capability::Enforceable => None,
        // A level cap the game can't be stopped at is still a soft cap.
        Capability::GreyedOut { .. } if rule == Rule::LevelCap => None,
        Capability::GreyedOut { reason } => Some(format!("can't be enforced on this ROM: {reason}")),
        // Pactlocke sets the game's own option for it.
        Capability::NativeOption { .. } => None,
    }
}

/// The native options of the rules `ruleset` has on, each once: what
/// Pactlocke sets in every Player's game, and what setup shows them.
pub fn native_options_in_use<'a>(
    ruleset: &Ruleset,
    natives: impl IntoIterator<Item = &'a NativeOption>,
) -> Vec<NativeOption> {
    let mut in_use: Vec<NativeOption> = Vec::new();
    for native in natives {
        let known = in_use
            .iter()
            .any(|n| n.rule == native.rule && n.option == native.option);
        if native.in_use(|rule| ruleset.has(rule)) && !known {
            in_use.push(native.clone());
        }
    }
    in_use
}

/// What setup shows every Player of the game's own options in use on this ROM.
pub fn native_options(ruleset: &Ruleset, capabilities: &CapabilityReport) -> Vec<NativeOptionView> {
    native_options_in_use(ruleset, capabilities.native_options())
        .iter()
        .map(|native| NativeOptionView::of(native, None))
        .collect()
}

/// Turns off every rule that can't be chosen, the dependents of a rule
/// turned off included.
pub fn settle(ruleset: &mut Ruleset, capabilities: &CapabilityReport, players: usize) {
    // Shared Seed through the game's own randomizer needs one.
    if !capabilities.own_randomizer() {
        ruleset.romhack_seed = None;
    }
    // Twice: turning a rule off can make another unavailable.
    for _ in 0..2 {
        for rule in CHOOSABLE {
            if unavailable(rule, ruleset, capabilities, players).is_some() {
                turn_off(ruleset, rule);
            }
        }
    }
}

/// Turns a rule off. With Gifts unavailable every Gift is kept, since no
/// catalogue tells them apart, so turning Gifts "off" means keeping them all.
fn turn_off(ruleset: &mut Ruleset, rule: Rule) {
    match rule {
        Rule::FirstEncounter => ruleset.first_encounter = false,
        Rule::DupesClause => ruleset.dupes_clause = false,
        Rule::Gifts => ruleset.gifts = true,
        Rule::Statics => ruleset.statics = false,
        Rule::FaintIsDead => ruleset.faint_is_dead = false,
        Rule::Links => ruleset.links = false,
        Rule::LevelCap => ruleset.level_caps = None,
        Rule::NoItems => ruleset.no_items = false,
        Rule::MustNickname => ruleset.must_nickname = false,
        Rule::SetMode => ruleset.set_mode = false,
        Rule::TypeClause => ruleset.type_clause = false,
        Rule::SyncGates => ruleset.sync_gates.clear(),
    }
}

/// What the lock compares across Players, from each Player's own app and game.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupCheck {
    /// The app's build: every Player must run the identical one.
    pub app_build: String,
    /// The SHA-1 of the Player's ROM after patching, once they picked one.
    pub game_sha1: Option<String>,
    /// A Romhack's own options, read from the game (name, setting).
    #[serde(default)]
    pub hack_options: Vec<(String, String)>,
    /// For a randomized Run, the SHA-1 of every Player's game as this app
    /// made it, in setup's Player order.
    #[serde(default)]
    pub randomized: Vec<String>,
}

/// A Player in setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupPlayer {
    /// Their install's key: their identity in the roster and the Run log.
    pub key: [u8; 32],
    pub name: String,
    pub check: SetupCheck,
    /// The Ruleset version they said Ready to, if they did.
    #[serde(default)]
    pub ready_for: Option<u64>,
}

/// One thing a Player has that doesn't match the Host's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mismatch {
    pub player: String,
    pub what: String,
}

/// A locked Run's roster, in join order: the Host first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub key: [u8; 32],
    pub name: String,
}

/// The lock: what every Player plays by, and when every game starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locked {
    pub run_name: String,
    pub game_name: String,
    pub game_sha1: String,
    pub ruleset: Ruleset,
    pub roster: Vec<RosterEntry>,
    /// When every game starts from a new game, after the countdown; every
    /// game's clock starts here too.
    pub start_ms: i64,
}

/// What every Player sees of setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupView {
    pub run_name: String,
    pub game_name: String,
    pub game_sha1: String,
    pub ruleset: Ruleset,
    /// Goes up with every change; Ready is said to a version.
    pub version: u64,
    pub players: Vec<SetupPlayer>,
    pub mismatches: Vec<Mismatch>,
    pub locked: Option<Locked>,
    /// The game's own options the Ruleset's rules are done with.
    #[serde(default)]
    pub native_options: Vec<NativeOptionView>,
}

impl SetupView {
    pub fn is_ready(&self, key: &[u8; 32]) -> bool {
        self.players
            .iter()
            .any(|p| p.key == *key && p.ready_for == Some(self.version))
    }
}

/// What crosses the network in setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SetupMessage {
    /// From the Host to everyone, after every change.
    State(Box<SetupView>),
    /// From a Player: what they bring.
    Check(SetupCheck),
    /// From a Player: Ready to this Ruleset version.
    Ready {
        version: u64,
    },
    Unready,
}

impl SetupMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a setup message serialises")
    }

    /// `None` for bytes that aren't a setup message.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

/// Why setup refused something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupError {
    /// The Ruleset and roster are locked: nobody joins and nothing changes.
    Locked,
    /// Three Players already.
    Full,
    /// The lock can't happen yet, for these reasons.
    NotYet(Vec<String>),
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupError::Locked => write!(f, "the Run is locked"),
            SetupError::Full => write!(f, "the Run already has {MAX_PLAYERS} Players"),
            SetupError::NotYet(why) => write!(f, "not yet: {}", why.join("; ")),
        }
    }
}

/// The Host's setup.
#[derive(Debug, Clone)]
pub struct RunSetup {
    run_name: String,
    game_name: String,
    game_sha1: String,
    capabilities: CapabilityReport,
    level_caps: Vec<LevelCap>,
    /// Every battle the Game Profile can make a Sync Gate, in order.
    sync_gates: Vec<String>,
    /// The gates the Ruleset has whenever there are two Players or more:
    /// every gate, or the Host's own choice since. A solo Run has none, so they
    /// come back when a second Player joins.
    gates_wanted: Vec<String>,
    ruleset: Ruleset,
    version: u64,
    players: Vec<SetupPlayer>,
    locked: Option<Locked>,
}

impl RunSetup {
    /// The Host's setup for a Run on their ROM, starting from `preset`.
    /// `level_caps` are the ROM's default caps, for presets that use them.
    pub fn new(
        host: SetupPlayer,
        run_name: &str,
        game_name: &str,
        game_sha1: &str,
        capabilities: CapabilityReport,
        level_caps: Vec<LevelCap>,
        preset: Preset,
    ) -> Self {
        let mut setup = Self {
            run_name: run_name.to_owned(),
            game_name: game_name.to_owned(),
            game_sha1: game_sha1.to_owned(),
            capabilities,
            level_caps,
            sync_gates: Vec::new(),
            gates_wanted: Vec::new(),
            ruleset: Ruleset::default(),
            version: 0,
            players: vec![host],
            locked: None,
        };
        // A new setup isn't locked, so choosing a preset can't fail.
        let _ = setup.choose(preset);
        setup
    }

    /// The battles this ROM's Game Profile can make Sync Gates. Every preset
    /// ticks them all once there is a second Player.
    pub fn set_sync_gates(&mut self, gates: Vec<String>) {
        self.sync_gates = gates;
        self.gates_wanted = self.sync_gates.clone();
        let wanted = self.gates_wanted.clone();
        // Only the gates change; a locked Run keeps its Ruleset.
        let _ = self.edit(|r| r.sync_gates = wanted);
    }

    /// Every battle this ROM can make a Sync Gate, in order.
    pub fn sync_gates(&self) -> &[String] {
        &self.sync_gates
    }

    /// Starts the Ruleset again from a preset.
    pub fn choose(&mut self, preset: Preset) -> Result<(), SetupError> {
        let mut ruleset = preset.ruleset(&self.level_caps);
        ruleset.sync_gates = self.sync_gates.clone();
        // A preset is about the rules; the randomizer stays as the Host set it.
        ruleset.randomizer = self.ruleset.randomizer.clone();
        ruleset.romhack_seed = self.ruleset.romhack_seed;
        let wanted = ruleset.sync_gates.clone();
        self.edit(|r| *r = ruleset)?;
        self.gates_wanted = wanted;
        Ok(())
    }

    /// Changes the Ruleset. Rules that can't be chosen are turned off, and
    /// every Player's Ready is cleared, so nobody agrees to what they haven't seen.
    pub fn edit(&mut self, change: impl FnOnce(&mut Ruleset)) -> Result<(), SetupError> {
        if self.locked.is_some() {
            return Err(SetupError::Locked);
        }
        let gates_before = self.ruleset.sync_gates.clone();
        change(&mut self.ruleset);
        if self.ruleset.sync_gates != gates_before && self.players.len() > 1 {
            self.gates_wanted = self.ruleset.sync_gates.clone();
        }
        settle(&mut self.ruleset, &self.capabilities, self.players.len());
        self.changed();
        Ok(())
    }

    /// A Player joins the lobby.
    pub fn join(&mut self, player: SetupPlayer) -> Result<(), SetupError> {
        if self.locked.is_some() {
            return Err(SetupError::Locked);
        }
        if self.players.iter().any(|p| p.key == player.key) {
            return Ok(());
        }
        if self.players.len() >= MAX_PLAYERS {
            return Err(SetupError::Full);
        }
        self.players.push(SetupPlayer {
            ready_for: None,
            ..player
        });
        if self.ruleset.sync_gates.is_empty() {
            self.ruleset.sync_gates = self.gates_wanted.clone();
            settle(&mut self.ruleset, &self.capabilities, self.players.len());
        }
        self.changed();
        Ok(())
    }

    /// A Player leaves the lobby before the lock.
    pub fn leave(&mut self, key: &[u8; 32]) {
        if self.locked.is_none() && self.players.iter().any(|p| p.key == *key) {
            self.players.retain(|p| p.key != *key);
            settle(&mut self.ruleset, &self.capabilities, self.players.len());
            self.changed();
        }
    }

    /// A message from a Player (or the Host's own UI, by the Host's key).
    pub fn receive(&mut self, from: &[u8; 32], message: SetupMessage) {
        if self.locked.is_some() {
            return;
        }
        let version = self.version;
        let Some(player) = self.players.iter_mut().find(|p| p.key == *from) else {
            return;
        };
        match message {
            SetupMessage::Check(check) => {
                if player.check != check {
                    player.check = check;
                    player.ready_for = None;
                }
            }
            SetupMessage::Ready { version: seen } if seen == version => player.ready_for = Some(version),
            SetupMessage::Unready => player.ready_for = None,
            _ => {}
        }
    }

    /// What doesn't match the Host's, Player by Player, each named.
    pub fn mismatches(&self) -> Vec<Mismatch> {
        let host = &self.players[0].check;
        let mut found = Vec::new();
        for player in &self.players {
            let mut mismatch = |what: String| {
                found.push(Mismatch {
                    player: player.name.clone(),
                    what,
                })
            };
            let check = &player.check;
            if check.app_build != host.app_build {
                mismatch(format!(
                    "Pactlocke {}, not the Host's {}",
                    check.app_build, host.app_build
                ));
            }
            match &check.game_sha1 {
                None => mismatch("no ROM chosen yet".to_owned()),
                Some(sha1) if !sha1.eq_ignore_ascii_case(&self.game_sha1) => {
                    mismatch(format!("a different ROM (SHA-1 {sha1}), not {}", self.game_name))
                }
                Some(_) => {}
            }
            for (option, setting) in &host.hack_options {
                let theirs = check.hack_options.iter().find(|(o, _)| o == option).map(|(_, s)| s);
                if theirs != Some(setting) {
                    let theirs = theirs.map_or("not set", String::as_str);
                    mismatch(format!("the game's {option} is {theirs}, not {setting}"));
                }
            }
            if self.ruleset.randomizer.is_some() {
                if check.randomized.len() != self.players.len() {
                    mismatch("hasn't made the randomized games yet".to_owned());
                } else if host.randomized.len() == self.players.len() && check.randomized != host.randomized {
                    mismatch("made different randomized games from the Host's".to_owned());
                }
            }
        }
        found
    }

    /// Why the lock can't happen yet, if it can't: someone isn't Ready, or something doesn't match.
    pub fn not_ready(&self) -> Vec<String> {
        let mut why: Vec<String> = self
            .players
            .iter()
            .filter(|p| p.ready_for != Some(self.version))
            .map(|p| format!("{} isn't Ready", p.name))
            .collect();
        why.extend(
            self.mismatches()
                .into_iter()
                .map(|m| format!("{}: {}", m.player, m.what)),
        );
        why
    }

    /// Locks the Ruleset and roster, if everyone is Ready and everything
    /// matches. Every game starts after the countdown, with its clock at
    /// that moment.
    pub fn lock(&mut self, now_ms: i64) -> Result<Locked, SetupError> {
        if self.locked.is_some() {
            return Err(SetupError::Locked);
        }
        let why = self.not_ready();
        if !why.is_empty() {
            return Err(SetupError::NotYet(why));
        }
        let locked = Locked {
            run_name: self.run_name.clone(),
            game_name: self.game_name.clone(),
            game_sha1: self.game_sha1.clone(),
            ruleset: self.ruleset.clone(),
            roster: self
                .players
                .iter()
                .map(|p| RosterEntry {
                    key: p.key,
                    name: p.name.clone(),
                })
                .collect(),
            start_ms: now_ms + COUNTDOWN_MS,
        };
        self.locked = Some(locked.clone());
        Ok(locked)
    }

    pub fn ruleset(&self) -> &Ruleset {
        &self.ruleset
    }

    /// Whether the game has its own randomizer, whose seed the Host can share.
    pub fn own_randomizer(&self) -> bool {
        self.capabilities.own_randomizer()
    }

    /// Why a rule can't be chosen now, if it can't.
    pub fn unavailable(&self, rule: Rule) -> Option<String> {
        unavailable(rule, &self.ruleset, &self.capabilities, self.players.len())
    }

    /// What every Player sees.
    pub fn view(&self) -> SetupView {
        SetupView {
            run_name: self.run_name.clone(),
            game_name: self.game_name.clone(),
            game_sha1: self.game_sha1.clone(),
            ruleset: self.ruleset.clone(),
            version: self.version,
            players: self.players.clone(),
            mismatches: self.mismatches(),
            locked: self.locked.clone(),
            native_options: native_options(&self.ruleset, &self.capabilities),
        }
    }

    fn changed(&mut self) {
        self.version += 1;
        for player in &mut self.players {
            player.ready_for = None;
        }
    }
}
