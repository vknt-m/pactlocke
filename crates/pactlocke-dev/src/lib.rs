//! Developer-only tools, never shipped to Players. The scanner drafts a Game
//! Profile from a ROM and a base profile it is likely laid out like: tables
//! found by their contents, patch sites checked and found by byte pattern,
//! Gift and Static candidates from the game's script instructions, and Sync
//! Gate candidates by trainer class. A person reviews every entry, and the
//! profile's acceptance scripts prove it, before it can ship.
#![warn(missing_docs)]

mod scan;

pub use scan::{Findings, GateCandidate, ScriptSpecies, SiteFinding, SourceCandidate, TableFinding, scan};
