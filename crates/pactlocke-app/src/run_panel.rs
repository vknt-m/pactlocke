//! The docked side panel during a Run: what the Run is, its level cap, the Areas whose
//! Encounter is used, the live feed, newest first, and the Graveyard tab
//! under it. It only displays the Run engine's view.

use eframe::egui;
use pactlocke_run::{FeedKind, RunClock, RunView, SystemRunClock};

pub fn show(ui: &mut egui::Ui, view: &RunView) {
    ui.heading(&view.name);
    ui.weak(&view.game_name);
    if let Some(cap) = &view.level_cap {
        let soft = if cap.soft { " (soft cap)" } else { "" };
        match &cap.next {
            Some(next) => ui.label(format!("Level cap {} until {next}{soft}", cap.level)),
            None => ui.label("No level cap: every cap is passed"),
        };
    }
    ui.add_space(6.0);

    let closed = &view.closed_areas;
    egui::CollapsingHeader::new(format!("Encounters used · {}", closed.len()))
        .id_salt("closed areas")
        .show(ui, |ui| {
            if closed.is_empty() {
                ui.weak("None yet: the first wild battle in each Area is its Encounter.");
            }
            for area in closed {
                ui.label(area);
            }
        });
    ui.separator();

    let now = SystemRunClock.now_ms();
    // The tab bar sits at the bottom; an open tab opens above it, and the feed takes the rest.
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        let open_id = egui::Id::unique("graveyard tab open");
        let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
        ui.horizontal(|ui| {
            let tab = format!("Graveyard · {}", view.graveyard.len());
            if ui.selectable_label(open, tab).clicked() {
                open = !open;
            }
        });
        ui.data_mut(|d| d.insert_temp(open_id, open));
        if open {
            ui.separator();
            graveyard(ui, view, now);
        }
        ui.separator();

        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.strong("Live feed");
            egui::ScrollArea::vertical()
                .id_salt("feed")
                .auto_shrink(false)
                .show(ui, |ui| {
                    for entry in &view.feed {
                        let visuals = ui.visuals();
                        let colour = match entry.kind {
                            FeedKind::Run => visuals.weak_text_color(),
                            FeedKind::Encounter => visuals.text_color(),
                            FeedKind::Catch | FeedKind::Gift => egui::Color32::from_rgb(0x3F, 0xA3, 0x5B),
                            FeedKind::Block => visuals.warn_fg_color,
                            FeedKind::Death => egui::Color32::from_rgb(0x9A, 0x7B, 0xC9),
                            FeedKind::Problem | FeedKind::Violation => visuals.error_fg_color,
                        };
                        ui.horizontal_wrapped(|ui| {
                            ui.weak(ago(now - entry.at_ms));
                            if entry.kind == FeedKind::Violation {
                                ui.colored_label(colour, egui::RichText::new(&entry.text).strong());
                            } else {
                                ui.colored_label(colour, &entry.text);
                            }
                        });
                        ui.add_space(2.0);
                    }
                });
        });
    });
}

/// The Dead, newest first, with what killed each.
fn graveyard(ui: &mut egui::Ui, view: &RunView, now: i64) {
    egui::ScrollArea::vertical()
        .id_salt("graveyard")
        .max_height(200.0)
        .show(ui, |ui| {
            if view.graveyard.is_empty() {
                ui.weak("No one has died yet.");
            }
            for grave in &view.graveyard {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(&grave.name);
                    if grave.name != grave.species_name {
                        ui.weak(format!("({})", grave.species_name));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Killed by {}", grave.killed_by));
                    ui.weak(ago(now - grave.at_ms));
                });
                ui.add_space(4.0);
            }
        });
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "just now".to_owned(),
        1..=59 => format!("{minutes} min ago"),
        60..=2879 => format!("{} h ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}
