//! Pieces of the side panel during a Run: the must-nickname
//! prompt, and how long ago something happened.

use eframe::egui;
use pactlocke_run::{NicknameWanted, RunCommand};

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "just now".to_owned(),
        1..=59 => format!("{minutes} min ago"),
        60..=2879 => format!("{} h ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}

/// Must nickname: asks for a name for a Pokémon left with its species name.
pub fn nickname_prompt(ui: &mut egui::Ui, wanted: &NicknameWanted) -> Option<RunCommand> {
    let mut command = None;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong(format!("Name your {}", wanted.species_name));
        ui.label("Must nickname: up to 10 characters, not the species name.");
        let id = ui.make_persistent_id(("nickname", wanted.pid));
        let mut name = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
        let mut send = false;
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .char_limit(10)
                    .desired_width(140.0),
            );
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                send = true;
            }
            send |= ui.button("Name it").clicked();
        });
        if send && !name.trim().is_empty() {
            command = Some(RunCommand::Nickname {
                pid: wanted.pid,
                name: name.trim().to_owned(),
            });
            name.clear();
        }
        ui.data_mut(|d| d.insert_temp(id, name));
    });
    command
}
