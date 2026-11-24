//! The home screen: New Run, Join a Run and Open a ROM on top, then two
//! tabs. Runs lists every Run as a table row, newest played first, with the
//! lost ones after the rest. Games shows the Player's ROMs as tiles with
//! their title screens, the folders they come from, and the Tested Romhacks.
//! It only draws and says what the Player asked for; the app does it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use pactlocke_game::{ProfileStore, TestedRomhack};
use pactlocke_run::{RunClock, RunSummary, SystemRunClock};

use crate::games::{Done, Games};
use crate::rom_library::KnownRom;
use crate::settings::Accent;
use crate::{logger, metal, run_panel, theme};

/// A title-screen thumbnail's width in the Runs table, in points (GBA's 3:2).
const THUMB: f32 = 60.0;
/// A game tile's width, in points.
const TILE: f32 = 210.0;

/// The home screen's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Runs,
    Games,
}

/// What the Player asked for on the home screen.
#[derive(Debug, Clone)]
pub enum Ask {
    NewRun,
    JoinRun,
    OpenRom,
    Resume(Box<RunSummary>),
    Summary(Box<RunSummary>),
    ContinueSolo(Box<RunSummary>),
    Delete(Box<RunSummary>),
    /// New Run, with this ROM picked.
    NewRunOn(PathBuf),
    /// Just play this ROM, no Run.
    Play(PathBuf),
    AddFolder,
}

/// Title screens as textures, by SHA-1, asked of the games thread once each.
#[derive(Default)]
pub struct TitleArt {
    textures: BTreeMap<String, Option<egui::TextureHandle>>,
}

impl TitleArt {
    /// Takes in what the games threads sent back: ROMs found go to `found`.
    pub fn take_done(&mut self, games: &Games, mut found: impl FnMut(KnownRom)) {
        for done in games.done() {
            match done {
                Done::Found(rom) => found(rom),
                // Loaded on the next look.
                Done::Title { sha1, kept: true } => {
                    self.textures.remove(&sha1);
                }
                Done::Title { .. } => {}
            }
        }
    }

    /// The title screen of the ROM with this SHA-1, asking for it the first
    /// time if `path` says where the ROM is.
    fn get(
        &mut self,
        ctx: &egui::Context,
        games: &Games,
        sha1: &str,
        path: Option<&Path>,
    ) -> Option<egui::TextureHandle> {
        let key = sha1.to_ascii_lowercase();
        if let Some(texture) = self.textures.get(&key) {
            return texture.clone();
        }
        let texture = games.titles().load(&key).map(|frame| {
            let image =
                egui::ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.rgba);
            ctx.load_texture(format!("title {key}"), image, egui::TextureOptions::LINEAR)
        });
        if texture.is_none()
            && !games.titles().tried(&key)
            && let Some(path) = path
        {
            games.want_title(&key, path);
        }
        self.textures.insert(key, texture.clone());
        texture
    }
}

/// Everything the home screen shows.
pub struct Home<'a> {
    pub runs: &'a [RunSummary],
    pub roms: &'a [KnownRom],
    pub folders: &'a [PathBuf],
    pub profiles: Option<&'a ProfileStore>,
    pub accent: Accent,
    /// Notes under the actions: the Run library's error, where Runs are
    /// kept, the developer setting.
    pub notes: Vec<Note>,
}

/// A note under the actions.
pub enum Note {
    /// Something wrong, in the error colour.
    Error(String),
    /// Something to know, dimmed.
    Info(String),
}

/// The home screen; `tab` is the open tab, changed here.
pub fn show(ui: &mut egui::Ui, home: &Home<'_>, tab: &mut Tab, art: &mut TitleArt, games: &Games) -> Option<Ask> {
    let mut ask = None;
    egui::Frame::new().inner_margin(egui::Margin::same(24)).show(ui, |ui| {
        ui.heading("Pactlocke");
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if new_run(ui, home.accent).clicked() {
                ask = Some(Ask::NewRun);
            }
            if quiet_action(ui, "Join a Run", "Paste a friend's invite link").clicked() {
                ask = Some(Ask::JoinRun);
            }
            if quiet_action(ui, "Open a ROM", "Just play, no Run").clicked() {
                ask = Some(Ask::OpenRom);
            }
        });
        for note in &home.notes {
            match note {
                Note::Error(text) => ui.colored_label(ui.visuals().error_fg_color, text),
                Note::Info(text) => ui.weak(text),
            };
        }
        ui.add_space(14.0);
        tabs(ui, home, tab);
        ui.add_space(8.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let asked = match tab {
                Tab::Runs => runs(ui, home, art, games),
                Tab::Games => games_tab(ui, home, art, games),
            };
            ask = ask.take().or(asked);
        });
    });
    ask
}

/// The size of each of the three actions.
fn action_size(ui: &egui::Ui) -> Vec2 {
    Vec2::new(250.0, crate::ui_scale::text_points(ui.style(), 15.0) * 2.0 + 26.0)
}

/// A title and its description, as one block of text.
fn title_and_line(
    ui: &egui::Ui,
    title: &str,
    line: &str,
    title_colour: Color32,
    line_colour: Color32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    RichText::new(format!("{title}\n"))
        .strong()
        .size(crate::ui_scale::text_points(ui.style(), 15.0))
        .color(title_colour)
        .append_to(&mut job, ui.style(), egui::FontSelection::Default, egui::Align::LEFT);
    RichText::new(line).color(line_colour).append_to(
        &mut job,
        ui.style(),
        egui::FontSelection::Default,
        egui::Align::LEFT,
    );
    job
}

/// New Run, in the accent.
fn new_run(ui: &mut egui::Ui, accent: Accent) -> egui::Response {
    let ink = metal::ink(accent);
    let job = title_and_line(
        ui,
        "New Run",
        "A solo Run, or invite up to two friends",
        ink,
        ink.gamma_multiply(0.85),
    );
    metal::button(ui, accent, job.into(), action_size(ui))
}

/// Join a Run and Open a ROM: a quiet fill that brightens under the pointer.
fn quiet_action(ui: &mut egui::Ui, title: &str, line: &str) -> egui::Response {
    let size = action_size(ui);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let (fill, edge) = if hovered {
            (Color32::from_gray(0x31), Color32::from_gray(0x4A))
        } else {
            (Color32::from_gray(0x2B), Color32::from_gray(0x35))
        };
        ui.painter()
            .rect(rect, 6.0, fill, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
        let job = title_and_line(
            ui,
            title,
            line,
            ui.visuals().strong_text_color(),
            ui.visuals().weak_text_color(),
        );
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let at = egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, Color32::WHITE);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Runs and Games, the open one underlined in the accent.
fn tabs(ui: &mut egui::Ui, home: &Home<'_>, tab: &mut Tab) {
    let accent = metal::colour(home.accent);
    let row = ui.horizontal(|ui| {
        for (choice, name, count) in [
            (Tab::Runs, "Runs", home.runs.len()),
            (Tab::Games, "Games", home.roms.len()),
        ] {
            let open = *tab == choice;
            let colour = if open {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            let label = RichText::new(format!("{name} · {count}")).color(colour);
            let response = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
            let underline = response.rect.expand2(Vec2::new(8.0, 0.0));
            if open {
                ui.painter()
                    .hline(underline.x_range(), underline.bottom() + 6.0, Stroke::new(2.0, accent));
            }
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *tab = choice;
            }
            ui.add_space(14.0);
        }
    });
    let line = row.response.rect;
    ui.painter().hline(
        line.left()..=ui.max_rect().right(),
        line.bottom() + 7.0,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );
    ui.add_space(8.0);
}

/// A title screen at `width`, or a plain tile with the game's name where there is none.
fn thumbnail(ui: &mut egui::Ui, texture: Option<egui::TextureHandle>, name: &str, width: f32) {
    let size = Vec2::new(width, width * 2.0 / 3.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    match texture {
        Some(texture) => {
            egui::Image::new(&texture).corner_radius(4.0).paint_at(ui, rect);
        }
        // Too small for the name to read: a plain tile.
        None if width < 100.0 => {
            ui.painter().rect_filled(rect, 4.0, Color32::from_gray(0x2B));
        }
        None => {
            ui.painter().rect_filled(rect, 4.0, Color32::from_gray(0x2B));
            let font = egui::FontId::proportional(crate::ui_scale::text_points(ui.style(), 10.0));
            let galley = ui.fonts_mut(|f| {
                f.layout(
                    name.to_owned(),
                    font,
                    ui.visuals().weak_text_color(),
                    rect.width() - 8.0,
                )
            });
            ui.painter()
                .galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
        }
    }
}

/// Where the ROM with this SHA-1 is, for taking its title screen.
fn rom_path<'a>(home: &'a Home<'_>, sha1: &str, kept: Option<&'a str>) -> Option<PathBuf> {
    home.roms
        .iter()
        .find(|rom| rom.sha1.eq_ignore_ascii_case(sha1))
        .map(|rom| rom.path.clone())
        .or_else(|| kept.map(PathBuf::from))
}

/// The Runs tab: a row per Run, newest played first, the lost ones after the rest.
fn runs(ui: &mut egui::Ui, home: &Home<'_>, art: &mut TitleArt, games: &Games) -> Option<Ask> {
    if home.runs.is_empty() {
        ui.weak("No Runs yet: start one with New Run above.");
        return None;
    }
    let mut order: Vec<&RunSummary> = home.runs.iter().filter(|r| r.game_over.is_none()).collect();
    order.extend(home.runs.iter().filter(|r| r.game_over.is_some()));
    let now = SystemRunClock.now_ms();
    let width = ui.available_width();
    // The thumbnail, Run (what's left), Players, Preset, Deaths, Last played, the buttons.
    let fixed = [THUMB + 12.0, 0.0, 170.0, 140.0, 60.0, 130.0, 150.0];
    let name_width = (width - fixed.iter().sum::<f32>()).max(160.0);
    let widths = [fixed[0], name_width, fixed[2], fixed[3], fixed[4], fixed[5], fixed[6]];
    let mut ask = None;
    // A window too narrow for every column scrolls sideways.
    egui::ScrollArea::horizontal().id_salt("runs table").show(ui, |ui| {
        header(ui, &widths);
        for run in order {
            if let Some(asked) = run_row(ui, home, run, &widths, now, art, games) {
                ask = Some(asked);
            }
        }
    });
    ask
}

fn header(ui: &mut egui::Ui, widths: &[f32; 7]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (heading, width) in ["", "Run", "Players", "Preset", "Deaths", "Last played", ""]
            .iter()
            .zip(widths)
        {
            ui.allocate_ui(Vec2::new(*width, 18.0), |ui| {
                ui.set_width(*width);
                ui.weak(*heading);
            });
        }
    });
}

/// One Run's row. The row under the pointer is tinted in the accent, with
/// an accent bar on its left.
fn run_row(
    ui: &mut egui::Ui,
    home: &Home<'_>,
    run: &RunSummary,
    widths: &[f32; 7],
    now: i64,
    art: &mut TitleArt,
    games: &Games,
) -> Option<Ask> {
    let mut ask = None;
    let lost = run.game_over.is_some();
    let background = ui.painter().add(egui::Shape::Noop);
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if lost {
            ui.set_opacity(0.55);
        }
        let cell = |ui: &mut egui::Ui, width: f32, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.allocate_ui_with_layout(
                Vec2::new(width, THUMB * 2.0 / 3.0 + 12.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(width);
                    ui.add_space(6.0);
                    add(ui);
                },
            );
        };
        cell(ui, widths[0], &mut |ui| {
            let path = rom_path(home, &run.game_sha1, run.rom_path.as_deref());
            let texture = art.get(ui.ctx(), games, &run.game_sha1, path.as_deref());
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                thumbnail(ui, texture, &run.game_name, THUMB);
            });
        });
        cell(ui, widths[1], &mut |ui| {
            ui.add(egui::Label::new(RichText::new(&run.name).strong()).truncate());
            let mut under = run.game_name.clone();
            if let Some(from) = &run.copied_from {
                under.push_str(&format!(" · Copied from {} on {}", from.name, logger::date(from.at_ms)));
            }
            ui.add(egui::Label::new(RichText::new(under).small().weak()).truncate());
        });
        cell(ui, widths[2], &mut |ui| {
            ui.add_space(6.0);
            if run.players.is_empty() {
                // A Run with friends whose roster isn't locked yet has no names to show.
                ui.weak(if run.with_friends { "With friends" } else { "Solo" });
            } else {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (index, name) in run.players.iter().enumerate() {
                        theme::chip(ui, name, index);
                    }
                });
            }
        });
        cell(ui, widths[3], &mut |ui| {
            ui.add_space(6.0);
            ui.label(crate::side_panel::preset_name(&run.ruleset));
        });
        cell(ui, widths[4], &mut |ui| {
            ui.add_space(6.0);
            ui.label(run.deaths.to_string());
        });
        cell(ui, widths[5], &mut |ui| {
            match &run.game_over {
                Some(over) => {
                    ui.colored_label(theme::DANGER, format!("Game over · {}", logger::date(over.at_ms)));
                }
                None => {
                    ui.label(run_panel::ago(now - run.last_played_ms));
                }
            }
            ui.label(
                RichText::new(format!("Started {}", logger::date(run.created_ms)))
                    .small()
                    .weak(),
            );
        });
        cell(ui, widths[6], &mut |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if lost {
                    if ui.button("Summary").clicked() {
                        ask = Some(Ask::Summary(Box::new(run.clone())));
                    }
                } else if metal::button(ui, home.accent, "Resume".into(), Vec2::ZERO).clicked() {
                    ask = Some(Ask::Resume(Box::new(run.clone())));
                }
                ui.menu_button("…", |ui| {
                    if run.with_friends
                        && !lost
                        && ui
                            .button("Continue as solo…")
                            .on_hover_text("A new solo Run from where you left this one")
                            .clicked()
                    {
                        ask = Some(Ask::ContinueSolo(Box::new(run.clone())));
                    }
                    if ui.button(RichText::new("Delete…").color(theme::DANGER)).clicked() {
                        ask = Some(Ask::Delete(Box::new(run.clone())));
                    }
                });
            });
        });
    });
    let rect = row.response.rect.with_max_x(ui.max_rect().right());
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke.color;
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, stroke));
    if ui.rect_contains_pointer(rect) {
        let accent = metal::colour(home.accent);
        ui.painter().set(
            background,
            egui::Shape::rect_filled(rect, 0.0, accent.gamma_multiply(0.12)),
        );
        ui.painter()
            .vline(rect.left() + 1.0, rect.y_range(), Stroke::new(2.0, accent));
    }
    ask
}

/// A ROM's name for its tile: the game the app named, or its file's.
fn game_name(rom: &KnownRom) -> String {
    match rom.game.rsplit_once(": ") {
        Some((name, _)) => name.to_owned(),
        None => crate::file_stem(&rom.path),
    }
}

/// The Games tab: a tile per ROM, the folders they come from, the Tested Romhacks.
fn games_tab(ui: &mut egui::Ui, home: &Home<'_>, art: &mut TitleArt, games: &Games) -> Option<Ask> {
    let mut ask = None;
    ui.horizontal(|ui| {
        ui.weak(match home.folders {
            [] => "Your games: every ROM you open shows here.".to_owned(),
            [folder] => format!("Your games, from {} and its subfolders", folder.display()),
            folders => format!(
                "Your games, from {} and their subfolders",
                folders
                    .iter()
                    .map(|f| f.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if metal::button(ui, home.accent, "Add a folder…".into(), Vec2::ZERO)
                .on_hover_text("Every GBA ROM in it and its subfolders shows here, and new ones appear by themselves")
                .clicked()
            {
                ask = Some(Ask::AddFolder);
            }
        });
    });
    ui.add_space(10.0);
    if home.roms.is_empty() {
        ui.weak("No games yet: add the folder your ROMs are in, or open a ROM.");
    }
    // As many tiles to a row as fit.
    let gap = 12.0;
    // A tile is its width plus its 1-point border on each side.
    let per_row = (((ui.available_width() + gap) / (TILE + 2.0 + gap)).floor() as usize).max(1);
    for row in home.roms.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(gap);
            for rom in row {
                if let Some(asked) = game_tile(ui, home, rom, art, games) {
                    ask = Some(asked);
                }
            }
        });
        ui.add_space(gap);
    }
    ui.add_space(18.0);
    if let Some(store) = home.profiles {
        tested_romhacks(ui, store);
    }
    ask
}

/// One game: its title screen, name, whether a Run can be played on it, and its buttons.
fn game_tile(ui: &mut egui::Ui, home: &Home<'_>, rom: &KnownRom, art: &mut TitleArt, games: &Games) -> Option<Ask> {
    let mut ask = None;
    let name = game_name(rom);
    let response = egui::Frame::new()
        .fill(Color32::from_gray(0x23))
        .stroke(Stroke::new(1.0, Color32::from_gray(0x35)))
        .corner_radius(8)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.set_width(TILE - 16.0);
            ui.vertical(|ui| {
                let texture = art.get(ui.ctx(), games, &rom.sha1, Some(&rom.path));
                thumbnail(ui, texture, &name, TILE - 16.0);
                ui.add(egui::Label::new(RichText::new(&name).strong()).truncate())
                    .on_hover_text(rom.path.display().to_string());
                if rom.runs {
                    ui.colored_label(theme::ON, RichText::new("Ready for a Run").small());
                } else {
                    ui.weak(RichText::new("Play only: no Game Profile for this ROM").small());
                }
                ui.horizontal(|ui| {
                    if rom.runs && metal::button(ui, home.accent, "New Run".into(), Vec2::ZERO).clicked() {
                        ask = Some(Ask::NewRunOn(rom.path.clone()));
                    }
                    if ui.button("Play").on_hover_text("Just play, no Run").clicked() {
                        ask = Some(Ask::Play(rom.path.clone()));
                    }
                });
            });
        })
        .response;
    if response.hovered() {
        ui.painter().rect_stroke(
            response.rect,
            8.0,
            Stroke::new(1.0, metal::colour(home.accent)),
            egui::StrokeKind::Inside,
        );
    }
    ask
}

/// The Romhacks Pactlocke is tested on, folded away.
fn tested_romhacks(ui: &mut egui::Ui, store: &ProfileStore) {
    egui::CollapsingHeader::new("Tested Romhacks").show(ui, |ui| {
        ui.weak(
            "Bring the patched ROM of one of these: Pactlocke hosts and links to nothing, and checks the ROM by SHA-1.",
        );
        for hack in store.tested_romhacks() {
            romhack_line(ui, store, hack);
        }
    });
}

fn romhack_line(ui: &mut egui::Ui, store: &ProfileStore, hack: &TestedRomhack) {
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
