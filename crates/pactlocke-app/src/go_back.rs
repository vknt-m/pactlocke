//! Going back: the menu of points a Rescue or a Retry can go back to, each
//! with its local time, how long ago, and every Player's Area; and the vote
//! on one, in a Run with other Players. A Rescue offers the Rescue Points;
//! a Retry offers those and the latest Run Checkpoint, so a wipe moments
//! after the newest one can go back further.

use eframe::egui;
use pactlocke_run::{Call, CheckpointView, PlayerKey, RunEvent, RunLog, Vote, minutes_ago};

/// Which way back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoBack {
    /// Free, when a game is stuck: to a Rescue Point.
    Rescue,
    /// After a wipe, spending one from the pool: to a Rescue Point or the
    /// latest Run Checkpoint.
    Retry,
}

impl GoBack {
    /// "Rescue" or "Retry".
    pub fn name(self) -> &'static str {
        match self {
            GoBack::Rescue => "Rescue",
            GoBack::Retry => "Retry",
        }
    }

    /// The points the Run log offers.
    fn points(self, log: &RunLog) -> Vec<Call> {
        match self {
            GoBack::Rescue => log.rescue_points(),
            GoBack::Retry => log.retry_points(),
        }
    }

    /// The vote under way of this kind, if any.
    fn vote(self, log: &RunLog) -> Option<Vote> {
        match self {
            GoBack::Rescue => log.rescue_vote(),
            GoBack::Retry => log.vote(),
        }
    }

    /// A vote on going back to the `call`th Run Checkpoint.
    pub fn vote_event(self, call: u64, yes: bool) -> RunEvent {
        match self {
            GoBack::Rescue => RunEvent::RescueVote { call, yes },
            GoBack::Retry => RunEvent::RetryVote { call, yes },
        }
    }
}

/// What going back to a point takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// With other Players: a vote on the `call`th Run Checkpoint.
    Vote(u64),
    /// Alone: straight back to this Player's checkpoint with this hash.
    Alone(String),
}

/// A point as the menu lists it.
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

/// The points the Run log offers for going back `way`, newest first, as `me` reads them.
pub fn with_friends(log: &RunLog, me: &PlayerKey, way: GoBack) -> Vec<Offered> {
    let roster = log.active();
    way.points(log)
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

/// This Player's own points, alone, newest first.
pub fn alone(points: &[CheckpointView]) -> Vec<Offered> {
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

/// Every Player's Area at a point: "You: Route 3 · Blue: Pallet Town".
fn areas_line(areas: &[(String, Option<String>)]) -> String {
    areas
        .iter()
        .map(|(name, area)| format!("{name}: {}", area.as_deref().unwrap_or("not in an Area")))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Why going back `way` can't be asked for now in a Run with other
/// Players, if it can't.
pub fn blocked_with_friends(log: &RunLog, way: GoBack, online: bool) -> Option<&'static str> {
    let retries = log.locked().map_or(0, |l| l.ruleset.retries);
    match way {
        _ if !online => Some("You're offline: a vote can be started once you're back."),
        GoBack::Rescue if log.wiped().is_some() => Some("A party is wiped: only a Retry can undo that."),
        GoBack::Retry if retries == 0 => Some("This Run has no Retries."),
        GoBack::Retry if log.retries_left() == 0 => Some("No Retries are left."),
        _ if log.vote_under_way() => Some("Another vote is under way: a new one can be started once it's over."),
        _ => None,
    }
}

/// The menu for going back `way`, while `open`. `blocked` says why it
/// can't be asked for now, if it can't. Returns the point the Player picked.
pub fn window(
    ctx: &egui::Context,
    open: &mut bool,
    way: GoBack,
    points: &[Offered],
    blocked: Option<&str>,
    now_ms: i64,
) -> Option<Pick> {
    let mut picked = None;
    let with_friends = points.iter().any(|p| matches!(p.pick, Pick::Vote(_)));
    let every = pactlocke_run::RESCUE_POINT_EVERY_MS / 60_000;
    egui::Window::new(way.name())
        .open(open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(match way {
                GoBack::Rescue => format!(
                    "If your game is stuck, go back to a Rescue Point. One is kept about every {every} minutes, \
                     the last {} of them. Everything since is undone, and no Retry is spent.",
                    pactlocke_run::RESCUE_POINTS_KEPT
                ),
                GoBack::Retry => format!(
                    "Choose where to Retry from: the latest Run Checkpoint, or a Rescue Point from about \
                     every {every} minutes before it. Everything since is undone, and one Retry is spent, \
                     whichever you choose."
                ),
            });
            if with_friends {
                ui.weak("Everyone goes back together if everyone says Yes; the first No ends the vote.");
            }
            ui.add_space(6.0);
            if points.is_empty() {
                ui.weak(match way {
                    GoBack::Rescue => {
                        format!("No Rescue Point yet: the first comes about {every} minutes into the Run.")
                    }
                    GoBack::Retry => "No Run Checkpoint to go back to yet.".to_owned(),
                });
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

/// The point a vote under way would go back to, if there is one: "17:51, 8
/// min ago", and every Player's Area there.
pub fn voted_point(log: &RunLog, me: &PlayerKey, way: GoBack, now_ms: i64) -> Option<(Vote, String, String)> {
    let vote = way.vote(log)?;
    let point = with_friends(log, me, way)
        .into_iter()
        .find(|p| p.pick == Pick::Vote(vote.call))?;
    Some((vote, when(point.at_ms, now_ms), areas_line(&point.areas)))
}

/// A vote under way on going back `way`, as a banner with Yes and No for a
/// Player who hasn't said. Returns their vote.
pub fn vote(
    ui: &mut egui::Ui,
    log: &RunLog,
    me: PlayerKey,
    online: bool,
    now_ms: i64,
    way: GoBack,
) -> Option<RunEvent> {
    let (vote, when, areas) = voted_point(log, &me, way, now_ms)?;
    let players = log.active().len();
    let left = match way {
        GoBack::Rescue => String::new(),
        GoBack::Retry => format!(" · {} left", log.retries_left()),
    };
    let mut cast = None;
    crate::spectate::vote_banner(ui, |ui| {
        ui.strong(format!(
            "{} vote: everyone back to {when} · {} of {players} said Yes{left}",
            way.name(),
            vote.yes.len()
        ));
        ui.weak(areas);
        if !vote.yes.contains(&me) && online {
            ui.horizontal(|ui| {
                for (label, yes) in [("Yes", true), ("No", false)] {
                    if ui.button(label).clicked() {
                        cast = Some(way.vote_event(vote.call, yes));
                    }
                }
            });
        }
    });
    cast
}
