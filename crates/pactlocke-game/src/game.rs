//! The game interface: the rules engine's view of one Player's game. Reads
//! report the game's state; actions return the memory writes and ROM patches
//! that would make something happen, and never apply them. Only the Run engine
//! applies them, stamped to the frame boundary where they land.

use std::fmt;

use pactlocke_emu::CoreAdapter;

use crate::{Area, Source};

/// Read access to a game's address space.
pub trait Memory {
    /// Fills `out` with the bytes at `addr`.
    fn read(&mut self, addr: u32, out: &mut [u8]);
}

impl<C: CoreAdapter> Memory for C {
    fn read(&mut self, addr: u32, out: &mut [u8]) {
        CoreAdapter::read(self, addr, out);
    }
}

/// One change an action asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Bytes written through the bus (RAM).
    Write {
        /// Where the bytes go.
        addr: u32,
        /// The bytes to put there.
        bytes: Vec<u8>,
    },
    /// ROM bytes replaced, until patched back.
    Patch {
        /// Where the bytes go.
        addr: u32,
        /// The bytes to put there.
        bytes: Vec<u8>,
    },
}

/// Where a Player stands: the map (group and number) and the tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Whereabouts {
    /// The map's group and number.
    pub map: (u8, u8),
    /// The tile's column.
    pub x: i16,
    /// The tile's row.
    pub y: i16,
}

/// A sprite's look: its 4bpp tiles (in 1D order), its 16-colour palette,
/// whether it is mirrored, and its priority against the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteImage {
    /// The 4bpp tiles, in 1D order.
    pub tiles: Vec<u8>,
    /// The 16-colour palette, as the GBA holds it.
    pub palette: Vec<u8>,
    /// Whether it is mirrored left to right.
    pub hflip: bool,
    /// Its priority against the background.
    pub priority: u8,
    /// 32x32 (a bike, surfing, a party icon) rather than 16x32.
    pub square: bool,
}

/// Hardware OAM on the GBA's bus.
pub const OAM: u32 = 0x0700_0000;
/// Hardware OBJ tiles on the GBA's bus.
pub const OBJ_TILES: u32 = 0x0601_0000;
/// Hardware OBJ palettes on the GBA's bus.
pub const OBJ_PALETTES: u32 = 0x0500_0200;

/// The Pokémon that leads the party: the first one alive that isn't an egg.
pub fn lead_of(party: &[Pokemon]) -> Option<&Pokemon> {
    party.iter().find(|p| !p.is_egg && p.stats.is_some_and(|s| s.hp > 0))
}

/// Where a game lets Partner Avatars be drawn (see the profile's `[avatars]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarRoom {
    /// Where the Player's own 16x32 sprite's top-left corner is on screen.
    pub anchor: (u16, u16),
    /// The first hardware OAM entry the game never fills on the overworld.
    pub first_oam: u8,
    /// The first 4bpp OBJ tile Pactlocke may draw into.
    pub first_tile: u16,
    /// The OBJ palette slots Pactlocke may load.
    pub palettes: Vec<u8>,
}

/// A Pokémon in the party or the PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pokemon {
    /// The personality value, which identifies the Pokémon for the whole Run.
    pub pid: u32,
    /// The original trainer's ID.
    pub ot_id: u32,
    /// The species number.
    pub species: u16,
    /// The nickname, as the game holds it.
    pub nickname: String,
    /// The region-map section it was met in (the game's "met location").
    pub met_section: u16,
    /// Whether it is still an egg.
    pub is_egg: bool,
    /// Only party Pokémon carry these.
    pub stats: Option<PartyStats>,
}

/// What only a party Pokémon carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartyStats {
    /// Its level.
    pub level: u8,
    /// Its HP now; 0 is fainted.
    pub hp: u16,
    /// Its HP when full.
    pub max_hp: u16,
}

/// A Pokémon in the PC; box and slot count from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPokemon {
    /// The box, counting from 1.
    pub box_number: u8,
    /// The slot in the box, counting from 1.
    pub slot: u8,
    /// The Pokémon there.
    pub pokemon: Pokemon,
}

/// What kind of battle is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleKind {
    /// Against a wild Pokémon.
    Wild,
    /// Against a trainer.
    Trainer,
    /// A battle nobody can catch in, such as a catching demo or a ghost: no one's Encounter.
    Uncatchable,
}

/// Whose side of a battle a battler is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The Player's side.
    Player,
    /// The opponent's side.
    Opponent,
}

/// The battle that is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battle {
    /// What kind of battle it is.
    pub kind: BattleKind,
    /// Whether it is a double battle.
    pub double: bool,
    /// The Pokémon out now, on both sides.
    pub battlers: Vec<Battler>,
}

/// A Pokémon out in battle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battler {
    /// The game's battler position: even for the Player's side, odd for the opponent's.
    pub position: u8,
    /// Whose side it is on.
    pub side: Side,
    /// The species number.
    pub species: u16,
    /// Its HP now; 0 is fainted.
    pub hp: u16,
    /// The party slot (from 0) of a Player's battler.
    pub party_slot: Option<u8>,
}

/// What the game's script engine is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptStatus {
    /// No script is running.
    Idle,
    /// A script is running.
    Running,
    /// A script is waiting, such as on a message box or a battle it started.
    Waiting,
}

/// The game's script engine: what it is doing, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptContext {
    /// Whether a script is running or waiting.
    pub status: ScriptStatus,
    /// The script instruction it is at, while not idle.
    pub position: Option<u32>,
}

/// The map's music: the song the game has as playing, what its map-music
/// state machine is doing, and whether the music player is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapMusic {
    /// The song the game has as the map's music (`sCurrentMapMusic`); 0 for none.
    pub song: u16,
    /// What the map-music state machine is doing (`sMapMusicState`).
    pub state: MapMusicState,
    /// Whether the music player for map music (`gMPlayInfo_BGM`) is paused.
    pub paused: bool,
}

/// What the map-music state machine is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapMusicState {
    /// The song is playing, and nothing is changing it (state 2).
    Playing,
    /// Waiting for the old song to stop before a warp goes on (state 5,
    /// `FadeOutMapMusic`).
    WaitingToStop,
    /// Starting a song, or fading between songs.
    Other(u8),
}

impl MapMusicState {
    /// The state machine's byte as the game keeps it.
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            2 => Self::Playing,
            5 => Self::WaitingToStop,
            other => Self::Other(other),
        }
    }
}

/// The badges a Player holds, one bit per badge in game order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Badges(pub u8);

impl Badges {
    /// How many badges the Player holds.
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    /// Whether the Player has badge `n` (counting from 1).
    pub fn has(self, n: u8) -> bool {
        (1..=8).contains(&n) && self.0 & (1 << (n - 1)) != 0
    }
}

/// Why an action can't be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameError {
    /// The Game Profile has no method for this rule.
    NoMethod(&'static str),
    /// The Game Profile lists no Sync Gate by this name.
    NoSuchGate(String),
    /// No Pokémon in that party slot.
    NoSuchSlot(u8),
    /// The PC has no box of that number.
    NoSuchBox(u8),
    /// That box has no empty slot.
    BoxFull(u8),
    /// No battle is on.
    NotInBattle,
    /// No battler at that position.
    NoSuchBattler(u8),
    /// Not a level from 1 to 100.
    BadLevel(u8),
    /// The name is empty, too long, or uses characters the game lacks.
    BadName(String),
    /// The game has no option of that name, or the option no such setting.
    UnknownOption(String),
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMethod(rule) => write!(f, "the Game Profile has no way to enforce {rule}"),
            Self::NoSuchGate(name) => write!(f, "the Game Profile has no Sync Gate called {name}"),
            Self::NoSuchSlot(slot) => write!(f, "no Pokémon in party slot {slot}"),
            Self::NoSuchBox(number) => write!(f, "there is no box {number}"),
            Self::BoxFull(number) => write!(f, "box {number} is full"),
            Self::NotInBattle => write!(f, "not in a battle"),
            Self::NoSuchBattler(position) => write!(f, "no battler at position {position}"),
            Self::BadLevel(level) => write!(f, "{level} is not a level from 1 to 100"),
            Self::BadName(why) => write!(f, "{why}"),
            Self::UnknownOption(option) => write!(f, "no option {option}"),
        }
    }
}

impl std::error::Error for GameError {}

/// The changes an action asks for, or why it can't be taken.
pub type Actions = Result<Vec<Change>, GameError>;

/// What the rules engine can read from a game and ask of it.
pub trait Game {
    // Reads.

    /// The Area the Player is in, or `None` in a section the profile doesn't list.
    fn area(&mut self) -> Option<Area>;
    /// The Player's party, in slot order.
    fn party(&mut self) -> Vec<Pokemon>;
    /// Every Pokémon in the PC.
    fn pc(&mut self) -> Vec<StoredPokemon>;
    /// The battle that is on, if one is.
    fn battle(&mut self) -> Option<Battle>;
    /// The opponent's party. The game fills it in before a battle starts,
    /// while the battlers still hold the last battle's Pokémon for a while.
    fn opponent_party(&mut self) -> Vec<Pokemon>;
    /// What the game's script engine is doing, and where.
    fn script(&mut self) -> ScriptContext;
    /// Where a running script returns to from the scripts it has called,
    /// innermost first: a Gift's own script may be a call or two up.
    fn script_calls(&mut self) -> Vec<u32>;
    /// The badges the Player holds.
    fn badges(&mut self) -> Badges;
    /// Which level cap stage the Player is at: how many of the Game Profile's
    /// stages, in order, they are past (by badges, then Elite Four defeats).
    fn cap_stage(&mut self) -> usize;
    /// The named option's current setting, if the game has that option and
    /// it holds a known setting.
    fn option(&mut self, name: &str) -> Option<String>;
    /// The Player's trainer ID.
    fn trainer_id(&mut self) -> u32;
    /// The game's play time in frames. It only runs backwards when the game
    /// is reset to an older save.
    fn play_time(&mut self) -> u32;
    /// A species' primary type, from the loaded ROM (randomizers change types).
    fn primary_type(&mut self, species: u16) -> u8;
    /// A species' name as the game shows it, from the loaded ROM.
    fn species_name(&mut self, species: u16) -> String;
    /// A type's name as the game shows it, from the loaded ROM.
    fn type_name(&mut self, type_id: u8) -> String;
    /// For each species from 0, the species it can evolve into, from the
    /// loaded ROM (randomizers change evolutions).
    fn evolutions(&mut self) -> Vec<Vec<u16>>;
    /// The catalogued Gift or Static whose instruction is nearest before this
    /// script position, if one is close enough.
    fn source_before(&mut self, position: u32) -> Option<Source>;
    /// On the overworld and out of battle: where Pokémon can be moved safely.
    fn at_safe_overworld_point(&mut self) -> bool;
    /// Whether the Bag holds any kind of Poké Ball.
    fn holds_poke_balls(&mut self) -> bool;
    /// Whether "Give a nickname?" is up with its YES/NO box: in battle after
    /// a catch, or on the overworld for a Gift.
    fn nickname_prompt_up(&mut self) -> bool;

    // Actions. Each returns changes for the Run engine to apply.

    /// Refuses (or allows again) ball throws, with the game's "time and place" line.
    fn block_catches(&mut self, blocked: bool) -> Actions;
    /// EXP and Rare Candy stop at `level`.
    fn set_level_cap(&mut self, level: u8) -> Actions;
    /// Only balls keep a use in wild battles.
    fn ban_items_in_wild_battles(&mut self, banned: bool) -> Actions;
    /// The Bag is refused in trainer battles.
    fn lock_bag_in_trainer_battles(&mut self, locked: bool) -> Actions;
    /// The battler faints through the game's own Perish Song path at the end of the turn.
    fn faint_active_battler(&mut self, position: u8) -> Actions;
    /// A party Pokémon not in battle faints at once.
    fn faint_benched(&mut self, party_slot: u8) -> Actions;
    /// Moves a party Pokémon to the first empty slot of a PC box (counting from 1).
    fn move_to_box(&mut self, party_slot: u8, box_number: u8) -> Actions;
    /// Releases a party Pokémon: it leaves the game, and the party closes up.
    fn release_from_party(&mut self, party_slot: u8) -> Actions;
    /// Releases a Pokémon in the PC (box and slot counting from 1).
    fn release_from_pc(&mut self, box_number: u8, slot: u8) -> Actions;
    /// The PC box (counting from 1) that holds the Dead.
    fn dead_box(&mut self) -> u8;
    /// Moves a party Pokémon to the Dead box.
    fn move_to_dead_box(&mut self, party_slot: u8) -> Actions;
    /// Names a party Pokémon.
    fn write_nickname(&mut self, party_slot: u8, name: &str) -> Actions;
    /// Names a Pokémon in the PC (box and slot counting from 1).
    fn write_pc_nickname(&mut self, box_number: u8, slot: u8, name: &str) -> Actions;
    /// Keeps the nickname prompt's cursor on YES, where the game has one to keep.
    fn keep_nickname_cursor_on_yes(&mut self) -> Actions;
    /// The writes that put the battle style on Set through the game's own
    /// option; none when it is on Set already.
    fn force_set_mode(&mut self) -> Actions;
    /// Sets one of the game's options, such as a native option written back.
    fn set_option(&mut self, name: &str, value: &str) -> Actions;
    /// The rules this game does through its own options.
    fn native_options(&mut self) -> Vec<crate::NativeOption>;

    // Map music.

    /// The map's music, on a game whose Game Profile says where it is kept.
    fn map_music(&mut self) -> Option<MapMusic>;
    /// Starts the map's song again, as `PlayNewMapMusic` does: the game
    /// plays it afresh, unpaused, on its next sound tick.
    fn restart_map_music(&mut self) -> Actions;
    /// Unpauses the music player for map music, so a fade-out set on it runs.
    fn unpause_map_music(&mut self) -> Actions;

    // Partner Avatars.

    /// Where the Player stands, on the overworld, if the game says (only the
    /// original games' profiles know).
    fn whereabouts(&mut self) -> Option<Whereabouts>;
    /// The Player's own sprite as the game drew it last: its tiles, palette
    /// and flip, to draw them in another Player's game.
    fn player_sprite(&mut self) -> Option<SpriteImage>;
    /// A species' party icon, its first frame: what a Follower looks like.
    fn party_icon(&mut self, species: u16) -> Option<SpriteImage>;
    /// Where this game lets Pactlocke draw Partner Avatars: none on a Romhack.
    fn avatar_room(&mut self) -> Option<AvatarRoom>;

    // Sync Gates.

    /// Every battle the Game Profile lists as a possible Sync Gate, by name, in order.
    fn sync_gates(&mut self) -> Vec<String>;
    /// Holds the gate: its battle's `trainerbattle` becomes a jump to a hold
    /// script that says "Waiting for your partners to be ready..." and
    /// lets the Player go.
    fn close_gate(&mut self, gate: &str) -> Actions;
    /// Lets the gate's real battle run again: the original bytes back.
    fn open_gate(&mut self, gate: &str) -> Actions;
    /// The gate whose hold script is running, if one is: the Player has
    /// come to it while it is closed.
    fn at_gate(&mut self) -> Option<String>;
    /// Whether the Player is past this gate's battle: the Game Profile's
    /// level cap stage of the same name is passed (its badge, or its defeat
    /// flag). `false` where the profile has no such stage.
    fn gate_passed(&mut self, gate: &str) -> bool;

    // Shared Seed on a Romhack's own randomizer.

    /// Whether the game has its own randomizer Shared Seed can share.
    fn has_own_randomizer(&mut self) -> bool;
    /// What gives the Romhack's own randomizer `seed`: the seed where the game
    /// keeps it, its draws made to depend on it alone, and the modes no seed
    /// makes the same kept off. Nothing for what is so already.
    fn share_seed(&mut self, seed: u32) -> Actions;
    /// The RNG set from `seed` and the coming trainer, from a trainer
    /// battle's trigger until the enemy party is made; nothing otherwise.
    fn pin_rng(&mut self, seed: u32) -> Actions;
}
