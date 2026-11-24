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
    /// The zoom in place last frame: the one the screen's size is given at.
    zoom_before: Option<f32>,
}

impl UiScale {
    /// Takes egui's sizes as they are now as the 100% ones, and turns off
    /// egui's Ctrl+plus and Ctrl+minus: the scale is set in Settings only.
    pub fn new(ctx: &egui::Context) -> Self {
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        Self {
            base: ctx.global_style().text_styles.clone(),
            text_size: None,
            zoom_before: None,
        }
    }

    /// Sizes for a test, with egui's default text sizes.
    #[cfg(test)]
    fn without_context() -> Self {
        Self {
            base: egui::Style::default().text_styles,
            text_size: None,
            zoom_before: None,
        }
    }

    /// The zoom wanted this frame, from the screen's height in points as egui
    /// gives it, the OS scale (`native`) and the zoom in place now.
    ///
    /// eframe gives the screen's size in points at the zoom in place *before*
    /// the frame, and a zoom set last frame only takes effect after that: so
    /// the points are turned back into pixels with last frame's zoom. Using
    /// this frame's would read the screen as bigger or smaller after every
    /// change of zoom, and the zoom would never settle.
    fn zoom(&mut self, monitor_points: Option<f32>, native: f32, zoom_now: f32, ui_scale: u16) -> f32 {
        let given_at = self.zoom_before.replace(zoom_now).unwrap_or(zoom_now);
        let height = monitor_points.map(|points| points * given_at * native);
        screen_zoom(height) * f32::from(ui_scale) / 100.0
    }

    /// Run every frame: a window moved to another screen, or a setting changed, shows at once.
    pub fn apply(&mut self, ctx: &egui::Context, ui_scale: u16, text_size: u16) {
        let (monitor_points, native) = ctx.input(|i| {
            let viewport = i.viewport();
            (
                viewport.monitor_size.map(|size| size.y),
                viewport.native_pixels_per_point.unwrap_or(1.0),
            )
        });
        let zoom = self.zoom(monitor_points, native, ctx.zoom_factor(), ui_scale);
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

    /// eframe's order each frame: the screen's size is given in points at the
    /// zoom in place before the frame, then a zoom set last frame takes
    /// effect, then the app runs. Returns each frame's zoom.
    fn frames(screen_height: f32, native: f32, ui_scale: u16, count: usize) -> Vec<f32> {
        let mut scale = UiScale::without_context();
        let (mut zoom, mut pending) = (1.0_f32, None::<f32>);
        let mut zooms = Vec::new();
        for _ in 0..count {
            let monitor_points = screen_height / (zoom * native);
            if let Some(new) = pending.take() {
                zoom = new;
            }
            let wanted = scale.zoom(Some(monitor_points), native, zoom, ui_scale);
            if (zoom - wanted).abs() > 1e-3 {
                pending = Some(wanted);
            }
            zooms.push(zoom);
        }
        zooms
    }

    #[test]
    fn the_zoom_settles_on_any_screen_and_scale_and_stays() {
        for (height, native, ui_scale, settled) in [
            (1440.0, 1.0, 100, 4.0 / 3.0), // An ultrawide 3440x1440.
            (2160.0, 1.5, 100, 2.0),       // 4K at the OS's 150%.
            (1080.0, 1.0, 150, 1.5),       // 1080p at a UI scale of 150%.
            (1080.0, 1.0, 100, 1.0),
        ] {
            let zooms = frames(height, native, ui_scale, 12);
            assert!(
                zooms[2..].iter().all(|z| (z - settled).abs() < 1e-3),
                "{height}p at native {native}, UI scale {ui_scale}%: {zooms:?}"
            );
        }
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
