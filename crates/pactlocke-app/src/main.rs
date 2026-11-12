//! Pactlocke: pick a GBA ROM and play it, with keyboard input, sound and a
//! speed from 1x to 4x. The walking skeleton the rest of the app builds on.
//!
//! Usage: `pactlocke [ROM]` opens the ROM straight away; `pactlocke --smoke`
//! starts the UI, draws a few frames and quits (used by CI).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod rom_report;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use pactlocke_emu::{CoreAdapter, Emulator, EmulatorOptions, Keys, Speed, SystemClock};
use pactlocke_game::{ProfileStore, RomIdentity};
use pactlocke_mgba::MgbaCore;

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
            .with_inner_size([720.0, 520.0])
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

struct Playing {
    emulator: Emulator<MgbaCore>,
    /// Kept alive for as long as the game plays.
    _audio: Option<audio::AudioOut>,
    texture: Option<egui::TextureHandle>,
    shown_seq: u64,
    name: String,
    /// What the ROM is, and what its Game Profile can enforce.
    identity: RomIdentity,
}

struct PactlockeApp {
    ctx: egui::Context,
    playing: Option<Playing>,
    /// The bundled Game Profiles; an error here is a broken build.
    profiles: Result<ProfileStore, String>,
    speed: Speed,
    error: Option<String>,
    smoke_frames_left: Option<u32>,
}

impl PactlockeApp {
    fn new(ctx: egui::Context, smoke: bool) -> Self {
        Self {
            ctx,
            playing: None,
            profiles: ProfileStore::bundled().map_err(|e| format!("The bundled Game Profiles are broken: {e}")),
            speed: Speed::X1,
            error: None,
            smoke_frames_left: smoke.then_some(SMOKE_FRAMES),
        }
    }

    fn pick_rom(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Choose a GBA ROM")
            .add_filter("GBA ROM", &["gba", "agb", "bin"])
            .pick_file();
        if let Some(path) = picked {
            self.open_rom(&path);
        }
    }

    fn open_rom(&mut self, path: &Path) {
        // Dropping the old game stops its emulator thread.
        self.playing = None;
        match self.start(path) {
            Ok(playing) => {
                self.playing = Some(playing);
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }

    fn start(&self, path: &Path) -> Result<Playing, String> {
        // The ROM is read into memory only; it is never copied anywhere.
        let rom = std::fs::read(path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
        let identity = self.profiles.as_ref().map_err(Clone::clone)?.identify(&rom);
        let mut core = MgbaCore::new();
        core.load_rom(&rom).map_err(|e| format!("{}: {e}", path.display()))?;
        // The game clock starts now; a Run will start it at the Ruleset lock.
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        core.set_clock(now_ms);

        let audio = audio::AudioOut::open();
        let ctx = self.ctx.clone();
        let mut options = EmulatorOptions::new(SystemClock::default())
            .speed(self.speed)
            .on_frame(move || ctx.request_repaint());
        if let Some(out) = &audio {
            options = options.audio(out.sink());
        }
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Playing {
            emulator: Emulator::spawn(core, options),
            _audio: audio,
            texture: None,
            shown_seq: 0,
            name,
            identity,
        })
    }

    fn set_speed(&mut self, speed: Speed) {
        self.speed = speed;
        if let Some(playing) = &self.playing {
            playing.emulator.set_speed(speed);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Open ROM…").clicked() {
                self.pick_rom();
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

    fn game_screen(&mut self, ui: &mut egui::Ui) {
        let Some(playing) = self.playing.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.label(format!("Open a GBA ROM to play.\n\n{}", key_help()));
            });
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
        playing.emulator.set_keys(keys);

        // Show the newest published frame.
        if let Some(newest) = playing.emulator.frames().latest()
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
            egui::Panel::right("rules")
                .resizable(true)
                .default_size(300.0)
                .show(ui, |ui| rom_report::rules_panel(ui, &playing.identity));
        }
        egui::CentralPanel::default_margins().show(ui, |ui| self.game_screen(ui));

        if let Some(left) = self.smoke_frames_left.as_mut() {
            *left = left.saturating_sub(1);
            if *left == 0 {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.ctx().request_repaint();
        }
    }
}
