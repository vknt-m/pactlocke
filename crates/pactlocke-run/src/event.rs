use pactlocke_game::Area;
use serde::{Deserialize, Serialize};

/// Something that happened in the Run, as the Run log records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    /// The Run began.
    Started {
        /// The name of the game the Run is played on.
        game_name: String,
    },
    /// The Run was opened again; the game was booted afresh `from` there.
    Resumed {
        /// Where the game was booted from.
        #[serde(default)]
        from: ResumedFrom,
    },
    /// A checkpoint was stored with this event; `hash` is its SHA-1 (save
    /// memory, savestate and patch set). A Run Checkpoint (any reason but
    /// a boot) goes in the shared Run log, with the boot and frame boundary
    /// it was taken at, so every app checkpoints that Player's Shadow Game there.
    Checkpoint {
        /// Why the checkpoint was taken.
        reason: CheckpointReason,
        /// The checkpoint's SHA-1.
        hash: String,
        /// The boot of the game it was taken in.
        #[serde(default)]
        boot: u32,
        /// The frame boundary it was taken at.
        #[serde(default)]
        frame: u64,
        /// When it was taken (Unix milliseconds; 0 in a Run file from before this was kept).
        #[serde(default)]
        at_ms: i64,
        /// The Area the game was in, by name, if it was in one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        area: Option<String>,
    },
    /// A Rescue: the game went back to the Rescue Point whose event is
    /// `to_seq` (in this Player's own Run file) and whose hash is `hash`,
    /// taken at `to_ms` in `area`. Everything this Player recorded after it
    /// is void, as for a Retry, but no Retry is spent. In the shared Run log
    /// it says this Player's game is back.
    Rescued {
        /// The Rescue Point's event, in this Player's own Run file.
        to_seq: u64,
        /// The Rescue Point's SHA-1.
        hash: String,
        /// When the Rescue Point was taken (Unix milliseconds).
        to_ms: i64,
        /// The Area the game was in there, by name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        area: Option<String>,
    },
    /// A Retry: the game went back to the Run Checkpoint whose event is
    /// `to_seq` (in this Player's own Run file) and whose hash is `hash`.
    /// Everything this Player recorded after it is void. In the shared Run
    /// log it says this Player's game is back.
    Retried {
        /// The Run Checkpoint's event, in this Player's own Run file.
        to_seq: u64,
        /// The Run Checkpoint's SHA-1.
        hash: String,
    },
    /// A Retry was called for but couldn't be made.
    RetryFailed {
        /// Why the Retry couldn't be made.
        reason: String,
    },
    /// A Rescue was called for but couldn't be made.
    RescueFailed {
        /// Why the Rescue couldn't be made.
        reason: String,
    },
    /// The battle that was a source's Encounter is over: `pid` is the
    /// Pokémon caught in it, if one was. With Links, this Player's part of
    /// the source's Link.
    EncounterOver {
        /// The source whose Encounter it was.
        source: SourceRef,
        /// The Pokémon caught in it, if one was.
        pid: Option<u32>,
    },
    /// The Player chose to skip this Gift: its Link breaks for everyone.
    Skipped {
        /// The Gift skipped.
        source: SourceRef,
    },
    /// Links: another Player missed, skipped or lost this source's
    /// Encounter, so it is closed here too (`why` says whose).
    SourceClosed {
        /// The source closed.
        source: SourceRef,
        /// Whose Encounter was missed, skipped or lost, as a sentence.
        why: String,
    },
    /// The Player changed their speed (1 to 4, times a GBA's).
    SpeedChanged {
        /// The speed now, 1 to 4 times a GBA's.
        speed: u8,
    },
    /// Another Player changed their speed, as the Run log says.
    OthersSpeed {
        /// The other Player's name.
        player: String,
        /// Their speed now, 1 to 4 times a GBA's.
        speed: u8,
    },
    /// The Player came to this Sync Gate and is Ready there; it stays so.
    GateReady {
        /// The Sync Gate's name.
        gate: String,
    },
    /// The Player came to this Sync Gate but can't ready up, and why.
    GateRefused {
        /// The Sync Gate's name.
        gate: String,
        /// Why the Player can't ready up.
        reason: String,
    },
    /// The Player's game is past this Sync Gate's battle though the gate
    /// held: it counts as Ready there, so the gate lets the others go.
    GatePassed {
        /// The Sync Gate's name.
        gate: String,
    },
    /// A Player's vote on letting this Sync Gate go without everyone Ready:
    /// the first Yes starts the vote, everyone must say Yes, and the first
    /// No ends it.
    GateVote {
        /// The Sync Gate's name.
        gate: String,
        /// Whether this Player says Yes.
        yes: bool,
    },
    /// Every Player is Ready at this Sync Gate, or everyone voted to let it
    /// go: its battle runs here now.
    GateOpened {
        /// The Sync Gate's name.
        gate: String,
        /// Let go by everyone's vote, not by everyone being Ready.
        #[serde(default)]
        by_vote: bool,
    },
    /// A Sync Gate couldn't be held or let go in this game.
    GateFailed {
        /// The Sync Gate's name.
        gate: String,
        /// Why it couldn't be held or let go.
        reason: String,
    },
    /// A Player's vote on dropping `player` from the Run for good: the first
    /// Yes starts the vote, everyone but `player` must say Yes, and the
    /// first No ends it.
    DropoutVote {
        /// The Player the vote is on dropping.
        player: [u8; 32],
        /// Whether this Player says Yes.
        yes: bool,
    },
    /// This Player was dropped from the Run by the others' vote.
    DroppedOut,
    /// A Player's vote on a Retry back to the `call`th Run Checkpoint: the
    /// first Yes starts the vote, and the first No ends it.
    RetryVote {
        /// Which Run Checkpoint in the Run log, counting from 1.
        call: u64,
        /// Whether this Player says Yes.
        yes: bool,
    },
    /// A Player's vote on a Rescue back to the `call`th Run Checkpoint, a
    /// Rescue Point: the first Yes starts the vote, everyone must say Yes,
    /// and the first No ends it.
    RescueVote {
        /// Which Run Checkpoint in the Run log, counting from 1.
        call: u64,
        /// Whether this Player says Yes.
        yes: bool,
    },
    /// The Player suspended and quit; the suspend slot holds where.
    Suspended,
    /// The Run was closed cleanly. A boot that ends without this or
    /// `Suspended` ended in a crash.
    Closed,
    /// The game was reset to an older save, or a new game was started (its
    /// play time ran backwards).
    GameReset,
    /// A cheat flag, in the Run log: what `player`'s game did that it can't
    /// have done honestly, at that boot and frame boundary, seen at `at_ms`
    /// by the Player who logs it. `None`: about the Player who logs it, whose
    /// own app saw it (a tampered Run file). Only a vote clears it (`FlagVote`).
    CheatFlag {
        /// Whose game, or `None` for the Player who logs it.
        player: Option<[u8; 32]>,
        /// What their game did.
        cheat: crate::Cheat,
        /// The boot of their game, and the frame boundary in it.
        boot: u32,
        /// The frame boundary in that boot.
        frame: u64,
        /// When it was seen (Unix milliseconds).
        at_ms: i64,
    },
    /// A Player's vote on clearing the cheat flag on `player` for `cheat` in
    /// their game's `boot`: everyone else still in the Run must say Yes, the
    /// flagged Player has no say, and the first No ends the vote.
    FlagVote {
        /// The flagged Player.
        player: [u8; 32],
        /// What the flag is for.
        cheat: crate::Cheat,
        /// The boot of their game the flag was raised in.
        boot: u32,
        /// Whether this Player says Yes.
        yes: bool,
    },
    /// A cheat flag the Run log says was cleared by everyone's vote, for the feed.
    FlagCleared {
        /// Whose game: their name, or "you".
        player: String,
        /// The frame boundary in their game it was raised at.
        frame: u64,
    },
    /// A cheat flag in the Run log, for the feed.
    Flagged {
        /// Whose game: their name, or "you".
        player: String,
        /// What their game did.
        cheat: crate::Cheat,
        /// The frame boundary in their game.
        frame: u64,
        /// Who saw it: their name, "you", or "their own app".
        by: String,
    },
    /// Another Player's Shadow Game here stopped matching their game at
    /// this frame boundary, so it can't be watched for now.
    OutOfSync {
        /// The other Player's name.
        player: String,
        /// The frame boundary their Shadow Game stopped matching at.
        frame: u64,
    },
    /// The Player first holds Poké Balls; wild Encounters count from here on.
    PokeBallsObtained,
    /// A wild battle before the Player had Poké Balls: no Encounter is used.
    NotCounted {
        /// The Area the battle was in.
        area: AreaRef,
        /// The wild Pokémon's species number.
        species: u16,
        /// Its species name, as the game shows it.
        species_name: String,
    },
    /// The first wild battle in an Area: its one Encounter is used.
    Encounter {
        /// The Area whose Encounter is used.
        area: AreaRef,
        /// The wild Pokémon's species number.
        species: u16,
        /// Its species name, as the game shows it.
        species_name: String,
    },
    /// A Pokémon joined the party or PC from a wild battle.
    Caught {
        /// The Area it was caught in, if the Game Profile lists it.
        area: Option<AreaRef>,
        /// The Pokémon's personality value.
        pid: u32,
        /// Its species number.
        species: u16,
        /// Its species name, as the game shows it.
        species_name: String,
        /// When it happened, in Unix milliseconds: for how long each Pokémon
        /// survived (0 in logs from before it was kept).
        #[serde(default)]
        at_ms: i64,
    },
    /// The dupes clause: a wild battle with a Pokémon from a family the Player
    /// has alive doesn't use the Encounter, and ball throws are refused for it.
    DupeSkipped {
        /// The Area the battle was in.
        area: AreaRef,
        /// The wild Pokémon's species number.
        species: u16,
        /// Its species name, as the game shows it.
        species_name: String,
        /// The species of that family the Player has alive.
        dupe_of: String,
    },
    /// A wild battle in a closed Area: ball throws are refused for it.
    CatchesBlocked {
        /// The closed Area.
        area: AreaRef,
        /// The wild Pokémon's species number.
        species: u16,
        /// Its species name, as the game shows it.
        species_name: String,
    },
    /// A block the rules called for couldn't be applied.
    BlockFailed {
        /// The Area the block was for, if it was for one.
        area: Option<AreaRef>,
        /// Why it couldn't be applied.
        reason: String,
    },
    /// A script gave the Player a Pokémon. `source` is the catalogued Gift it
    /// matched, the starter, or for a Gift the Game Profile doesn't list, one
    /// keyed by its Area and species ([`SourceRef::unlisted_gift`]). `None` in logs from
    /// before unlisted Gifts had a source.
    Gift {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its species number.
        species: u16,
        /// Its species name, as the game shows it.
        species_name: String,
        /// The Area it was given in, if the Game Profile lists it.
        area: Option<AreaRef>,
        /// The catalogued Gift it matched, if any.
        source: Option<SourceRef>,
        /// Whether it is the starter, which is always allowed.
        starter: bool,
        /// An egg; one no catalogue lists was bred at the Day Care.
        #[serde(default)]
        egg: bool,
        /// When it happened, in Unix milliseconds: for how long each Pokémon
        /// survived (0 in logs from before it was kept).
        #[serde(default)]
        at_ms: i64,
    },
    /// An in-game trade: the Player gave `gave` and received this Pokémon, a Gift.
    Traded {
        /// The species name of the Pokémon the Player gave.
        gave: String,
        /// The received Pokémon's personality value.
        pid: u32,
        /// Its species number.
        species: u16,
        /// Its species name, as the game shows it.
        species_name: String,
        /// The Area the trade was in, if the Game Profile lists it.
        area: Option<AreaRef>,
        /// When it happened, in Unix milliseconds (0 in logs from before it was kept).
        #[serde(default)]
        at_ms: i64,
    },
    /// The Ruleset doesn't let the Player keep this Gift, so it is to be released.
    GiftDeclined {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its species name, as the game shows it.
        species_name: String,
        /// Why the Gift isn't kept.
        reason: DeclineReason,
    },
    /// A declined Gift was released.
    Released {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its species name, as the game shows it.
        species_name: String,
        /// When it happened, in Unix milliseconds (0 in logs from before it was kept).
        #[serde(default)]
        at_ms: i64,
    },
    /// A Violation: a declined Gift couldn't be released.
    ReleaseFailed {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its species name, as the game shows it.
        species_name: String,
        /// Why it couldn't be released.
        reason: String,
    },
    /// The Statics rule: a Static is a bonus Encounter of its own.
    StaticEncounter {
        /// The Area the battle was in, if the Game Profile lists it.
        area: Option<AreaRef>,
        /// The Static whose Encounter is used.
        source: SourceRef,
        /// The Static's species number.
        species: u16,
        /// Its species name, as the game shows it.
        species_name: String,
    },
    /// A Static whose Encounter is used already: ball throws are refused for it.
    StaticBlocked {
        /// The Area the battle was in, if the Game Profile lists it.
        area: Option<AreaRef>,
        /// The Static whose Encounter was used before.
        source: SourceRef,
        /// The Static's species number.
        species: u16,
        /// Its species name, as the game shows it.
        species_name: String,
    },
    /// A Pokémon fainted, so it is Dead for the rest of the Run.
    Died {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its species number.
        species: u16,
        /// Its species name, as the game shows it.
        species_name: String,
        /// Its nickname, or its species name if it has none.
        name: String,
        /// The Area it died in, if the Game Profile lists it.
        area: Option<AreaRef>,
        /// What made it faint.
        cause: DeathCause,
        /// When it happened, in Unix milliseconds: for how long each Pokémon
        /// survived (0 in logs from before it was kept).
        #[serde(default)]
        at_ms: i64,
    },
    /// Faint = Dead: every Pokémon in the Player's party is Dead, the last
    /// being `last`, where and how it died. A Retry is offered if one is
    /// left; otherwise it is a game over.
    PartyWiped {
        /// The last Pokémon to die, with where and how.
        last: crate::LastDeath,
    },
    /// The Run is lost: `how` it ended, and each Player's Pokémon that
    /// survived the longest.
    GameOver {
        /// How the Run ended, as a sentence.
        how: String,
        /// Each Player's Pokémon that survived the longest.
        longest: Vec<crate::Survivor>,
        /// This Player's play time by their game's own clock, in milliseconds.
        #[serde(default)]
        play_ms: i64,
    },
    /// A Dead Pokémon was moved from the party to the Dead box.
    MovedToDeadBox {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its nickname, or its species name if it has none.
        name: String,
        /// The box it went to, counting from 1.
        box_number: u8,
    },
    /// A Violation: a Dead Pokémon was back in the party, and was moved to the Dead box again.
    DeadReturned {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its nickname, or its species name if it has none.
        name: String,
        /// The box it went to, counting from 1.
        box_number: u8,
    },
    /// A Violation: the Dead box was full, so a Dead Pokémon went to another box.
    DeadBoxFull {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its nickname, or its species name if it has none.
        name: String,
        /// The box it went to instead, counting from 1.
        box_number: u8,
    },
    /// A Violation: a Dead Pokémon couldn't be moved out of the party.
    DeadBoxFailed {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its nickname, or its species name if it has none.
        name: String,
        /// Why it couldn't be moved.
        reason: String,
    },
    /// The Player is at level cap stage `stage` (from 0), so the cap is
    /// `level` until they pass `next`; with `next` `None` every stage is
    /// passed and there is no cap. `raised` when it follows an earlier stage.
    LevelCapSet {
        /// The stage the Player is at, counting from 0.
        stage: usize,
        /// The cap until the stage is passed.
        level: u8,
        /// Who the Player must beat to pass the stage, if anyone is left.
        next: Option<String>,
        /// Whether the cap follows an earlier stage's.
        raised: bool,
    },
    /// The game's level checks can't be patched, so the cap is only a soft
    /// cap: a Player over it can't ready up at a Sync Gate.
    SoftCap {
        /// Why the level checks can't be patched.
        reason: String,
    },
    /// No items: the Bag is refused in trainer battles and only Poké Balls
    /// work in wild battles. Said once per Run.
    ItemsBanned,
    /// No items couldn't be put in place on this game. Said once per Run.
    ItemBanFailed {
        /// Why it couldn't be put in place.
        reason: String,
    },
    /// Set mode: the battle style was written to Set; `again` when the
    /// Player had changed it back.
    SetModeForced {
        /// Whether the Player had changed it back.
        again: bool,
    },
    /// Set mode couldn't be forced on this game. Said once per Run.
    SetModeFailed {
        /// Why it couldn't be forced.
        reason: String,
    },
    /// A rule the game does through its own option: the option was set for
    /// the rule (said once per Run), or set back (`again`) after the Player
    /// changed a watched one.
    NativeOptionSet {
        /// The rule's name as Players see it.
        rule: String,
        /// The option's in-game name.
        name: String,
        /// The setting the rule needs.
        value: String,
        /// Whether the Player had changed it and it was set back.
        again: bool,
        /// Pactlocke enforces the rule too: the option goes with its own method.
        #[serde(default)]
        pactlocke_too: bool,
    },
    /// The option couldn't be set. Said once per Run for each option.
    NativeOptionFailed {
        /// The rule's name as Players see it.
        rule: String,
        /// The option's in-game name.
        name: String,
        /// Why it couldn't be set.
        reason: String,
    },
    /// Shared Seed: the game's own randomizer has the Run's seed. Said once per Run.
    SeedShared,
    /// The game left its map music paused on the overworld, so Pactlocke
    /// started it again: a warp to a map with other music would have waited
    /// for it forever. `stuck_warp` when a warp was already waiting, and the
    /// music was unpaused so the warp went on.
    MapMusicRestarted {
        /// Whether a warp was already stuck on it.
        stuck_warp: bool,
    },
    /// The Run's seed couldn't be given to the game. Said once per Run.
    SeedShareFailed {
        /// Why the seed couldn't be given.
        reason: String,
    },
    /// Must nickname: a Pokémon was left with its species name, so the side
    /// panel asks the Player for one.
    NicknameWanted {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its species name, as the game shows it.
        species_name: String,
    },
    /// The Player named it from the side panel.
    Nicknamed {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its species name, as the game shows it.
        species_name: String,
        /// The nickname the Player gave it.
        name: String,
    },
    /// A name the Player gave couldn't be used.
    NicknameRefused {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its species name, as the game shows it.
        species_name: String,
        /// The name the Player asked for.
        name: String,
        /// Why it couldn't be used.
        reason: String,
    },
    /// The type clause: a second Pokémon of a primary type in the party was
    /// moved to the PC. `holder` has that type in the party.
    TypeClauseMoved {
        /// The moved Pokémon's personality value.
        pid: u32,
        /// Its nickname, or its species name if it has none.
        name: String,
        /// The name of the Pokémon that has that type in the party.
        holder: String,
        /// The type's name, as the game shows it.
        type_name: String,
        /// The box it went to, counting from 1.
        box_number: u8,
    },
    /// Links: a Pokémon waiting on its Link went to the PC, as the party is
    /// full. `until` finishes the sentence: "until Blue catches on Route 1".
    WaitsForLink {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its nickname, or its species name if it has none.
        name: String,
        /// What it waits for, finishing the sentence.
        until: String,
        /// The box it went to, counting from 1.
        box_number: u8,
    },
    /// Links: a Pokémon waiting on its Link couldn't be moved out of a full party.
    LinkWaitFailed {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its nickname, or its species name if it has none.
        name: String,
        /// Why it couldn't be moved.
        reason: String,
    },
    /// A Violation: the type clause couldn't move a Pokémon out of the party.
    TypeClauseFailed {
        /// The Pokémon's personality value.
        pid: u32,
        /// Its nickname, or its species name if it has none.
        name: String,
        /// The type's name, as the game shows it.
        type_name: String,
        /// Why it couldn't be moved.
        reason: String,
    },
}

/// Where a resumed Run's game was booted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "boot")]
pub enum ResumedFrom {
    /// The last in-game save, as a console would after being switched off.
    #[default]
    Save,
    /// No in-game save yet: a new game from the start.
    Start,
    /// The suspend slot: exactly where the Player suspended.
    Suspend,
    /// After a crash: the last checkpoint and the button presses recorded after it.
    Rebuild {
        /// How many frames of recorded button presses were replayed.
        frames: u64,
    },
}

/// Why a Run Checkpoint was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointReason {
    /// The game was booted: a boot checkpoint, only for rebuilding after a
    /// crash, never a Retry point.
    Boot,
    /// The Player saved in-game.
    InGameSave,
    /// Asked for at a moment that needs one.
    Asked,
    /// Every Player was Ready at a Sync Gate.
    SyncGate,
    /// Another Player's Run Checkpoint, the `call`th in the Run log (from
    /// 1): every Player's game is captured with it.
    Joined {
        /// Which Run Checkpoint in the Run log, counting from 1.
        call: u64,
    },
    /// A Rescue Point: called about every [`crate::RESCUE_POINT_EVERY_MS`],
    /// by the Host in a Run with friends, at a quiet overworld moment.
    RescuePoint,
    /// Another Player's Rescue Point, the `call`th Run Checkpoint in the Run
    /// log: this Player's part of it, taken at a quiet overworld moment.
    JoinedRescuePoint {
        /// Which Run Checkpoint in the Run log, counting from 1.
        call: u64,
    },
}

impl CheckpointReason {
    /// Part of a Rescue Point: the Run file keeps the newest
    /// [`crate::RESCUE_POINTS_KEPT`] of these apart from other Run Checkpoints.
    pub fn is_rescue_point(&self) -> bool {
        matches!(self, Self::RescuePoint | Self::JoinedRescuePoint { .. })
    }
}

impl RunEvent {
    /// Run state, which goes in the Run log every Player shares: Encounters,
    /// catches and Gifts, releases, Deaths and Violations. The rest (a
    /// feed note, a checkpoint, a boot) is this Player's own.
    pub fn is_run_state(&self) -> bool {
        matches!(
            self,
            RunEvent::Encounter { .. }
                | RunEvent::StaticEncounter { .. }
                | RunEvent::Caught { .. }
                | RunEvent::Gift { .. }
                | RunEvent::Traded { .. }
                | RunEvent::Released { .. }
                | RunEvent::Died { .. }
                | RunEvent::PartyWiped { .. }
                | RunEvent::RetryVote { .. }
                | RunEvent::RescueVote { .. }
                | RunEvent::DropoutVote { .. }
                | RunEvent::FlagVote { .. }
                | RunEvent::Retried { .. }
                | RunEvent::Rescued { .. }
                | RunEvent::EncounterOver { .. }
                | RunEvent::Skipped { .. }
                | RunEvent::GateReady { .. }
                | RunEvent::GatePassed { .. }
                | RunEvent::GateVote { .. }
                | RunEvent::SpeedChanged { .. }
                | RunEvent::CheatFlag { .. }
        ) || self.is_run_checkpoint()
            || self.is_violation()
    }

    /// The Sync Gate this makes its Player Ready at: a Ready, or a gate
    /// their game went past.
    pub fn readies(&self) -> Option<&str> {
        match self {
            RunEvent::GateReady { gate } | RunEvent::GatePassed { gate } => Some(gate),
            _ => None,
        }
    }

    /// A Run Checkpoint: one a Retry can go back to (not a boot checkpoint).
    pub fn is_run_checkpoint(&self) -> bool {
        matches!(self, RunEvent::Checkpoint { reason, .. } if *reason != CheckpointReason::Boot)
    }

    /// A Retry or a Rescue: the game went back to the Run Checkpoint stored
    /// with event `to_seq`, whose hash this is.
    pub fn went_back(&self) -> Option<(u64, &str)> {
        match self {
            RunEvent::Retried { to_seq, hash } | RunEvent::Rescued { to_seq, hash, .. } => Some((*to_seq, hash)),
            _ => None,
        }
    }

    /// What a Retry never undoes: a cheat flag stands whatever happens after it.
    pub fn outlives_retries(&self) -> bool {
        matches!(self, RunEvent::CheatFlag { .. })
    }

    /// A rule broken in a way the app couldn't stop, shown in red to everyone.
    pub fn is_violation(&self) -> bool {
        matches!(
            self,
            RunEvent::DeadReturned { .. }
                | RunEvent::DeadBoxFull { .. }
                | RunEvent::DeadBoxFailed { .. }
                | RunEvent::ReleaseFailed { .. }
                | RunEvent::TypeClauseFailed { .. }
        )
    }
}

/// A Gift or Static source, as the log keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    /// The same in every Player's game for the same Gift or Static.
    pub key: String,
    /// The source's name as Players see it.
    pub name: String,
}

impl SourceRef {
    /// The start of the key of a Gift the Game Profile doesn't list.
    pub const UNLISTED_GIFT: &str = "unknown_gift_";

    /// Whether this is a Gift the Game Profile doesn't list, keyed by its Area and species.
    pub fn unlisted_gift(&self) -> bool {
        self.key.starts_with(Self::UNLISTED_GIFT)
    }

    /// An Area's own Encounter, as a Link source.
    pub fn area(area: &AreaRef) -> Self {
        Self {
            key: format!("area:{}", area.id),
            name: area.name.clone(),
        }
    }

    /// The Area it is, if it is an Area's Encounter.
    pub fn area_id(&self) -> Option<u16> {
        self.key.strip_prefix("area:")?.parse().ok()
    }
}

impl From<pactlocke_game::Source> for SourceRef {
    fn from(source: pactlocke_game::Source) -> Self {
        Self {
            key: source.key,
            name: source.name,
        }
    }
}

/// Why a Gift isn't kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclineReason {
    /// The Ruleset allows no Gifts but the starter.
    GiftsOff,
    /// The Gift's source has given its one Encounter already.
    SourceUsed,
}

/// What made a Pokémon faint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum DeathCause {
    /// A wild Pokémon; `foe` is its species name.
    Wild {
        /// The wild Pokémon's species name.
        foe: String,
    },
    /// A trainer's Pokémon; `foe` is its species name.
    Trainer {
        /// The trainer's Pokémon's species name.
        foe: String,
    },
    /// Fainting outside battle, which only poison does.
    Poison,
    /// Its Link broke in another Player's game: `why`, such as "Blue's
    /// PIDGEY died" or "Blue didn't catch it".
    Link {
        /// What broke the Link, as a sentence.
        why: String,
    },
}

/// An Area as the log keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AreaRef {
    /// The Area's region-map section.
    pub id: u16,
    /// The Area's name as Players see it.
    pub name: String,
}

impl From<&Area> for AreaRef {
    fn from(area: &Area) -> Self {
        Self {
            id: area.id,
            name: area.name.clone(),
        }
    }
}

/// An event with where and when it happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorded {
    /// Its place in the Run log, from 1.
    pub seq: u64,
    /// Which time the Run was opened (1 for the first), since frames count
    /// from power-on in each.
    pub boot: u32,
    /// The frame boundary it was decided at.
    pub frame: u64,
    /// Unix time in milliseconds.
    pub at_ms: i64,
    /// What happened.
    pub event: RunEvent,
}
