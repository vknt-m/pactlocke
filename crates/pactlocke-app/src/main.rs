//! Pactlocke: start or resume a solo Run on a supported game and play it
//! with its rules enforced, start one with friends from a Lobby, or just
//! open a GBA ROM and play it. One window: a menu bar along the top, the game
//! on the left and the side panel on the right.
//!
//! Usage: `pactlocke [ROM]` opens the ROM straight away; `pactlocke --smoke`
//! starts the UI, draws a few frames and quits (used by CI). For a developer
//! looking at the UI: `--shot FILE` saves the window as a PNG once it has
//! settled and quits, `--demo-run ROM` starts a solo Run on that ROM straight
//! away with the side panel open, and `--open settings` opens Settings.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod coop;
mod data_dir;
mod game_over;
mod gamepad;
mod headless;
mod links_panel;
mod lobby_ui;
mod logger;
mod player_strip;
mod resume;
mod rom_library;
mod rom_report;
mod rules_text;
mod run_panel;
mod settings;
mod settings_ui;
mod side_panel;
mod spectate;
mod theme;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use eframe::egui;
use pactlocke_emu::{CoreAdapter, Emulator, EmulatorOptions, FrameSlot, Keys, Speed, SystemClock};
use pactlocke_game::{
    Capability, CapabilityReport, DEV_PROFILES_VAR, GameProfile, LevelCap, MAX_LEVEL, ProfileGame, ProfileStore,
    RomIdentity, Rule, sha1_hex,
};
use pactlocke_mgba::MgbaCore;
use pactlocke_run::{
    Locked, NewRun, Preset, Ruleset, RunClock, RunCommand, RunEngine, RunEvent, RunLibrary, RunLog, RunSummary,
    RunView, RunningGame, SystemRunClock,
};
use rom_library::RomLibrary;
use settings::Settings;
use settings_ui::{Category, SettingsDialog};

/// Frames the smoke launch draws before it closes itself.
const SMOKE_FRAMES: u32 = 30;
/// How many ROMs File ▸ Recent ROMs shows.
const RECENT_SHOWN: usize = 8;
/// Frames `--shot` waits for the window to settle before saving it.
const SHOT_FRAMES: u32 = 120;

/// A GBA screen, in pixels.
const GBA_W: f32 = 240.0;
const GBA_H: f32 = 160.0;

fn main() -> std::process::ExitCode {
    // Before anything else, and before any other thread starts.
    pactlocke_mgba::force_utc_time_zone();
    if std::env::args_os().nth(1).is_some_and(|a| a == "--headless") {
        let args: Vec<String> = std::env::args_os()
            .skip(2)
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        return headless::main(&args);
    }
    match windowed() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Pactlocke couldn't start: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn windowed() -> eframe::Result {
    logger::init();

    let mut smoke = false;
    let (mut rom, mut shot, mut demo, mut demo_lobby, mut open) = (None, None, None, None, None);
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--smoke") => smoke = true,
            Some("--shot") => shot = args.next().map(PathBuf::from),
            Some("--demo-run") => demo = args.next().map(PathBuf::from),
            Some("--demo-lobby") => demo_lobby = args.next().map(PathBuf::from),
            Some("--open") => open = args.next().and_then(|a| a.into_string().ok()),
            _ => rom = Some(PathBuf::from(arg)),
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Pactlocke")
            .with_inner_size([1100.0, 600.0])
            .with_min_inner_size([480.0, 320.0]),
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
            if let Some(rom) = demo {
                app.demo_run(&rom);
            }
            if let Some(rom) = demo_lobby {
                app.demo_lobby(&rom);
            }
            if open.as_deref() == Some("settings") {
                app.settings_dialog = Some(SettingsDialog::new(&app.settings, Category::General));
            }
            app.shot = shot.map(|path| (path, SHOT_FRAMES));
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
    /// The partner view: shown, and each other Player's tile.
    partner_view: bool,
    /// Each tile's texture, with the frame it shows.
    tiles: Vec<Option<(u64, egui::TextureHandle)>>,
    /// In a Run: party icons from its ROM, and its own Ruleset.
    icons: Option<theme::Icons>,
    ruleset: Ruleset,
    /// Whether the game is paused now, for whatever reason.
    paused: bool,
}

/// The folder of a developer's local Game Profiles, if the developer setting names one.
fn dev_profiles_dir() -> Option<PathBuf> {
    std::env::var_os(DEV_PROFILES_VAR).map(PathBuf::from)
}

/// The bundled Game Profiles, and with the developer setting (Settings ▸
/// Developer, or the environment) a developer's local ones too, for solo Runs only.
fn load_profiles(dev: Option<&Path>) -> Result<ProfileStore, String> {
    let store = match dev {
        Some(dir) => ProfileStore::bundled_with_local(dir),
        None => ProfileStore::from_env(),
    };
    store.map_err(|e| match dev.map(Path::to_path_buf).or_else(dev_profiles_dir) {
        Some(dir) => format!("The developer profiles in {} don't load: {e}", dir.display()),
        None => format!("The bundled Game Profiles are broken: {e}"),
    })
}

/// Why a rule can't be chosen for a solo Run on this ROM, if it can't.
fn unavailable_reason(capability: &Capability) -> Option<String> {
    match capability {
        Capability::Enforceable => None,
        Capability::GreyedOut { reason } => Some(format!("can't be enforced on this ROM: {reason}")),
        // Pactlocke sets the game's own option for it.
        Capability::NativeOption(_) => None,
    }
}

/// A rule's checkbox with what it does under it, off and greyed out with
/// the reason when it can't be chosen.
fn rule_checkbox(ui: &mut egui::Ui, on: &mut bool, rule: Rule, shown: &Ruleset, unavailable: Option<String>) {
    if unavailable.is_some() {
        *on = false;
    }
    rules_text::rule_row(ui, on, rule, shown, unavailable);
}

/// The level caps checkbox and, while it is on, each stage's cap to edit.
fn level_caps_editor(ui: &mut egui::Ui, draft: &mut NewRunDraft) {
    let shown = draft.ruleset.clone();
    // The game's own caps: on or off, but their levels are the game's.
    if matches!(draft.capabilities.get(Rule::LevelCap), Capability::NativeOption(_)) {
        rule_checkbox(ui, &mut draft.level_caps_on, Rule::LevelCap, &shown, None);
        return;
    }
    if draft.level_caps.is_empty() {
        rule_checkbox(
            ui,
            &mut draft.level_caps_on,
            Rule::LevelCap,
            &shown,
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
    rules_text::description(ui, &rules_text::summary(Rule::LevelCap, &shown), None);
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
    /// How this ROM can apply each rule.
    capabilities: CapabilityReport,
    /// Whether the Run has level caps, and each stage's cap as the Host left it.
    level_caps_on: bool,
    level_caps: Vec<LevelCap>,
    /// Why the caps are only soft caps on this ROM, if they are.
    soft_cap: Option<String>,
    /// Whether the Game Profile is a developer's local one (solo Runs only).
    local_profile: bool,
}

/// A Run with friends opened with Resume, its game not booted yet, and what
/// it boots on.
struct Resuming {
    resume: resume::Resuming,
    profile: Arc<GameProfile>,
    rom: Vec<u8>,
    identity: RomIdentity,
}

struct PactlockeApp {
    ctx: egui::Context,
    playing: Option<Playing>,
    /// The bundled Game Profiles; an error here is a broken build.
    profiles: Result<ProfileStore, String>,
    library: Result<RunLibrary, String>,
    /// Every ROM opened, by path and SHA-1: the right one is offered for a Run.
    roms: std::rc::Rc<std::cell::RefCell<RomLibrary>>,
    /// New Run's game chooser is open.
    choosing_game: bool,
    /// The Runs are in the user data folder because the app's folder isn't writable.
    library_fallback: bool,
    runs: Vec<RunSummary>,
    new_run: Option<NewRunDraft>,
    speed: Speed,
    error: Option<String>,
    smoke_frames_left: Option<u32>,
    /// Inviting friends to a Run, or joining one.
    lobbies: lobby_ui::Lobbies,
    /// The side panel is in its own window.
    panel_popped: bool,
    /// The side panel is showing (View ▸ Side panel), and its open tab.
    panel_shown: bool,
    panel_tab: side_panel::Tab,
    settings: Settings,
    settings_dialog: Option<SettingsDialog>,
    /// The first-launch prompt's name, as typed.
    name_draft: String,
    /// A Run waiting for Delete… to be confirmed.
    deleting: Option<RunSummary>,
    /// A Run with friends waiting for Continue as solo… to be confirmed.
    copying: Option<RunSummary>,
    /// A lost Run's summary, opened from the Runs list.
    lost: Option<RunSummary>,
    about: bool,
    /// Paused from Emulation ▸ Pause.
    paused: bool,
    /// The fast-forward key is held.
    fast_forward: bool,
    pads: gamepad::Pads,
    /// The volume every game plays at.
    gain: audio::Gain,
    /// Sound output devices, for Settings ▸ Sound.
    devices: Vec<String>,
    /// This app was in a Lobby last frame.
    was_in_lobby: bool,
    /// This Player's entries waiting for a Host, as last kept in the Run file.
    waiting_kept: Vec<pactlocke_run::Submission>,
    /// A Run with friends being resumed, waiting to boot until its Host has
    /// given back the presses a crash lost.
    resuming: Option<Resuming>,
    /// `--shot`: where to save the window, and frames until then; then set once saved.
    shot: Option<(PathBuf, u32)>,
    shot_saved: Arc<std::sync::atomic::AtomicBool>,
}

impl PactlockeApp {
    fn new(ctx: egui::Context, smoke: bool) -> Self {
        let (dir, library_fallback) = data_dir::runs_dir();
        let library = RunLibrary::open(dir).map_err(|e| format!("Runs can't be kept: {e}"));
        let runs = library.as_ref().map(RunLibrary::list).unwrap_or_default();
        let settings = Settings::load();
        let mut app = Self {
            ctx,
            playing: None,
            profiles: load_profiles(settings.dev_profiles.as_deref()),
            library,
            roms: std::rc::Rc::new(std::cell::RefCell::new(RomLibrary::open(rom_library::path()))),
            choosing_game: false,
            library_fallback,
            runs,
            new_run: None,
            speed: Speed::ALL[usize::from(settings.default_speed.clamp(1, 4) - 1)],
            error: None,
            smoke_frames_left: smoke.then_some(SMOKE_FRAMES),
            lobbies: lobby_ui::Lobbies::new(&data_dir::identity_file()),
            panel_popped: false,
            panel_shown: false,
            panel_tab: side_panel::Tab::Feed,
            settings_dialog: None,
            name_draft: String::new(),
            deleting: None,
            copying: None,
            lost: None,
            about: false,
            paused: false,
            fast_forward: false,
            pads: gamepad::Pads::new(),
            gain: audio::gain(settings.volume),
            devices: audio::output_devices(),
            was_in_lobby: false,
            resuming: None,
            waiting_kept: Vec::new(),
            shot: None,
            shot_saved: Arc::default(),
            settings,
        };
        app.apply_settings();
        app.lobbies.roms = Some(app.roms.clone());
        app.adopt_recent_roms();
        app
    }

    /// The first time the ROM library is kept, the old Recent ROMs list joins it.
    fn adopt_recent_roms(&mut self) {
        if self.roms.borrow().is_kept() {
            return;
        }
        for path in std::mem::take(&mut self.settings.recent_roms).iter().rev() {
            // Read only to be remembered; one that is gone is left out.
            let _ = self.read_rom(path);
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
        self.close_run();
        let result = self.read_rom(path).and_then(|(rom, identity)| {
            let core = self.boot(&rom).map_err(|e| format!("{}: {e}", path.display()))?;
            let (emulator, audio) = self.spawn(core);
            Ok(Playing::new(Play::Free(emulator), audio, file_stem(path), identity))
        });
        self.show_result(result);
    }

    /// Reads a ROM and says what it is; the ROM library remembers it.
    fn read_rom(&self, path: &Path) -> Result<(Vec<u8>, RomIdentity), String> {
        // The ROM is read into memory only; it is never copied anywhere.
        let rom = std::fs::read(path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
        let identity = self.profiles.as_ref().map_err(Clone::clone)?.identify(&rom);
        self.roms.borrow_mut().remember(
            path,
            &sha1_hex(&rom),
            &rom_report::status_text(&identity),
            matches!(identity, RomIdentity::Profiled { .. }),
        );
        Ok((rom, identity))
    }

    /// A core with the ROM loaded and its clock started, for playing a game outside any Run.
    fn boot(&self, rom: &[u8]) -> Result<MgbaCore, String> {
        let mut core = Self::core_with_rom(rom)?;
        core.set_clock(SystemRunClock.now_ms());
        Ok(core)
    }

    fn spawn<C: CoreAdapter + 'static>(&self, core: C) -> (Emulator<C>, Option<audio::AudioOut>) {
        let audio = audio::AudioOut::open(self.settings.output_device.as_deref(), self.gain.clone());
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
                // With friends the panel shows from the start; alone it waits for View ▸ Side panel.
                self.panel_shown = playing.spectate.is_some();
                self.panel_tab = side_panel::Tab::Feed;
                self.paused = false;
                self.playing = Some(playing);
                self.error = None;
                // Every game starts at the default speed from Settings.
                self.set_speed(Speed::ALL[usize::from(self.settings.default_speed.clamp(1, 4) - 1)]);
            }
            Err(message) => self.error = Some(message),
        }
    }

    /// Picks a ROM for a New Run: a game whose Game Profile can enforce the
    /// first-Encounter rule.
    fn begin_new_run(&mut self) {
        if !self.roms.borrow().roms().iter().any(|rom| rom.runs) {
            self.browse_for_new_run();
        } else {
            self.choosing_game = true;
        }
    }

    /// New Run on a ROM the Player browses for.
    fn browse_for_new_run(&mut self) {
        let Some(path) = Self::pick_rom_file("Choose the ROM for the new Run") else {
            return;
        };
        self.new_run_on(&path);
    }

    /// New Run on this ROM.
    fn new_run_on(&mut self, path: &Path) {
        self.choosing_game = false;
        match self.draft_for(path) {
            Ok(draft) => {
                self.error = None;
                self.new_run = Some(draft);
            }
            Err(message) => self.error = Some(message),
        }
    }

    /// `--demo-run`: a solo Run on this ROM, started at once, side panel open.
    fn demo_run(&mut self, path: &Path) {
        match self.draft_for(path) {
            Ok(draft) => {
                self.start_run(draft);
                self.panel_shown = true;
            }
            Err(message) => self.error = Some(message),
        }
    }

    /// `--demo-lobby`: hosts a Lobby on this ROM at once.
    fn demo_lobby(&mut self, path: &Path) {
        match self.draft_for(path) {
            Ok(draft) => self.host_lobby(&draft),
            Err(message) => self.error = Some(message),
        }
    }

    /// Invite friends: a Lobby for this New Run, with an invite link. A game
    /// being played closes: the Lobby takes its place.
    fn host_lobby(&mut self, draft: &NewRunDraft) {
        self.close_run();
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
            sync_gates: draft.profile.sync_gate_names(),
        };
        self.lobbies.host(run, rom);
    }

    /// A New Run on this ROM, with the rules it can enforce on.
    fn draft_for(&self, path: &Path) -> Result<NewRunDraft, String> {
        let path = path.to_path_buf();
        self.read_rom(&path).and_then(|(rom, identity)| {
            let (profile, capabilities) = match &identity {
                RomIdentity::Profiled { profile, capabilities } => (profile, capabilities),
                RomIdentity::Unprofiled {
                    verdict: pactlocke_game::Unprofiled::ProfileComing(hack),
                    ..
                } => {
                    return Err(format!(
                        "{} {} is a tested Romhack, but its Game Profile comes in a later update, so it can't be \
                         played in a Run yet.",
                        hack.name, hack.version
                    ));
                }
                RomIdentity::Unprofiled { .. } => {
                    return Err(
                        "A Run needs a game Pactlocke has a Game Profile for, or a Romhack laid out like one, \
                         and this ROM is neither."
                            .to_owned(),
                    );
                }
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
                capabilities: capabilities.clone(),
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
        })
    }

    fn start_run(&mut self, mut draft: NewRunDraft) {
        self.close_run();
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
                Some((&locked, None)),
            )
        });
        self.show_result(result);
        self.refresh_runs();
    }

    fn resume_run(&mut self, summary: &RunSummary) {
        if summary.game_over.is_some() {
            self.error = Some("This Run is over: a game over can't be resumed.".to_owned());
            return;
        }
        self.close_run();
        if summary.with_friends {
            self.resume_coop_run(summary);
            return;
        }
        let result = self.find_rom(summary).and_then(|(rom, identity, _)| {
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

    /// Resume for a Run with friends, never as a solo game: the Run's
    /// network opens from this Player's copy of its Run log, and the game
    /// boots once [`resume::Resuming::ready_to_boot`] says so.
    fn resume_coop_run(&mut self, summary: &RunSummary) {
        self.resuming = None;
        let result = self.find_rom(summary).and_then(|(rom, identity, path)| {
            let RomIdentity::Profiled { profile, .. } = &identity else {
                return Err("This build has no Game Profile for the Run's game.".to_owned());
            };
            let profile = profile.clone();
            let file = self.run_library()?.open_run(&summary.id).map_err(|e| e.to_string())?;
            let engine = RunEngine::resume(Box::new(file), Box::new(SystemRunClock)).map_err(|e| e.to_string())?;
            let resume = resume::Resuming::begin(engine, path, &mut self.lobbies, SystemRunClock.now_ms())?;
            self.panel_shown = true;
            self.panel_tab = side_panel::Tab::Lobby;
            Ok(Resuming {
                resume,
                profile,
                rom,
                identity,
            })
        });
        match result {
            Ok(resuming) => self.resuming = Some(resuming),
            Err(e) => self.error = Some(e),
        }
    }

    /// Boots a resumed Run with friends' game once it can.
    fn finish_resuming(&mut self) {
        let now = SystemRunClock.now_ms();
        let Some(resuming) = self.resuming.as_mut() else {
            return;
        };
        if !resuming.resume.ready_to_boot(&mut self.lobbies, now) {
            return;
        }
        let Some(Resuming {
            resume,
            profile,
            rom,
            identity,
        }) = self.resuming.take()
        else {
            return;
        };
        let result = Self::core_with_rom(&rom).and_then(|core| {
            self.play_run(
                resume.engine,
                core,
                profile,
                rom,
                identity,
                now,
                Some((&resume.locked, Some(&resume.log))),
            )
        });
        self.show_result(result);
        self.refresh_runs();
    }

    /// The Run's ROM: where it was last time if it is still there, otherwise
    /// one the Player picks, checked by SHA-1. With where it was found.
    fn find_rom(&self, summary: &RunSummary) -> Result<(Vec<u8>, RomIdentity, PathBuf), String> {
        if let Some(path) = &summary.rom_path
            && let Ok((rom, identity)) = self.read_rom(Path::new(path))
            && sha1_hex(&rom) == summary.game_sha1
        {
            return Ok((rom, identity, PathBuf::from(path)));
        }
        let known = self
            .roms
            .borrow_mut()
            .locate(&summary.game_sha1, |path| std::fs::read(path).ok());
        if let Some((path, _)) = known
            && let Ok((rom, identity)) = self.read_rom(&path)
        {
            return Ok((rom, identity, path));
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
        Ok((rom, identity, path))
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
    /// A Run with friends resumed from its Run file comes with its log as kept there.
    #[allow(clippy::too_many_arguments)]
    fn play_run(
        &self,
        engine: RunEngine,
        core: MgbaCore,
        profile: Arc<GameProfile>,
        rom: Vec<u8>,
        identity: RomIdentity,
        clock_ms: i64,
        coop: Option<(&Locked, Option<&RunLog>)>,
    ) -> Result<Playing, String> {
        let name = engine.info().name.clone();
        let ruleset = engine.info().ruleset.clone();
        let rom: Arc<[u8]> = rom.into();
        let icons = theme::Icons::new(profile.clone(), rom.clone());
        let mut running = RunningGame::boot(ProfileGame::new(profile.clone(), rom.clone(), core), engine, clock_ms)
            .map_err(|e| format!("The Run's game couldn't be booted: {e}"))?;
        let view = running.view();
        let commands = running.commands();
        let shared = coop.map(|_| running.share());
        let spectate = match coop {
            Some((locked, resumed)) => {
                let me = self
                    .lobbies
                    .public_key()
                    .ok_or("This install has no network key, so the Run can't be shared.")?;
                let outbox = running.stream_as(me);
                let kept = running.engine().kept_shadows().to_vec();
                let mut spectate = spectate::Spectate::new(me, locked, &rom, &profile, outbox, &commands, &kept)?;
                if let Some(log) = resumed {
                    spectate.resumed(log, running.streamed_before());
                }
                // Partner Avatars: the game draws none where its profile gives no room (a Romhack).
                spectate.draw_partner_avatars(running.draw_partner_avatars());
                Some(spectate)
            }
            None => None,
        };
        let _ = commands.send(RunCommand::Speed(self.speed.multiplier() as u8));
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
        playing.icons = Some(icons);
        playing.ruleset = ruleset;
        Ok(playing)
    }

    /// Stops the game. A Run's file has finished writing when this returns.
    /// A Run with friends is always suspended, so Resume goes on exactly here.
    fn stop_playing(&mut self) {
        self.resuming = None;
        let Some(playing) = self.playing.take() else {
            return;
        };
        if let (Play::Run { emulator, .. }, Some(spectate)) = (playing.play, &playing.spectate) {
            spectate.keep_checkpoints(true);
            emulator.stop().suspend();
        }
        self.refresh_runs();
    }

    /// In a Run with friends, keeps this Player's entries waiting for a Host
    /// in the Run file whenever they change, so leaving loses none of them.
    fn keep_waiting(&mut self) {
        let Some(Playing {
            play: Play::Run { commands, .. },
            spectate: Some(_),
            ..
        }) = &self.playing
        else {
            return;
        };
        let waiting = self.lobbies.waiting();
        if waiting != self.waiting_kept {
            // The emulator thread only stops when the Run is closed.
            let _ = commands.send(RunCommand::KeepWaiting(waiting.clone()));
            self.waiting_kept = waiting;
        }
    }

    /// Close Run: the game stops, and a Run with friends is left as a lost
    /// connection would be: the others see this Player offline and play on.
    fn close_run(&mut self) {
        let with_friends = self.resuming.is_some() || self.playing.as_ref().is_some_and(|p| p.spectate.is_some());
        self.stop_playing();
        if with_friends {
            self.lobbies.leave_run();
        }
    }

    /// Suspend and quit: the Run resumes exactly here next time.
    /// A Run with friends is closed as by Close Run, which suspends it too.
    fn suspend_and_quit(&mut self) {
        if self.playing.as_ref().is_some_and(|p| p.spectate.is_some()) {
            self.close_run();
            return;
        }
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
            if let Play::Run { commands, .. } = &playing.play {
                let _ = commands.send(RunCommand::Speed(speed.multiplier() as u8));
            }
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
            ui.label(&draft.profile.name);
            if draft.local_profile {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "A developer's local Game Profile, not a bundled one: for solo Runs only.",
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Ruleset");
                egui::ComboBox::from_id_salt("preset")
                    .selected_text(preset_name(&draft.ruleset))
                    .show_ui(ui, |ui| {
                        for preset in Preset::ALL {
                            if ui
                                .selectable_label(false, preset.name())
                                .on_hover_text(rules_text::preset(preset))
                                .clicked()
                            {
                                let mut ruleset = preset.ruleset(&draft.level_caps);
                                // Alone there are no Links; rules this ROM can't enforce stay off.
                                ruleset.links = false;
                                ruleset.retries = draft.ruleset.retries;
                                draft.level_caps_on = ruleset.level_caps.is_some();
                                for (rule, on) in [
                                    (Rule::FirstEncounter, &mut ruleset.first_encounter),
                                    (Rule::DupesClause, &mut ruleset.dupes_clause),
                                    (Rule::Statics, &mut ruleset.statics),
                                    (Rule::FaintIsDead, &mut ruleset.faint_is_dead),
                                    (Rule::NoItems, &mut ruleset.no_items),
                                    (Rule::SetMode, &mut ruleset.set_mode),
                                    (Rule::MustNickname, &mut ruleset.must_nickname),
                                    (Rule::TypeClause, &mut ruleset.type_clause),
                                ] {
                                    if draft.unavailable.contains_key(&rule) {
                                        *on = false;
                                    }
                                }
                                draft.ruleset = ruleset;
                            }
                        }
                    });
            });
            ui.weak("A preset fills in the rules below. With friends, every rule can still be changed in the Lobby before the lock.");
            let rules_height = (ui.ctx().content_rect().height() * 0.55).max(200.0);
            egui::ScrollArea::vertical().max_height(rules_height).show(ui, |ui| {
            let shown = draft.ruleset.clone();
            let ruleset = &mut draft.ruleset;
            rule_checkbox(
                ui,
                &mut ruleset.first_encounter,
                Rule::FirstEncounter,
                &shown,
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
                Rule::DupesClause,
                &shown,
                dupes_unavailable,
            );
            rule_checkbox(
                ui,
                &mut ruleset.statics,
                Rule::Statics,
                &shown,
                statics_unavailable,
            );
            // Gifts that can't be released are all kept: the box stays as it is.
            let gifts_unavailable = draft
                .unavailable
                .get(&Rule::Gifts)
                .map(|reason| format!("{reason}, so every Gift is kept"));
            rules_text::rule_row(ui, &mut ruleset.gifts, Rule::Gifts, &shown, gifts_unavailable);
            rule_checkbox(
                ui,
                &mut ruleset.faint_is_dead,
                Rule::FaintIsDead,
                &shown,
                draft.unavailable.get(&Rule::FaintIsDead).cloned(),
            );
            rule_checkbox(
                ui,
                &mut ruleset.no_items,
                Rule::NoItems,
                &shown,
                draft.unavailable.get(&Rule::NoItems).cloned(),
            );
            rule_checkbox(
                ui,
                &mut ruleset.set_mode,
                Rule::SetMode,
                &shown,
                draft.unavailable.get(&Rule::SetMode).cloned(),
            );
            rule_checkbox(
                ui,
                &mut ruleset.must_nickname,
                Rule::MustNickname,
                &shown,
                draft.unavailable.get(&Rule::MustNickname).cloned(),
            );
            rule_checkbox(
                ui,
                &mut ruleset.type_clause,
                Rule::TypeClause,
                &shown,
                draft.unavailable.get(&Rule::TypeClause).cloned(),
            );
            level_caps_editor(ui, draft);
            let mut chosen = draft.ruleset.clone();
            chosen.level_caps = draft.level_caps_on.then(Vec::new);
            rules_text::native_options(ui, &pactlocke_run::native_options(&chosen, &draft.capabilities));
            ui.horizontal(|ui| {
                ui.label("Retries");
                ui.add(egui::DragValue::new(&mut draft.ruleset.retries).range(0..=pactlocke_run::MAX_RETRIES));
            });
            rules_text::description(ui, &rules_text::retries(&draft.ruleset), None);
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
                if ui.add_enabled(named, egui::Button::new("Start solo")).clicked() {
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
            let draft = self.new_run.take().expect("a draft is open");
            // The Ruleset is set in the Lobby from here, for everyone.
            self.host_lobby(&draft);
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

    /// The menu bar along the top, as other emulators have it, with the
    /// Run, its game and its speed on the right.
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let mut act: Option<Menu> = None;
        egui::MenuBar::new().ui(ui, |ui| {
            let playing = self.playing.as_ref();
            let in_run = playing.is_some_and(|p| matches!(p.play, Play::Run { .. }));
            let coop = playing.is_some_and(|p| p.spectate.is_some());
            ui.menu_button("File", |ui| {
                if item(ui, "New Run…", "Ctrl+N", true) {
                    act = Some(Menu::NewRun);
                }
                if item(ui, "Join Run…", "Ctrl+J", true) {
                    act = Some(Menu::JoinRun);
                }
                if item(ui, "Open ROM…", "Ctrl+O", true) {
                    act = Some(Menu::OpenRom);
                }
                ui.menu_button("Recent ROMs", |ui| {
                    let roms = self.roms.borrow();
                    if roms.roms().is_empty() {
                        ui.weak("None yet");
                    }
                    for rom in roms.roms().iter().take(RECENT_SHOWN) {
                        if ui
                            .button(file_stem(&rom.path))
                            .on_hover_text(format!("{}\n{}", rom.game, rom.path.display()))
                            .clicked()
                        {
                            act = Some(Menu::OpenRecent(rom.path.clone()));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                let close = if in_run { "Close Run" } else { "Close game" };
                if item(ui, close, "", playing.is_some()) {
                    act = Some(Menu::Close);
                }
                if item(ui, "Suspend and quit", "", in_run) {
                    act = Some(Menu::Suspend);
                }
                ui.separator();
                if item(ui, "Exit", "Alt+F4", true) {
                    act = Some(Menu::Exit);
                }
            });
            ui.menu_button("Emulation", |ui| {
                let paused = self.paused;
                if item(
                    ui,
                    if paused { "Resume" } else { "Pause" },
                    &self.settings.pause_key,
                    playing.is_some(),
                ) {
                    act = Some(Menu::Pause);
                }
                ui.menu_button("Speed", |ui| {
                    for speed in Speed::ALL {
                        if ui.radio(self.speed == speed, speed.to_string()).clicked() {
                            act = Some(Menu::Speed(speed));
                            ui.close();
                        }
                    }
                    ui.weak(format!("Hold {} for {}", self.settings.fast_forward_key, Speed::X4));
                });
                if let Some(left) = playing
                    .filter(|_| !coop)
                    .and_then(|p| p.view().and_then(|v| v.retries_left))
                    && item(ui, &format!("Retry · {left} left"), "", left > 0)
                {
                    act = Some(Menu::Retry);
                }
                ui.separator();
                if item(ui, "Take a screenshot", "F12", playing.is_some()) {
                    act = Some(Menu::Screenshot);
                }
            });
            ui.menu_button("View", |ui| {
                if check(ui, "Side panel", "Ctrl+B", self.panel_shown) {
                    act = Some(Menu::TogglePanel);
                }
                if check(ui, "Pop out the side panel", "", self.panel_popped) {
                    act = Some(Menu::TogglePopOut);
                }
                if coop {
                    if check(ui, "Partner view", "", playing.is_some_and(|p| p.partner_view)) {
                        act = Some(Menu::TogglePartners);
                    }
                    let saver = playing
                        .and_then(|p| p.spectate.as_ref())
                        .is_some_and(|s| s.battery_saver);
                    if check(ui, "Battery saver", "", saver) {
                        act = Some(Menu::ToggleSaver);
                    }
                }
                ui.separator();
                ui.menu_button("Window size", |ui| {
                    for n in 2..=4 {
                        if ui.button(format!("{n}x")).clicked() {
                            act = Some(Menu::WindowSize(n));
                            ui.close();
                        }
                    }
                });
                let full = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
                if check(ui, "Fullscreen", "F11", full) {
                    act = Some(Menu::Fullscreen);
                }
            });
            ui.menu_button("Options", |ui| {
                if item(ui, "Settings…", "Ctrl+,", true) {
                    act = Some(Menu::Settings(Category::General));
                }
                ui.separator();
                for category in Category::ALL {
                    if item(ui, &format!("{}…", category.name()), "", true) {
                        act = Some(Menu::Settings(category));
                    }
                }
            });
            ui.menu_button("Help", |ui| {
                if item(ui, "Open the log folder", "", true) {
                    act = Some(Menu::LogFolder);
                }
                if item(ui, "About Pactlocke", "", true) {
                    act = Some(Menu::About);
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(playing) = &self.playing {
                    ui.weak(format!("{}x", self.effective_speed().multiplier()));
                    ui.separator();
                    rom_report::status_label(ui, &playing.identity);
                    ui.separator();
                    ui.label(&playing.name);
                }
            });
        });
        if let Some(act) = act {
            self.menu(act, ui.ctx());
        }
    }

    fn menu(&mut self, act: Menu, ctx: &egui::Context) {
        match act {
            Menu::NewRun => self.begin_new_run(),
            Menu::JoinRun => self.lobbies.open_join_prompt(),
            Menu::OpenRom => self.pick_rom(),
            Menu::OpenRecent(path) => self.open_rom(&path),
            Menu::Close => self.close_run(),
            Menu::Suspend => self.suspend_and_quit(),
            Menu::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Menu::Pause => self.paused = !self.paused,
            Menu::Speed(speed) => self.set_speed(speed),
            Menu::Retry => {
                if let Some(Playing {
                    play: Play::Run { commands, .. },
                    ..
                }) = &self.playing
                {
                    let _ = commands.send(RunCommand::Retry {
                        to: pactlocke_run::RetryTo::Latest,
                    });
                }
            }
            Menu::Screenshot => self.screenshot(),
            Menu::TogglePanel => self.panel_shown = !self.panel_shown,
            Menu::TogglePopOut => self.panel_popped = !self.panel_popped,
            Menu::TogglePartners => {
                if let Some(playing) = self.playing.as_mut()
                    && let Some(spectate) = playing.spectate.as_mut()
                {
                    playing.partner_view = !playing.partner_view;
                    spectate.set_partner_view(playing.partner_view);
                }
            }
            Menu::ToggleSaver => {
                if let Some(spectate) = self.playing.as_mut().and_then(|p| p.spectate.as_mut()) {
                    let on = !spectate.battery_saver;
                    spectate.set_battery_saver(on);
                }
            }
            Menu::WindowSize(n) => {
                let panel = if self.panel_shown && !self.panel_popped {
                    self.settings.panel_width + 8.0
                } else {
                    0.0
                };
                let size = egui::vec2(GBA_W * n as f32 + panel + 16.0, GBA_H * n as f32 + 48.0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
            Menu::Fullscreen => {
                let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
            }
            Menu::Settings(category) => self.settings_dialog = Some(SettingsDialog::new(&self.settings, category)),
            Menu::LogFolder => open_folder(&data_folder()),
            Menu::About => self.about = true,
        }
    }

    /// The keyboard shortcuts the menus show, unless a text field has the keys.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Settings may be waiting for a key to rebind: no shortcut takes it.
        if ctx.egui_wants_keyboard_input() || self.settings_dialog.is_some() {
            return;
        }
        use egui::{Key, KeyboardShortcut as K, Modifiers as M};
        let acts = [
            (K::new(M::COMMAND, Key::N), Menu::NewRun),
            (K::new(M::COMMAND, Key::J), Menu::JoinRun),
            (K::new(M::COMMAND, Key::O), Menu::OpenRom),
            (K::new(M::COMMAND, Key::Comma), Menu::Settings(Category::General)),
            (K::new(M::COMMAND, Key::B), Menu::TogglePanel),
            (K::new(M::NONE, Key::F11), Menu::Fullscreen),
            (K::new(M::NONE, Key::F12), Menu::Screenshot),
        ];
        let mut fired = Vec::new();
        ctx.input_mut(|i| {
            for (shortcut, act) in acts {
                if i.consume_shortcut(&shortcut) {
                    fired.push(act);
                }
            }
        });
        let pause = egui::Key::from_name(&self.settings.pause_key);
        if self.playing.is_some() && pause.is_some_and(|k| ctx.input(|i| i.key_pressed(k))) {
            fired.push(Menu::Pause);
        }
        for act in fired {
            self.menu(act, ctx);
        }
    }

    /// The speed the game runs at now: 4x while fast-forward is held.
    fn effective_speed(&self) -> Speed {
        if self.fast_forward { Speed::X4 } else { self.speed }
    }

    /// The newest frame of the game shown, saved as a PNG beside the Runs.
    fn screenshot(&mut self) {
        let Some(playing) = &self.playing else {
            return;
        };
        let frames = match playing.spectate.as_ref().and_then(|s| s.watched()) {
            Some(shadow) => shadow.frames(),
            None => playing.play.frames(),
        };
        let Some(newest) = frames.latest() else {
            return;
        };
        let dir = data_folder().join("screenshots");
        let path = dir.join(format!(
            "{}-{}.png",
            file_stem(Path::new(&playing.name)),
            SystemRunClock.now_ms()
        ));
        let written = std::fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|()| write_png(&path, &newest.frame).map_err(|e| e.to_string()));
        match written {
            Ok(()) => log::info!("Screenshot saved to {}", path.display()),
            Err(e) => self.error = Some(format!("The screenshot couldn't be saved: {e}")),
        }
    }

    /// The start page: the three actions, then the Runs.
    fn home_screen(&mut self, ui: &mut egui::Ui) {
        let mut resume = None;
        egui::Frame::new().inner_margin(egui::Margin::same(24)).show(ui, |ui| {
            ui.heading("Pactlocke");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let action = |ui: &mut egui::Ui, title: &str, line: &str| {
                    let text = egui::RichText::new(format!("{title}\n")).strong().size(15.0);
                    let mut job = egui::text::LayoutJob::default();
                    text.append_to(&mut job, ui.style(), egui::FontSelection::Default, egui::Align::LEFT);
                    egui::RichText::new(line).weak().append_to(
                        &mut job,
                        ui.style(),
                        egui::FontSelection::Default,
                        egui::Align::LEFT,
                    );
                    ui.add_sized([200.0, 56.0], egui::Button::new(job)).clicked()
                };
                if action(ui, "New Run", "A solo Run, or invite up to two friends") {
                    self.begin_new_run();
                }
                if action(ui, "Join a Run", "Paste a friend's invite link") {
                    self.lobbies.open_join_prompt();
                }
                if action(ui, "Open a ROM", "Just play, no Run") {
                    self.pick_rom();
                }
            });
            ui.add_space(20.0);
            if let Err(e) = &self.library {
                ui.colored_label(ui.visuals().error_fg_color, e);
            } else if self.runs.is_empty() {
                ui.weak("No Runs yet. Start one with New Run, or open a ROM to just play.");
            }
            if self.library_fallback {
                ui.weak("The app's folder can't be written to, so Runs are kept in your user data folder.");
            }
            if let Some(dir) = self.dev_profiles() {
                ui.weak(format!(
                    "Developer setting: local Game Profiles from {}, for solo Runs only.",
                    dir.display()
                ));
            }
            let now = SystemRunClock.now_ms();
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("runs")
                    .num_columns(6)
                    .striped(true)
                    .spacing([18.0, 8.0])
                    .show(ui, |ui| {
                        if !self.runs.is_empty() {
                            for heading in ["Run", "Game", "Preset", "Deaths", "Last played", ""] {
                                ui.weak(heading);
                            }
                            ui.end_row();
                        }
                        for summary in &self.runs {
                            ui.vertical(|ui| {
                                ui.strong(&summary.name);
                                if let Some(from) = &summary.copied_from {
                                    ui.weak(format!("Copied from {} on {}", from.name, logger::date(from.at_ms)));
                                }
                            });
                            ui.label(&summary.game_name);
                            ui.label(preset_name(&summary.ruleset));
                            ui.label(summary.deaths.to_string());
                            ui.label(run_panel::ago(now - summary.last_played_ms));
                            ui.horizontal(|ui| {
                                if let Some(over) = &summary.game_over {
                                    ui.colored_label(
                                        ui.visuals().error_fg_color,
                                        format!("Game over · {}", logger::date(over.at_ms)),
                                    );
                                    if ui.button("Summary").clicked() {
                                        self.lost = Some(summary.clone());
                                    }
                                } else if ui.button("Resume").clicked() {
                                    resume = Some(summary.clone());
                                }
                                if summary.with_friends
                                    && summary.game_over.is_none()
                                    && ui
                                        .button("Continue as solo…")
                                        .on_hover_text("A new solo Run from where you left this one")
                                        .clicked()
                                {
                                    self.copying = Some(summary.clone());
                                }
                                if ui.button("Delete…").clicked() {
                                    self.deleting = Some(summary.clone());
                                }
                            });
                            ui.end_row();
                        }
                    });
                ui.add_space(16.0);
                self.known_games(ui);
            });
        });
        if let Some(summary) = resume {
            self.resume_run(&summary);
        }
    }

    /// The home screen's lists of the Player's games (from the ROM library)
    /// and of the Romhacks Pactlocke is tested on.
    fn known_games(&mut self, ui: &mut egui::Ui) {
        let mut open = None;
        egui::CollapsingHeader::new("Your games")
            .default_open(true)
            .show(ui, |ui| {
                let roms = self.roms.borrow();
                if roms.roms().is_empty() {
                    ui.weak("None yet: every ROM you open is remembered here (where it is, never a copy).");
                }
                for rom in roms.roms() {
                    ui.horizontal(|ui| {
                        if ui.button("Play").on_hover_text("Just play, no Run").clicked() {
                            open = Some(rom.path.clone());
                        }
                        ui.label(&rom.game);
                        ui.weak(rom.path.display().to_string());
                    });
                }
            });
        if let Some(path) = open {
            self.open_rom(&path);
        }
        if let Ok(store) = &self.profiles {
            egui::CollapsingHeader::new("Tested Romhacks").show(ui, |ui| {
                ui.weak(
                    "Bring the patched ROM of one of these: Pactlocke hosts and links to nothing, and \
                     checks the ROM by SHA-1.",
                );
                for hack in store.tested_romhacks() {
                    let base = store.by_id(&hack.base).map_or(hack.base.as_str(), |p| p.name.as_str());
                    let playable = store.find(&hack.sha1).is_some();
                    ui.horizontal(|ui| {
                        ui.strong(format!("{} {}", hack.name, hack.version));
                        ui.label(format!("by {}, on {base}", hack.author));
                        if !playable {
                            ui.weak("(Game Profile coming)");
                        }
                    });
                }
            });
        }
    }

    /// New Run's game chooser: the Player's games, newest first, or another ROM.
    fn game_chooser(&mut self, ctx: &egui::Context) {
        if !self.choosing_game {
            return;
        }
        let mut open = self.choosing_game;
        let mut chosen = None;
        let mut browse = false;
        egui::Window::new("New Run: choose the game")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for rom in self.roms.borrow().roms().iter().filter(|rom| rom.runs) {
                        if ui
                            .button(&rom.game)
                            .on_hover_text(rom.path.display().to_string())
                            .clicked()
                        {
                            chosen = Some(rom.path.clone());
                        }
                    }
                });
                ui.separator();
                browse = ui.button("Another ROM…").clicked();
            });
        self.choosing_game = open;
        if let Some(path) = chosen {
            self.new_run_on(&path);
        } else if browse {
            self.choosing_game = false;
            self.browse_for_new_run();
        }
    }

    /// A lost Run's summary, from the Runs list.
    fn lost_summary(&mut self, ctx: &egui::Context) {
        let Some(summary) = &self.lost else {
            return;
        };
        let Some(over) = &summary.game_over else {
            return;
        };
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("lost run")).show(ctx, |ui| {
            ui.heading(format!("{}: game over", summary.name));
            ui.weak(format!("{} · {}", summary.game_name, logger::date(over.at_ms)));
            ui.add_space(6.0);
            ui.label(&over.how);
            ui.add_space(6.0);
            game_over::survivors(ui, &over.longest);
            ui.label(format!(
                "Deaths: {} · play time {}",
                summary.deaths,
                game_over::duration(over.play_ms)
            ));
            ui.add_space(6.0);
            close = ui.button("Close").clicked();
        });
        if close || modal.should_close() {
            self.lost = None;
        }
    }

    /// Delete…: asks first, then deletes the Run's file.
    fn delete_prompt(&mut self, ctx: &egui::Context) {
        let Some(summary) = self.deleting.clone() else {
            return;
        };
        let (mut delete, mut keep) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("delete run")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Delete this Run?");
            ui.label(format!(
                "Delete {} ({}, {} deaths)?",
                summary.name, summary.game_name, summary.deaths
            ));
            ui.weak(
                "Its Run file, checkpoints and log are removed from this PC and can't be brought back. \
                 Friends keep their own copies. Your ROM and saves aren't touched.",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                // Keep it is the default: it has the focus, so Enter keeps the Run.
                let keep_button = ui.button("Keep it");
                if !ui.memory(|m| m.focused().is_some()) {
                    keep_button.request_focus();
                }
                keep = keep_button.clicked();
                delete = ui
                    .button(egui::RichText::new("Delete the Run").color(theme::DANGER))
                    .clicked();
            });
        });
        if delete {
            if let Ok(library) = &self.library
                && let Err(e) = library.delete(&summary.id)
            {
                self.error = Some(format!("{} couldn't be deleted: {e}", summary.name));
            }
            log::info!("Deleted the Run {}", summary.name);
            self.deleting = None;
            self.refresh_runs();
        } else if keep || modal.should_close() {
            self.deleting = None;
        }
    }

    /// Continue as solo…: asks first, then makes a solo copy of the Run
    /// with friends from where this Player left it, and resumes the copy.
    fn solo_copy_prompt(&mut self, ctx: &egui::Context) {
        let Some(summary) = self.copying.clone() else {
            return;
        };
        let (mut copy, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("solo copy")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Continue as solo");
            ui.label(format!(
                "Make a solo copy of {}? The Run with friends stays as it is.",
                summary.name
            ));
            ui.weak(
                "The copy goes on from where you left the Run with friends, with the same rules except \
                 Links and Sync Gates, and the Retries the group has left.",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                copy = ui.button("Make a solo copy").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if copy {
            self.copying = None;
            let made = self.run_library().and_then(|library| {
                library
                    .solo_copy(&summary.id, SystemRunClock.now_ms())
                    .map_err(|e| format!("{} couldn't be copied: {e}", summary.name))
            });
            self.refresh_runs();
            match made {
                Ok(id) => {
                    log::info!("Made a solo copy of the Run {}", summary.name);
                    if let Some(copy) = self.runs.iter().find(|r| r.id == id).cloned() {
                        self.resume_run(&copy);
                    }
                }
                Err(e) => self.error = Some(e),
            }
        } else if cancel || modal.should_close() {
            self.copying = None;
        }
    }

    /// The first start: the Player's name, which other Players see.
    fn first_launch(&mut self, ctx: &egui::Context) {
        if self.settings.name.is_some() || self.smoke_frames_left.is_some() {
            return;
        }
        let mut done = false;
        egui::Modal::new(egui::Id::unique("first launch")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Welcome to Pactlocke");
            ui.label("What should other Players call you? It shows in their Lobby, Player strip and feed.");
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.name_draft)
                    .hint_text("Your name")
                    .char_limit(settings::NAME_LIMIT),
            );
            field.request_focus();
            ui.weak("Settings → General changes it any time.");
            let named = !self.name_draft.trim().is_empty();
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            done = ui.add_enabled(named, egui::Button::new("Continue")).clicked() || (enter && named);
        });
        if done {
            self.settings.name = Some(self.name_draft.trim().to_owned());
            self.settings.save();
            self.apply_settings();
        }
    }

    fn about(&mut self, ctx: &egui::Context) {
        if !self.about {
            return;
        }
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("about")).show(ctx, |ui| {
            ui.heading("Pactlocke");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.label("Nuzlocke and Soul Link challenge Runs on GBA Pokémon games, alone or with up to two friends.");
            ui.weak("GPL-3.0-or-later. The emulator core is libmgba. Pactlocke never copies or changes ROMs, saves or patches.");
            close = ui.button("Close").clicked();
        });
        if close || modal.should_close() {
            self.about = false;
        }
    }

    /// Settings, while open; a solo game is paused meanwhile.
    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.settings_dialog.as_mut() else {
            return;
        };
        let gamepad = self.pads.connected();
        let facts = settings_ui::Facts {
            devices: &self.devices,
            data_dir: &data_folder(),
            gamepad: gamepad.as_deref(),
            pausing: self.playing.as_ref().is_some_and(|p| p.spectate.is_none()),
        };
        match dialog.show(ctx, &facts) {
            settings_ui::Outcome::Open => {}
            settings_ui::Outcome::Cancel => self.settings_dialog = None,
            settings_ui::Outcome::Save(settings) => {
                self.settings = *settings;
                self.settings.save();
                self.apply_settings();
                self.settings_dialog = None;
                // A new filter or colour correction shows at once, paused or not.
                if let Some(playing) = self.playing.as_mut() {
                    playing.texture = None;
                }
            }
        }
    }

    /// Puts the settings to work where they apply at once.
    fn apply_settings(&mut self) {
        self.lobbies.name = self.settings.name().to_owned();
        self.lobbies.relays = self.settings.relay_choice();
        logger::set_verbose(self.settings.verbose_log);
    }

    /// The developer's local Game Profiles folder, from Settings or the environment.
    fn dev_profiles(&self) -> Option<PathBuf> {
        self.settings.dev_profiles.clone().or_else(dev_profiles_dir)
    }

    fn game_screen(&mut self, ui: &mut egui::Ui) {
        if let Some(resuming) = &self.resuming {
            ui.centered_and_justified(|ui| {
                ui.label(format!(
                    "Resuming {}: getting your last moves back from your Host…",
                    resuming.resume.locked.run_name
                ));
            });
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let Some(playing) = self.playing.as_mut() else {
            self.home_screen(ui);
            return;
        };

        // Faint = Dead: a game over replaces the game; a wiped party waits
        // on a Retry over the stopped game.
        if let Some(view) = playing.view() {
            let me = self.lobbies.public_key();
            if let Some(over) = &view.game_over {
                game_over::screen(ui, over, &view, self.lobbies.log(), me);
                return;
            }
            if let Some(wipe) = game_over::wipe_of(&view, self.lobbies.log(), me) {
                let online = self.lobbies.online();
                match game_over::wipe_bar(ui, &wipe, self.lobbies.log(), me, online) {
                    Some(game_over::WipeAnswer::Command(command)) => {
                        if let Play::Run { commands, .. } = &playing.play {
                            let _ = commands.send(command);
                        }
                    }
                    Some(game_over::WipeAnswer::Vote(vote)) => self.lobbies.share(vec![vote]),
                    None => {}
                }
            }
        }

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
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .inner_margin(6)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(format!("Watching {name}"));
                            if let Some(why) = cant_watch {
                                ui.colored_label(ui.visuals().error_fg_color, why);
                            } else if status.catching_up {
                                ui.weak("catching up…");
                            } else {
                                ui.weak("Their game and sound, about half a second behind. Yours is paused.");
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                back = ui.button("Back to your game").clicked();
                            });
                        });
                    });
                if back && let Some(spectate) = playing.spectate.as_mut() {
                    spectate.stop_watching();
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
        let pad = if self.settings.gamepad {
            self.pads.held()
        } else {
            Keys::NONE
        };
        // Asked outside `input`: egui's context is locked while that runs.
        // Keys pressed in Settings (rebinding one, say) never reach the game.
        let typing = ui.ctx().egui_wants_keyboard_input() || self.settings_dialog.is_some();
        let keys = ui.input(|i| {
            if watched.is_some() || typing {
                return Keys::NONE;
            }
            let keyboard = if i.focused { self.settings.held(i) } else { Keys::NONE };
            keyboard | pad
        });
        playing.play.set_keys(keys);

        // Show the newest published frame.
        let filter = if self.settings.smooth {
            egui::TextureOptions::LINEAR
        } else {
            egui::TextureOptions::NEAREST
        };
        if let Some(newest) = frames.latest()
            && (playing.texture.is_none() || newest.seq != playing.shown_seq)
        {
            let rgba = if self.settings.colour_correction {
                colour_correct(&newest.frame.rgba)
            } else {
                newest.frame.rgba.clone()
            };
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [newest.frame.width as usize, newest.frame.height as usize],
                &rgba,
            );
            match &mut playing.texture {
                Some(texture) => texture.set(image, filter),
                None => playing.texture = Some(ui.ctx().load_texture("game", image, filter)),
            }
            playing.shown_seq = newest.seq;
        }

        if let Some(texture) = &playing.texture {
            let available = ui.available_size();
            let size = texture.size_vec2();
            let fit = (available.x / size.x).min(available.y / size.y);
            let scale = if self.settings.integer_scale && fit >= 1.0 {
                fit.floor()
            } else {
                fit.max(0.1)
            };
            ui.centered_and_justified(|ui| {
                let image = ui.add(egui::Image::new(texture).fit_to_exact_size(size * scale));
                if self.paused && watched.is_none() {
                    ui.painter().text(
                        image.rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Paused",
                        egui::FontId::proportional(28.0),
                        egui::Color32::WHITE,
                    );
                }
            });
        }
    }
}

/// What a menu item asked for.
#[derive(Clone)]
enum Menu {
    NewRun,
    JoinRun,
    OpenRom,
    OpenRecent(PathBuf),
    Close,
    Suspend,
    Exit,
    Pause,
    Speed(Speed),
    Retry,
    Screenshot,
    TogglePanel,
    TogglePopOut,
    TogglePartners,
    ToggleSaver,
    WindowSize(u8),
    Fullscreen,
    Settings(Category),
    LogFolder,
    About,
}

/// A menu item with its shortcut; whether it was clicked.
fn item(ui: &mut egui::Ui, text: &str, shortcut: &str, enabled: bool) -> bool {
    let clicked = ui
        .add_enabled(enabled, egui::Button::new(text).shortcut_text(shortcut))
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// A menu item that is on or off, with a tick when on; whether it was clicked.
fn check(ui: &mut egui::Ui, text: &str, shortcut: &str, on: bool) -> bool {
    let label = if on {
        format!("✔ {text}")
    } else {
        format!("    {text}")
    };
    item(ui, &label, shortcut, true)
}

use side_panel::preset_name;

/// The folder the app keeps its data in: the Runs, the log, settings.
fn data_folder() -> PathBuf {
    logger::path().parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Opens a folder in the system's file manager.
pub fn open_folder(dir: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(program).arg(dir).spawn() {
        log::warn!("Couldn't open {}: {e}", dir.display());
    }
}

/// A frame as a PNG file.
fn write_png(path: &Path, frame: &pactlocke_emu::Frame) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&frame.rgba)?;
    Ok(())
}

/// A GBA screen's colours, for a modern one: its LCD was darker and its
/// colours ran into each other a little.
fn colour_correct(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.as_chunks_mut::<4>().0 {
        let [r, g, b] = [px[0], px[1], px[2]].map(|c| (f32::from(c) / 255.0).powf(2.2));
        let mix = [
            0.82 * r + 0.24 * g - 0.06 * b,
            0.09 * r + 0.70 * g + 0.21 * b,
            0.09 * r + 0.06 * g + 0.85 * b,
        ];
        for (c, v) in px[..3].iter_mut().zip(mix) {
            *c = (v.clamp(0.0, 1.0).powf(1.0 / 2.0) * 255.0) as u8;
        }
    }
    out
}

/// The partner view: each other Player's Shadow Game as a tile, live, with
/// Watch. Returns the Shadow to watch, if one was asked for.
fn partner_tiles(ui: &mut egui::Ui, playing: &mut Playing) -> Option<usize> {
    let spectate = playing.spectate.as_ref()?;
    let mut watch = None;
    playing.tiles.resize_with(spectate.shadows.len(), || None);
    ui.strong("Partner view");
    for (i, (shadow, tile)) in spectate.shadows.iter().zip(playing.tiles.iter_mut()).enumerate() {
        ui.add_space(6.0);
        let newest = shadow.frames().latest();
        if let Some(newest) = newest.filter(|n| tile.as_ref().is_none_or(|(seq, _)| *seq != n.seq)) {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [newest.frame.width as usize, newest.frame.height as usize],
                &newest.frame.rgba,
            );
            match tile {
                Some((seq, texture)) => {
                    texture.set(image, egui::TextureOptions::NEAREST);
                    *seq = newest.seq;
                }
                None => {
                    let texture =
                        ui.ctx()
                            .load_texture(format!("tile {}", shadow.name), image, egui::TextureOptions::NEAREST);
                    *tile = Some((newest.seq, texture));
                }
            }
        }
        match tile {
            Some((_, texture)) => {
                ui.add(egui::Image::new(&*texture).fit_to_exact_size(egui::vec2(GBA_W * 0.85, GBA_H * 0.85)));
            }
            None => {
                ui.weak("no picture yet");
            }
        }
        ui.horizontal(|ui| {
            ui.label(&shadow.name);
            if shadow.cant_watch().is_none() && ui.button("Watch").clicked() {
                watch = Some(i);
            }
        });
    }
    ui.add_space(6.0);
    ui.weak("View → Partner view turns this off.");
    watch
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
            partner_view: false,
            tiles: Vec::new(),
            icons: None,
            ruleset: Ruleset::default(),
            paused: false,
        }
    }

    /// The Run's view, in a Run.
    fn view(&self) -> Option<RunView> {
        match &self.play {
            Play::Run { view, .. } => Some(view.lock().unwrap_or_else(|e| e.into_inner()).clone()),
            Play::Free(_) => None,
        }
    }
}

/// The file's name without its extension.
fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The side panel's contents: a Run's panel, the ROM report outside a Run,
/// or the Lobby before a Run with friends starts.
#[allow(clippy::too_many_arguments)]
fn panel_contents(
    ui: &mut egui::Ui,
    playing: Option<&mut Playing>,
    lobbies: &mut lobby_ui::Lobbies,
    tab: &mut side_panel::Tab,
    speed: u8,
) -> side_panel::Asks {
    let Some(playing) = playing else {
        side_panel::lobby_only(ui, lobbies);
        return side_panel::Asks::default();
    };
    let Some(view) = playing.view() else {
        rom_report::rules_panel(ui, &playing.identity);
        return side_panel::Asks::default();
    };
    let locked_ruleset = lobbies.log().and_then(|l| l.locked()).map(|l| l.ruleset);
    let ruleset = match (&playing.spectate, locked_ruleset) {
        (Some(_), Some(locked)) => locked,
        _ => playing.ruleset.clone(),
    };
    let Some(icons) = playing.icons.as_mut() else {
        return side_panel::Asks::default();
    };
    let run = side_panel::RunPanel {
        view: &view,
        ruleset: &ruleset,
        spectate: playing.spectate.as_ref(),
        speed,
    };
    side_panel::show(ui, run, icons, lobbies, tab)
}

impl eframe::App for PactlockeApp {
    /// The window's close button or Exit: a Run with friends is suspended
    /// and its network left, so Resume goes on exactly where the Player stopped.
    fn on_exit(&mut self) {
        self.close_run();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        egui::Panel::top("menu bar").show(ui, |ui| self.menu_bar(ui));
        if let Some(error) = &self.error {
            let error = error.clone();
            egui::Panel::bottom("error").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    if ui.small_button("✕").clicked() {
                        self.error = None;
                    }
                });
            });
        }

        // Fast-forward while its key is held; mute meanwhile if Settings say so.
        let keyboard = !ctx.egui_wants_keyboard_input()
            && self.settings_dialog.is_none()
            && egui::Key::from_name(&self.settings.fast_forward_key).is_some_and(|k| ctx.input(|i| i.key_down(k)));
        let held = keyboard || (self.settings.gamepad && self.pads.fast_forward());
        if held != self.fast_forward {
            self.fast_forward = held;
            let speed = self.effective_speed();
            if let Some(playing) = &self.playing {
                playing.play.set_speed(speed);
                if let Play::Run { commands, .. } = &playing.play {
                    let _ = commands.send(RunCommand::Speed(speed.multiplier() as u8));
                }
            }
        }
        let muted = self.settings.mute_fast_forward && self.fast_forward;
        audio::set_gain(&self.gain, if muted { 0.0 } else { self.settings.volume });

        // Pause: asked for, Settings open in a solo game, or a solo game unfocused.
        if let Some(playing) = self.playing.as_mut() {
            let solo = playing.spectate.is_none();
            let unfocused = self.settings.pause_unfocused && !ctx.input(|i| i.focused);
            let watching = playing.spectate.as_ref().is_some_and(|s| s.watching.is_some());
            let pause = self.paused || watching || (solo && (self.settings_dialog.is_some() || unfocused));
            if pause != playing.paused {
                playing.paused = pause;
                playing.play.set_paused(pause);
            }
        }

        let in_lobby_only = self.playing.is_none() && self.lobbies.in_lobby();
        let show_panel = (self.playing.is_some() && self.panel_shown) || in_lobby_only;
        let speed = self.effective_speed().multiplier() as u8;
        let mut asks = side_panel::Asks::default();
        if show_panel && self.panel_popped && self.playing.is_some() {
            let mut closed = false;
            let title = self.playing.as_ref().map_or_else(String::new, |p| p.name.clone());
            let (mut playing, lobbies, tab) = (self.playing.as_mut(), &mut self.lobbies, &mut self.panel_tab);
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("side panel window"),
                egui::ViewportBuilder::default()
                    .with_title(format!("{title} · Pactlocke"))
                    .with_inner_size([400.0, 760.0]),
                |ui, _class| {
                    egui::CentralPanel::default_margins().show(ui, |ui| {
                        asks = panel_contents(ui, playing.as_deref_mut(), lobbies, tab, speed);
                    });
                    closed = ui.ctx().input(|i| i.viewport().close_requested());
                },
            );
            if closed {
                self.panel_popped = false;
            }
        } else if show_panel {
            let (playing, lobbies, tab) = (self.playing.as_mut(), &mut self.lobbies, &mut self.panel_tab);
            let shown = egui::Panel::right("side panel")
                .resizable(true)
                .default_size(self.settings.panel_width)
                .size_range(settings::PANEL_WIDTHS.0..=settings::PANEL_WIDTHS.1)
                .show(ui, |ui| asks = panel_contents(ui, playing, lobbies, tab, speed));
            let width = shown.response.rect.width();
            if (width - self.settings.panel_width).abs() > 1.0 && !ctx.input(|i| i.pointer.any_down()) {
                self.settings.panel_width = width;
                self.settings.save();
            }
        }
        if let Some(playing) = self.playing.as_mut() {
            if let Some(command) = asks.command.take()
                && let Play::Run { commands, .. } = &playing.play
            {
                // The emulator thread only stops when the Run is closed.
                let _ = commands.send(command);
            }
            let mut watch = asks.watch;
            if playing.partner_view && playing.spectate.is_some() {
                egui::Panel::left("partner view")
                    .resizable(false)
                    .show(ui, |ui| watch = watch.or(partner_tiles(ui, playing)));
            }
            if let Some(spectate) = playing.spectate.as_mut() {
                if let Some(i) = watch {
                    let audio = playing.audio_out.as_ref().filter(|_| self.settings.watch_sound);
                    spectate.watch(i, audio);
                }
                spectate.pump(&mut self.lobbies);
                match asks.dropout {
                    Some(spectate::DropoutAsk::Vote(vote)) => self.lobbies.share(vec![vote]),
                    Some(spectate::DropoutAsk::KeepWaiting(player)) => spectate.keep_waiting(player),
                    None => {}
                }
                if let Some(vote) = asks.vote {
                    self.lobbies.share(vec![vote]);
                }
                if let (Play::Run { commands, .. }, Some(log)) = (&playing.play, self.lobbies.log()) {
                    for command in spectate.follow(log) {
                        // The emulator thread only stops when the Run is closed.
                        let _ = commands.send(command);
                    }
                }
                // Streams are passed on from the UI thread: keep it awake.
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        egui::CentralPanel::default_margins().show(ui, |ui| self.game_screen(ui));
        self.new_run_prompt(&ctx);
        self.delete_prompt(&ctx);
        self.lost_summary(&ctx);
        self.game_chooser(&ctx);
        self.solo_copy_prompt(&ctx);
        self.settings_window(&ctx);
        self.about(&ctx);
        self.first_launch(&ctx);
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
        self.keep_waiting();
        let joined = self.lobbies.in_lobby();
        if let Some(start) = self.lobbies.show(&ctx) {
            self.start_coop_run(start);
        }
        self.finish_resuming();
        // Hosting or joining opens the side panel on its Lobby tab; a game
        // played alone closes, so the Lobby can be seen.
        if !self.was_in_lobby && joined {
            if self.playing.as_ref().is_some_and(|p| p.spectate.is_none()) {
                self.stop_playing();
            }
            self.panel_shown = true;
            self.panel_tab = side_panel::Tab::Lobby;
        }
        self.was_in_lobby = joined;
        if self.settings.gamepad && self.playing.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }

        if let Some((path, left)) = self.shot.as_mut() {
            *left = left.saturating_sub(1);
            if *left == 0 {
                let (path, saved) = (path.clone(), self.shot_saved.clone());
                ctx.request_screenshot(move |image| {
                    let frame = pactlocke_emu::Frame {
                        width: image.size[0] as u32,
                        height: image.size[1] as u32,
                        rgba: image.as_raw().to_vec(),
                    };
                    if let Err(e) = write_png(&path, &frame) {
                        log::warn!("--shot couldn't write {}: {e}", path.display());
                    }
                    saved.store(true, std::sync::atomic::Ordering::Release);
                });
                self.shot = None;
            }
            ctx.request_repaint();
        }
        if self.shot_saved.load(std::sync::atomic::Ordering::Acquire) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.shot.is_none() && Arc::strong_count(&self.shot_saved) > 1 {
            // A screenshot is on its way: keep drawing until it lands.
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }

        if let Some(left) = self.smoke_frames_left.as_mut() {
            *left = left.saturating_sub(1);
            if *left == 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
    }
}
