//! The Player strip at the top of the side panel: each Player's name ("you"
//! on your own row), Area, speed, connection and ping, what they are doing,
//! and a Watch button; then the battery saver, the partner view and the
//! pop-out. It displays what the Run engine, the Run log and the Shadow
//! Games say; its questions answer with [`Asked`].

use eframe::egui;

/// How this app reaches another Player's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Direct {
        ping_ms: u32,
    },
    Relayed {
        ping_ms: u32,
    },
    /// A joiner reaches the other joiners through the Host.
    ViaHost,
    /// Not heard from for a while: their game is frozen here until they are back.
    Offline,
}

/// One Player's row.
pub struct Row {
    pub name: String,
    /// This app's own Player.
    pub you: bool,
    pub area: Option<String>,
    pub speed: u8,
    /// `None` on your own row, or while a connection is being made.
    pub reach: Option<Reach>,
    /// "Exploring", "In battle", "Ready at Brock", "catching up…"…
    pub doing: String,
    /// Another Player's Shadow Game: its index, and why it can't be watched, if it can't.
    pub watch: Option<(usize, Option<String>)>,
}

/// What the strip asks for.
pub enum Asked {
    Watch(usize),
    BatterySaver(bool),
    PartnerView(bool),
    PopOut(bool),
}

/// The toggles under the strip, as they are now.
pub struct Toggles {
    pub battery_saver: bool,
    pub partner_view: bool,
    pub popped_out: bool,
}

pub fn show(ui: &mut egui::Ui, rows: &[Row], toggles: &Toggles) -> Option<Asked> {
    let mut asked = None;
    egui::Grid::new("player strip")
        .num_columns(6)
        .striped(true)
        .show(ui, |ui| {
            for heading in ["Player", "Area", "Speed", "Connection", "Status", ""] {
                ui.weak(heading);
            }
            ui.end_row();
            for row in rows {
                if row.you {
                    ui.strong(format!("{} (you)", row.name));
                } else {
                    ui.label(&row.name);
                }
                ui.label(row.area.as_deref().unwrap_or("—"));
                ui.label(format!("{}x", row.speed));
                match row.reach {
                    _ if row.you => ui.weak("—"),
                    Some(Reach::Direct { ping_ms }) => ui.label(format!("direct · {ping_ms} ms")),
                    Some(Reach::Relayed { ping_ms }) => ui.label(format!("relayed · {ping_ms} ms")),
                    Some(Reach::ViaHost) => ui.label("via the Host"),
                    Some(Reach::Offline) => ui.colored_label(ui.visuals().warn_fg_color, "offline"),
                    None => ui.weak("connecting…"),
                };
                ui.label(&row.doing);
                match &row.watch {
                    Some((index, None)) => {
                        if ui.button("Watch").clicked() {
                            asked = Some(Asked::Watch(*index));
                        }
                    }
                    Some((_, Some(why))) => {
                        ui.add_enabled(false, egui::Button::new("Watch"))
                            .on_disabled_hover_text(why);
                    }
                    None => {
                        ui.label("");
                    }
                }
                ui.end_row();
            }
        });
    ui.horizontal_wrapped(|ui| {
        let mut saver = toggles.battery_saver;
        if ui
            .checkbox(&mut saver, "Battery saver")
            .on_hover_text("Run the other Players' games only while you watch one; it catches up when you do")
            .changed()
        {
            asked = Some(Asked::BatterySaver(saver));
        }
        let mut tiles = toggles.partner_view;
        if ui
            .checkbox(&mut tiles, "Partner view")
            .on_hover_text("The other Players' games as small tiles beside yours")
            .changed()
        {
            asked = Some(Asked::PartnerView(tiles));
        }
        let pop = if toggles.popped_out {
            "Dock the panel"
        } else {
            "Pop out"
        };
        if ui.button(pop).clicked() {
            asked = Some(Asked::PopOut(!toggles.popped_out));
        }
    });
    asked
}
