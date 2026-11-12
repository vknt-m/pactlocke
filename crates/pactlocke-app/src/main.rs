//! Pactlocke: start or resume a solo Run on a supported game and play it
//! with its rules enforced, or just open a GBA ROM and play it, with keyboard
//! input, sound and a speed from 1x to 4x.
//!
//! Usage: `pactlocke [ROM]` opens the ROM straight away; `pactlocke --smoke`
//! starts the UI, draws a few frames and quits (used by CI).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod data_dir;
mod rom_report;
mod run_panel;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui;
use pactlocke_emu::{CoreAdapter, Emulator, EmulatorOptions, FrameSlot, Keys, Speed, SystemClock};
use pactlocke_game::{Capability, GameProfile, ProfileGame, ProfileStore, RomIdentity, Rule, sha1_hex};
use pactlocke_mgba::MgbaCore;
use pactlocke_run::{
    NewRun, Ruleset, RunClock, RunEngine, RunLibrary, RunSummary, RunView, RunningGame, SystemRunClock,
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
        emulator: Emulator<RunningGame<MgbaCore>>,
        view: Arc<Mutex<RunView>>,
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
}

struct Playing {
    play: Play,
    /// Kept alive for as long as the game plays.
    _audio: Option<audio::AudioOut>,
    texture: Option<egui::TextureHandle>,
    shown_seq: u64,
    name: String,
    /// What the ROM is, and what its Game Profile can enforce.
    identity: RomIdentity,
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
}

impl PactlockeApp {
    fn new(ctx: egui::Context, smoke: bool) -> Self {
        let (dir, library_fallback) = data_dir::runs_dir();
        let library = RunLibrary::open(dir).map_err(|e| format!("Runs can't be kept: {e}"));
        let runs = library.as_ref().map(RunLibrary::list).unwrap_or_default();
        Self {
            ctx,
            playing: None,
            profiles: ProfileStore::bundled().map_err(|e| format!("The bundled Game Profiles are broken: {e}")),
            library,
            library_fallback,
            runs,
            new_run: None,
            speed: Speed::X1,
            error: None,
            smoke_frames_left: smoke.then_some(SMOKE_FRAMES),
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
            let core = self.boot(&rom, None).map_err(|e| format!("{}: {e}", path.display()))?;
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

    /// A core with the ROM loaded, and the Run's save memory if it has one.
    fn boot(&self, rom: &[u8], save: Option<&[u8]>) -> Result<MgbaCore, String> {
        let mut core = MgbaCore::new();
        core.load_rom(rom).map_err(|e| e.to_string())?;
        if let Some(save) = save {
            core.load_save_memory(save)
                .map_err(|e| format!("The Run's in-game save couldn't be loaded: {e}"))?;
        }
        // The game clock starts now; a co-op Run will start it at the Ruleset lock.
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
            let unavailable: BTreeMap<Rule, String> = [Rule::FirstEncounter, Rule::DupesClause, Rule::FaintIsDead]
                .into_iter()
                .filter_map(|rule| unavailable_reason(capabilities.get(rule)).map(|why| (rule, why)))
                .collect();
            let mut ruleset = Ruleset {
                first_encounter: !unavailable.contains_key(&Rule::FirstEncounter),
                faint_is_dead: !unavailable.contains_key(&Rule::FaintIsDead),
                ..Ruleset::default()
            };
            ruleset.dupes_clause = ruleset.dupes_clause_available() && !unavailable.contains_key(&Rule::DupesClause);
            Ok(NewRunDraft {
                ruleset,
                unavailable,
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

    fn start_run(&mut self, draft: NewRunDraft) {
        self.stop_playing();
        let result = self.run_library().and_then(|library| {
            // Boot first, so a game that won't start leaves no Run behind.
            let core = self.boot(&draft.rom, None)?;
            let file = library.create().map_err(|e| e.to_string())?;
            let new = NewRun {
                name: draft.name.trim().to_owned(),
                game_name: draft.profile.name.clone(),
                game_sha1: draft.profile.sha1.clone(),
                rom_path: Some(draft.path.display().to_string()),
                ruleset: draft.ruleset,
            };
            let engine = RunEngine::start(new, Box::new(file), Box::new(SystemRunClock)).map_err(|e| e.to_string())?;
            Ok(self.play_run(engine, core, draft.profile, draft.rom, draft.identity))
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
            let core = self.boot(&rom, engine.save_memory())?;
            Ok(self.play_run(engine, core, profile, rom, identity))
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

    /// Plays a Run's booted game with the Run engine watching it.
    fn play_run(
        &self,
        engine: RunEngine,
        core: MgbaCore,
        profile: Arc<GameProfile>,
        rom: Vec<u8>,
        identity: RomIdentity,
    ) -> Playing {
        let name = engine.info().name.clone();
        let running = RunningGame::new(ProfileGame::new(profile, rom, core), engine);
        let view = running.view();
        let (emulator, audio) = self.spawn(running);
        Playing::new(Play::Run { emulator, view }, audio, name, identity)
    }

    /// Stops the game. A Run's file has finished writing when this returns.
    fn stop_playing(&mut self) {
        if self.playing.take().is_some() {
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
                    if ui.button(label).clicked() {
                        self.stop_playing();
                    }
                }
                None => {
                    if ui.button("New Run…").clicked() {
                        self.begin_new_run();
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
        let modal = egui::Modal::new(egui::Id::unique("new run")).show(ctx, |ui| {
            ui.heading("New Run");
            ui.label(format!("{}: solo.", draft.profile.name));
            ui.add_space(6.0);
            let ruleset = &mut draft.ruleset;
            rule_checkbox(
                ui,
                &mut ruleset.first_encounter,
                "First Encounter: only the first wild battle in each Area can be caught",
                draft.unavailable.get(&Rule::FirstEncounter).cloned(),
            );
            if !ruleset.dupes_clause_available() {
                ruleset.dupes_clause = false;
            }
            let dupes_unavailable = if ruleset.dupes_clause_available() {
                draft.unavailable.get(&Rule::DupesClause).cloned()
            } else {
                Some("it needs First Encounter on".to_owned())
            };
            rule_checkbox(
                ui,
                &mut ruleset.dupes_clause,
                "Dupes clause: a wild Pokémon from a family you've had doesn't count",
                dupes_unavailable,
            );
            rule_checkbox(
                ui,
                &mut ruleset.faint_is_dead,
                "Faint = Dead: fainted Pokémon go to the Dead box",
                draft.unavailable.get(&Rule::FaintIsDead).cloned(),
            );
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
            });
        });
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

        // Hold the keys that are down right now (only while the window has focus).
        let keys = ui.input(|i| {
            if !i.focused {
                return Keys::NONE;
            }
            KEY_MAP
                .iter()
                .filter(|(key, _, _)| i.key_down(*key))
                .fold(Keys::NONE, |held, (_, gba, _)| held | *gba)
        });
        playing.play.set_keys(keys);

        // Show the newest published frame.
        if let Some(newest) = playing.play.frames().latest()
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
            _audio: audio,
            texture: None,
            shown_seq: 0,
            name,
            identity,
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
        if let Some(playing) = &self.playing {
            egui::Panel::right("side panel")
                .resizable(true)
                .default_size(360.0)
                .show(ui, |ui| match &playing.play {
                    Play::Run { view, .. } => {
                        let view = view.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        run_panel::show(ui, &view);
                    }
                    Play::Free(_) => rom_report::rules_panel(ui, &playing.identity),
                });
        }
        egui::CentralPanel::default_margins().show(ui, |ui| self.game_screen(ui));
        let ctx = ui.ctx().clone();
        self.new_run_prompt(&ctx);

        if let Some(left) = self.smoke_frames_left.as_mut() {
            *left = left.saturating_sub(1);
            if *left == 0 {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.ctx().request_repaint();
        }
    }
}
