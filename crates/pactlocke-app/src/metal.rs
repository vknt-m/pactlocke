//! The home screen's metallic accent: a fill that shades from a bright top
//! to a darker band below the middle, with a light line along its top
//! edge, a dark one along its bottom, and a glint that sweeps across while
//! the pointer is over it. egui fills a shape with one colour, so the
//! shading is a small gradient texture, one per accent, stretched over a
//! rounded rectangle.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, TextureHandle, Vec2};

use crate::settings::Accent;

/// How long the glint takes to cross.
const GLINT_SECONDS: f32 = 0.55;
/// The gradient texture's height in texels.
const SHADES: usize = 64;

/// An accent's colours.
struct Metal {
    /// Top to bottom: the bright top, the middle, the darker band, the bottom.
    stops: [Color32; 4],
    /// The border.
    edge: Color32,
    /// Text on it.
    ink: Color32,
}

fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

fn metal(accent: Accent) -> Metal {
    let (stops, edge, ink) = match accent {
        Accent::Silver => ([0xFFFFFF, 0xD3D8DE, 0xBEC4CB, 0xD9DEE3], 0x7C848C, 0x1A1D21),
        Accent::Cobalt => ([0xA9D0FF, 0x4E8FE6, 0x3C77CF, 0x5A9AEE], 0x1C3F7D, 0x04142E),
        Accent::Emerald => ([0xB4F5C9, 0x47C27A, 0x35A563, 0x52CA84], 0x165C33, 0x04200F),
        Accent::Gold => ([0xFFE9A3, 0xE8B93E, 0xD8A62E, 0xE9BE52], 0x8A6210, 0x2A1C00),
    };
    Metal {
        stops: stops.map(hex),
        edge: hex(edge),
        ink: hex(ink),
    }
}

/// The accent's middle colour, for the open tab's underline and the row under the pointer.
pub fn colour(accent: Accent) -> Color32 {
    metal(accent).stops[1]
}

/// The colour of text on the accent.
pub fn ink(accent: Accent) -> Color32 {
    metal(accent).ink
}

/// The shade at `t` from the top (0) to the bottom (1): the middle at 46%,
/// the darker band from 54%.
fn shade(stops: &[Color32; 4], t: f32) -> Color32 {
    let at = [0.0, 0.46, 0.54, 1.0];
    let i = (1..4).find(|&i| t <= at[i]).unwrap_or(3);
    let f = ((t - at[i - 1]) / (at[i] - at[i - 1])).clamp(0.0, 1.0);
    let (a, b) = (stops[i - 1], stops[i]);
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * f).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// The accent's gradient texture, made once and kept in egui's memory.
fn gradient(ctx: &egui::Context, accent: Accent) -> TextureHandle {
    let id = egui::Id::unique(("metal gradient", accent.name()));
    if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return texture;
    }
    let stops = metal(accent).stops;
    let pixels: Vec<Color32> = (0..SHADES)
        .map(|i| shade(&stops, i as f32 / (SHADES - 1) as f32))
        .collect();
    let image = egui::ColorImage::new([1, SHADES], pixels);
    let texture = ctx.load_texture(format!("metal {}", accent.name()), image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    texture
}

/// Paints the metal over `rect`, with its glint if `response` is hovered.
pub fn paint(ui: &egui::Ui, rect: Rect, accent: Accent, response: &egui::Response) {
    let Metal { edge, .. } = metal(accent);
    let radius = 6.0;
    let texture = gradient(ui.ctx(), accent);
    let painter = ui.painter();
    painter.add(
        egui::epaint::RectShape::filled(rect, radius, Color32::WHITE).with_texture(
            texture.id(),
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
        ),
    );
    // Light catches the top edge, and the bottom edge sits in shadow.
    let inner = rect.shrink(1.0);
    painter.line_segment(
        [
            inner.left_top() + Vec2::new(radius - 1.0, 0.0),
            inner.right_top() - Vec2::new(radius - 1.0, 0.0),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(170)),
    );
    painter.line_segment(
        [
            inner.left_bottom() + Vec2::new(radius - 1.0, 0.0),
            inner.right_bottom() - Vec2::new(radius - 1.0, 0.0),
        ],
        Stroke::new(1.0, Color32::from_black_alpha(60)),
    );
    painter.rect_stroke(rect, radius, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    glint(ui, rect, response);
}

/// A light band sweeping across while the pointer is over it.
fn glint(ui: &egui::Ui, rect: Rect, response: &egui::Response) {
    let t = ui
        .ctx()
        .animate_bool_with_time(response.id.with("glint"), response.hovered(), GLINT_SECONDS);
    if !response.hovered() || t >= 1.0 {
        return;
    }
    let width = rect.width() * 0.4;
    let x = egui::lerp((rect.left() - width)..=(rect.right() + width), t);
    let (clear, bright) = (Color32::TRANSPARENT, Color32::from_white_alpha(120));
    let mut mesh = egui::Mesh::default();
    let columns = [(x - width / 2.0, clear), (x, bright), (x + width / 2.0, clear)];
    for (px, colour) in columns {
        mesh.colored_vertex(Pos2::new(px + 6.0, rect.top()), colour);
        mesh.colored_vertex(Pos2::new(px - 6.0, rect.bottom()), colour);
    }
    for i in 0..2u32 {
        let k = i * 2;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
    ui.painter().with_clip_rect(rect.shrink(1.0)).add(mesh);
}

/// A button in the metal: `text` laid out in its ink. Its size is `size`, or the text's.
pub fn button(ui: &mut egui::Ui, accent: Accent, text: egui::WidgetText, size: Vec2) -> egui::Response {
    let padding = Vec2::new(12.0, 5.0);
    let galley = text.into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let wanted = size.max(galley.size() + padding * 2.0);
    let (rect, response) = ui.allocate_exact_size(wanted, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        paint(ui, rect, accent, &response);
        let at = if size.x > 0.0 {
            Pos2::new(rect.left() + padding.x, rect.center().y - galley.size().y / 2.0)
        } else {
            rect.center() - galley.size() / 2.0
        };
        ui.painter().galley(at, galley, ink(accent));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(c: Color32) -> f32 {
        0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b())
    }

    #[test]
    fn every_metal_shades_from_a_bright_top_to_a_darker_band() {
        for accent in Accent::ALL {
            let stops = metal(accent).stops;
            assert_eq!(shade(&stops, 0.0), stops[0], "{accent:?}");
            assert_eq!(shade(&stops, 1.0), stops[3], "{accent:?}");
            assert!(light(shade(&stops, 0.0)) > light(shade(&stops, 0.6)), "{accent:?}");
        }
    }

    #[test]
    fn the_ink_reads_on_every_shade_of_its_metal() {
        for accent in Accent::ALL {
            let Metal { stops, ink, .. } = metal(accent);
            for i in 0..=10 {
                let gap = (light(shade(&stops, i as f32 / 10.0)) - light(ink)).abs();
                assert!(gap > 75.0, "{accent:?} at {}%: {gap}", i * 10);
            }
        }
    }
}
