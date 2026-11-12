use std::collections::BTreeSet;

use pactlocke_emu::Keys;
use pactlocke_game::{Area, Battle, BattleKind, Change, Game, GameError, Pokemon, ScriptStatus, Side};

use crate::event::{AreaRef, DeathCause};
use crate::families::EvolutionFamilies;
use crate::rules::{BattleStart, DeadMove, Rules, Verdict};
use crate::storage::{InputBatch, RunInfo, RunStorage, Stamped, StorageError};
use crate::view::{FeedEntry, FeedKind, Grave, RunView};
use crate::{LiveGame, NewRun, Recorded, RunClock, RunEvent};

/// How often the input stream is flushed to storage, so a crash loses at most this much.
const FLUSH_EVERY_MS: i64 = 1000;

/// The Run engine for one Player's game. Call [`RunEngine::frame`] after
/// every emulated frame; it reads the game, decides what the rules say, and
/// applies any changes at that frame boundary.
pub struct RunEngine {
    info: RunInfo,
    rules: Rules,
    storage: Box<dyn RunStorage>,
    clock: Box<dyn RunClock>,
    /// Which time this Run has been opened, from 1.
    boot: u32,
    /// Frames run since the game was booted.
    frame: u64,
    next_seq: u64,
    battle: Option<BattleWatch>,
    /// The input stream not yet flushed.
    pending: InputBatch,
    last_flush_ms: i64,
    /// Every change stamped since the game was booted.
    stamped: Vec<Stamped>,
    /// The save memory as last stored, and as found on resume.
    save_memory: Option<Vec<u8>>,
    view: RunView,
    view_version: u64,
    /// Whether the feed has said the Run file can't be written.
    write_failure_shown: bool,
    /// Dead PIDs that couldn't be moved to the Dead box since the game was booted;
    /// they aren't tried again until it is booted afresh.
    dead_box_failed: BTreeSet<u32>,
}

/// A battle in progress, as the engine saw it begin.
struct BattleWatch {
    kind: BattleKind,
    script_idle: bool,
    area: Option<Area>,
    /// Every PID in the party and PC when it began.
    pids_before: BTreeSet<u32>,
    /// Their species.
    species_before: Vec<u16>,
    /// Whether the rules have looked at it yet; they wait until the opponent is known.
    decided: bool,
    catches_blocked: bool,
}

impl RunEngine {
    /// Starts a new Run and records it in `storage`.
    pub fn start(
        new: NewRun,
        mut storage: Box<dyn RunStorage>,
        clock: Box<dyn RunClock>,
    ) -> Result<Self, StorageError> {
        let info = RunInfo {
            name: new.name,
            game_name: new.game_name,
            game_sha1: new.game_sha1,
            rom_path: new.rom_path,
            ruleset: new.ruleset,
            created_ms: clock.now_ms(),
        };
        storage.create(&info)?;
        let mut engine = Self::with(info, storage, clock, 1, None);
        let game_name = engine.info.game_name.clone();
        engine.record(RunEvent::Started { game_name });
        Ok(engine)
    }

    /// Opens a Run again from its storage, for a game booted afresh.
    pub fn resume(mut storage: Box<dyn RunStorage>, clock: Box<dyn RunClock>) -> Result<Self, StorageError> {
        let stored = storage.load()?;
        let boot = stored.events.iter().map(|e| e.boot).max().unwrap_or(0) + 1;
        let mut engine = Self::with(stored.info, storage, clock, boot, stored.save_memory);
        for recorded in &stored.events {
            engine.take_in(recorded);
        }
        engine.next_seq = stored.events.iter().map(|e| e.seq).max().unwrap_or(0) + 1;
        engine.record(RunEvent::Resumed);
        Ok(engine)
    }

    fn with(
        info: RunInfo,
        storage: Box<dyn RunStorage>,
        clock: Box<dyn RunClock>,
        boot: u32,
        save_memory: Option<Vec<u8>>,
    ) -> Self {
        let now = clock.now_ms();
        let view = RunView {
            name: info.name.clone(),
            game_name: info.game_name.clone(),
            ..RunView::default()
        };
        Self {
            rules: Rules::new(info.ruleset),
            info,
            storage,
            clock,
            boot,
            frame: 0,
            next_seq: 1,
            battle: None,
            pending: InputBatch {
                boot,
                first_frame: 0,
                keys: Vec::new(),
                stamped: Vec::new(),
            },
            last_flush_ms: now,
            stamped: Vec::new(),
            save_memory,
            view,
            view_version: 0,
            write_failure_shown: false,
            dead_box_failed: BTreeSet::new(),
        }
    }

    pub fn info(&self) -> &RunInfo {
        &self.info
    }

    /// What the side panel shows.
    pub fn view(&self) -> &RunView {
        &self.view
    }

    /// Goes up whenever the view changes.
    pub fn view_version(&self) -> u64 {
        self.view_version
    }

    /// Every write and patch applied since the game was booted, with its frame boundary.
    pub fn stamped(&self) -> &[Stamped] {
        &self.stamped
    }

    /// Frames run since the game was booted.
    pub fn frames_run(&self) -> u64 {
        self.frame
    }

    /// The save memory the game should boot with: the one last stored.
    pub fn save_memory(&self) -> Option<&[u8]> {
        self.save_memory.as_deref()
    }

    /// Tells the engine what the game's save memory holds now; it is stored when it changed.
    pub fn save_memory_seen(&mut self, save: &[u8]) {
        if save.is_empty() || self.save_memory.as_deref() == Some(save) {
            return;
        }
        self.storage.store_save_memory(save);
        self.save_memory = Some(save.to_vec());
    }

    /// One frame has just run with `keys` held. Reads the game at this frame
    /// boundary and applies whatever the rules call for.
    pub fn frame(&mut self, keys: Keys, game: &mut impl LiveGame) {
        self.pending.keys.push(keys);
        self.frame += 1;

        // Only on the overworld: before a game is loaded the Bag reads as garbage.
        if self.rules.waiting_for_poke_balls() && game.at_safe_overworld_point() && game.holds_poke_balls() {
            self.record(RunEvent::PokeBallsObtained);
        }

        let battle = game.battle();
        match (self.battle.is_some(), battle.clone()) {
            (false, Some(battle)) => {
                let (pids_before, species_before) = owned(game);
                self.battle = Some(BattleWatch {
                    kind: battle.kind,
                    script_idle: game.script().status == ScriptStatus::Idle,
                    area: game.area(),
                    pids_before,
                    species_before,
                    decided: false,
                    catches_blocked: false,
                });
                self.decide(game);
            }
            (true, Some(_)) => self.decide(game),
            (true, None) => self.battle_ended(game),
            (false, None) => {}
        }

        if self.rules.faint_is_dead() {
            match &battle {
                Some(battle) => self.record_faints(game, Some(battle)),
                // Moved before the Player can heal it: the overworld is the first chance to.
                None if game.at_safe_overworld_point() => {
                    self.record_faints(game, None);
                    // A running script may hold a party slot, such as a trade's or the Day Care's.
                    if game.script().status == ScriptStatus::Idle {
                        self.move_the_dead(game);
                    }
                }
                None => {}
            }
        }

        if self.clock.now_ms() - self.last_flush_ms >= FLUSH_EVERY_MS {
            self.flush();
        }
    }

    /// Writes the input stream so far to storage.
    pub fn flush(&mut self) {
        self.last_flush_ms = self.clock.now_ms();
        self.show_write_failure();
        if self.pending.keys.is_empty() && self.pending.stamped.is_empty() {
            return;
        }
        let next = InputBatch {
            boot: self.boot,
            first_frame: self.frame,
            keys: Vec::new(),
            stamped: Vec::new(),
        };
        let batch = std::mem::replace(&mut self.pending, next);
        self.storage.record_inputs(&batch);
    }

    /// Says once in the feed that the Run file can't be written. It is the
    /// one feed entry that isn't a Run event, since the Run log can't hold it.
    fn show_write_failure(&mut self) {
        if self.write_failure_shown {
            return;
        }
        if let Some(why) = self.storage.write_failure() {
            self.write_failure_shown = true;
            self.view.feed.insert(
                0,
                FeedEntry {
                    at_ms: self.clock.now_ms(),
                    kind: FeedKind::Problem,
                    text: format!("The Run file can't be written, so this Run may not resume as it is now: {why}"),
                },
            );
            self.view_version += 1;
        }
    }

    /// Lets the rules look at a battle in progress, once its opponent is
    /// known. The opponent comes from their party, not the battlers, which
    /// hold the last battle's Pokémon until the battle intro fills them in.
    fn decide(&mut self, game: &mut impl LiveGame) {
        let watch = self.battle.as_ref().expect("a battle is being watched");
        if watch.decided {
            return;
        }
        let Some(species) = game
            .opponent_party()
            .first()
            .map(|pokemon| pokemon.species)
            .filter(|species| *species != 0)
        else {
            return;
        };
        if self.rules.wants_families() {
            self.rules
                .set_families(EvolutionFamilies::from_evolutions(&game.evolutions()));
        }
        let watch = self.battle.as_ref().expect("a battle is being watched");
        let verdict = self.rules.battle_started(&BattleStart {
            kind: watch.kind,
            script_idle: watch.script_idle,
            area: watch.area.as_ref(),
            foe: species,
            owned: &watch.species_before,
        });
        let area = watch.area.as_ref().map(AreaRef::from);
        self.battle.as_mut().unwrap().decided = true;

        match (verdict, area) {
            (Verdict::NotCounted, Some(area)) => {
                let species_name = game.species_name(species);
                self.record(RunEvent::NotCounted {
                    area,
                    species,
                    species_name,
                });
            }
            (Verdict::Encounter, Some(area)) => {
                let species_name = game.species_name(species);
                self.record(RunEvent::Encounter {
                    area,
                    species,
                    species_name,
                });
            }
            (Verdict::BlockCatches, Some(area)) => {
                let event = RunEvent::CatchesBlocked {
                    area: area.clone(),
                    species,
                    species_name: game.species_name(species),
                };
                self.block_catches_then(game, area, event);
            }
            (Verdict::Dupe { of }, Some(area)) => {
                let event = RunEvent::DupeSkipped {
                    area: area.clone(),
                    species,
                    species_name: game.species_name(species),
                    dupe_of: game.species_name(of),
                };
                self.block_catches_then(game, area, event);
            }
            _ => {}
        }
    }

    /// Refuses ball throws for this battle and records `event`, or records why it couldn't.
    fn block_catches_then(&mut self, game: &mut impl LiveGame, area: AreaRef, event: RunEvent) {
        match game.block_catches(true) {
            Ok(changes) => {
                self.apply(changes, game);
                self.battle.as_mut().expect("a battle is being watched").catches_blocked = true;
                self.record(event);
            }
            Err(e) => self.record(RunEvent::BlockFailed {
                area,
                reason: e.to_string(),
            }),
        }
    }

    fn battle_ended(&mut self, game: &mut impl LiveGame) {
        let watch = self.battle.take().expect("a battle was being watched");
        if watch.catches_blocked {
            // The block applied, so lifting it can't fail.
            if let Ok(changes) = game.block_catches(false) {
                self.apply(changes, game);
            }
        }
        if watch.kind != BattleKind::Wild {
            return;
        }
        let area = watch.area.as_ref().map(AreaRef::from);
        let party = game.party();
        let pc = game.pc().into_iter().map(|stored| stored.pokemon);
        for pokemon in party.into_iter().chain(pc) {
            if !watch.pids_before.contains(&pokemon.pid) {
                let species_name = game.species_name(pokemon.species);
                self.record(RunEvent::Caught {
                    area: area.clone(),
                    pid: pokemon.pid,
                    species: pokemon.species,
                    species_name,
                });
            }
        }
    }

    /// Records a Death for each party Pokémon newly at 0 HP. Outside battle
    /// only poison makes a Pokémon faint.
    fn record_faints(&mut self, game: &mut impl LiveGame, battle: Option<&Battle>) {
        let fainted: Vec<Pokemon> = game
            .party()
            .into_iter()
            .filter(|p| !p.is_egg && p.stats.is_some_and(|s| s.hp == 0) && !self.rules.is_dead(p.pid))
            .collect();
        if fainted.is_empty() {
            return;
        }
        let cause = match battle {
            None => DeathCause::Poison,
            Some(battle) => {
                let foe = foe_name(battle, game);
                match battle.kind {
                    BattleKind::Wild => DeathCause::Wild { foe },
                    BattleKind::Trainer => DeathCause::Trainer { foe },
                }
            }
        };
        let area = game.area().as_ref().map(AreaRef::from);
        for pokemon in fainted {
            let species_name = game.species_name(pokemon.species);
            self.record(RunEvent::Died {
                pid: pokemon.pid,
                species: pokemon.species,
                name: display_name(&pokemon, &species_name),
                species_name,
                area: area.clone(),
                cause: cause.clone(),
            });
        }
    }

    /// Moves every Dead Pokémon in the party to the Dead box, one at a time,
    /// since each move compacts the party.
    fn move_the_dead(&mut self, game: &mut impl LiveGame) {
        loop {
            let party = game.party();
            let Some((slot, pokemon, why)) = party.into_iter().enumerate().find_map(|(slot, p)| {
                let why = self
                    .rules
                    .dead_move(p.pid)
                    .filter(|_| !self.dead_box_failed.contains(&p.pid))?;
                Some((slot, p, why))
            }) else {
                return;
            };
            let species_name = game.species_name(pokemon.species);
            let name = display_name(&pokemon, &species_name);
            let pid = pokemon.pid;
            let slot = u8::try_from(slot).expect("a party has six slots");
            match move_to_dead_box_or_another(game, slot) {
                Ok((changes, dead_box_full)) => {
                    self.apply(changes, game);
                    let box_number = game
                        .pc()
                        .into_iter()
                        .find(|stored| stored.pokemon.pid == pid)
                        .map_or(0, |stored| stored.box_number);
                    self.record(match (dead_box_full, why) {
                        (true, _) => RunEvent::DeadBoxFull { pid, name, box_number },
                        (false, DeadMove::First) => RunEvent::MovedToDeadBox { pid, name, box_number },
                        (false, DeadMove::Returned) => RunEvent::DeadReturned { pid, name, box_number },
                    });
                }
                Err(e) => {
                    self.dead_box_failed.insert(pid);
                    self.record(RunEvent::DeadBoxFailed {
                        pid,
                        name,
                        reason: e.to_string(),
                    });
                }
            }
        }
    }

    /// Stamps each change with this frame boundary, then applies it.
    fn apply(&mut self, changes: Vec<Change>, game: &mut impl LiveGame) {
        for change in changes {
            game.apply(&change);
            let stamped = Stamped {
                frame: self.frame,
                change,
            };
            self.pending.stamped.push(stamped.clone());
            self.stamped.push(stamped);
        }
    }

    fn record(&mut self, event: RunEvent) {
        let recorded = Recorded {
            seq: self.next_seq,
            boot: self.boot,
            frame: self.frame,
            at_ms: self.clock.now_ms(),
            event,
        };
        self.next_seq += 1;
        self.storage.append(&recorded);
        self.take_in(&recorded);
    }

    /// Updates the rules and the view with an event.
    fn take_in(&mut self, recorded: &Recorded) {
        self.rules.record(&recorded.event);
        if let RunEvent::Encounter { area, .. } = &recorded.event {
            self.view.closed_areas.push(area.name.clone());
        }
        if let Some(grave) = Grave::of(recorded) {
            self.view.graveyard.insert(0, grave);
        }
        self.view.feed.insert(0, FeedEntry::of(recorded));
        self.view_version += 1;
    }
}

impl Drop for RunEngine {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Moves a party Pokémon to the Dead box or, when that is full, to the
/// nearest box below it with room, so it is out of the party either way.
/// Says whether the Dead box was full.
fn move_to_dead_box_or_another(game: &mut impl Game, slot: u8) -> Result<(Vec<Change>, bool), GameError> {
    match game.move_to_dead_box(slot) {
        Ok(changes) => Ok((changes, false)),
        Err(GameError::BoxFull(dead_box)) => {
            let mut last = GameError::BoxFull(dead_box);
            for box_number in (1..dead_box).rev() {
                match game.move_to_box(slot, box_number) {
                    Ok(changes) => return Ok((changes, true)),
                    Err(e) => last = e,
                }
            }
            Err(last)
        }
        Err(e) => Err(e),
    }
}

/// The opponent's Pokémon out in battle, by species name.
fn foe_name(battle: &Battle, game: &mut impl Game) -> String {
    let mut species: Vec<u16> = battle
        .battlers
        .iter()
        .filter(|b| b.side == Side::Opponent && b.species != 0)
        .map(|b| b.species)
        .collect();
    if species.is_empty() {
        species.extend(game.opponent_party().first().map(|p| p.species));
    }
    let names: Vec<String> = species.into_iter().map(|s| game.species_name(s)).collect();
    names.join(" and ")
}

/// Its nickname, or its species name if it has none.
fn display_name(pokemon: &Pokemon, species_name: &str) -> String {
    if pokemon.nickname.is_empty() {
        species_name.to_owned()
    } else {
        pokemon.nickname.clone()
    }
}

/// Every PID in the party and PC, and their species.
fn owned(game: &mut impl Game) -> (BTreeSet<u32>, Vec<u16>) {
    let party = game.party().into_iter();
    let pc = game.pc().into_iter().map(|s| s.pokemon);
    party.chain(pc).map(|p| (p.pid, p.species)).unzip()
}
