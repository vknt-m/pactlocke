//! The docked side panel during a Run: what the Run is, its level cap, the
//! nickname prompt, the Areas whose Encounter is used, the live feed, newest
//! first, and the Graveyard tab under it. It displays the Run engine's view,
//! and its one question, the nickname prompt, answers with a [`RunCommand`].

use eframe::egui;
use pactlocke_run::{FeedKind, NicknameWanted, RunClock, RunCommand, RunView, SystemRunClock};

/// Shows the panel; returns what the Player asked of the Run, if anything.
/// `alone`: a solo Run, where a Retry needs no vote. `links`: in a Run
/// with Links, how many there are and what shows the Links tab.
pub fn show(
    ui: &mut egui::Ui,
    view: &RunView,
    alone: bool,
    mut links: Option<(usize, &mut LinksTab<'_>)>,
) -> Option<RunCommand> {
    ui.heading(&view.name);
    ui.weak(&view.game_name);
    if let Some(cap) = &view.level_cap {
        let soft = if cap.soft { " (soft cap)" } else { "" };
        match &cap.next {
            Some(next) => ui.label(format!("Level cap {} until {next}{soft}", cap.level)),
            None => ui.label("No level cap: every cap is passed"),
        };
    }
    let mut command = view
        .nickname_wanted
        .first()
        .and_then(|wanted| nickname_prompt(ui, wanted));
    if alone && let Some(left) = view.retries_left {
        let retry = ui.add_enabled(
            left > 0 && view.run_checkpoint,
            egui::Button::new(format!("Retry · {left} left")),
        );
        let retry = retry
            .on_hover_text("Go back to your latest Run Checkpoint (your last in-game save), undoing everything since");
        if retry.clicked() {
            command = Some(RunCommand::Retry {
                to: pactlocke_run::RetryTo::Latest,
            });
        }
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
        let open_id = egui::Id::unique("open tab");
        let mut open: Option<Tab> = ui.data(|d| d.get_temp(open_id)).unwrap_or_default();
        ui.horizontal(|ui| {
            if let Some((count, _)) = &links {
                let tab = format!("Links · {count}");
                if ui.selectable_label(open == Some(Tab::Links), tab).clicked() {
                    open = (open != Some(Tab::Links)).then_some(Tab::Links);
                }
            }
            let tab = format!("Graveyard · {}", view.graveyard.len());
            if ui.selectable_label(open == Some(Tab::Graveyard), tab).clicked() {
                open = (open != Some(Tab::Graveyard)).then_some(Tab::Graveyard);
            }
        });
        ui.data_mut(|d| d.insert_temp(open_id, open));
        match (open, links.as_mut()) {
            (Some(Tab::Graveyard), _) => {
                ui.separator();
                graveyard(ui, view, now);
            }
            (Some(Tab::Links), Some((_, show))) => {
                ui.separator();
                // Bottom-up here: the tab's own layout reads top-down.
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    if let Some(skip) = show(ui) {
                        command = Some(skip);
                    }
                });
            }
            _ => {}
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
    command
}

/// What shows the Links tab; it returns a Skip once the Player confirms one.
pub type LinksTab<'a> = dyn FnMut(&mut egui::Ui) -> Option<RunCommand> + 'a;

/// The tab open above the tab bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Links,
    Graveyard,
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

/// Must nickname: asks for a name for a Pokémon left with its species name.
fn nickname_prompt(ui: &mut egui::Ui, wanted: &NicknameWanted) -> Option<RunCommand> {
    let mut command = None;
    ui.add_space(6.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong(format!("Name your {}", wanted.species_name));
        ui.label("Must nickname: up to 10 characters, not the species name.");
        let id = ui.make_persistent_id(("nickname", wanted.pid));
        let mut name = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
        let mut send = false;
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .char_limit(10)
                    .desired_width(140.0),
            );
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                send = true;
            }
            send |= ui.button("Name it").clicked();
        });
        if send && !name.trim().is_empty() {
            command = Some(RunCommand::Nickname {
                pid: wanted.pid,
                name: name.trim().to_owned(),
            });
            name.clear();
        }
        ui.data_mut(|d| d.insert_temp(id, name));
    });
    command
}
