//! The app's settings: the Player's name, controls, sound, video, network,
//! logging and the developer setting, plus a few things the app remembers
//! (the side panel's width, recent ROMs). They are kept as `settings.json`
//! beside the Runs folder, and read again at every start. A file that can't
//! be read gives the defaults; a value that can't be read gives its default.

use std::path::PathBuf;

use eframe::egui;
use pactlocke_emu::Keys;
use serde::{Deserialize, Serialize};

/// The GBA's buttons in the order Settings ▸ Controls lists them.
pub const BUTTONS: [(Keys, &str); 10] = [
    (Keys::A, "A"),
    (Keys::B, "B"),
    (Keys::L, "L"),
    (Keys::R, "R"),
    (Keys::START, "Start"),
    (Keys::SELECT, "Select"),
    (Keys::UP, "Up"),
    (Keys::DOWN, "Down"),
    (Keys::LEFT, "Left"),
    (Keys::RIGHT, "Right"),
];

/// How many recent ROMs File ▸ Recent ROMs keeps.
const RECENT_KEPT: usize = 8;
/// The narrowest and widest the side panel can be dragged, in points.
pub const PANEL_WIDTHS: (f32, f32) = (280.0, 720.0);
/// The longest name a Player can give.
pub const NAME_LIMIT: usize = 16;

/// Which relays the app uses to reach other Players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RelayChoice {
    /// n0's public relays.
    #[default]
    Public,
    /// The relays listed in `own_relays`.
    Own,
    /// None: only Players on the same network.
    Off,
}

/// Everything Settings holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// What other Players see; `None` until the first-launch prompt.
    pub name: Option<String>,
    /// The speed a game starts at, 1 to 4.
    pub default_speed: u8,
    /// Solo only: the game pauses while the window isn't focused.
    pub pause_unfocused: bool,
    /// Each GBA button's key, by button name, as egui names keys.
    pub keys: Vec<(String, String)>,
    /// Held, the game runs at 4x.
    pub fast_forward_key: String,
    /// Pauses and unpauses the game.
    pub pause_key: String,
    /// Read a gamepad as well as the keyboard.
    pub gamepad: bool,
    /// 0 to 1.
    pub volume: f32,
    pub mute_fast_forward: bool,
    /// Play the sound of the game you Watch.
    pub watch_sound: bool,
    /// The output device by name; `None` for the system's default.
    pub output_device: Option<String>,
    /// Scale the game by whole numbers only (the sharpest picture).
    pub integer_scale: bool,
    /// Smooth the picture instead of keeping each pixel sharp.
    pub smooth: bool,
    /// Correct the colours for a modern screen, as a GBA's screen looked.
    pub colour_correction: bool,
    pub relays: RelayChoice,
    /// One relay URL per line, for [`RelayChoice::Own`].
    pub own_relays: String,
    /// Write Debug lines to the log.
    pub verbose_log: bool,
    /// A folder of local Game Profiles, for solo Runs only.
    pub dev_profiles: Option<PathBuf>,
    /// The side panel's width in points, as last dragged.
    pub panel_width: f32,
    /// ROMs opened, newest first.
    pub recent_roms: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: None,
            default_speed: 1,
            pause_unfocused: false,
            keys: default_keys(),
            fast_forward_key: egui::Key::Tab.name().to_owned(),
            pause_key: egui::Key::P.name().to_owned(),
            gamepad: true,
            volume: 0.8,
            mute_fast_forward: true,
            watch_sound: true,
            output_device: None,
            integer_scale: true,
            smooth: false,
            colour_correction: false,
            relays: RelayChoice::Public,
            own_relays: String::new(),
            verbose_log: false,
            dev_profiles: None,
            panel_width: 360.0,
            recent_roms: Vec::new(),
        }
    }
}

/// mGBA's default keys.
fn default_keys() -> Vec<(String, String)> {
    use egui::Key;
    let keys = [
        Key::X,
        Key::Z,
        Key::A,
        Key::S,
        Key::Enter,
        Key::Backspace,
        Key::ArrowUp,
        Key::ArrowDown,
        Key::ArrowLeft,
        Key::ArrowRight,
    ];
    BUTTONS
        .iter()
        .zip(keys)
        .map(|((_, button), key)| ((*button).to_owned(), key.name().to_owned()))
        .collect()
}

/// Where the settings are kept.
pub fn path() -> PathBuf {
    crate::data_dir::identity_file().with_file_name("settings.json")
}

impl Settings {
    /// The settings as kept, or the defaults.
    pub fn load() -> Self {
        let mut settings: Settings = std::fs::read_to_string(path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        // A button missing from the file keeps its default key.
        for (button, key) in default_keys() {
            if !settings.keys.iter().any(|(b, _)| *b == button) {
                settings.keys.push((button, key));
            }
        }
        settings.default_speed = settings.default_speed.clamp(1, 4);
        settings.volume = settings.volume.clamp(0.0, 1.0);
        settings.panel_width = settings.panel_width.clamp(PANEL_WIDTHS.0, PANEL_WIDTHS.1);
        settings
    }

    /// Keeps the settings; a failure is logged, never shown as an error.
    pub fn save(&self) {
        let written = serde_json::to_string_pretty(self)
            .map_err(|e| e.to_string())
            .and_then(|text| std::fs::write(path(), text).map_err(|e| e.to_string()));
        if let Err(e) = written {
            log::warn!("Settings couldn't be saved to {}: {e}", path().display());
        }
    }

    /// The key a GBA button is on.
    pub fn key_of(&self, button: &str) -> Option<egui::Key> {
        let (_, key) = self.keys.iter().find(|(b, _)| b == button)?;
        egui::Key::from_name(key)
    }

    /// Puts a GBA button on another key.
    pub fn bind(&mut self, button: &str, key: egui::Key) {
        match self.keys.iter_mut().find(|(b, _)| b == button) {
            Some((_, k)) => *k = key.name().to_owned(),
            None => self.keys.push((button.to_owned(), key.name().to_owned())),
        }
    }

    /// The GBA buttons held on the keyboard now.
    pub fn held(&self, input: &egui::InputState) -> Keys {
        BUTTONS
            .iter()
            .filter(|(_, name)| self.key_of(name).is_some_and(|key| input.key_down(key)))
            .fold(Keys::NONE, |held, (gba, _)| held | *gba)
    }

    /// The relays to reach other Players through.
    pub fn relay_choice(&self) -> pactlocke_net::Relays {
        match self.relays {
            RelayChoice::Public => pactlocke_net::Relays::N0,
            RelayChoice::Own => pactlocke_net::Relays::parse(&self.own_relays),
            RelayChoice::Off => pactlocke_net::Relays::Off,
        }
    }

    /// A ROM was opened: it goes to the top of the recent list.
    pub fn opened(&mut self, rom: &std::path::Path) {
        self.recent_roms.retain(|p| p != rom);
        self.recent_roms.insert(0, rom.to_path_buf());
        self.recent_roms.truncate(RECENT_KEPT);
        self.save();
    }

    /// The name other Players see, or a stand-in before the first launch.
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("Player")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settings_file_round_trips_and_missing_values_keep_their_defaults() {
        let mut settings = Settings {
            name: Some("Red".into()),
            volume: 0.3,
            ..Settings::default()
        };
        settings.bind("A", egui::Key::K);
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), settings);
        let partial: Settings = serde_json::from_str(r#"{"name":"Blue"}"#).unwrap();
        assert_eq!(partial.name.as_deref(), Some("Blue"));
        assert_eq!(partial.key_of("Start"), Some(egui::Key::Enter));
        assert_eq!(settings.key_of("A"), Some(egui::Key::K));
    }
}
