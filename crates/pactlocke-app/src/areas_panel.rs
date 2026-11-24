//! The Areas tab of the side panel: every Area of the game, how many each
//! Player has used their Encounter in, and in each one what they met and
//! whether they caught it. It displays the Run engine's data: this Player's
//! own Encounters from their Run, the others' from the Run log.

use eframe::egui;
use pactlocke_game::Area;
use pactlocke_run::AreaEncounter;

use crate::theme::{self, Icons};

/// The icon size of a Pokémon met in an Area.
const ICON: f32 = 28.0;

/// One Player's column: their name, colour and Area Encounters.
pub struct Column<'a> {
    pub name: &'a str,
    pub index: usize,
    pub you: bool,
    pub encounters: &'a [AreaEncounter],
}

/// Which Areas the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Show {
    #[default]
    All,
    Open,
    Used,
}

/// The tab's label: how many Areas this Player has used, of the game's.
pub fn label(mine: &[AreaEncounter], areas: &[Area]) -> String {
    format!("Areas · {}/{}", used(mine, areas), areas.len())
}

/// How many of the game's Areas these Encounters used.
fn used(encounters: &[AreaEncounter], areas: &[Area]) -> usize {
    areas
        .iter()
        .filter(|a| encounters.iter().any(|e| e.area == a.name))
        .count()
}

/// The Areas tab.
pub fn tab(ui: &mut egui::Ui, areas: &[Area], columns: &[Column<'_>], icons: &mut Icons) {
    if areas.is_empty() {
        ui.weak("This game's Game Profile lists no Areas.");
        return;
    }
    for column in columns {
        let used = used(column.encounters, areas);
        let caught = column.encounters.iter().filter(|e| e.caught).count();
        ui.horizontal(|ui| {
            theme::dot(ui, column.index);
            let who = if column.you { "You" } else { column.name };
            ui.strong(who);
            ui.label(format!(
                "{used} of {} used · {caught} caught · {} open",
                areas.len(),
                areas.len() - used
            ));
        });
    }
    let show_id = egui::Id::unique("areas shown");
    let mut show: Show = ui.data(|d| d.get_temp(show_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.weak("Show");
        for (choice, name) in [(Show::All, "All"), (Show::Open, "Open"), (Show::Used, "Used")] {
            if ui.selectable_label(show == choice, name).clicked() {
                show = choice;
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(show_id, show));
    ui.separator();

    // Open and Used are this Player's own: the Areas they can still use, or have.
    let mine = columns.iter().find(|c| c.you).map_or(&[][..], |c| c.encounters);
    egui::Grid::new("areas grid")
        .striped(true)
        .spacing([8.0, 4.0])
        .min_row_height(ICON)
        .show(ui, |ui| {
            ui.strong("Area");
            for column in columns {
                let who = if column.you { "You" } else { column.name };
                ui.label(
                    egui::RichText::new(who)
                        .color(theme::player_colour(column.index))
                        .strong(),
                );
            }
            ui.end_row();
            for area in areas {
                let used_here = mine.iter().any(|e| e.area == area.name);
                let wanted = match show {
                    Show::All => true,
                    Show::Open => !used_here,
                    Show::Used => used_here,
                };
                if !wanted {
                    continue;
                }
                ui.label(&area.name);
                for column in columns {
                    encounter(ui, column, &area.name, icons);
                }
                ui.end_row();
            }
        });
}

/// One Player's Encounter in an Area, or that it's still open.
fn encounter(ui: &mut egui::Ui, column: &Column<'_>, area: &str, icons: &mut Icons) {
    let who = if column.you { "You" } else { column.name };
    match column.encounters.iter().find(|e| e.area == area) {
        Some(e) => {
            let tip = if e.caught {
                format!("{who} caught {} here", e.species_name)
            } else {
                format!("{who} met {} here, not caught", e.species_name)
            };
            let shown = icons.show(ui, e.species, ICON, false, &tip);
            if !e.caught {
                // Not caught: the icon dimmed under a veil.
                ui.painter()
                    .rect_filled(shown.rect, 4.0, ui.visuals().panel_fill.gamma_multiply(0.6));
            }
        }
        None => {
            let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(ICON), egui::Sense::hover());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "open",
                egui::FontId::proportional(crate::ui_scale::text_points(ui.style(), 10.0)),
                ui.visuals().weak_text_color(),
            );
            response.on_hover_text(format!("{who}: this Area's Encounter is still open"));
        }
    }
}
