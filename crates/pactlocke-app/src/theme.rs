//! What the side panel draws with: each Player's colour, a small icon for
//! each kind of feed line, and party icons read from the Run's ROM.

use std::collections::BTreeMap;
use std::sync::Arc;

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use pactlocke_game::GameProfile;
use pactlocke_run::FeedIcon;

/// The Players' colours in join order: the Host first.
pub const PLAYER_COLOURS: [Color32; 3] = [
    Color32::from_rgb(0xE8, 0xC2, 0x3A),
    Color32::from_rgb(0xA9, 0x8B, 0xF0),
    Color32::from_rgb(0x4F, 0xB8, 0x73),
];
/// Violations, and Dead Pokémon.
pub const DANGER: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
/// Cheat flags.
pub const CHEAT: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x3D);

/// The colour of the Player `index`th in join order.
pub fn player_colour(index: usize) -> Color32 {
    PLAYER_COLOURS[index % PLAYER_COLOURS.len()]
}

/// A Player's name on their colour.
pub fn chip(ui: &mut egui::Ui, name: &str, index: usize) -> egui::Response {
    let text = egui::RichText::new(name).color(Color32::from_gray(20)).strong().small();
    egui::Frame::new()
        .fill(player_colour(index))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(text))
        .response
}

/// A small dot in a Player's colour.
pub fn dot(ui: &mut egui::Ui, index: usize) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, player_colour(index));
}

/// The icon for a kind of feed line, drawn in `colour`.
pub fn feed_icon(ui: &mut egui::Ui, icon: FeedIcon, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
    let p = ui.painter();
    let s = Stroke::new(1.4, colour);
    let c = rect.center();
    let at = |x: f32, y: f32| Pos2::new(rect.left() + x, rect.top() + y);
    match icon {
        FeedIcon::Catch => {
            p.circle_stroke(c, 5.5, s);
            p.line_segment([at(1.5, 7.0), at(12.5, 7.0)], s);
            p.circle_filled(c, 1.6, colour);
        }
        FeedIcon::Encounter => {
            for (x, top) in [(2.0, 3.0), (6.0, 4.0), (10.0, 7.0)] {
                p.line_segment([at(x, 12.0), at(x + 1.5, top)], s);
            }
        }
        FeedIcon::Death => {
            p.line_segment([at(3.0, 3.0), at(11.0, 11.0)], s);
            p.line_segment([at(11.0, 3.0), at(3.0, 11.0)], s);
        }
        FeedIcon::Link => {
            p.rect_stroke(
                Rect::from_min_size(at(1.0, 5.0), Vec2::new(6.0, 4.0)),
                2.0,
                s,
                egui::StrokeKind::Middle,
            );
            p.rect_stroke(
                Rect::from_min_size(at(7.0, 5.0), Vec2::new(6.0, 4.0)),
                2.0,
                s,
                egui::StrokeKind::Middle,
            );
        }
        FeedIcon::Gate => {
            p.line_segment([at(3.0, 13.0), at(3.0, 1.5)], s);
            p.add(egui::Shape::closed_line(
                vec![at(3.0, 2.0), at(11.0, 2.0), at(9.0, 4.5), at(11.0, 7.0), at(3.0, 7.0)],
                s,
            ));
        }
        FeedIcon::Retry => {
            p.add(egui::Shape::line(
                (0..=10)
                    .map(|i| {
                        let a = std::f32::consts::PI * (0.2 + 1.6 * i as f32 / 10.0);
                        c + Vec2::new(a.cos(), a.sin()) * 4.5
                    })
                    .collect(),
                s,
            ));
            p.line_segment([at(3.0, 2.0), at(3.0, 5.0)], s);
            p.line_segment([at(3.0, 5.0), at(6.0, 5.0)], s);
        }
        FeedIcon::Speed => {
            p.add(egui::Shape::closed_line(
                vec![at(2.0, 3.0), at(7.0, 7.0), at(2.0, 11.0)],
                s,
            ));
            p.add(egui::Shape::closed_line(
                vec![at(7.0, 3.0), at(12.0, 7.0), at(7.0, 11.0)],
                s,
            ));
        }
        FeedIcon::Violation => {
            p.add(egui::Shape::closed_line(
                vec![at(7.0, 1.5), at(13.0, 12.5), at(1.0, 12.5)],
                s,
            ));
            p.line_segment([at(7.0, 5.5), at(7.0, 8.7)], s);
            p.circle_filled(at(7.0, 10.6), 0.8, colour);
        }
        FeedIcon::Cheat => {
            p.add(egui::Shape::closed_line(
                vec![
                    at(7.0, 1.0),
                    at(12.0, 3.0),
                    at(12.0, 7.0),
                    at(7.0, 13.0),
                    at(2.0, 7.0),
                    at(2.0, 3.0),
                ],
                s,
            ));
            p.line_segment([at(7.0, 4.5), at(7.0, 7.5)], s);
            p.circle_filled(at(7.0, 9.6), 0.8, colour);
        }
        FeedIcon::Gift => {
            p.rect_stroke(
                Rect::from_min_size(at(2.0, 5.0), Vec2::new(10.0, 8.0)),
                1.0,
                s,
                egui::StrokeKind::Middle,
            );
            p.line_segment([at(7.0, 5.0), at(7.0, 13.0)], s);
            p.line_segment([at(2.0, 5.0), at(12.0, 5.0)], s);
        }
        FeedIcon::Block => {
            p.rect_stroke(
                Rect::from_min_size(at(3.0, 6.0), Vec2::new(8.0, 7.0)),
                1.0,
                s,
                egui::StrokeKind::Middle,
            );
            p.circle_stroke(at(7.0, 6.0), 2.8, s);
        }
        FeedIcon::Run => {
            p.circle_filled(c, 2.0, colour);
        }
    }
}

/// Party icons for one ROM, by species, uploaded once each.
pub struct Icons {
    profile: Arc<GameProfile>,
    rom: Arc<[u8]>,
    textures: BTreeMap<u16, Option<egui::TextureHandle>>,
}

impl Icons {
    pub fn new(profile: Arc<GameProfile>, rom: Arc<[u8]>) -> Self {
        Self {
            profile,
            rom,
            textures: BTreeMap::new(),
        }
    }

    /// The Game Profile of the ROM these icons come from.
    pub fn profile(&self) -> &GameProfile {
        &self.profile
    }

    /// A species' party icon, if the ROM has one.
    pub fn get(&mut self, ctx: &egui::Context, species: u16) -> Option<&egui::TextureHandle> {
        let (profile, rom) = (&self.profile, &self.rom);
        self.textures
            .entry(species)
            .or_insert_with(|| {
                let rgba = pactlocke_game::party_icon_rgba(profile, rom, species)?;
                let size = pactlocke_game::ICON_SIZE;
                let image = egui::ColorImage::from_rgba_unmultiplied([size, size], &rgba);
                Some(ctx.load_texture(format!("icon {species}"), image, egui::TextureOptions::NEAREST))
            })
            .as_ref()
    }

    /// A species' icon at `size` points, Dead ones on a red tile; a blank
    /// tile where there's no icon. Hover shows `tooltip`.
    pub fn show(&mut self, ui: &mut egui::Ui, species: u16, size: f32, dead: bool, tooltip: &str) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
        let fill = if dead {
            DANGER.gamma_multiply(0.25)
        } else {
            ui.visuals().faint_bg_color
        };
        ui.painter().rect_filled(rect, 4.0, fill);
        if dead {
            ui.painter()
                .rect_stroke(rect, 4.0, Stroke::new(1.0, DANGER), egui::StrokeKind::Inside);
        }
        if let Some(texture) = self.get(ui.ctx(), species) {
            egui::Image::new(texture).paint_at(ui, rect);
        }
        response.on_hover_text(tooltip)
    }
}

/// An empty party slot.
pub fn empty_slot(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    ui.painter().rect_stroke(
        rect.shrink(0.5),
        4.0,
        Stroke::new(1.0, ui.visuals().weak_text_color().gamma_multiply(0.4)),
        egui::StrokeKind::Inside,
    );
}
