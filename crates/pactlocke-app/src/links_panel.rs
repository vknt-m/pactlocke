//! The Links tab of the side panel: one row per source and one column per
//! Player. It displays the Run engine's Link data; its one question,
//! skipping a Gift the others took, warns first.

use eframe::egui;
use pactlocke_run::{Link, Member, PlayerKey, RunCommand};

/// The Links tab: one row per source, one column per Player. Returns a
/// Skip once the Player confirms it.
pub fn tab(ui: &mut egui::Ui, links: &[Link], players: &[(PlayerKey, String)], me: &PlayerKey) -> Option<RunCommand> {
    let confirm_id = egui::Id::unique("skip to confirm");
    let mut confirming: Option<String> = ui.data(|d| d.get_temp(confirm_id));
    let mut command = None;
    if links.is_empty() {
        ui.weak("No Links yet: each Player's Encounter from the same source is Linked.");
    }
    egui::ScrollArea::vertical()
        .id_salt("links")
        .max_height(220.0)
        .show(ui, |ui| {
            egui::Grid::new("links grid").striped(true).show(ui, |ui| {
                ui.strong("Source");
                for (_, name) in players {
                    ui.strong(name);
                }
                ui.end_row();
                for link in links {
                    match &link.broken {
                        Some(why) => ui.label(&link.source.name).on_hover_text(format!("Broken: {why}")),
                        None => ui.label(&link.source.name),
                    };
                    for (player, _) in players {
                        match link.member(player) {
                            Some(Member::Caught { species_name, dead, .. }) => {
                                let text = egui::RichText::new(species_name);
                                ui.label(if *dead { text.strikethrough() } else { text });
                            }
                            Some(Member::Missed { skipped: true }) => {
                                ui.weak("skipped");
                            }
                            Some(Member::Missed { skipped: false }) => {
                                ui.weak("missed");
                            }
                            Some(Member::Pending) | None => {
                                let others_took = link.members.iter().any(|(_, m)| matches!(m, Member::Caught { .. }));
                                if player == me && link.gift && others_took && link.broken.is_none() {
                                    if ui.small_button("Skip…").clicked() {
                                        confirming = Some(link.source.key.clone());
                                    }
                                } else {
                                    ui.weak("not yet");
                                }
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        });
    if let Some(key) = &confirming
        && let Some(link) = links.iter().find(|l| &l.source.key == key)
    {
        ui.separator();
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!(
                "Skip {}? The others took theirs, so their copies will die.",
                link.source.name
            ),
        );
        ui.horizontal(|ui| {
            if ui.button("Skip it").clicked() {
                command = Some(RunCommand::Skip {
                    source: link.source.clone(),
                });
                confirming = None;
            }
            if ui.button("Keep it open").clicked() {
                confirming = None;
            }
        });
    }
    ui.data_mut(|d| match confirming {
        Some(key) => {
            d.insert_temp(confirm_id, key);
        }
        None => d.remove::<String>(confirm_id),
    });
    command
}
