//! The Settings dialog, opened from Options ▸ Settings: one dialog with its
//! categories down the left, as other emulators have. It edits a copy of the
//! settings; OK keeps the copy, Cancel throws it away.

use eframe::egui;

use crate::settings::{AreaStatusStyle, BUTTONS, RelayChoice, Settings};

/// A Settings category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    General,
    Controls,
    Sound,
    Video,
    Network,
    Data,
    Developer,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::General,
        Category::Controls,
        Category::Sound,
        Category::Video,
        Category::Network,
        Category::Data,
        Category::Developer,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Controls => "Controls",
            Category::Sound => "Sound",
            Category::Video => "Video",
            Category::Network => "Network",
            Category::Data => "Data and logs",
            Category::Developer => "Developer",
        }
    }
}

/// The open dialog: the copy being edited, and what is being rebound.
pub struct SettingsDialog {
    pub draft: Settings,
    pub category: Category,
    /// The button (or "fast-forward", "pause") waiting for its new key.
    rebinding: Option<String>,
}

/// What came of the dialog this frame.
pub enum Outcome {
    Open,
    Save(Box<Settings>),
    Cancel,
}

/// What the dialog shows besides the settings themselves.
pub struct Facts<'a> {
    /// The sound output devices by name.
    pub devices: &'a [String],
    /// Where the Runs and the log are kept.
    pub data_dir: &'a std::path::Path,
    /// A gamepad is connected now, by name.
    pub gamepad: Option<&'a str>,
    /// A solo game is paused while the dialog is open.
    pub pausing: bool,
}

impl SettingsDialog {
    pub fn new(settings: &Settings, category: Category) -> Self {
        Self {
            draft: settings.clone(),
            category,
            rebinding: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, facts: &Facts<'_>) -> Outcome {
        let mut outcome = Outcome::Open;
        let mut took_key = false;
        if let Some(target) = self.rebinding.clone() {
            // The key is taken out of egui's input, so it neither clicks the
            // button again (Space, Enter) nor closes the dialog (Esc).
            let pressed = ctx.input_mut(|i| {
                let key = i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } => Some(*key),
                    _ => None,
                })?;
                i.events.retain(|e| !matches!(e, egui::Event::Key { .. }));
                i.keys_down.remove(&key);
                Some(key)
            });
            if let Some(key) = pressed {
                took_key = true;
                if key != egui::Key::Escape {
                    match target.as_str() {
                        "fast-forward" => self.draft.fast_forward_key = key.name().to_owned(),
                        "pause" => self.draft.pause_key = key.name().to_owned(),
                        button => self.draft.bind(button, key),
                    }
                }
                self.rebinding = None;
            }
        }
        let modal = egui::Modal::new(egui::Id::unique("settings")).show(ctx, |ui| {
            ui.set_width(640.0);
            ui.heading("Settings");
            ui.separator();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(130.0);
                    for category in Category::ALL {
                        if ui
                            .selectable_label(self.category == category, category.name())
                            .clicked()
                        {
                            self.category = category;
                            self.rebinding = None;
                        }
                    }
                });
                // A gap, not a separator: a vertical one would take the modal's unbounded height.
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_min_height(320.0);
                    self.pane(ui, facts);
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                let named = self.draft.name.as_deref().is_some_and(|n| !n.trim().is_empty());
                if ui.add_enabled(named, egui::Button::new("OK")).clicked() {
                    outcome = Outcome::Save(Box::new(self.draft.clone()));
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
                if facts.pausing {
                    ui.weak("Your game is paused while Settings is open.");
                }
            });
        });
        if matches!(outcome, Outcome::Open) && modal.should_close() && self.rebinding.is_none() && !took_key {
            outcome = Outcome::Cancel;
        }
        outcome
    }

    fn pane(&mut self, ui: &mut egui::Ui, facts: &Facts<'_>) {
        let s = &mut self.draft;
        match self.category {
            Category::General => {
                egui::Grid::new("general")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Your name");
                        let mut name = s.name.clone().unwrap_or_default();
                        if ui
                            .add(egui::TextEdit::singleline(&mut name).char_limit(crate::settings::NAME_LIMIT))
                            .changed()
                        {
                            s.name = Some(name);
                        }
                        ui.end_row();
                        ui.label("");
                        ui.weak("What other Players see.");
                        ui.end_row();
                        ui.label("Default speed");
                        egui::ComboBox::from_id_salt("default speed")
                            .selected_text(format!("{}x", s.default_speed))
                            .show_ui(ui, |ui| {
                                for speed in 1..=4u8 {
                                    ui.selectable_value(&mut s.default_speed, speed, format!("{speed}x"));
                                }
                            });
                        ui.end_row();
                        ui.label("Pause when unfocused");
                        ui.checkbox(&mut s.pause_unfocused, "");
                        ui.end_row();
                        ui.label("");
                        ui.weak("Solo only: in a Run with friends nothing pauses for the others.");
                        ui.end_row();
                        let (low, high, step) = crate::ui_scale::PERCENTS;
                        for (label, value) in [("UI scale", &mut s.ui_scale), ("Text size", &mut s.text_size)] {
                            ui.label(label);
                            ui.add(
                                egui::Slider::new(value, low..=high)
                                    .step_by(f64::from(step))
                                    .suffix("%"),
                            );
                            ui.end_row();
                        }
                        ui.label("");
                        ui.weak(format!(
                            "At 100% you see the app as you would on a {}p screen.",
                            crate::ui_scale::REFERENCE_HEIGHT
                        ));
                        ui.end_row();
                        ui.label("Area status");
                        ui.horizontal(|ui| {
                            for style in AreaStatusStyle::ALL {
                                ui.radio_value(&mut s.area_status_style, style, style.name());
                            }
                        });
                        ui.end_row();
                        ui.label("");
                        // With Overlay picked, the overlay shows already.
                        ui.add_enabled(
                            s.area_status_style != AreaStatusStyle::Overlay,
                            egui::Checkbox::new(
                                &mut s.catch_overlay,
                                "Also pop up an overlay on the game screen when a catch is allowed",
                            ),
                        );
                        ui.end_row();
                    });
            }
            Category::Controls => {
                ui.checkbox(&mut s.gamepad, "Read a gamepad too");
                match facts.gamepad {
                    Some(name) => ui.weak(format!("Connected: {name}. Its buttons: A, B, the shoulders, Start, Back as Select, the D-pad and the left stick; hold the right trigger to fast-forward.")),
                    None => ui.weak("No gamepad connected."),
                };
                ui.add_space(6.0);
                egui::Grid::new("keys")
                    .num_columns(4)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        let mut binds: Vec<(String, String)> = BUTTONS
                            .iter()
                            .map(|(_, b)| {
                                let key = s.key_of(b).map_or("—".to_owned(), |k| k.name().to_owned());
                                ((*b).to_owned(), key)
                            })
                            .collect();
                        binds.push(("Fast-forward (hold)".into(), s.fast_forward_key.clone()));
                        binds.push(("Pause".into(), s.pause_key.clone()));
                        for (i, (button, key)) in binds.iter().enumerate() {
                            let target = match button.as_str() {
                                "Fast-forward (hold)" => "fast-forward".to_owned(),
                                "Pause" => "pause".to_owned(),
                                b => b.to_owned(),
                            };
                            ui.label(button);
                            let waiting = self.rebinding.as_deref() == Some(target.as_str());
                            let text = if waiting {
                                "press a key…".to_owned()
                            } else {
                                key.clone()
                            };
                            if ui
                                .button(text)
                                .on_hover_text("Click, then press the new key (Esc keeps it)")
                                .clicked()
                            {
                                self.rebinding = Some(target);
                            }
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                if ui.button("Back to mGBA's keys").clicked() {
                    let defaults = Settings::default();
                    s.keys = defaults.keys;
                    s.fast_forward_key = defaults.fast_forward_key;
                    s.pause_key = defaults.pause_key;
                }
            }
            Category::Sound => {
                egui::Grid::new("sound")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Volume");
                        ui.add(egui::Slider::new(&mut s.volume, 0.0..=1.0).show_value(false));
                        ui.end_row();
                        ui.label("Mute while fast-forwarding");
                        ui.checkbox(&mut s.mute_fast_forward, "");
                        ui.end_row();
                        ui.label("Sound of the game you Watch");
                        ui.checkbox(&mut s.watch_sound, "");
                        ui.end_row();
                        ui.label("Output");
                        egui::ComboBox::from_id_salt("output")
                            .selected_text(s.output_device.clone().unwrap_or_else(|| "System default".into()))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut s.output_device, None, "System default");
                                for device in facts.devices {
                                    ui.selectable_value(&mut s.output_device, Some(device.clone()), device);
                                }
                            });
                        ui.end_row();
                        ui.label("");
                        ui.weak("A new output device is used from the next game you open.");
                        ui.end_row();
                    });
            }
            Category::Video => {
                egui::Grid::new("video")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Scale");
                        egui::ComboBox::from_id_salt("scale")
                            .selected_text(if s.integer_scale {
                                "Whole numbers (sharpest)"
                            } else {
                                "Fit the window"
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut s.integer_scale, true, "Whole numbers (sharpest)");
                                ui.selectable_value(&mut s.integer_scale, false, "Fit the window");
                            });
                        ui.end_row();
                        ui.label("Filter");
                        egui::ComboBox::from_id_salt("filter")
                            .selected_text(if s.smooth { "Smooth" } else { "None (sharp pixels)" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut s.smooth, false, "None (sharp pixels)");
                                ui.selectable_value(&mut s.smooth, true, "Smooth");
                            });
                        ui.end_row();
                        ui.label("Colour correction");
                        ui.checkbox(&mut s.colour_correction, "as a GBA's own screen showed it");
                        ui.end_row();
                    });
            }
            Category::Network => {
                egui::Grid::new("network")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Relays");
                        egui::ComboBox::from_id_salt("relays")
                            .selected_text(match s.relays {
                                RelayChoice::Public => "n0's public relays",
                                RelayChoice::Own => "My own relays",
                                RelayChoice::Off => "Off (same network only)",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut s.relays, RelayChoice::Public, "n0's public relays");
                                ui.selectable_value(&mut s.relays, RelayChoice::Own, "My own relays");
                                ui.selectable_value(&mut s.relays, RelayChoice::Off, "Off (same network only)");
                            });
                        ui.end_row();
                    });
                if s.relays == RelayChoice::Own {
                    ui.weak("One iroh-relay URL per line:");
                    ui.add(egui::TextEdit::multiline(&mut s.own_relays).desired_rows(3));
                }
                ui.weak("Relays carry the connection when two Players' apps can't reach each other directly. A change is used from the next Lobby.");
            }
            Category::Data => {
                egui::Grid::new("data")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Data folder");
                        ui.monospace(facts.data_dir.display().to_string());
                        ui.end_row();
                        ui.label("Log");
                        if ui.button("Open the log folder").clicked() {
                            crate::open_folder(facts.data_dir);
                        }
                        ui.end_row();
                        ui.label("More detail in the log");
                        ui.checkbox(&mut s.verbose_log, "");
                        ui.end_row();
                    });
            }
            Category::Developer => {
                ui.label("Local Game Profiles");
                ui.horizontal(|ui| {
                    match &s.dev_profiles {
                        Some(dir) => ui.monospace(dir.display().to_string()),
                        None => ui.weak("none"),
                    };
                    if ui.button("Choose folder…").clicked()
                        && let Some(dir) = rfd::FileDialog::new().set_title("Local Game Profiles").pick_folder()
                    {
                        s.dev_profiles = Some(dir);
                    }
                    if s.dev_profiles.is_some() && ui.button("Clear").clicked() {
                        s.dev_profiles = None;
                    }
                });
                ui.weak("Solo Runs only: an unbundled profile is never used with friends. Used from the next start.");
            }
        }
    }
}
