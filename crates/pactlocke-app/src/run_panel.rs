//! The docked side panel during a Run: what the Run is, the Areas whose
//! Encounter is used, and the live feed, newest first. It only displays the
//! Run engine's view.

use eframe::egui;
use pactlocke_run::{FeedKind, RunClock, RunView, SystemRunClock};

pub fn show(ui: &mut egui::Ui, view: &RunView) {
    ui.heading(&view.name);
    ui.weak(&view.game_name);
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

    ui.strong("Live feed");
    let now = SystemRunClock.now_ms();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for entry in &view.feed {
            let visuals = ui.visuals();
            let colour = match entry.kind {
                FeedKind::Run => visuals.weak_text_color(),
                FeedKind::Encounter => visuals.text_color(),
                FeedKind::Catch => egui::Color32::from_rgb(0x3F, 0xA3, 0x5B),
                FeedKind::Block => visuals.warn_fg_color,
                FeedKind::Problem => visuals.error_fg_color,
            };
            ui.horizontal_wrapped(|ui| {
                ui.weak(ago(now - entry.at_ms));
                ui.colored_label(colour, &entry.text);
            });
            ui.add_space(2.0);
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
