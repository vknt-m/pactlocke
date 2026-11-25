//! Rescues: the Rescue menu, listing the Rescue Points a Rescue can go back
//! to, each with its local time, how long ago, and every Player's Area; and
//! the vote on one, in a Run with other Players.

use eframe::egui;
use pactlocke_run::{PlayerKey, RescuePointView, RunEvent, RunLog, minutes_ago};

/// What going back to a Rescue Point takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// With other Players: a vote on the `call`th Run Checkpoint.
    Vote(u64),
    /// Alone: straight back to this Player's checkpoint with this hash.
    Alone(String),
}

/// A Rescue Point as the menu lists it.
pub struct Offered {
    /// When it was called (Unix milliseconds).
    pub at_ms: i64,
    /// Each Player's name ("You" for this Player) and Area there.
    pub areas: Vec<(String, Option<String>)>,
    /// What going back to it takes.
    pub pick: Pick,
}

/// The local time of day: "17:51".
pub fn local_time(at_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(at_ms)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

/// "17:51, 8 min ago".
pub fn when(at_ms: i64, now_ms: i64) -> String {
    format!("{}, {}", local_time(at_ms), minutes_ago(now_ms - at_ms))
}

/// The Rescue Points the Run log offers, newest first, as `me` reads them.
pub fn with_friends(log: &RunLog, me: &PlayerKey) -> Vec<Offered> {
    let roster = log.active();
    log.rescue_points()
        .into_iter()
        .rev()
        .map(|call| Offered {
            at_ms: call.called_at_ms().unwrap_or_default(),
            areas: roster
                .iter()
                .map(|p| {
                    let name = if p == me { "You".to_owned() } else { log.name_of(p) };
                    (name, call.parts.get(p).and_then(|part| part.area.clone()))
                })
                .collect(),
            pick: Pick::Vote(call.n),
        })
        .collect()
}

/// This Player's own Rescue Points, alone, newest first.
pub fn alone(points: &[RescuePointView]) -> Vec<Offered> {
    points
        .iter()
        .rev()
        .map(|p| Offered {
            at_ms: p.at_ms,
            areas: vec![("You".to_owned(), p.area.clone())],
            pick: Pick::Alone(p.hash.clone()),
        })
        .collect()
}

/// Every Player's Area at a Rescue Point: "You: Route 3 · Blue: Pallet Town".
fn areas_line(areas: &[(String, Option<String>)]) -> String {
    areas
        .iter()
        .map(|(name, area)| format!("{name}: {}", area.as_deref().unwrap_or("not in an Area")))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The Rescue menu, while `open`. `blocked` says why a Rescue can't be
/// asked for now, if it can't. Returns the Rescue Point the Player picked.
pub fn window(
    ctx: &egui::Context,
    open: &mut bool,
    points: &[Offered],
    blocked: Option<&str>,
    now_ms: i64,
) -> Option<Pick> {
    let mut picked = None;
    let with_friends = points.iter().any(|p| matches!(p.pick, Pick::Vote(_)));
    egui::Window::new("Rescue")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!(
                "If your game is stuck, go back to a Rescue Point. One is kept about every {} minutes, \
                 the last {} of them. Everything since is undone, and no Retry is spent.",
                pactlocke_run::RESCUE_POINT_EVERY_MS / 60_000,
                pactlocke_run::RESCUE_POINTS_KEPT
            ));
            if with_friends {
                ui.weak("Everyone goes back together if everyone says Yes; the first No ends the vote.");
            }
            ui.add_space(6.0);
            if points.is_empty() {
                ui.weak(format!(
                    "No Rescue Point yet: the first comes about {} minutes into the Run.",
                    pactlocke_run::RESCUE_POINT_EVERY_MS / 60_000
                ));
            }
            if let Some(why) = blocked {
                ui.weak(why);
            }
            for point in points {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.strong(when(point.at_ms, now_ms));
                        ui.weak(areas_line(&point.areas));
                    });
                    let text = match point.pick {
                        Pick::Vote(_) => "Ask everyone",
                        Pick::Alone(_) => "Go back here",
                    };
                    if ui.add_enabled(blocked.is_none(), egui::Button::new(text)).clicked() {
                        picked = Some(point.pick.clone());
                    }
                });
            }
        });
    if picked.is_some() {
        *open = false;
    }
    picked
}

/// A Rescue vote under way, as a banner with Yes and No for a Player who
/// hasn't said. Returns their vote.
pub fn vote(ui: &mut egui::Ui, log: &RunLog, me: PlayerKey, online: bool, now_ms: i64) -> Option<RunEvent> {
    let vote = log.rescue_vote()?;
    let point = with_friends(log, &me)
        .into_iter()
        .find(|p| p.pick == Pick::Vote(vote.call))?;
    let players = log.active().len();
    let mut cast = None;
    crate::spectate::vote_banner(ui, |ui| {
        ui.strong(format!(
            "Rescue vote: everyone back to {} · {} of {players} said Yes",
            when(point.at_ms, now_ms),
            vote.yes.len()
        ));
        ui.weak(areas_line(&point.areas));
        if !vote.yes.contains(&me) && online {
            ui.horizontal(|ui| {
                for (label, yes) in [("Yes", true), ("No", false)] {
                    if ui.button(label).clicked() {
                        cast = Some(RunEvent::RescueVote { call: vote.call, yes });
                    }
                }
            });
        }
    });
    cast
}
