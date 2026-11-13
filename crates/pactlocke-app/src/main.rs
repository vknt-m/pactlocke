//! Pactlocke: start or resume a solo Run on a supported game and play it
//! with its rules enforced, start one with friends from a lobby, or just
//! open a GBA ROM and play it, with keyboard input, sound and a speed from
//! 1x to 4x.
//!
//! Usage: `pactlocke [ROM]` opens the ROM straight away; `pactlocke --smoke`
//! starts the UI, draws a few frames and quits (used by CI).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod coop;
mod data_dir;
mod links_panel;
mod lobby_ui;
mod rom_report;
mod run_panel;
mod spectate;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use eframe::egui;
use pactlocke_emu::{CoreAdapter, Emulator, EmulatorOptions, FrameSlot, Keys, Speed, SystemClock};
use pactlocke_game::{
    Capability, DEV_PROFILES_VAR, GameProfile, LevelCap, MAX_LEVEL, ProfileGame, ProfileStore, RomIdentity, Rule,
    sha1_hex,
};
use pactlocke_mgba::MgbaCore;
use pactlocke_run::{
    Locked, NewRun, Ruleset, RunClock, RunCommand, RunEngine, RunEvent, RunLibrary, RunSummary, RunView, RunningGame,
    SystemRunClock,
};

/// Frames the smoke launch draws before it closes itself.
const SMOKE_FRAMES: u32 = 30;

/// Keyboard layout, following mGBA's defaults: keyboard key, GBA button, its name.
const KEY_MAP: [(egui::Key, Keys, &str); 10] = [
    (egui::Key::X, Keys::A, "A"),
    (egui::Key::Z, Keys::B, "B"),
    (egui::Key::A, Keys::L, "L"),
    (egui::Key::S, Keys::R, "R"),
    (egui::Key::Enter, Keys::START, "Start"),
    (egui::Key::Backspace, Keys::SELECT, "Select"),
    (egui::Key::ArrowUp, Keys::UP, "Up"),
    (egui::Key::ArrowDown, Keys::DOWN, "Down"),
    (egui::Key::ArrowLeft, Keys::LEFT, "Left"),
    (egui::Key::ArrowRight, Keys::RIGHT, "Right"),
];

/// "A: X   B: Z   …", for the home screen.
fn key_help() -> String {
    KEY_MAP
        .iter()
        .map(|(key, _, gba)| format!("{gba}: {}", key.name()))
        .collect::<Vec<_>>()
        .join("   ")
}

fn main() -> eframe::Result {
    // Before anything else, and before any other thread starts.
    pactlocke_mgba::force_utc_time_zone();

    let mut smoke = false;
    let mut rom = None;
    for arg in std::env::args_os().skip(1) {
        if arg == "--smoke" {
            smoke = true;
        } else {
            rom = Some(PathBuf::from(arg));
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Pactlocke")
            .with_inner_size([880.0, 520.0])
            .with_min_inner_size([240.0, 200.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Pactlocke",
        options,
        Box::new(move |cc| {
            let mut app = PactlockeApp::new(cc.egui_ctx.clone(), smoke);
            if let Some(rom) = rom {
                app.open_rom(&rom);
            }
            Ok(Box::new(app))
        }),
    )
}

/// The emulator thread for what is being played: a game on its own, or a
/// game in a Run with the Run engine watching every frame.
enum Play {
    Free(Emulator<MgbaCore>),
    Run {
        emulator: Emulator<RunningGame<ProfileGame<MgbaCore>>>,
        view: Arc<Mutex<RunView>>,
        /// What the side panel asks of the Run, carried out at a frame boundary.
        commands: Sender<RunCommand>,
        /// In a Run with other Players: Run state for the shared Run log.
        shared: Option<Arc<Mutex<Vec<RunEvent>>>>,
    },
}

impl Play {
    fn set_keys(&self, keys: Keys) {
        match self {
            Play::Free(emulator) => emulator.set_keys(keys),
            Play::Run { emulator, .. } => emulator.set_keys(keys),
        }
    }

    fn set_speed(&self, speed: Speed) {
        match self {
            Play::Free(emulator) => emulator.set_speed(speed),
            Play::Run { emulator, .. } => emulator.set_speed(speed),
        }
    }

    fn frames(&self) -> Arc<FrameSlot> {
        match self {
            Play::Free(emulator) => emulator.frames(),
            Play::Run { emulator, .. } => emulator.frames(),
        }
    }

    fn set_paused(&self, paused: bool) {
        match self {
            Play::Free(emulator) => emulator.set_paused(paused),
            Play::Run { emulator, .. } => emulator.set_paused(paused),
        }
    }
}

struct Playing {
    play: Play,
    /// Kept alive for as long as the game plays.
    audio_out: Option<audio::AudioOut>,
    texture: Option<egui::TextureHandle>,
    shown_seq: u64,
    name: String,
    /// What the ROM is, and what its Game Profile can enforce.
    identity: RomIdentity,
    /// In a Run with other Players: their Shadow Games, and Spectate.
    spectate: Option<spectate::Spectate>,
    /// Whose game the screen shows: `None` for this Player's own.
    shown_source: Option<String>,
}

/// The folder of a developer's local Game Profiles, if the developer setting names one.
fn dev_profiles_dir() -> Option<PathBuf> {
    std::env::var_os(DEV_PROFILES_VAR).map(PathBuf::from)
}

/// The bundled Game Profiles, and with the developer setting a developer's
/// local ones too (for solo Runs only).
fn load_profiles() -> Result<ProfileStore, String> {
    ProfileStore::from_env().map_err(|e| match dev_profiles_dir() {
        Some(dir) => format!("The developer profiles in {} don't load: {e}", dir.display()),
        None => format!("The bundled Game Profiles are broken: {e}"),
    })
}

/// Why a rule can't be chosen for a solo Run on this ROM, if it can't.
fn unavailable_reason(capability: &Capability) -> Option<String> {
    match capability {
        Capability::Enforceable => None,
        Capability::GreyedOut { reason } => Some(format!("can't be enforced on this ROM: {reason}")),
        Capability::NativeOption { .. } => Some("this game's own option, which solo Runs don't use yet".to_owned()),
    }
}

/// A rule's checkbox, greyed out with the reason on hover when it can't be chosen.
fn rule_checkbox(ui: &mut egui::Ui, on: &mut bool, text: &str, unavailable: Option<String>) {
    if unavailable.is_some() {
        *on = false;
    }
    let checkbox = ui.add_enabled(unavailable.is_none(), egui::Checkbox::new(on, text));
    if let Some(reason) = unavailable {
        checkbox.on_disabled_hover_text(format!("Unavailable: {reason}"));
    }
}

/// The level caps checkbox and, while it is on, each stage's cap to edit.
fn level_caps_editor(ui: &mut egui::Ui, draft: &mut NewRunDraft) {
    if draft.level_caps.is_empty() {
        rule_checkbox(
            ui,
            &mut draft.level_caps_on,
            "Level caps",
            Some("this Game Profile lists no gym leaders or Elite Four for caps yet".to_owned()),
        );
        return;
    }
    let text = match draft.soft_cap {
        Some(_) => "Level caps (soft cap: over the cap, you can't ready up at a Sync Gate)",
        None => "Level caps: EXP and Rare Candy stop at the next leader's strongest Pokémon",
    };
    let checkbox = ui.checkbox(&mut draft.level_caps_on, text);
    if let Some(reason) = &draft.soft_cap {
        checkbox.on_hover_text(format!("The game can't be stopped at the cap on this ROM: {reason}"));
    }
    if !draft.level_caps_on {
        return;
    }
    ui.indent("level caps", |ui| {
        egui::Grid::new("level caps").num_columns(4).show(ui, |ui| {
            for (i, cap) in draft.level_caps.iter_mut().enumerate() {
                ui.label(&cap.name);
                ui.add(egui::DragValue::new(&mut cap.level).range(1..=MAX_LEVEL));
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    });
}

/// A New Run waiting for its name.
struct NewRunDraft {
    rom: Vec<u8>,
    path: PathBuf,
    profile: Arc<GameProfile>,
    identity: RomIdentity,
    name: String,
    ruleset: Ruleset,
    /// Why each rule this ROM can't enforce can't be chosen.
    unavailable: BTreeMap<Rule, String>,
    /// Whether the Run has level caps, and each stage's cap as the Host left it.
    level_caps_on: bool,
    level_caps: Vec<LevelCap>,
    /// Why the caps are only soft caps on this ROM, if they are.
    soft_cap: Option<String>,
    /// Whether the Game Profile is a developer's local one (solo Runs only).
    local_profile: bool,
}

struct PactlockeApp {
    ctx: egui::Context,
    playing: Option<Playing>,
    /// The bundled Game Profiles; an error here is a broken build.
    profiles: Result<ProfileStore, String>,
    library: Result<RunLibrary, String>,
    /// The Runs are in the user data folder because the app's folder isn't writable.
    library_fallback: bool,
    runs: Vec<RunSummary>,
    new_run: Option<NewRunDraft>,
    speed: Speed,
    error: Option<String>,
    smoke_frames_left: Option<u32>,
    /// Inviting friends to a Run, or joining one.
    lobbies: lobby_ui::Lobbies,
}

impl PactlockeApp {
    fn new(ctx: egui::Context, smoke: bool) -> Self {
        let (dir, library_fallback) = data_dir::runs_dir();
        let library = RunLibrary::open(dir).map_err(|e| format!("Runs can't be kept: {e}"));
        let runs = library.as_ref().map(RunLibrary::list).unwrap_or_default();
        Self {
            ctx,
            playing: None,
            profiles: load_profiles(),
            library,
            library_fallback,
            runs,
            new_run: None,
            speed: Speed::X1,
            error: None,
            smoke_frames_left: smoke.then_some(SMOKE_FRAMES),
            lobbies: lobby_ui::Lobbies::new(&data_dir::identity_file()),
        }
    }

    /// The Run library, or why there is none.
    fn run_library(&self) -> Result<&RunLibrary, String> {
        self.library.as_ref().map_err(Clone::clone)
    }

    fn pick_rom_file(title: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title(title)
            .add_filter("GBA ROM", &["gba", "agb", "bin"])
            .pick_file()
    }

    fn pick_rom(&mut self) {
        if let Some(path) = Self::pick_rom_file("Choose a GBA ROM") {
            self.open_rom(&path);
        }
    }

    /// Plays a ROM on its own, outside any Run.
    fn open_rom(&mut self, path: &Path) {
        self.stop_playing();
        let result = self.read_rom(path).and_then(|(rom, identity)| {
            let core = self.boot(&rom).map_err(|e| format!("{}: {e}", path.display()))?;
            let (emulator, audio) = self.spawn(core);
            Ok(Playing::new(Play::Free(emulator), audio, file_stem(path), identity))
        });
        self.show_result(result);
    }

    fn read_rom(&self, path: &Path) -> Result<(Vec<u8>, RomIdentity), String> {
        // The ROM is read into memory only; it is never copied anywhere.
        let rom = std::fs::read(path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
        let identity = self.profiles.as_ref().map_err(Clone::clone)?.identify(&rom);
        Ok((rom, identity))
    }

    /// A core with the ROM loaded and its clock started, for playing a game outside any Run.
    fn boot(&self, rom: &[u8]) -> Result<MgbaCore, String> {
        let mut core = Self::core_with_rom(rom)?;
        core.set_clock(SystemRunClock.now_ms());
        Ok(core)
    }

    fn spawn<C: CoreAdapter + 'static>(&self, core: C) -> (Emulator<C>, Option<audio::AudioOut>) {
        let audio = audio::AudioOut::open();
        let ctx = self.ctx.clone();
        let mut options = EmulatorOptions::new(SystemClock::default())
            .speed(self.speed)
            .on_frame(move || ctx.request_repaint());
        if let Some(out) = &audio {
            options = options.audio(out.sink());
        }
        (Emulator::spawn(core, options), audio)
    }

    fn show_result(&mut self, result: Result<Playing, String>) {
        match result {
            Ok(playing) => {
                self.playing = Some(playing);
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }

    /// Picks a ROM for a New Run: a game whose Game Profile can enforce the
    /// first-Encounter rule.
    fn begin_new_run(&mut self) {
        let Some(path) = Self::pick_rom_file("Choose the ROM for the new Run") else {
            return;
        };
        let draft = self.read_rom(&path).and_then(|(rom, identity)| {
            let RomIdentity::Profiled { profile, capabilities } = &identity else {
                return Err("A Run needs a game Pactlocke has a Game Profile for, and this ROM has none.".to_owned());
            };
            let unavailable: BTreeMap<Rule, String> = [
                Rule::FirstEncounter,
                Rule::DupesClause,
                Rule::Gifts,
                Rule::Statics,
                Rule::FaintIsDead,
                Rule::NoItems,
                Rule::SetMode,
                Rule::MustNickname,
                Rule::TypeClause,
            ]
            .into_iter()
            .filter_map(|rule| unavailable_reason(capabilities.get(rule)).map(|why| (rule, why)))
            .collect();
            let mut ruleset = Ruleset {
                first_encounter: !unavailable.contains_key(&Rule::FirstEncounter),
                faint_is_dead: !unavailable.contains_key(&Rule::FaintIsDead),
                // Without a catalogue no Gift can be told apart, so every one is kept.
                gifts: true,
                ..Ruleset::default()
            };
            ruleset.dupes_clause = ruleset.dupes_clause_available() && !unavailable.contains_key(&Rule::DupesClause);
            // Where the level checks can't be patched the caps still apply, as soft caps.
            let soft_cap = match capabilities.get(Rule::LevelCap) {
                Capability::GreyedOut { reason } => Some(reason.clone()),
                _ => None,
            };
            let level_caps = profile.default_level_caps(&rom);
            Ok(NewRunDraft {
                ruleset,
                unavailable,
                // On by default; where the game can't be stopped at the cap it is a soft cap.
                level_caps_on: !level_caps.is_empty(),
                level_caps,
                soft_cap,
                local_profile: self.profiles.as_ref().is_ok_and(|store| store.is_local(&profile.id)),
                profile: profile.clone(),
                name: format!("Solo Run {}", self.runs.len() + 1),
                rom,
                path: path.clone(),
                identity: identity.clone(),
            })
        });
        match draft {
            Ok(draft) => {
                self.error = None;
                self.new_run = Some(draft);
            }
            Err(message) => self.error = Some(message),
        }
    }

    fn start_run(&mut self, mut draft: NewRunDraft) {
        self.stop_playing();
        draft.ruleset.level_caps = draft.level_caps_on.then(|| draft.level_caps.clone());
        let result = self.run_library().and_then(|library| {
            // Load first, so a ROM the core refuses leaves no Run behind.
            let core = Self::core_with_rom(&draft.rom)?;
            let file = library.create().map_err(|e| e.to_string())?;
            let new = NewRun {
                name: draft.name.trim().to_owned(),
                game_name: draft.profile.name.clone(),
                game_sha1: draft.profile.sha1.clone(),
                rom_path: Some(draft.path.display().to_string()),
                ruleset: draft.ruleset,
            };
            let engine = RunEngine::start(new, Box::new(file), Box::new(SystemRunClock)).map_err(|e| e.to_string())?;
            self.play_run(
                engine,
                core,
                draft.profile,
                draft.rom,
                draft.identity,
                SystemRunClock.now_ms(),
                None,
            )
        });
        self.show_result(result);
        self.refresh_runs();
    }

    /// The lock's countdown is over: this Player's Run starts from a new
    /// game with the locked Ruleset, its game clock at the lock's start time.
    fn start_coop_run(&mut self, start: coop::CoopStart) {
        self.stop_playing();
        let locked = start.locked;
        let result = self.run_library().and_then(|library| {
            let identity = self.profiles.as_ref().map_err(Clone::clone)?.identify(&start.rom);
            let RomIdentity::Profiled { profile, .. } = &identity else {
                return Err("This build has no Game Profile for the Run's game.".to_owned());
            };
            let profile = profile.clone();
            let core = Self::core_with_rom(&start.rom)?;
            let file = library.create().map_err(|e| e.to_string())?;
            let new = NewRun {
                name: locked.run_name.clone(),
                game_name: locked.game_name.clone(),
                game_sha1: locked.game_sha1.clone(),
                rom_path: Some(start.path.display().to_string()),
                ruleset: locked.ruleset.clone(),
            };
            let engine = RunEngine::start(new, Box::new(file), Box::new(SystemRunClock)).map_err(|e| e.to_string())?;
            self.play_run(
                engine,
                core,
                profile,
                start.rom,
                identity,
                locked.start_ms,
                Some(&locked),
            )
        });
        self.show_result(result);
        self.refresh_runs();
    }

    fn resume_run(&mut self, summary: &RunSummary) {
        self.stop_playing();
        let result = self.find_rom(summary).and_then(|(rom, identity)| {
            let RomIdentity::Profiled { profile, .. } = &identity else {
                return Err("This build has no Game Profile for the Run's game.".to_owned());
            };
            let profile = profile.clone();
            let library = self.run_library()?;
            let file = library.open_run(&summary.id).map_err(|e| e.to_string())?;
            let engine = RunEngine::resume(Box::new(file), Box::new(SystemRunClock)).map_err(|e| e.to_string())?;
            let core = Self::core_with_rom(&rom)?;
            self.play_run(engine, core, profile, rom, identity, SystemRunClock.now_ms(), None)
        });
        self.show_result(result);
        self.refresh_runs();
    }

    /// The Run's ROM: where it was last time if it is still there, otherwise
    /// one the Player picks, checked by SHA-1.
    fn find_rom(&self, summary: &RunSummary) -> Result<(Vec<u8>, RomIdentity), String> {
        if let Some(path) = &summary.rom_path
            && let Ok((rom, identity)) = self.read_rom(Path::new(path))
            && sha1_hex(&rom) == summary.game_sha1
        {
            return Ok((rom, identity));
        }
        let title = format!("Find the ROM for {} ({})", summary.name, summary.game_name);
        let path = Self::pick_rom_file(&title).ok_or("The Run's ROM wasn't found, so it can't be resumed.")?;
        let (rom, identity) = self.read_rom(&path)?;
        if sha1_hex(&rom) != summary.game_sha1 {
            return Err(format!(
                "That isn't the ROM this Run is played on: it needs {} (SHA-1 {}).",
                summary.game_name, summary.game_sha1
            ));
        }
        Ok((rom, identity))
    }

    /// A core with the ROM loaded and nothing run yet.
    fn core_with_rom(rom: &[u8]) -> Result<MgbaCore, String> {
        let mut core = MgbaCore::new();
        core.load_rom(rom).map_err(|e| e.to_string())?;
        Ok(core)
    }

    /// Boots a Run's game the way its Run engine planned (from power-on,
    /// the last in-game save, the suspend slot, or rebuilt after a crash),
    /// then plays it with the engine watching. The game clock starts at
    /// `clock_ms`: now, or the lock's start time in a Run with friends
    /// (`coop`), whose Run state goes to the Run log, whose input stream goes
    /// to the other Players, and whose other Players get Shadow Games here.
    #[allow(clippy::too_many_arguments)]
    fn play_run(
        &self,
        engine: RunEngine,
        core: MgbaCore,
        profile: Arc<GameProfile>,
        rom: Vec<u8>,
        identity: RomIdentity,
        clock_ms: i64,
        coop: Option<&Locked>,
    ) -> Result<Playing, String> {
        let name = engine.info().name.clone();
        let rom: Arc<[u8]> = rom.into();
        let mut running = RunningGame::boot(ProfileGame::new(profile.clone(), rom.clone(), core), engine, clock_ms)
            .map_err(|e| format!("The Run's game couldn't be booted: {e}"))?;
        let view = running.view();
        let commands = running.commands();
        let shared = coop.map(|_| running.share());
        let spectate = match coop {
            Some(locked) => {
                let me = self
                    .lobbies
                    .public_key()
                    .ok_or("This install has no network key, so the Run can't be shared.")?;
                let outbox = running.stream_as(me);
                Some(spectate::Spectate::new(me, locked, &rom, &profile, outbox, &commands)?)
            }
            None => None,
        };
        let (emulator, audio) = self.spawn(running);
        let mut playing = Playing::new(
            Play::Run {
                emulator,
                view,
                commands,
                shared,
            },
            audio,
            name,
            identity,
        );
        playing.spectate = spectate;
        Ok(playing)
    }

    /// Stops the game. A Run's file has finished writing when this returns.
    fn stop_playing(&mut self) {
        if self.playing.take().is_some() {
            self.refresh_runs();
        }
    }

    /// Suspend and quit: the Run resumes exactly here next time.
    fn suspend_and_quit(&mut self) {
        if let Some(Playing {
            play: Play::Run { emulator, .. },
            ..
        }) = self.playing.take()
        {
            emulator.stop().suspend();
            self.refresh_runs();
        }
    }

    fn refresh_runs(&mut self) {
        if let Ok(library) = &self.library {
            self.runs = library.list();
        }
    }

    fn set_speed(&mut self, speed: Speed) {
        self.speed = speed;
        if let Some(playing) = &self.playing {
            playing.play.set_speed(speed);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match &self.playing {
                Some(playing) => {
                    let label = match playing.play {
                        Play::Run { .. } => "Close Run",
                        Play::Free(_) => "Close game",
                    };
                    let in_run = matches!(playing.play, Play::Run { .. });
                    if ui.button(label).clicked() {
                        self.stop_playing();
                    } else if in_run
                        && ui
                            .button("Suspend and quit")
                            .on_hover_text("Close the Run and resume exactly here next time, unsaved progress and all")
                            .clicked()
                    {
                        self.suspend_and_quit();
                    }
                }
                None => {
                    if ui.button("New Run…").clicked() {
                        self.begin_new_run();
                    }
                    if ui.button("Join Run…").clicked() {
                        self.lobbies.open_join_prompt();
                    }
                    if ui.button("Open ROM…").clicked() {
                        self.pick_rom();
                    }
                }
            }
            ui.separator();
            ui.label("Speed");
            let mut speed = self.speed;
            for option in Speed::ALL {
                ui.selectable_value(&mut speed, option, option.to_string());
            }
            if speed != self.speed {
                self.set_speed(speed);
            }
            if let Some(playing) = &self.playing {
                ui.separator();
                ui.label(&playing.name);
                ui.separator();
                rom_report::status_label(ui, &playing.identity);
            }
        });
    }

    /// The Run list, when nothing is playing.
    fn home_screen(&mut self, ui: &mut egui::Ui) {
        let mut resume = None;
        ui.vertical_centered(|ui| {
            ui.add_space(16.0);
            ui.heading("Runs");
            ui.add_space(8.0);
            if let Err(e) = &self.library {
                ui.colored_label(ui.visuals().error_fg_color, e);
            } else if self.runs.is_empty() {
                ui.label("No Runs yet. Start one with New Run…, or open a ROM to just play.");
            }
            if self.library_fallback {
                ui.weak("The app's folder can't be written to, so Runs are kept in your user data folder.");
            }
            if let Some(dir) = dev_profiles_dir() {
                ui.weak(format!(
                    "Developer setting: local Game Profiles from {}, for solo Runs only.",
                    dir.display()
                ));
            }
        });
        ui.add_space(8.0);
        let now = SystemRunClock.now_ms();
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - 60.0).max(80.0))
            .show(ui, |ui| {
                for summary in &self.runs {
                    ui.horizontal(|ui| {
                        if ui.button("Resume").clicked() {
                            resume = Some(summary.clone());
                        }
                        ui.strong(&summary.name);
                        ui.label(&summary.game_name);
                        ui.weak(format!("played {}", run_panel::ago(now - summary.last_played_ms)));
                    });
                }
            });
        ui.add_space(12.0);
        ui.vertical_centered(|ui| ui.weak(key_help()));
        if let Some(summary) = resume {
            self.resume_run(&summary);
        }
    }

    /// Names a New Run before it starts.
    fn new_run_prompt(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.new_run.as_mut() else {
            return;
        };
        let mut start = false;
        let mut cancel = false;
        let mut invite = false;
        let modal = egui::Modal::new(egui::Id::unique("new run")).show(ctx, |ui| {
            ui.heading("New Run");
            ui.label(format!("{}: solo.", draft.profile.name));
            if draft.local_profile {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "A developer's local Game Profile, not a bundled one: for solo Runs only.",
                );
            }
            ui.add_space(6.0);
            let ruleset = &mut draft.ruleset;
            rule_checkbox(
                ui,
                &mut ruleset.first_encounter,
                "First Encounter: only the first wild battle in each Area can be caught",
                draft.unavailable.get(&Rule::FirstEncounter).cloned(),
            );
            let needs_first_encounter = |available: bool, rule: Rule| {
                if available {
                    draft.unavailable.get(&rule).cloned()
                } else {
                    Some("it needs First Encounter on".to_owned())
                }
            };
            let dupes_unavailable = needs_first_encounter(ruleset.dupes_clause_available(), Rule::DupesClause);
            let statics_unavailable = needs_first_encounter(ruleset.statics_available(), Rule::Statics);
            rule_checkbox(
                ui,
                &mut ruleset.dupes_clause,
                "Dupes clause: a wild Pokémon from a family you have alive doesn't count",
                dupes_unavailable,
            );
            rule_checkbox(
                ui,
                &mut ruleset.statics,
                "Statics: a legendary or other Static is a bonus Encounter of its own",
                statics_unavailable,
            );
            let gifts = ui.add_enabled(
                !draft.unavailable.contains_key(&Rule::Gifts),
                egui::Checkbox::new(
                    &mut ruleset.gifts,
                    "Gifts: keep Gifts besides the starter (when off, they are released)",
                ),
            );
            if let Some(reason) = draft.unavailable.get(&Rule::Gifts) {
                gifts.on_disabled_hover_text(format!("Every Gift is kept: {reason}"));
            }
            rule_checkbox(
                ui,
                &mut ruleset.faint_is_dead,
                "Faint = Dead: fainted Pokémon go to the Dead box",
                draft.unavailable.get(&Rule::FaintIsDead).cloned(),
            );
            rule_checkbox(
                ui,
                &mut ruleset.no_items,
                "No items: the Bag is refused in trainer battles, and only Poké Balls work in wild battles",
                draft.unavailable.get(&Rule::NoItems).cloned(),
            );
            rule_checkbox(
                ui,
                &mut ruleset.set_mode,
                "Set mode: the battle style is kept on Set",
                draft.unavailable.get(&Rule::SetMode).cloned(),
            );
            rule_checkbox(
                ui,
                &mut ruleset.must_nickname,
                "Must nickname: every catch and Gift gets a nickname",
                draft.unavailable.get(&Rule::MustNickname).cloned(),
            );
            rule_checkbox(
                ui,
                &mut ruleset.type_clause,
                "Type clause: one Pokémon of each primary type in your party",
                draft.unavailable.get(&Rule::TypeClause).cloned(),
            );
            level_caps_editor(ui, draft);
            ui.horizontal(|ui| {
                ui.label("Retries");
                ui.add(egui::DragValue::new(&mut draft.ruleset.retries).range(0..=pactlocke_run::MAX_RETRIES))
                    .on_hover_text("Each sends you back to your latest in-game save, undoing everything since. 0 is off.");
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Name");
                let field = ui.text_edit_singleline(&mut draft.name);
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    start = true;
                }
                field.request_focus();
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let named = !draft.name.trim().is_empty();
                if ui.add_enabled(named, egui::Button::new("Start")).clicked() {
                    start = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                let shareable = !draft.local_profile;
                let invite_button = ui.add_enabled(shareable, egui::Button::new("Invite friends…"));
                if shareable {
                    if invite_button
                        .on_hover_text("Open a lobby and get an invite link for each friend")
                        .clicked()
                    {
                        invite = true;
                    }
                } else {
                    invite_button.on_disabled_hover_text(
                        "A developer's local Game Profile can't be shared: Runs with other Players use bundled profiles only.",
                    );
                }
            });
        });
        if invite {
            let run = pactlocke_net::RunAdvert {
                name: draft.name.trim().to_owned(),
                game_name: draft.profile.name.clone(),
                game_sha1: draft.profile.sha1.clone(),
            };
            let RomIdentity::Profiled { capabilities, .. } = &draft.identity else {
                unreachable!("a New Run's ROM has a Game Profile")
            };
            let rom = coop::HostRom {
                rom: draft.rom.clone(),
                path: draft.path.clone(),
                capabilities: capabilities.clone(),
                level_caps: draft.level_caps.clone(),
            };
            self.lobbies.host(run, rom);
            // The Ruleset is set in the lobby from here, for everyone.
            self.new_run = None;
            return;
        }
        start &= !draft.name.trim().is_empty();
        if start {
            let draft = self.new_run.take().expect("a draft is open");
            self.start_run(draft);
        } else if cancel || modal.should_close() {
            self.new_run = None;
        }
    }

    fn game_screen(&mut self, ui: &mut egui::Ui) {
        let Some(playing) = self.playing.as_mut() else {
            self.home_screen(ui);
            return;
        };

        // Spectate: the watched Player's Shadow Game, with a way back.
        let watched = playing.spectate.as_ref().and_then(|s| s.watched()).map(|shadow| {
            (
                shadow.name.clone(),
                shadow.frames(),
                shadow.status(),
                shadow.cant_watch(),
            )
        });
        let frames = match &watched {
            Some((name, frames, status, cant_watch)) => {
                let mut back = false;
                ui.horizontal(|ui| {
                    ui.strong(format!("Watching {name}"));
                    if let Some(why) = cant_watch {
                        ui.colored_label(ui.visuals().error_fg_color, why);
                    } else if status.catching_up {
                        ui.weak("catching up…");
                    }
                    back = ui.button("Back to your game").clicked();
                });
                if back && let Some(spectate) = playing.spectate.as_mut() {
                    spectate.stop_watching();
                    playing.play.set_paused(false);
                }
                ui.ctx().request_repaint();
                frames.clone()
            }
            None => playing.play.frames(),
        };
        let source = watched.as_ref().map(|(name, ..)| name.clone());
        if source != playing.shown_source {
            playing.shown_source = source;
            playing.texture = None;
        }

        // Hold the keys that are down right now (only while the window has focus).
        let keys = ui.input(|i| {
            if !i.focused || watched.is_some() {
                return Keys::NONE;
            }
            KEY_MAP
                .iter()
                .filter(|(key, _, _)| i.key_down(*key))
                .fold(Keys::NONE, |held, (_, gba, _)| held | *gba)
        });
        playing.play.set_keys(keys);

        // Show the newest published frame.
        if let Some(newest) = frames.latest()
            && (playing.texture.is_none() || newest.seq != playing.shown_seq)
        {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [newest.frame.width as usize, newest.frame.height as usize],
                &newest.frame.rgba,
            );
            match &mut playing.texture {
                Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
                None => {
                    playing.texture = Some(ui.ctx().load_texture("game", image, egui::TextureOptions::NEAREST));
                }
            }
            playing.shown_seq = newest.seq;
        }

        if let Some(texture) = &playing.texture {
            let available = ui.available_size();
            let size = texture.size_vec2();
            let scale = (available.x / size.x).min(available.y / size.y).max(0.1);
            ui.centered_and_justified(|ui| {
                ui.add(egui::Image::new(texture).fit_to_exact_size(size * scale));
            });
        }
    }
}

impl Playing {
    fn new(play: Play, audio: Option<audio::AudioOut>, name: String, identity: RomIdentity) -> Self {
        Self {
            play,
            audio_out: audio,
            texture: None,
            shown_seq: 0,
            name,
            identity,
            spectate: None,
            shown_source: None,
        }
    }
}

/// The file's name without its extension.
fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl eframe::App for PactlockeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        if let Some(error) = &self.error {
            let error = error.clone();
            egui::Panel::bottom("error").show(ui, |ui| {
                ui.colored_label(ui.visuals().error_fg_color, error);
            });
        }
        if let Some(playing) = &mut self.playing {
            let mut asked = None;
            let mut votes = None;
            egui::Panel::right("side panel")
                .resizable(true)
                .default_size(360.0)
                .show(ui, |ui| match &playing.play {
                    Play::Run { view, commands, .. } => {
                        if let Some(spectate) = &playing.spectate {
                            asked = spectate::panel(ui, spectate);
                            ui.separator();
                        }
                        let view = view.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        let mut links_tab = None;
                        if let (Some(spectate), Some(log)) = (&playing.spectate, self.lobbies.log()) {
                            votes = spectate::votes(ui, log, spectate.me());
                            ui.separator();
                            let links_on = log.locked().is_some_and(|l| l.ruleset.links);
                            let links = if links_on { spectate.links(log) } else { Vec::new() };
                            let players: Vec<(pactlocke_run::PlayerKey, String)> = log
                                .locked()
                                .map(|l| l.roster.iter().map(|p| (p.key, p.name.clone())).collect())
                                .unwrap_or_default();
                            let statuses: Vec<_> = spectate.shadows.iter().map(|s| (s.key, s.status())).collect();
                            let mut parties = Vec::new();
                            for (key, name) in &players {
                                let pokemon = if *key == spectate.me() {
                                    &view.party[..]
                                } else {
                                    statuses
                                        .iter()
                                        .find(|(k, _)| k == key)
                                        .map_or(&[][..], |(_, status)| &status.seen.party[..])
                                };
                                parties.push(links_panel::Party {
                                    player: *key,
                                    name,
                                    pokemon,
                                });
                            }
                            links_panel::parties(ui, &parties, &links);
                            ui.separator();
                            if links_on {
                                links_tab = Some((links, players, spectate.me()));
                            }
                        }
                        let mut show_links = links_tab.as_ref().map(|(links, players, me)| {
                            (links.len(), move |ui: &mut egui::Ui| {
                                links_panel::tab(ui, links, players, me)
                            })
                        });
                        let tab = show_links
                            .as_mut()
                            .map(|(count, show)| (*count, show as &mut run_panel::LinksTab<'_>));
                        if let Some(command) = run_panel::show(ui, &view, playing.spectate.is_none(), tab) {
                            // The emulator thread only stops when the Run is closed.
                            let _ = commands.send(command);
                        }
                    }
                    Play::Free(_) => rom_report::rules_panel(ui, &playing.identity),
                });
            if let Some(spectate) = playing.spectate.as_mut() {
                match asked {
                    // Spectate pauses this Player's own game until they come back.
                    Some(spectate::Asked::Watch(i)) => {
                        spectate.watch(i, playing.audio_out.as_ref());
                        playing.play.set_paused(true);
                    }
                    Some(spectate::Asked::BatterySaver(on)) => spectate.set_battery_saver(on),
                    None => {}
                }
                spectate.pump(&mut self.lobbies);
                if let Some(vote) = votes {
                    self.lobbies.share(vec![vote]);
                }
                if let (Play::Run { commands, .. }, Some(log)) = (&playing.play, self.lobbies.log()) {
                    for command in spectate.follow(log) {
                        // The emulator thread only stops when the Run is closed.
                        let _ = commands.send(command);
                    }
                }
                // Streams are passed on from the UI thread: keep it awake.
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        egui::CentralPanel::default_margins().show(ui, |ui| self.game_screen(ui));
        let ctx = ui.ctx().clone();
        self.new_run_prompt(&ctx);
        if let Some(Playing {
            play: Play::Run {
                shared: Some(shared), ..
            },
            ..
        }) = &self.playing
        {
            let events = std::mem::take(&mut *shared.lock().unwrap_or_else(|e| e.into_inner()));
            if !events.is_empty() {
                self.lobbies.share(events);
            }
        }
        if let Some(start) = self.lobbies.show(&ctx) {
            self.start_coop_run(start);
        }

        if let Some(left) = self.smoke_frames_left.as_mut() {
            *left = left.saturating_sub(1);
            if *left == 0 {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.ctx().request_repaint();
        }
    }
}
