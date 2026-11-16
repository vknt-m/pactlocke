//! The Links tab of the side panel: one row per source, with each Player's
//! Pokémon from it as a party icon (its name on hover). It displays the Run
//! engine's Link data; its one question, skipping a Gift the others took,
//! warns first.

use eframe::egui;
use pactlocke_run::{Link, Member, PlayerKey, RunCommand};

use crate::theme::{self, Icons};

/// How big each Pokémon's icon is: twice the size of the party bar's.
const ICON: f32 = 40.0;

/// The Links tab: one row per source, one column per Player (`players` in
/// join order, with each one's colour index). Returns a Skip once the
/// Player confirms it.
pub fn tab(
    ui: &mut egui::Ui,
    links: &[Link],
    players: &[(PlayerKey, String, usize)],
    me: &PlayerKey,
    icons: &mut Icons,
) -> Option<RunCommand> {
    let confirm_id = egui::Id::unique("skip to confirm");
    let mut confirming: Option<String> = ui.data(|d| d.get_temp(confirm_id));
    let mut command = None;
    if links.is_empty() {
        ui.weak("No Links yet: each Player's Encounter from the same source is Linked.");
    }
    egui::Grid::new("links grid")
        .striped(true)
        .spacing([10.0, 6.0])
        .min_row_height(ICON)
        .show(ui, |ui| {
            ui.strong("Source");
            for (_, name, index) in players {
                ui.label(egui::RichText::new(name).color(theme::player_colour(*index)).strong());
            }
            ui.end_row();
            for link in links {
                // A fixed column for the name, wrapped: `add_sized` would justify
                // a wrapped name, spreading its letters across the column.
                let source = ui
                    .allocate_ui_with_layout(
                        egui::vec2(90.0, ICON),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| ui.add(egui::Label::new(&link.source.name).wrap()),
                    )
                    .inner;
                if let Some(why) = &link.broken {
                    source.on_hover_text(format!("Broken: {why}"));
                }
                for (player, name, _) in players {
                    if let Some(skip) = part(ui, link, player, name, me, icons) {
                        confirming = Some(skip);
                    }
                }
                ui.end_row();
            }
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

/// One Player's part of a Link, as a tile. Returns the source's key when
/// this Player asks to skip it.
fn part(
    ui: &mut egui::Ui,
    link: &Link,
    player: &PlayerKey,
    name: &str,
    me: &PlayerKey,
    icons: &mut Icons,
) -> Option<String> {
    let whose = if player == me {
        "Your".to_owned()
    } else {
        format!("{name}'s")
    };
    match link.member(player) {
        Some(Member::Caught {
            species,
            species_name,
            judged,
            dead,
            ..
        }) => {
            let state = if *dead {
                " · Dead"
            } else if !judged {
                " · not seen in their game yet"
            } else {
                ""
            };
            let tip = format!("{whose} {species_name}{state}");
            let shown = icons.show(ui, *species, ICON, *dead, &tip);
            if !judged {
                ui.painter().rect_stroke(
                    shown.rect,
                    4.0,
                    egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
                    egui::StrokeKind::Inside,
                );
            }
            None
        }
        Some(Member::Missed { skipped }) => {
            let (mark, why) = if *skipped {
                ("skip", "skipped it")
            } else {
                ("×", "didn't catch it")
            };
            let who = if player == me {
                "You".to_owned()
            } else {
                name.to_owned()
            };
            marked_tile(ui, mark).on_hover_text(format!("{who} {why}"));
            None
        }
        Some(Member::Pending) | None => {
            let others_took = link.members.iter().any(|(_, m)| matches!(m, Member::Caught { .. }));
            if player == me && link.gift && others_took && link.broken.is_none() {
                let skip = ui
                    .add_sized([ICON, ICON], egui::Button::new("Skip…").small())
                    .on_hover_text("Skip this Gift: the others' copies will die");
                return skip.clicked().then(|| link.source.key.clone());
            }
            let tip = if player == me {
                "You haven't had this Encounter yet".to_owned()
            } else {
                format!("{name} hasn't had this Encounter yet")
            };
            marked_tile(ui, "").on_hover_text(tip);
            None
        }
    }
}

/// A tile with a short mark in it, for a part that isn't a Pokémon.
fn marked_tile(ui: &mut egui::Ui, mark: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(ICON), egui::Sense::hover());
    ui.painter().rect_stroke(
        rect.shrink(0.5),
        4.0,
        egui::Stroke::new(1.0, ui.visuals().weak_text_color().gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        mark,
        egui::FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );
    response
}
