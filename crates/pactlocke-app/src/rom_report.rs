//! What a loaded ROM is, and which rules its Game Profile can enforce.

use eframe::egui;
use pactlocke_game::{Capability, GameProfile, RomIdentity, SupportStatus, UNTESTED_PREFIX, Unprofiled};

/// Whether this profile was made for an Untested Romhack, from its game's.
pub fn untested_romhack(profile: &GameProfile) -> bool {
    profile.id.starts_with(UNTESTED_PREFIX)
}

/// "Pokémon FireRed (USA) 1.0: Supported", or the ROM's other status, in a few words.
pub fn status_text(identity: &RomIdentity) -> String {
    match identity {
        RomIdentity::Profiled { profile, .. } => {
            let status = match profile.status {
                SupportStatus::Supported => "Supported",
                SupportStatus::Untested => "Untested",
            };
            format!("{}: {status}", profile.name)
        }
        RomIdentity::Unprofiled {
            verdict: Unprofiled::ProfileComing(hack),
            ..
        } => format!("{} {}: Game Profile coming", hack.name, hack.version),
        RomIdentity::Unprofiled {
            verdict: Unprofiled::NotSupported,
            ..
        } => "Not supported".to_owned(),
    }
}

/// The ROM's status, for the toolbar.
pub fn status_label(ui: &mut egui::Ui, identity: &RomIdentity) {
    let text = status_text(identity);
    match identity {
        RomIdentity::Unprofiled {
            verdict: Unprofiled::NotSupported,
            ..
        } => {
            ui.colored_label(ui.visuals().warn_fg_color, text);
        }
        _ => {
            ui.label(text);
        }
    }
}

/// Each game-dependent rule: enforced, the game's own option, or greyed out and why.
pub fn rules_panel(ui: &mut egui::Ui, identity: &RomIdentity) {
    ui.heading("Rules on this ROM");
    ui.add_space(4.0);
    let capabilities = match identity {
        RomIdentity::Profiled { capabilities, profile } => {
            if untested_romhack(profile) {
                let base = profile.parent.as_deref().unwrap_or("its game");
                ui.label(format!(
                    "No Game Profile matches this exact ROM, but it keeps the layout of the game it patches \
                     ({base}), so it plays as an Untested Romhack on that game's profile. Rules its changes \
                     broke are off, Gifts are only seen once they arrive, and nothing here was tested on \
                     this Romhack."
                ));
                ui.add_space(8.0);
            }
            capabilities
        }
        RomIdentity::Unprofiled { sha1, verdict } => {
            let text = match verdict {
                Unprofiled::ProfileComing(hack) => format!(
                    "{} {} by {} is a tested Romhack. Its Game Profile comes in a later update: until then \
                     it can be played, but not in a Run.",
                    hack.name, hack.version, hack.author
                ),
                Unprofiled::NotSupported => "Pactlocke doesn't know this game, or this Romhack moved the \
                                             game's tables, so no rules can be applied to it."
                    .to_owned(),
            };
            ui.label(text);
            ui.add_space(8.0);
            ui.weak(format!("SHA-1 {sha1}"));
            return;
        }
    };

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (rule, capability) in capabilities.iter() {
            ui.horizontal_wrapped(|ui| match capability {
                Capability::Enforceable => {
                    ui.strong(rule.name());
                    ui.label("enforced by Pactlocke");
                }
                Capability::NativeOption(native) => {
                    ui.strong(rule.name());
                    ui.label(format!(
                        "the game's own option: Pactlocke sets {} to {} ({})",
                        native.name, native.value, native.menu
                    ));
                }
                Capability::GreyedOut { reason } => {
                    ui.weak(rule.name());
                    ui.weak(format!("unavailable: {reason}"));
                }
            });
            ui.add_space(2.0);
        }
    });
}
