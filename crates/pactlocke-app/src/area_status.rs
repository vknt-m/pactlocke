//! The Area status: whether the Area this Player is in allows a catch, in
//! the style Settings picks. The Area card sits at the top of the Feed tab
//! and the Strip above the side panel's tabs, both in bold fills; the
//! Overlay pops up on the game screen in traffic-light tints, then fades.
//! It displays the Run engine's [`AreaStatus`].

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke};
use pactlocke_run::{AreaState, AreaStatus};

use crate::settings::AreaStatusStyle;
use crate::theme::Icons;
use crate::ui_scale::text_points;

/// How long the overlay shows before it fades.
pub const OVERLAY_SHOWS_FOR: Duration = Duration::from_secs(4);
/// How long the fade takes.
const OVERLAY_FADE: Duration = Duration::from_millis(500);
/// The party icons' size on the Area card.
const ICON: f32 = 24.0;

/// A state's colours: its accent (fill and border alike) and the ink that reads on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Colours {
    accent: Color32,
    ink: Color32,
}

/// Bold fills, for the Area card and the Strip.
fn bold(state: AreaState) -> Colours {
    let (accent, ink) = match state {
        AreaState::CatchAllowed => ((0x45, 0xD2, 0x7E), (0x06, 0x20, 0x0F)),
        AreaState::EncounterDone => ((0x5C, 0x5C, 0x5C), (0xF2, 0xF2, 0xF2)),
        AreaState::Dupe => ((0xF2, 0xB6, 0x40), (0x2A, 0x1D, 0x00)),
        AreaState::NotCountedYet => ((0x3F, 0x7B, 0xD6), (0xF2, 0xF2, 0xF2)),
    };
    Colours {
        accent: Color32::from_rgb(accent.0, accent.1, accent.2),
        ink: Color32::from_rgb(ink.0, ink.1, ink.2),
    }
}

/// Traffic-light tints, for the Overlay: the accent on a dark backing.
fn traffic(state: AreaState) -> Color32 {
    match state {
        AreaState::CatchAllowed => Color32::from_rgb(0x3F, 0xB4, 0x6A),
        AreaState::EncounterDone => Color32::from_rgb(0x8F, 0x8F, 0x8F),
        AreaState::Dupe => Color32::from_rgb(0xD9, 0xA2, 0x3A),
        AreaState::NotCountedYet => Color32::from_rgb(0x6E, 0x9B, 0xD6),
    }
}

/// `a` mixed with `b`, `t` of the way to `b`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(channel(a.r(), b.r()), channel(a.g(), b.g()), channel(a.b(), b.b()))
}

/// The state's mark: a small filled circle.
fn mark(ui: &mut egui::Ui, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, colour);
}

/// The state's few words, as the card and strip shout them.
fn say(ui: &egui::Ui, status: &AreaStatus, colour: Color32) -> RichText {
    RichText::new(status.say().to_uppercase())
        .strong()
        .size(text_points(ui.style(), 12.0))
        .color(colour)
}

/// The Area's name, large and bold.
fn area_name(ui: &egui::Ui, status: &AreaStatus, colour: Color32, points: f32) -> RichText {
    RichText::new(&status.area)
        .strong()
        .size(text_points(ui.style(), points))
        .color(colour)
}

/// The Area card, at the top of the Feed tab: the state, the Area's
/// Encounter, and the wild battles since as party icons, newest first.
pub fn card(ui: &mut egui::Ui, status: &AreaStatus, icons: &mut Icons) {
    let Colours { accent, ink } = bold(status.state);
    egui::Frame::new()
        .stroke(Stroke::new(1.5, accent))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            egui::Frame::new()
                .fill(accent)
                .corner_radius(CornerRadius {
                    nw: 7,
                    ne: 7,
                    sw: 0,
                    se: 0,
                })
                .inner_margin(Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        mark(ui, ink);
                        ui.label(area_name(ui, status, ink, 14.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(say(ui, status, ink));
                        });
                    });
                });
            egui::Frame::new()
                .inner_margin(Margin {
                    left: 10,
                    right: 10,
                    top: 4,
                    bottom: 8,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let key_width = 70.0;
                    ui.horizontal(|ui| {
                        key(ui, "Encounter", key_width);
                        match &status.encounter {
                            Some(e) => {
                                let tip = if e.caught {
                                    format!("{}: caught here", e.species_name)
                                } else {
                                    format!("{}: met here, not caught", e.species_name)
                                };
                                icon(ui, icons, e.species, &tip, e.caught.then_some(accent));
                                let how = if e.caught { "caught" } else { "not caught" };
                                ui.label(format!("{}, {how}", e.species_name));
                            }
                            None => {
                                ui.weak(match status.state {
                                    AreaState::NotCountedYet => "not yet: no Poké Balls",
                                    AreaState::EncounterDone => "used",
                                    _ => "still open",
                                });
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        key(
                            ui,
                            if status.encounter.is_some() { "Since" } else { "Battles" },
                            key_width,
                        );
                        if status.battles.is_empty() {
                            ui.weak("none yet");
                            return;
                        }
                        ui.spacing_mut().item_spacing.x = 2.0;
                        // As many as fit, newest first.
                        let fit = ((ui.available_width() + 2.0) / (ICON + 2.0)).floor().max(1.0) as usize;
                        for battle in status.battles.iter().take(fit) {
                            let tip = format!("{}: {}", battle.species_name, battle.what());
                            icon(ui, icons, battle.species, &tip, None);
                        }
                    });
                    ui.label(RichText::new(status.why()).small().weak());
                });
        });
}

/// A row's label on the Area card.
fn key(ui: &mut egui::Ui, text: &str, width: f32) {
    ui.add_sized([width, ICON], egui::Label::new(RichText::new(text).weak()));
}

/// A party icon on the Area card: one caught is outlined in `caught`, the
/// rest dimmed under a veil, as in the Areas tab.
fn icon(ui: &mut egui::Ui, icons: &mut Icons, species: u16, tip: &str, caught: Option<Color32>) {
    let shown = icons.show(ui, species, ICON, false, tip);
    match caught {
        Some(outline) => {
            ui.painter()
                .rect_stroke(shown.rect, 4.0, Stroke::new(1.5, outline), egui::StrokeKind::Inside);
        }
        None => {
            ui.painter()
                .rect_filled(shown.rect, 4.0, ui.visuals().panel_fill.gamma_multiply(0.6));
        }
    }
}

/// The Strip, above the side panel's tabs: the state and why, in a bold fill.
pub fn strip(ui: &mut egui::Ui, status: &AreaStatus) {
    let Colours { accent, ink } = bold(status.state);
    egui::Frame::new()
        .fill(accent)
        .stroke(Stroke::new(1.0, accent))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                mark(ui, ink);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(area_name(ui, status, ink, 14.0));
                        ui.label(say(ui, status, ink));
                    });
                    ui.label(RichText::new(status.why()).small().color(mix(ink, accent, 0.2)));
                });
            });
        });
}

/// When the overlay shows: on entering an Area and at each wild battle,
/// for [`OVERLAY_SHOWS_FOR`], then it fades.
#[derive(Debug, Default)]
pub struct Overlay {
    /// The Area and its wild battles as last seen.
    seen: Option<(String, usize)>,
    /// When it last popped up.
    since: Option<Instant>,
}

impl Overlay {
    /// How opaque the overlay is at `now`, from 1 to 0, with `status` as
    /// the Area status is now.
    pub fn opacity(&mut self, status: Option<&AreaStatus>, now: Instant) -> f32 {
        let key = status.map(|s| (s.area.clone(), s.met));
        if key != self.seen {
            self.since = key.is_some().then_some(now);
            self.seen = key;
        }
        let Some(since) = self.since else {
            return 0.0;
        };
        let shown = now.saturating_duration_since(since);
        if shown <= OVERLAY_SHOWS_FOR {
            return 1.0;
        }
        let fading = (shown - OVERLAY_SHOWS_FOR).as_secs_f32() / OVERLAY_FADE.as_secs_f32();
        (1.0 - fading).max(0.0)
    }
}

/// Whether the overlay shows in this state with these settings: always as
/// the Overlay style; with the Card or Strip only when a catch is allowed,
/// if the Player ticked that.
pub fn overlay_wanted(style: AreaStatusStyle, catch_overlay: bool, state: AreaState) -> bool {
    match style {
        AreaStatusStyle::Overlay => true,
        AreaStatusStyle::Card | AreaStatusStyle::Strip => catch_overlay && state == AreaState::CatchAllowed,
    }
}

/// The overlay on the game screen `screen`, at `opacity`, in traffic-light tints.
pub fn overlay(ctx: &egui::Context, screen: egui::Rect, status: &AreaStatus, opacity: f32) {
    let accent = traffic(status.state);
    egui::Area::new(egui::Id::unique("area status overlay"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .fixed_pos(screen.min + egui::vec2(12.0, 12.0))
        .show(ctx, |ui| {
            ui.set_opacity(opacity);
            ui.set_max_width(screen.width() * 0.7);
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(14, 14, 14, 220))
                .stroke(Stroke::new(2.0, accent))
                .corner_radius(8)
                .inner_margin(Margin {
                    left: 16,
                    right: 12,
                    top: 7,
                    bottom: 8,
                })
                .show(ui, |ui| {
                    let inner = ui.max_rect();
                    // The bar down the left, inside the border.
                    let bar = egui::Rect::from_min_max(
                        egui::pos2(inner.left() - 14.0, inner.top() - 5.0),
                        egui::pos2(inner.left() - 8.0, inner.bottom() + 6.0),
                    );
                    ui.painter().rect_filled(bar, 0.0, accent);
                    ui.label(area_name(ui, status, Color32::from_gray(0xF2), 15.0));
                    ui.label(say(ui, status, accent));
                });
        });
}

/// Draws the Area status on the game screen, if it should show there now.
/// Returns whether it is still on screen, so the caller keeps repainting.
pub fn on_game_screen(
    ctx: &egui::Context,
    screen: egui::Rect,
    status: Option<&AreaStatus>,
    shown: &mut Overlay,
    style: AreaStatusStyle,
    catch_overlay: bool,
) -> bool {
    let opacity = shown.opacity(status, Instant::now());
    match status {
        Some(status) if opacity > 0.0 && overlay_wanted(style, catch_overlay, status.state) => {
            overlay(ctx, screen, status, opacity);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pactlocke_run::AreaStatus;

    fn status(area: &str, state: AreaState, met: usize) -> AreaStatus {
        AreaStatus {
            area: area.to_owned(),
            state,
            encounter: None,
            battles: Vec::new(),
            met,
        }
    }

    #[test]
    fn the_overlay_shows_on_entering_an_area_and_at_each_wild_battle_then_fades() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut shown = Overlay::default();
        let route = status("Route 1", AreaState::CatchAllowed, 0);
        assert_eq!(shown.opacity(None, at(0)), 0.0, "nowhere listed: nothing");
        assert_eq!(shown.opacity(Some(&route), at(0)), 1.0, "entering Route 1");
        assert_eq!(shown.opacity(Some(&route), at(4_000)), 1.0, "still up at 4 seconds");
        assert!(shown.opacity(Some(&route), at(4_250)) < 1.0, "fading");
        assert_eq!(shown.opacity(Some(&route), at(4_500)), 0.0, "gone");

        let battle = status("Route 1", AreaState::EncounterDone, 1);
        assert_eq!(
            shown.opacity(Some(&battle), at(9_000)),
            1.0,
            "a wild battle shows it again"
        );
        let caught = AreaStatus {
            encounter: Some(pactlocke_run::AreaEncounter {
                area: "Route 1".to_owned(),
                species: 16,
                species_name: "PIDGEY".to_owned(),
                caught: true,
            }),
            ..battle.clone()
        };
        assert_eq!(
            shown.opacity(Some(&caught), at(14_000)),
            0.0,
            "the same battle: no new pop-up"
        );
        let town = status("Viridian City", AreaState::CatchAllowed, 0);
        assert_eq!(shown.opacity(Some(&town), at(15_000)), 1.0, "a new Area");
    }

    #[test]
    fn the_extra_overlay_only_pops_up_when_a_catch_is_allowed() {
        use AreaStatusStyle::*;
        assert!(overlay_wanted(Overlay, false, AreaState::EncounterDone));
        assert!(!overlay_wanted(Card, false, AreaState::CatchAllowed), "off by default");
        assert!(overlay_wanted(Card, true, AreaState::CatchAllowed));
        assert!(overlay_wanted(Strip, true, AreaState::CatchAllowed));
        for state in [AreaState::EncounterDone, AreaState::Dupe, AreaState::NotCountedYet] {
            assert!(!overlay_wanted(Card, true, state), "{state:?}");
        }
    }

    #[test]
    fn every_bold_fill_has_its_border_in_the_same_colour_and_ink_that_reads_on_it() {
        for state in [
            AreaState::CatchAllowed,
            AreaState::EncounterDone,
            AreaState::Dupe,
            AreaState::NotCountedYet,
        ] {
            let Colours { accent, ink } = bold(state);
            let light = |c: Color32| 0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b());
            assert!((light(accent) - light(ink)).abs() > 90.0, "{state:?}");
        }
    }
}
