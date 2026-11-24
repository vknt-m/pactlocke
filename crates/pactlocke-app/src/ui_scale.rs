//! How big the app is drawn: at any resolution it looks as it does on a 1080p
//! screen, on top of the OS scale, then Settings' UI scale and Text size.

use std::collections::BTreeMap;

use eframe::egui::{self, FontId, TextStyle};

/// The screen height, in pixels, the app is laid out for at 100%.
pub const REFERENCE_HEIGHT: f32 = 1080.0;
/// The lowest and highest UI scale and Text size, and the step between, in percent.
pub const PERCENTS: (u16, u16, u16) = (50, 200, 10);

/// A percent from the settings file, put on the nearest step between the lowest and highest.
pub fn snap_percent(percent: u16) -> u16 {
    let (low, high, step) = PERCENTS;
    let snapped = (percent.saturating_add(step / 2) / step) * step;
    snapped.clamp(low, high)
}

/// The zoom that makes a screen `height` pixels tall look like a 1080p one;
/// 1 while the screen isn't known.
pub fn screen_zoom(height: Option<f32>) -> f32 {
    match height {
        Some(height) if height > 0.0 => height / REFERENCE_HEIGHT,
        _ => 1.0,
    }
}

/// egui's text sizes, `percent` of `base`.
pub fn text_sizes(base: &BTreeMap<TextStyle, FontId>, percent: u16) -> BTreeMap<TextStyle, FontId> {
    let factor = f32::from(percent) / 100.0;
    base.iter()
        .map(|(style, font)| (style.clone(), FontId::new(font.size * factor, font.family.clone())))
        .collect()
}

/// A text size drawn by hand, `points` at 100%, at the Text size `style` has.
pub fn text_points(style: &egui::Style, points: f32) -> f32 {
    let body = |style: &egui::Style| style.text_styles.get(&TextStyle::Body).map(|font| font.size);
    match (body(style), body(&egui::Style::default())) {
        (Some(now), Some(base)) => points * now / base,
        _ => points,
    }
}

/// Keeps egui's zoom and text sizes on the settings and the screen the window is on.
pub struct UiScale {
    /// egui's own text sizes, before any Text size.
    base: BTreeMap<TextStyle, FontId>,
    /// The Text size last put to work.
    text_size: Option<u16>,
}

impl UiScale {
    /// Takes egui's sizes as they are now as the 100% ones, and turns off
    /// egui's Ctrl+plus and Ctrl+minus: the scale is set in Settings only.
    pub fn new(ctx: &egui::Context) -> Self {
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        Self {
            base: ctx.global_style().text_styles.clone(),
            text_size: None,
        }
    }

    /// Run every frame: a window moved to another screen, or a setting changed, shows at once.
    pub fn apply(&mut self, ctx: &egui::Context, ui_scale: u16, text_size: u16) {
        // egui gives the screen's size in points at this frame's pixels per point.
        let height = ctx.input(|i| i.viewport().monitor_size.map(|size| size.y * i.pixels_per_point));
        let zoom = screen_zoom(height) * f32::from(ui_scale) / 100.0;
        if (ctx.zoom_factor() - zoom).abs() > 1e-3 {
            ctx.set_zoom_factor(zoom);
        }
        if self.text_size != Some(text_size) {
            let sizes = text_sizes(&self.base, text_size);
            ctx.all_styles_mut(|style| style.text_styles = sizes.clone());
            self.text_size = Some(text_size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percent_snaps_to_the_nearest_step_between_50_and_200() {
        assert_eq!(snap_percent(100), 100);
        assert_eq!(snap_percent(124), 120);
        assert_eq!(snap_percent(125), 130);
        assert_eq!(snap_percent(0), 50);
        assert_eq!(snap_percent(900), 200);
    }

    #[test]
    fn the_screen_zoom_goes_by_the_screens_height_against_1080p() {
        assert_eq!(screen_zoom(Some(1080.0)), 1.0);
        assert_eq!(screen_zoom(Some(2160.0)), 2.0);
        // An ultrawide 3440x1440 scales like any 1440p screen.
        assert!((screen_zoom(Some(1440.0)) - 4.0 / 3.0).abs() < 1e-6);
        assert_eq!(screen_zoom(None), 1.0);
        assert_eq!(screen_zoom(Some(0.0)), 1.0);
    }

    #[test]
    fn text_size_scales_every_text_style_and_keeps_its_font() {
        let base = egui::Style::default().text_styles;
        let bigger = text_sizes(&base, 150);
        assert_eq!(bigger[&TextStyle::Body].size, 13.0 * 1.5);
        assert_eq!(bigger[&TextStyle::Heading].size, 18.0 * 1.5);
        assert_eq!(bigger[&TextStyle::Monospace].family, base[&TextStyle::Monospace].family);
        assert_eq!(text_sizes(&base, 100), base);
    }

    #[test]
    fn a_text_size_drawn_by_hand_follows_the_text_size_setting() {
        let mut style = egui::Style::default();
        assert_eq!(text_points(&style, 10.0), 10.0);
        style.text_styles = text_sizes(&style.text_styles, 150);
        assert_eq!(text_points(&style, 10.0), 15.0);
    }
}
