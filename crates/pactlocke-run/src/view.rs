//! What the side panel shows of a Run. The UI only displays it.

use crate::event::{AreaRef, DeathCause, DeclineReason, Recorded, RunEvent};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunView {
    pub name: String,
    pub game_name: String,
    /// Newest first.
    pub feed: Vec<FeedEntry>,
    /// The Areas whose Encounter is used, in the order they closed.
    pub closed_areas: Vec<String>,
    /// The Dead, newest first.
    pub graveyard: Vec<Grave>,
    /// The level cap now, in a Run with level caps.
    pub level_cap: Option<LevelCapView>,
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
    pub name: String,
    pub species_name: String,
    /// What killed it and where, such as "a wild RATTATA on Route 1".
    pub killed_by: String,
    /// Unix time in milliseconds.
    pub at_ms: i64,
}

impl Grave {
    pub(crate) fn of(recorded: &Recorded) -> Option<Self> {
        let RunEvent::Died {
            name,
            species_name,
            area,
            cause,
            ..
        } = &recorded.event
        else {
            return None;
        };
        Some(Self {
            name: name.clone(),
            species_name: species_name.clone(),
            killed_by: killed_by(cause, area.as_ref()),
            at_ms: recorded.at_ms,
        })
    }
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

fn killed_by(cause: &DeathCause, area: Option<&AreaRef>) -> String {
    let what = match cause {
        DeathCause::Wild { foe } => format!("a wild {foe}"),
        DeathCause::Trainer { foe } => format!("a trainer's {foe}"),
        DeathCause::Poison => "poison".to_owned(),
    };
    match area {
        // "on Route 1", "on Cycling Road", but "in Pallet Town" and "in Viridian Forest".
        Some(area) if area.name.starts_with("Route") || area.name.ends_with("Road") => {
            format!("{what} on {}", area.name)
        }
        Some(area) => format!("{what} in {}", area.name),
        None => what,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEntry {
    /// Unix time in milliseconds.
    pub at_ms: i64,
    pub kind: FeedKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    Run,
    Encounter,
    Catch,
    Block,
    Problem,
    /// A Death, or a Dead Pokémon moved to the Dead box.
    Death,
    /// A Gift or trade received, declined or released.
    Gift,
    /// A rule broken in a way the app couldn't stop; shown in red.
    Violation,
}

impl FeedEntry {
    pub(crate) fn of(recorded: &Recorded) -> Self {
        let (kind, text) = match &recorded.event {
            RunEvent::Started { game_name } => (FeedKind::Run, format!("Run started on {game_name}")),
            RunEvent::Resumed => (FeedKind::Run, "Run resumed".to_owned()),
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
            RunEvent::CatchesBlocked { area, species_name, .. } => (
                FeedKind::Block,
                format!(
                    "{}: Encounter already used, so no catching this {species_name} (running is fine)",
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
                ..
            } => (
                FeedKind::Gift,
                at(
                    area.as_ref(),
                    match (source, starter) {
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
            RunEvent::SoftCap { reason } => (
                FeedKind::Run,
                format!(
                    "The level cap is a soft cap on this game ({reason}): \
                     a Player with a Pokémon over the cap can't ready up at a Sync Gate"
                ),
            ),
        };
        Self {
            at_ms: recorded.at_ms,
            kind,
            text,
        }
    }
}
