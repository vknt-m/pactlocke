//! Faint = Dead's game over: the bar over a stopped game after a wiped
//! party (a Retry, or the end of the Run), the game over screen that
//! replaces the game, and a lost Run's summary from the Runs list.

use std::collections::BTreeMap;

use eframe::egui;
use pactlocke_run::{FeedKind, GameOverView, PlayerKey, RunCommand, RunEvent, RunLog, RunView, Survivor, WipeView};

use crate::rules_text::plural;

/// What the Player answered the wipe with.
pub enum WipeAnswer {
    /// Alone: Retry, or end the Run.
    Command(RunCommand),
    /// With friends: their Retry vote, Yes or No.
    Vote(RunEvent),
}

/// The wiped party to show now, if the Run isn't over yet: alone, this
/// Player's own; with friends, the first one the Run log has, as `me` reads it.
pub fn wipe_of(view: &RunView, log: Option<&RunLog>, me: Option<PlayerKey>) -> Option<WipeView> {
    if view.game_over.is_some() {
        return None;
    }
    match (log, me) {
        (Some(log), Some(me)) => {
            if log.game_over().is_some() {
                return None;
            }
            let wiped = log.wiped()?;
            Some(WipeView {
                how: wiped.last.wipe_line(&log.whose(&wiped.player, &me)),
                retry_offered: false,
            })
        }
        _ => view.wipe.clone(),
    }
}

/// The bar over the stopped game after a wipe. Alone: Retry, or end the
/// Run. With friends: the Retry vote, where a No ends the Run for everyone.
pub fn wipe_bar(
    ui: &mut egui::Ui,
    wipe: &WipeView,
    log: Option<&RunLog>,
    me: Option<PlayerKey>,
    online: bool,
) -> Option<WipeAnswer> {
    let mut answer = None;
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.strong(&wipe.how);
            match (log, me) {
                (Some(log), Some(me)) => {
                    let call = log.vote().map(|v| v.call).or_else(|| log.latest_complete());
                    let voted = log.vote().is_some_and(|v| v.yes.contains(&me));
                    let left = log.retries_left();
                    ui.add(
                        egui::Label::new(format!(
                            "Under Faint = Dead, that is a game over for everyone, unless everyone votes to Retry from the latest Run Checkpoint ({left} {} left). The first No ends the Run.",
                            plural(left, "Retry", "Retries")
                        ))
                        .wrap(),
                    );
                    if let Some(call) = call.filter(|_| online && !voted) {
                        ui.horizontal(|ui| {
                            if ui.button("Vote to Retry").clicked() {
                                answer = Some(WipeAnswer::Vote(RunEvent::RetryVote { call, yes: true }));
                            }
                            if ui.button("End the Run").clicked() {
                                answer = Some(WipeAnswer::Vote(RunEvent::RetryVote { call, yes: false }));
                            }
                        });
                    } else if voted {
                        ui.weak("You voted to Retry: waiting for the others.");
                    }
                }
                _ if wipe.retry_offered => {
                    ui.add(
                        egui::Label::new(
                            "Under Faint = Dead, that is a game over, unless you Retry from your latest Run Checkpoint.",
                        )
                        .wrap(),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Retry").clicked() {
                            answer = Some(WipeAnswer::Command(RunCommand::Retry {
                                to: pactlocke_run::RetryTo::Latest,
                            }));
                        }
                        if ui.button("End the Run").clicked() {
                            answer = Some(WipeAnswer::Command(RunCommand::EndRun));
                        }
                    });
                }
                _ => {}
            }
        });
    answer
}

/// The game over screen, in place of the game.
pub fn screen(ui: &mut egui::Ui, over: &GameOverView, view: &RunView, log: Option<&RunLog>, me: Option<PlayerKey>) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::same(24)).show(ui, |ui| {
            ui.heading("Game over");
            ui.add_space(6.0);
            ui.add(egui::Label::new(egui::RichText::new(&over.how).size(16.0)).wrap());
            ui.add_space(12.0);
            survivors(ui, &over.longest);
            ui.add_space(12.0);
            tallies(ui, view, log);
            ui.add_space(6.0);
            ui.label(format!("Play time: {}", duration(over.play_ms)));
            ui.add_space(12.0);
            graveyard(ui, view, log, me);
            ui.add_space(12.0);
            ui.weak("File → Close Run goes back to your Runs. This Run can't be resumed.");
        });
    });
}

/// The longest survivor of the Run, then each Player's.
pub fn survivors(ui: &mut egui::Ui, longest: &[Survivor]) {
    let Some(best) = longest.iter().max_by_key(|s| s.lived_ms) else {
        return;
    };
    ui.strong("Survived the longest");
    ui.label(survivor_line(best));
    if longest.len() > 1 {
        for survivor in longest {
            ui.weak(survivor_line(survivor));
        }
    }
}

fn survivor_line(survivor: &Survivor) -> String {
    let whose = match &survivor.player {
        Some(player) => format!("{player}'s "),
        None => "Your ".to_owned(),
    };
    let species = if survivor.name == survivor.species_name {
        String::new()
    } else {
        format!(" the {}", survivor.species_name)
    };
    let still = if survivor.alive { ", still alive at the end" } else { "" };
    format!(
        "{whose}{}{species}: {}{still}",
        survivor.name,
        duration(survivor.lived_ms)
    )
}

/// Each Player's tallies: with friends from the Run log, alone from the feed.
fn tallies(ui: &mut egui::Ui, view: &RunView, log: Option<&RunLog>) {
    ui.strong("Tallies");
    match log.and_then(|l| Some((l, l.locked()?))) {
        Some((log, locked)) => {
            let tallies = log.tallies();
            let flags = log.flags();
            egui::Grid::new("game over tallies").striped(true).show(ui, |ui| {
                for heading in ["", "Encounters", "Catches", "Deaths", "Violations", "Cheat flags"] {
                    ui.weak(heading);
                }
                ui.end_row();
                for player in &locked.roster {
                    let n = |map: &BTreeMap<PlayerKey, u32>| map.get(&player.key).copied().unwrap_or(0);
                    ui.label(&player.name);
                    ui.label(n(&tallies.encounters).to_string());
                    ui.label(n(&tallies.catches).to_string());
                    ui.label(n(&tallies.deaths).to_string());
                    ui.label(n(&tallies.violations).to_string());
                    ui.label(flags.iter().filter(|f| f.player == player.key).count().to_string());
                    ui.end_row();
                }
            });
        }
        None => {
            let count = |kind: FeedKind| view.feed.iter().filter(|e| e.kind == kind).count();
            ui.add(
                egui::Label::new(format!(
                    "Encounters {} · catches {} · Deaths {} · Violations {} · cheat flags {}",
                    view.encounters.len(),
                    count(FeedKind::Catch),
                    view.graveyard.len(),
                    count(FeedKind::Violation),
                    count(FeedKind::Cheat),
                ))
                .wrap(),
            );
        }
    }
}

/// Every Pokémon that died: this Player's own graveyard, and with friends
/// the others' Deaths from the Run log.
fn graveyard(ui: &mut egui::Ui, view: &RunView, log: Option<&RunLog>, me: Option<PlayerKey>) {
    ui.strong("Graveyard");
    for grave in &view.graveyard {
        ui.label(format!("{} ({}): {}", grave.name, grave.species_name, grave.killed_by));
    }
    let Some(log) = log else {
        return;
    };
    for (player, event) in log.standing_events() {
        if let RunEvent::Died { name, species_name, .. } = event
            && Some(player) != me
        {
            ui.weak(format!("{}'s {name} ({species_name})", log.name_of(&player)));
        }
    }
}

/// "1 h 05 min", "12 min", "40 s".
pub fn duration(ms: i64) -> String {
    let seconds = ms.max(0) / 1000;
    match (seconds / 3600, seconds % 3600 / 60) {
        (0, 0) => format!("{seconds} s"),
        (0, minutes) => format!("{minutes} min"),
        (hours, minutes) => format!("{hours} h {minutes:02} min"),
    }
}
