//! What the side panel shows of a Run. The UI only displays it.

use crate::event::{Recorded, RunEvent};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunView {
    pub name: String,
    pub game_name: String,
    /// Newest first.
    pub feed: Vec<FeedEntry>,
    /// The Areas whose Encounter is used, in the order they closed.
    pub closed_areas: Vec<String>,
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
            RunEvent::CatchesBlocked { area, species_name, .. } => (
                FeedKind::Block,
                format!(
                    "{}: Encounter already used, so no catching this {species_name} (running is fine)",
                    area.name
                ),
            ),
            RunEvent::BlockFailed { area, reason } => (
                FeedKind::Problem,
                format!("{}: couldn't block catching: {reason}", area.name),
            ),
        };
        Self {
            at_ms: recorded.at_ms,
            kind,
            text,
        }
    }
}
