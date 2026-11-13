//! Links in the side panel: every Player's party, with a hovered Pokémon's
//! Link-mates highlighted, and the Links tab, one row per source and one
//! column per Player. It displays the Run engine's Link data; its one
//! question, skipping a Gift the others took, warns first.

use eframe::egui;
use pactlocke_run::{Link, Member, PlayerKey, RunCommand, SeenPokemon};

/// One Player's party, as the side panel shows it.
pub struct Party<'a> {
    pub player: PlayerKey,
    pub name: &'a str,
    pub pokemon: &'a [SeenPokemon],
}

/// Every Player's party. Hovering a Pokémon highlights its Link-mates.
pub fn parties(ui: &mut egui::Ui, parties: &[Party<'_>], links: &[Link]) {
    let hover_id = egui::Id::unique("hovered link");
    let hovered: Option<String> = ui.data(|d| d.get_temp(hover_id));
    let mut now_hovered = None;
    for party in parties {
        ui.horizontal_wrapped(|ui| {
            ui.strong(party.name);
            for pokemon in party.pokemon {
                let link = links.iter().find(
                    |l| matches!(l.member(&party.player), Some(Member::Caught { pid, .. }) if *pid == pokemon.pid),
                );
                let name = if pokemon.nickname.is_empty() {
                    &pokemon.species_name
                } else {
                    &pokemon.nickname
                };
                let mut text = egui::RichText::new(name);
                if pokemon.hp == 0 {
                    text = text.strikethrough();
                }
                if link.is_some_and(|l| Some(&l.source.key) == hovered.as_ref()) {
                    text = text.background_color(ui.visuals().selection.bg_fill);
                }
                let label = ui.label(text);
                if let Some(link) = link {
                    if link.pending() {
                        // The pending outline: not every Player has had this Encounter yet.
                        ui.painter().rect_stroke(
                            label.rect.expand(1.0),
                            2.0,
                            egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if label.hovered() {
                        now_hovered = Some(link.source.key.clone());
                    }
                    label.on_hover_text(format!("Linked from {}", link.source.name));
                }
            }
        });
    }
    ui.data_mut(|d| match now_hovered {
        Some(key) => {
            d.insert_temp(hover_id, key);
        }
        None => d.remove::<String>(hover_id),
    });
}

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
