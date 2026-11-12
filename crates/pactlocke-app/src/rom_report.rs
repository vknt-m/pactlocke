//! What a loaded ROM is, and which rules its Game Profile can enforce.

use eframe::egui;
use pactlocke_game::{Capability, RomIdentity, SupportStatus, Unprofiled};

/// "Pokémon FireRed (USA) 1.0: Supported", for the toolbar.
pub fn status_label(ui: &mut egui::Ui, identity: &RomIdentity) {
    match identity {
        RomIdentity::Profiled { profile, .. } => {
            let status = match profile.status {
                SupportStatus::Supported => "Supported",
                SupportStatus::Untested => "Untested",
            };
            ui.label(format!("{}: {status}", profile.name));
        }
        RomIdentity::Unprofiled {
            verdict: Unprofiled::Untested,
            ..
        } => {
            ui.label("No Game Profile: Untested");
        }
        RomIdentity::Unprofiled {
            verdict: Unprofiled::NotSupported,
            ..
        } => {
            ui.colored_label(ui.visuals().warn_fg_color, "Not supported");
        }
    }
}

/// Each game-dependent rule: enforced, the game's own option, or greyed out and why.
pub fn rules_panel(ui: &mut egui::Ui, identity: &RomIdentity) {
    ui.heading("Rules on this ROM");
    ui.add_space(4.0);
    let capabilities = match identity {
        RomIdentity::Profiled { capabilities, .. } => capabilities,
        RomIdentity::Unprofiled { sha1, verdict } => {
            let text = match verdict {
                Unprofiled::Untested => {
                    "This looks like a supported game, but no Game Profile matches this exact ROM. \
                     A Run on it would be Untested: rules are detected and penalised where possible, \
                     and anything the app can't stop is shown as a Violation."
                }
                Unprofiled::NotSupported => "Pactlocke doesn't know this game, so no rules can be applied to it.",
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
                Capability::NativeOption { name, menu, value } => {
                    ui.strong(rule.name());
                    ui.label(format!("the game's own option: set {name} to {value} ({menu})"));
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
