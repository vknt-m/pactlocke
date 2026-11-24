//! The Games tab's work away from the screen: looking through the Player's
//! game folders for ROMs, and taking each game's title screen once by
//! running its ROM unseen. Each runs on its own background thread, and the
//! title screens are kept by SHA-1 in the data folder, so each is taken once.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use pactlocke_emu::{CoreAdapter, Frame, Keys};
use pactlocke_game::{ProfileStore, RomIdentity, sha1_hex};

use crate::rom_library::KnownRom;

/// When the title screen is taken: START is tapped at these frames, which
/// skips the intros of the games tried (FireRed, Emerald, Radical Red, Gaia)
/// to their title screen, and the picture is taken at [`TITLE_FRAME`].
const START_TAPS: [u32; 3] = [120, 240, 360];
const TITLE_FRAME: u32 = 600;
/// A picture with less than this spread of brightness is a blank screen, not a title.
const BLANK_SPREAD: u8 = 24;

/// What the background thread sends back.
pub enum Done {
    /// A ROM found in a game folder that the library didn't know.
    Found(KnownRom),
    /// A game's title screen was taken and kept, or couldn't be.
    Title { sha1: String, kept: bool },
}

enum Job {
    /// Look through these folders, skipping the paths the library knows.
    Scan { folders: Vec<PathBuf>, known: Vec<PathBuf> },
    /// Take the title screen of the ROM at this path.
    Title { sha1: String, path: PathBuf },
}

/// The background threads, and what they have sent back.
pub struct Games {
    scans: mpsc::Sender<Job>,
    pictures: mpsc::Sender<Job>,
    done: mpsc::Receiver<Done>,
    titles: TitleScreens,
}

impl Games {
    /// Starts the threads, one looking through folders and one taking title
    /// screens, so a big folder doesn't hold up the pictures; `wake` is
    /// called whenever either sends something back.
    pub fn start(
        profiles: Option<ProfileStore>,
        titles: TitleScreens,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let wake = std::sync::Arc::new(wake);
        let (done_tx, done) = mpsc::channel();
        let (scans, scan_rx) = mpsc::channel::<Job>();
        let (pictures, picture_rx) = mpsc::channel::<Job>();
        for (name, jobs) in [("games: folders", scan_rx), ("games: title screens", picture_rx)] {
            let (done_tx, wake, profiles, kept) = (done_tx.clone(), wake.clone(), profiles.clone(), titles.clone());
            std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    for job in jobs {
                        match job {
                            Job::Scan { folders, known } => {
                                for path in folders.iter().flat_map(|f| crate::rom_library::roms_in(f)) {
                                    if known.contains(&path) {
                                        continue;
                                    }
                                    if let Some(found) = identify(&path, profiles.as_ref()) {
                                        let _ = done_tx.send(Done::Found(found));
                                        wake();
                                    }
                                }
                            }
                            Job::Title { sha1, path } => {
                                let taken = take_title(&path, &sha1);
                                let kept = match &taken {
                                    Some(frame) => kept.keep(&sha1, frame),
                                    // Tried once: it isn't tried again at the next launch.
                                    None => kept.keep_none(&sha1),
                                };
                                if let Err(e) = &kept {
                                    log::warn!("A title screen couldn't be kept: {e}");
                                }
                                let kept = kept.is_ok() && taken.is_some();
                                let _ = done_tx.send(Done::Title { sha1, kept });
                                wake();
                            }
                        }
                    }
                })
                .expect("the games threads start");
        }
        Self {
            scans,
            pictures,
            done,
            titles,
        }
    }

    /// Looks through the game folders for ROMs the library doesn't know.
    pub fn scan(&self, folders: Vec<PathBuf>, known: Vec<PathBuf>) {
        let _ = self.scans.send(Job::Scan { folders, known });
    }

    /// Takes a game's title screen and keeps it, or notes that it couldn't be.
    pub fn want_title(&self, sha1: &str, path: &Path) {
        let _ = self.pictures.send(Job::Title {
            sha1: sha1.to_owned(),
            path: path.to_path_buf(),
        });
    }

    /// What the threads have sent back since last asked.
    pub fn done(&self) -> Vec<Done> {
        self.done.try_iter().collect()
    }

    /// Where the title screens are kept.
    pub fn titles(&self) -> &TitleScreens {
        &self.titles
    }
}

/// Reads a ROM and says what it is, as the ROM library keeps it.
fn identify(path: &Path, profiles: Option<&ProfileStore>) -> Option<KnownRom> {
    let rom = std::fs::read(path).ok()?;
    let (game, runs) = match profiles {
        Some(store) => {
            let identity = store.identify(&rom);
            (
                crate::rom_report::status_text(&identity),
                matches!(identity, RomIdentity::Profiled { .. }),
            )
        }
        None => (crate::file_stem(path), false),
    };
    Some(KnownRom {
        path: path.to_path_buf(),
        sha1: sha1_hex(&rom),
        game,
        runs,
    })
}

/// Boots the ROM and takes its title screen; `None` when it can't be read,
/// isn't the ROM with this SHA-1, or shows a blank screen.
fn take_title(path: &Path, sha1: &str) -> Option<Frame> {
    let rom = std::fs::read(path).ok()?;
    if !sha1_hex(&rom).eq_ignore_ascii_case(sha1) {
        return None;
    }
    let mut core = pactlocke_mgba::MgbaCore::new();
    core.load_rom(&rom).ok()?;
    title_screen(&mut core)
}

/// Runs a freshly booted game to its title screen and takes it; `None` for
/// a blank screen.
pub fn title_screen(core: &mut impl CoreAdapter) -> Option<Frame> {
    for frame in 1..=TITLE_FRAME {
        let keys = if START_TAPS.contains(&frame) {
            Keys::START
        } else {
            Keys::NONE
        };
        core.run_frame(keys);
    }
    let frame = core.frame();
    (!is_blank(&frame)).then_some(frame)
}

/// Whether a picture is one flat colour, or nearly: a blank screen.
fn is_blank(frame: &Frame) -> bool {
    let brightness = frame
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| ((u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2])) / 3) as u8);
    let (low, high) = brightness.fold((u8::MAX, u8::MIN), |(lo, hi), b| (lo.min(b), hi.max(b)));
    high.saturating_sub(low) < BLANK_SPREAD
}

/// The title screens taken, kept as PNG files by SHA-1.
#[derive(Debug, Clone)]
pub struct TitleScreens {
    dir: PathBuf,
}

impl TitleScreens {
    /// Kept in `dir`, made when the first one is kept.
    pub fn in_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Beside the settings, in `titles/`.
    pub fn beside_settings() -> Self {
        Self::in_dir(crate::settings::path().with_file_name("titles"))
    }

    fn path(&self, sha1: &str) -> PathBuf {
        self.dir.join(format!("{}.png", sha1.to_ascii_lowercase()))
    }

    /// Where it is noted that a game has no title screen to take.
    fn none_path(&self, sha1: &str) -> PathBuf {
        self.dir.join(format!("{}.none", sha1.to_ascii_lowercase()))
    }

    /// Notes that this game's title screen couldn't be taken, so it isn't tried again.
    pub fn keep_none(&self, sha1: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(self.none_path(sha1), b"").map_err(|e| e.to_string())
    }

    /// Whether this game's title screen was tried, kept or not.
    pub fn tried(&self, sha1: &str) -> bool {
        self.path(sha1).exists() || self.none_path(sha1).exists()
    }

    /// Keeps a title screen.
    pub fn keep(&self, sha1: &str, frame: &Frame) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let file = std::fs::File::create(self.path(sha1)).map_err(|e| e.to_string())?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), frame.width, frame.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&frame.rgba))
            .map_err(|e| e.to_string())
    }

    /// A kept title screen, if there is one.
    pub fn load(&self, sha1: &str) -> Option<Frame> {
        let file = std::fs::File::open(self.path(sha1)).ok()?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().ok()?;
        let mut rgba = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut rgba).ok()?;
        if info.color_type != png::ColorType::Rgba {
            return None;
        }
        rgba.truncate(info.buffer_size());
        Some(Frame {
            width: info.width,
            height: info.height,
            rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(pixels: &[[u8; 3]]) -> Frame {
        Frame {
            width: pixels.len() as u32,
            height: 1,
            rgba: pixels.iter().flat_map(|[r, g, b]| [*r, *g, *b, 255]).collect(),
        }
    }

    #[test]
    fn a_flat_screen_is_blank_and_a_title_screen_is_not() {
        assert!(is_blank(&frame(&[[0, 0, 0]; 8])), "black");
        assert!(is_blank(&frame(&[[255, 255, 255]; 8])), "white, between intro scenes");
        assert!(
            is_blank(&frame(&[[30, 40, 90], [34, 44, 92], [28, 38, 88]])),
            "a flat colour, dithered"
        );
        assert!(
            !is_blank(&frame(&[[20, 60, 160], [250, 220, 40], [200, 40, 20]])),
            "a logo on a background"
        );
    }

    #[test]
    fn a_title_screen_is_kept_by_sha1_and_read_back() {
        let dir = std::env::temp_dir().join(format!("pactlocke-titles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let titles = TitleScreens::in_dir(dir.clone());
        let sha1 = "AB".repeat(20);
        assert_eq!(titles.load(&sha1), None, "none taken yet");
        assert!(!titles.tried(&sha1));
        let picture = frame(&[[20, 60, 160], [250, 220, 40], [200, 40, 20]]);
        titles.keep(&sha1, &picture).unwrap();
        assert_eq!(titles.load(&sha1.to_ascii_lowercase()), Some(picture));
        assert!(titles.tried(&sha1));

        let blank = "CD".repeat(20);
        titles.keep_none(&blank).unwrap();
        assert!(titles.tried(&blank), "a game with no title screen is tried once");
        assert_eq!(titles.load(&blank), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
