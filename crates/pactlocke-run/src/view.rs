//! What the side panel shows of a Run. The UI only displays it.

use crate::event::{AreaRef, CheckpointReason, DeathCause, DeclineReason, Recorded, ResumedFrom, RunEvent};

/// Everything the side panel shows of a Run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunView {
    /// The Run's name.
    pub name: String,
    /// The name of the game it is played on.
    pub game_name: String,
    /// Newest first.
    pub feed: Vec<FeedEntry>,
    /// Each Area whose Encounter is used, in the order they closed: what was
    /// met there and whether it was caught.
    pub encounters: Vec<crate::AreaEncounter>,
    /// The Dead, newest first.
    pub graveyard: Vec<Grave>,
    /// The level cap now, in a Run with level caps.
    pub level_cap: Option<LevelCapView>,
    /// Must nickname: the Pokémon the side panel asks the Player to name, oldest first.
    pub nickname_wanted: Vec<NicknameWanted>,
    /// In a Run with Retries: how many this Player's own pool has left (a
    /// Run with other Players counts them in the shared log instead).
    pub retries_left: Option<u8>,
    /// Whether there is a Run Checkpoint a Retry can go back to.
    pub run_checkpoint: bool,
    /// The Player's party, as of the last look.
    pub party: Vec<crate::SeenPokemon>,
    /// Where the Player is and what they are doing, for the Player strip.
    pub area: Option<String>,
    /// What the Player is doing, for the Player strip.
    pub doing: Doing,
    /// The Area status of the listed Area the Player is in, in a Run with
    /// the first-Encounter rule.
    pub area_status: Option<crate::AreaStatus>,
    /// Faint = Dead: this Player's party is wiped, and the game has stopped.
    pub wipe: Option<WipeView>,
    /// The Run is lost.
    pub game_over: Option<GameOverView>,
    /// The Ruleset's rules this game does through its own options, as of the last look.
    pub native_options: Vec<NativeOptionView>,
}

impl RunView {
    /// The Areas whose Encounter is used, in the order they closed.
    pub fn closed_areas(&self) -> Vec<String> {
        self.encounters.iter().map(|e| e.area.clone()).collect()
    }
}

/// A rule the game does through its own option, as setup and the side
/// panel show it: the setting it needs, and what the game reads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NativeOptionView {
    /// The Pactlocke rule, as Players see it: "Set mode".
    pub rule: String,
    /// The option's in-game name: "BATTLE STYLE".
    pub name: String,
    /// Where it is in the game's menus: "OPTIONS ▸ BATTLE STYLE".
    pub menu: String,
    /// The setting the rule needs.
    pub value: String,
    /// What the game holds now, if it holds a setting the Game Profile
    /// knows; `None` before the Run, in setup.
    pub read: Option<String>,
}

impl NativeOptionView {
    /// The view of a native option, with the setting the game holds now.
    pub fn of(native: &pactlocke_game::NativeOption, read: Option<String>) -> Self {
        Self {
            rule: native.rule.name().to_owned(),
            name: native.name.clone(),
            menu: native.menu.clone(),
            value: native.value.clone(),
            read,
        }
    }
}

/// This Player's party is wiped: the game waits on a Retry or the game over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WipeView {
    /// "Your party was wiped on Route 2: …".
    pub how: String,
    /// Alone, with a Retry left and a Run Checkpoint to go back to: the
    /// Player may Retry. With friends, the Run log's Retry vote decides.
    pub retry_offered: bool,
}

/// How a lost Run ended, for the game over screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameOverView {
    /// How the Run ended, as a sentence.
    pub how: String,
    /// Each Player's longest survivor, in join order.
    pub longest: Vec<crate::Survivor>,
    /// Unix time in milliseconds.
    pub at_ms: i64,
    /// This Player's play time by their game's own clock, in milliseconds.
    pub play_ms: i64,
}

/// What a Player is doing, as the Player strip shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Doing {
    /// Walking around, or anything else out of battle.
    #[default]
    Exploring,
    /// In a battle.
    InBattle,
    /// Held at a Sync Gate's hold script.
    AtGate(String),
}

impl Doing {
    /// From whether the Player is in battle and the Sync Gate they are held at.
    pub fn from(in_battle: bool, at_gate: Option<String>) -> Self {
        match (in_battle, at_gate) {
            (true, _) => Doing::InBattle,
            (false, Some(gate)) => Doing::AtGate(gate),
            (false, None) => Doing::Exploring,
        }
    }
}

impl std::fmt::Display for Doing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Doing::Exploring => write!(f, "Exploring"),
            Doing::InBattle => write!(f, "In battle"),
            Doing::AtGate(gate) => write!(f, "At {gate}'s Sync Gate"),
        }
    }
}

/// A Pokémon left with its species name, waiting for the Player to name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NicknameWanted {
    /// The Pokémon's personality value.
    pub pid: u32,
    /// Its species name, as the game shows it.
    pub species_name: String,
}

/// The current level cap, as the side panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelCapView {
    /// 100 once every stage is passed.
    pub level: u8,
    /// Who the cap is for, until they are passed.
    pub next: Option<String>,
    /// Only a soft cap: the game itself doesn't stop Pokémon at it.
    pub soft: bool,
}

/// A Dead Pokémon, as the Graveyard tab shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grave {
    /// The Pokémon's personality value, which identifies it for the whole Run.
    pub pid: u32,
    /// Its species number, for its party icon.
    pub species: u16,
    /// Its nickname, or its species name if it had none.
    pub name: String,
    /// Its species name, as the game shows it.
    pub species_name: String,
    /// What killed it and where, such as "a wild RATTATA on Route 1".
    pub killed_by: String,
    /// Unix time in milliseconds.
    pub at_ms: i64,
}

impl Grave {
    /// The grave a recorded Death makes; `None` for any other event.
    pub fn of(recorded: &Recorded) -> Option<Self> {
        Self::of_event(&recorded.event, recorded.at_ms)
    }

    /// The grave a Death makes, seen at `at_ms`; `None` for any other event.
    fn of_event(event: &RunEvent, at_ms: i64) -> Option<Self> {
        let RunEvent::Died {
            pid,
            species,
            name,
            species_name,
            area,
            cause,
            ..
        } = event
        else {
            return None;
        };
        Some(Self {
            pid: *pid,
            species: *species,
            name: name.clone(),
            species_name: species_name.clone(),
            killed_by: killed_by(cause, area.as_ref()),
            at_ms,
        })
    }
}

/// Every Player's Dead, newest first, as the shared Run log has them: the
/// Deaths a Retry undid aren't among them.
pub fn graves(log: &crate::RunLog) -> std::collections::BTreeMap<crate::PlayerKey, Vec<Grave>> {
    let mut all: std::collections::BTreeMap<crate::PlayerKey, Vec<Grave>> = Default::default();
    for (author, event) in log.standing_events() {
        let at_ms = match &event {
            RunEvent::Died { at_ms, .. } => *at_ms,
            _ => 0,
        };
        if let Some(grave) = Grave::of_event(&event, at_ms) {
            all.entry(author).or_default().insert(0, grave);
        }
    }
    all
}

/// "Route 1: …", or the text alone outside a listed Area.
fn at(area: Option<&AreaRef>, text: String) -> String {
    match area {
        Some(area) => format!("{}: {text}", area.name),
        None => {
            let mut chars = text.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// "on Route 1", "on Cycling Road", but "in Pallet Town" and "in Viridian Forest".
pub(crate) fn place(area: &AreaRef) -> String {
    if area.name.starts_with("Route") || area.name.ends_with("Road") {
        format!("on {}", area.name)
    } else {
        format!("in {}", area.name)
    }
}

fn killed_by(cause: &DeathCause, area: Option<&AreaRef>) -> String {
    let what = match cause {
        DeathCause::Wild { foe } => format!("a wild {foe}"),
        DeathCause::Trainer { foe } => format!("a trainer's {foe}"),
        DeathCause::Poison => "poison".to_owned(),
        // Its Link, wherever it was.
        DeathCause::Link { why } => return format!("its Link: {why}"),
    };
    match area {
        Some(area) => format!("{what} {}", place(area)),
        None => what,
    }
}

/// One line of the feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEntry {
    /// Unix time in milliseconds.
    pub at_ms: i64,
    /// What kind of line it is, which sets how it is shown.
    pub kind: FeedKind,
    /// The small icon the feed shows beside it.
    pub icon: FeedIcon,
    /// The other Player it is about, by name; `None` for this Player.
    pub about: Option<String>,
    /// The line itself, as the Player reads it.
    pub text: String,
}

/// What a feed line's icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedIcon {
    /// A catch.
    Catch,
    /// An Encounter.
    Encounter,
    /// A Death.
    Death,
    /// A Link, broken or closed.
    Link,
    /// A Sync Gate.
    Gate,
    /// A Retry.
    Retry,
    /// A Player's speed.
    Speed,
    /// A Violation.
    Violation,
    /// A cheat flag.
    Cheat,
    /// A Gift or trade.
    Gift,
    /// An in-game save.
    Save,
    /// Something the rules refused, or a problem putting a rule in place.
    Block,
    /// Anything else: the Run itself.
    Run,
}

impl FeedIcon {
    fn of(event: &RunEvent, kind: FeedKind) -> Self {
        match event {
            RunEvent::Caught { .. } => FeedIcon::Catch,
            RunEvent::Encounter { .. } | RunEvent::StaticEncounter { .. } | RunEvent::NotCounted { .. } => {
                FeedIcon::Encounter
            }
            RunEvent::Died {
                cause: DeathCause::Link { .. },
                ..
            }
            | RunEvent::SourceClosed { .. }
            | RunEvent::Skipped { .. } => FeedIcon::Link,
            RunEvent::GateReady { .. } | RunEvent::GateOpened { .. } | RunEvent::GateRefused { .. } => FeedIcon::Gate,
            RunEvent::Retried { .. } | RunEvent::RetryFailed { .. } => FeedIcon::Retry,
            RunEvent::SpeedChanged { .. } | RunEvent::OthersSpeed { .. } => FeedIcon::Speed,
            RunEvent::CheatFlag { .. } | RunEvent::Flagged { .. } | RunEvent::FlagCleared { .. } => FeedIcon::Cheat,
            RunEvent::Checkpoint { .. } => FeedIcon::Save,
            _ => match kind {
                FeedKind::Death => FeedIcon::Death,
                FeedKind::Violation => FeedIcon::Violation,
                // A problem isn't anyone's fault: no accusing icon.
                FeedKind::Problem => FeedIcon::Block,
                FeedKind::Gift => FeedIcon::Gift,
                FeedKind::Block => FeedIcon::Block,
                FeedKind::Catch => FeedIcon::Catch,
                FeedKind::Encounter => FeedIcon::Encounter,
                FeedKind::Cheat => FeedIcon::Cheat,
                FeedKind::Run => FeedIcon::Run,
            },
        }
    }
}

/// What kind of feed line an event makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    /// The Run itself: started, resumed, a rule put in place.
    Run,
    /// An Encounter used.
    Encounter,
    /// A catch.
    Catch,
    /// Something the rules refused: a ball throw, a ready-up, a source closed.
    Block,
    /// Something the app couldn't do, which is nobody's fault.
    Problem,
    /// A Death, or a Dead Pokémon moved to the Dead box.
    Death,
    /// A Gift or trade received, declined or released.
    Gift,
    /// A rule broken in a way the app couldn't stop; shown in red.
    Violation,
    /// A cheat flag: shown to every Player, apart from Violations.
    Cheat,
}

impl FeedEntry {
    /// What the feed says of another Player's event from the shared Run
    /// log, seen at `at_ms`. `None` for one the feed doesn't show, one
    /// written to the Player who made it ("You play at 3x"), or one every
    /// Player's own feed shows already (a Retry, a Sync Gate opening).
    pub fn of_others(event: &RunEvent, at_ms: i64) -> Option<Self> {
        if matches!(
            event,
            RunEvent::SpeedChanged { .. }
                | RunEvent::CheatFlag { .. }
                | RunEvent::DroppedOut
                | RunEvent::GateRefused { .. }
                | RunEvent::GateOpened { .. }
                | RunEvent::Retried { .. }
                | RunEvent::RetryFailed { .. }
        ) {
            return None;
        }
        Self::of(&Recorded {
            seq: 0,
            boot: 0,
            frame: 0,
            at_ms,
            event: event.clone(),
        })
    }

    /// What the feed says of an event; some, such as checkpoints, it doesn't show.
    pub fn of(recorded: &Recorded) -> Option<Self> {
        let (kind, text) = match &recorded.event {
            RunEvent::Started { game_name } => (FeedKind::Run, format!("Run started on {game_name}")),
            RunEvent::Resumed { from } => (
                FeedKind::Run,
                match from {
                    ResumedFrom::Save => "Run resumed from your last in-game save".to_owned(),
                    ResumedFrom::Start => "Run resumed from the start: the game hasn't been saved yet".to_owned(),
                    ResumedFrom::Suspend => "Run resumed where you suspended it".to_owned(),
                    ResumedFrom::Rebuild { frames } => format!(
                        "Run recovered after the app closed unexpectedly: rebuilt from the last checkpoint and \
                         {:.0} s of button presses",
                        *frames as f64 / 60.0
                    ),
                },
            ),
            RunEvent::Checkpoint {
                reason: CheckpointReason::InGameSave,
                ..
            } => (FeedKind::Run, "Saved in-game: a Run Checkpoint is kept".to_owned()),
            // The Area status says so instead, once, not at every battle.
            RunEvent::CatchesBlocked { .. } => return None,
            RunEvent::Checkpoint { .. }
            | RunEvent::Closed
            | RunEvent::RetryVote { .. }
            | RunEvent::DropoutVote { .. }
            | RunEvent::FlagVote { .. } => return None,
            RunEvent::FlagCleared { player, frame } => (
                FeedKind::Run,
                format!("The cheat flag on {player}, at frame {frame}, was cleared by everyone's vote"),
            ),
            RunEvent::DroppedOut => (
                FeedKind::Problem,
                "The other Players dropped you from the Run: it goes on without you".to_owned(),
            ),
            // The catch or the way it ended is in the feed already.
            RunEvent::EncounterOver { .. } => return None,
            RunEvent::SpeedChanged { speed } => (FeedKind::Run, format!("You play at {speed}x")),
            RunEvent::OthersSpeed { player, speed } => (FeedKind::Run, format!("{player} plays at {speed}x")),
            RunEvent::GateReady { gate } => (
                FeedKind::Run,
                format!("Ready at {gate}: waiting for your partners to be ready"),
            ),
            RunEvent::GateRefused { gate, reason } => (FeedKind::Block, format!("Can't ready up at {gate}: {reason}")),
            RunEvent::GateOpened { gate } => (FeedKind::Run, format!("Everyone is Ready at {gate}: the battle is on")),
            RunEvent::GateFailed { gate, reason } => (
                FeedKind::Problem,
                format!("The Sync Gate at {gate} doesn't work in this game: {reason}"),
            ),
            RunEvent::Skipped { source } => (
                FeedKind::Block,
                format!("You skipped {}: its Link is broken for everyone", source.name),
            ),
            RunEvent::SourceClosed { source, why } => {
                (FeedKind::Block, format!("{} is closed for you too: {why}", source.name))
            }
            RunEvent::RetryFailed { reason } => (FeedKind::Problem, format!("The Retry couldn't be made: {reason}")),
            RunEvent::Retried { .. } => (
                FeedKind::Run,
                "Retry: back to the latest Run Checkpoint, and everything since is undone".to_owned(),
            ),
            RunEvent::Suspended => (FeedKind::Run, "Run suspended".to_owned()),
            RunEvent::GameReset => (FeedKind::Run, "The game was reset to an older save".to_owned()),
            // Another Player's flag reaches the feed through the Run log, as `Flagged`.
            RunEvent::CheatFlag { player: Some(_), .. } => return None,
            RunEvent::CheatFlag {
                player: None,
                cheat,
                frame,
                ..
            } => (
                FeedKind::Cheat,
                format!("Cheat flag on you, at frame {frame}: {}", cheat.describe()),
            ),
            RunEvent::Flagged {
                player,
                cheat,
                frame,
                by,
            } => (
                FeedKind::Cheat,
                format!(
                    "Cheat flag on {player}, at frame {frame}: {} (seen by {by})",
                    cheat.describe()
                ),
            ),
            RunEvent::OutOfSync { player, .. } => (
                FeedKind::Problem,
                format!("Can't watch {player} right now: their game and its copy here are out of sync"),
            ),
            RunEvent::PokeBallsObtained => (
                FeedKind::Run,
                "You have Poké Balls: wild Encounters count from now on".to_owned(),
            ),
            RunEvent::NotCounted { area, species_name, .. } => (
                FeedKind::Encounter,
                format!("{}: a wild {species_name}, not counted: no Poké Balls yet", area.name),
            ),
            RunEvent::Encounter { area, species_name, .. } => (
                FeedKind::Encounter,
                format!("{}: a wild {species_name}, this Area's Encounter", area.name),
            ),
            RunEvent::Caught { area, species_name, .. } => (
                FeedKind::Catch,
                match area {
                    Some(area) => format!("Caught {species_name} on {}", area.name),
                    None => format!("Caught {species_name}"),
                },
            ),
            RunEvent::DupeSkipped {
                area,
                species_name,
                dupe_of,
                ..
            } => (
                FeedKind::Encounter,
                format!(
                    "{}: a wild {species_name}, not counted: a dupe of your {dupe_of}, so no catching it (running is fine)",
                    area.name
                ),
            ),
            RunEvent::BlockFailed { area, reason } => (
                FeedKind::Problem,
                at(area.as_ref(), format!("couldn't block catching: {reason}")),
            ),
            RunEvent::Gift {
                species_name,
                area,
                source,
                starter,
                egg,
                ..
            } => (
                FeedKind::Gift,
                at(
                    area.as_ref(),
                    match (source, starter) {
                        // Before the script key, an unlisted Gift had no source.
                        (None, _) if *egg => "received an Egg from the Day Care".to_owned(),
                        (Some(source), false) if source.unlisted_gift() && *egg => {
                            "received an Egg the Game Profile doesn't list (a Gift, or the Day Care's)".to_owned()
                        }
                        (Some(source), false) if source.unlisted_gift() => {
                            format!("received {species_name}, an unknown Gift (the Game Profile doesn't list it)")
                        }
                        (Some(source), true) => format!("received {species_name}, the {}", source.name),
                        (Some(source), false) => format!("received {species_name} from the {}", source.name),
                        (None, _) => {
                            format!("received {species_name}, an unknown Gift (the Game Profile doesn't list it)")
                        }
                    },
                ),
            ),
            RunEvent::Traded {
                gave,
                species_name,
                area,
                ..
            } => (
                FeedKind::Gift,
                at(area.as_ref(), format!("traded {gave} for {species_name}, a Gift")),
            ),
            RunEvent::GiftDeclined {
                species_name, reason, ..
            } => (
                FeedKind::Gift,
                match reason {
                    DeclineReason::GiftsOff => format!(
                        "{species_name} can't be kept: this Ruleset allows no Gifts but the starter, so it is released"
                    ),
                    DeclineReason::SourceUsed => {
                        format!("{species_name} can't be kept: that Gift's Encounter is used, so it is released")
                    }
                },
            ),
            RunEvent::Released { species_name, .. } => (FeedKind::Gift, format!("Released {species_name}")),
            RunEvent::ReleaseFailed {
                species_name, reason, ..
            } => (
                FeedKind::Violation,
                format!("Violation: {species_name} should have been released but couldn't be: {reason}"),
            ),
            RunEvent::StaticEncounter {
                area,
                source,
                species_name,
                ..
            } => (
                FeedKind::Encounter,
                at(
                    area.as_ref(),
                    format!("{species_name}, the {}: a bonus Static Encounter", source.name),
                ),
            ),
            RunEvent::StaticBlocked {
                area,
                source,
                species_name,
                ..
            } => (
                FeedKind::Block,
                at(
                    area.as_ref(),
                    format!(
                        "{species_name}, the {}: this Static's Encounter is used, so no catching it (running is fine)",
                        source.name
                    ),
                ),
            ),
            RunEvent::Died { name, area, cause, .. } => {
                let how = match cause {
                    DeathCause::Poison => "fainted from ",
                    DeathCause::Link { .. } => "fainted with ",
                    _ => "fainted against ",
                };
                (
                    FeedKind::Death,
                    format!("{name} {how}{}, so {name} is Dead", killed_by(cause, area.as_ref())),
                )
            }
            RunEvent::MovedToDeadBox { name, box_number, .. } => (
                FeedKind::Death,
                format!("Moved {name} to box {box_number}, the Dead box: Dead Pokémon can't be used again"),
            ),
            RunEvent::DeadReturned { name, box_number, .. } => (
                FeedKind::Violation,
                format!("Violation: {name} is Dead but was back in the party, so it went back to box {box_number}"),
            ),
            RunEvent::DeadBoxFull { name, box_number, .. } => (
                FeedKind::Violation,
                format!("Violation: {name} is Dead but the Dead box is full, so it went to box {box_number}"),
            ),
            RunEvent::DeadBoxFailed { name, reason, .. } => (
                FeedKind::Violation,
                format!("Violation: {name} is Dead but couldn't be moved out of the party: {reason}"),
            ),
            RunEvent::LevelCapSet {
                level, next, raised, ..
            } => (
                FeedKind::Run,
                match next {
                    Some(next) if *raised => format!("Level cap raised to {level} until {next}"),
                    Some(next) => format!("Level cap {level} until {next}"),
                    None => "No level cap any more: every cap is passed".to_owned(),
                },
            ),
            RunEvent::ItemsBanned => (
                FeedKind::Run,
                "No items in battle: the Bag is refused in trainer battles, and only Poké Balls work in wild battles"
                    .to_owned(),
            ),
            RunEvent::ItemBanFailed { reason } => {
                (FeedKind::Problem, format!("Couldn't ban items in battle: {reason}"))
            }
            RunEvent::SetModeForced { again: false } => {
                (FeedKind::Run, "Set mode: the battle style is set to Set".to_owned())
            }
            RunEvent::SetModeForced { again: true } => (
                FeedKind::Run,
                "Set mode: the battle style was changed, so it is set back to Set".to_owned(),
            ),
            RunEvent::SetModeFailed { reason } => (FeedKind::Problem, format!("Couldn't force set mode: {reason}")),
            RunEvent::NativeOptionSet {
                rule,
                name,
                value,
                again: false,
                pactlocke_too: true,
            } => (
                FeedKind::Run,
                format!("{rule}: Pactlocke sets the game's {name} option to {value} as well"),
            ),
            RunEvent::NativeOptionSet {
                rule,
                name,
                value,
                again: false,
                pactlocke_too: false,
            } => (
                FeedKind::Run,
                format!("{rule} is the game's own {name} option, set to {value}"),
            ),
            RunEvent::NativeOptionSet {
                rule,
                name,
                value,
                again: true,
                ..
            } => (
                FeedKind::Run,
                format!("{rule}: the game's {name} was changed, so it is set back to {value}"),
            ),
            RunEvent::MapMusicRestarted { stuck_warp: false } => (
                FeedKind::Run,
                "The game left its music paused, so it was started again (the next exit would have stayed black)"
                    .to_owned(),
            ),
            RunEvent::MapMusicRestarted { stuck_warp: true } => (
                FeedKind::Run,
                "An exit was stuck on a black screen waiting for paused music, so the music was unpaused".to_owned(),
            ),
            RunEvent::SeedShared => (
                FeedKind::Run,
                "Shared Seed: the game's own randomizer has the Run's seed".to_owned(),
            ),
            RunEvent::SeedShareFailed { reason } => (
                FeedKind::Problem,
                format!("Shared Seed: the game couldn't be given the Run's seed: {reason}"),
            ),
            RunEvent::NativeOptionFailed { rule, name, reason } => (
                FeedKind::Problem,
                format!("{rule}: couldn't set the game's {name} option: {reason}"),
            ),
            RunEvent::NicknameWanted { species_name, .. } => (
                FeedKind::Run,
                format!("Must nickname: {species_name} needs a nickname, so name it in the side panel"),
            ),
            RunEvent::Nicknamed { species_name, name, .. } => (FeedKind::Run, format!("Named {species_name} {name}")),
            RunEvent::NicknameRefused {
                species_name,
                name,
                reason,
                ..
            } => (
                FeedKind::Problem,
                format!("Couldn't name {species_name} {name:?}: {reason}"),
            ),
            RunEvent::TypeClauseMoved {
                name,
                holder,
                type_name,
                box_number,
                ..
            } => (
                FeedKind::Block,
                format!(
                    "Type clause: your {holder} is the {type_name} type in your party, so {name} went to box {box_number}"
                ),
            ),
            RunEvent::PartyWiped { last } => (FeedKind::Death, last.wipe_line("Your")),
            RunEvent::GameOver { how, .. } => (FeedKind::Run, format!("Game over. {how}")),
            RunEvent::WaitsForLink { name, until, .. } => (
                FeedKind::Block,
                format!("{name} waits in the PC {until}: your party is full"),
            ),
            RunEvent::LinkWaitFailed { name, reason, .. } => (
                FeedKind::Problem,
                format!("{name} waits on its Link but couldn't be moved out of your full party: {reason}"),
            ),
            RunEvent::TypeClauseFailed {
                name,
                type_name,
                reason,
                ..
            } => (
                FeedKind::Violation,
                format!(
                    "Violation: {name} is a second {type_name} type in your party but couldn't be moved to the PC: {reason}"
                ),
            ),
            RunEvent::SoftCap { reason } => (
                FeedKind::Run,
                format!(
                    "The level cap is a soft cap on this game ({reason}): \
                     a Player with a Pokémon over the cap can't ready up at a Sync Gate"
                ),
            ),
        };
        let about = match &recorded.event {
            RunEvent::OthersSpeed { player, .. } => Some(player.clone()),
            RunEvent::Flagged { player, .. } | RunEvent::FlagCleared { player, .. } if player != "you" => {
                Some(player.clone())
            }
            _ => None,
        };
        Some(Self {
            at_ms: recorded.at_ms,
            kind,
            icon: FeedIcon::of(&recorded.event, kind),
            about,
            text,
        })
    }
}
