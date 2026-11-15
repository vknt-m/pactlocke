//! A randomized Run's games: every Player's, made by this app from its own
//! clean ROM with UPR FVX (downloaded the first time a Run needs it), and
//! kept in the data folder so a resume doesn't randomize again.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui;
use pactlocke_randomizer::{Installed, SeedMode, seed_for, sha1_hex};
use pactlocke_run::{PlayerKey, RandomizerChoice};

fn folder() -> PathBuf {
    crate::data_dir::data_root()
}

/// FVX, downloaded and unpacked if this is the first randomized Run.
fn fvx(progress: &dyn Fn(String)) -> Result<Installed, String> {
    let data = folder();
    if let Some(done) = pactlocke_randomizer::installed(&data) {
        return Ok(done);
    }
    let asset = pactlocke_randomizer::asset_for_this_machine()
        .ok_or_else(|| pactlocke_randomizer::InstallError::NoBuild.to_string())?;
    progress(format!(
        "Downloading the randomizer, UPR FVX {} ({} MB)…",
        pactlocke_randomizer::FVX_VERSION,
        asset.size / 1_000_000
    ));
    pactlocke_randomizer::install(&data, asset, pactlocke_randomizer::download).map_err(|e| e.to_string())
}

fn randomized_dir() -> PathBuf {
    folder().join("randomized")
}

/// Where the game this FVX version makes for this base ROM, settings and seed is kept.
fn kept_at(base_sha1: &str, settings: &str, seed: i64) -> PathBuf {
    let version = pactlocke_randomizer::FVX_VERSION;
    let name = sha1_hex(format!("{version}\n{base_sha1}\n{settings}\n{seed}").as_bytes());
    randomized_dir().join(format!("{name}.gba"))
}

/// Every Player's game, in `players` order, from `base`: kept ones read
/// back, the rest made now. Blocks while FVX runs (about a second a game).
pub fn make(
    choice: &RandomizerChoice,
    base: &[u8],
    players: &[PlayerKey],
    progress: &dyn Fn(String),
) -> Result<Vec<Arc<[u8]>>, String> {
    let base_sha1 = sha1_hex(base);
    let mode = if choice.shared_seed {
        SeedMode::Shared
    } else {
        SeedMode::PerPlayer
    };
    let mut games = Vec::with_capacity(players.len());
    let mut installed = None;
    for (i, player) in players.iter().enumerate() {
        let seed = seed_for(&choice.secret, mode, player);
        let path = kept_at(&base_sha1, &choice.settings, seed);
        if let Ok(kept) = std::fs::read(&path) {
            games.push(kept.into());
            continue;
        }
        if installed.is_none() {
            installed = Some(fvx(progress)?);
        }
        let fvx = installed.as_ref().expect("installed above");
        progress(format!("Randomizing game {} of {}…", i + 1, players.len()));
        let work = randomized_dir().join("work");
        let made =
            pactlocke_randomizer::randomize(fvx, base, &choice.settings, seed, &work).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        // Written whole and then renamed, so a crash never leaves half a game to be read back.
        let partial = path.with_extension("partial");
        std::fs::write(&partial, &made.rom)
            .and_then(|()| std::fs::rename(&partial, &path))
            .map_err(|e| format!("The randomized game couldn't be kept: {e}"))?;
        games.push(made.rom.into());
    }
    Ok(games)
}

/// What setup shows of this app's randomized games.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Working(String),
    Failed(String),
    /// Every Player's game's SHA-1, in setup's Player order.
    Done(Vec<String>),
}

/// Making every Player's game in the background while setup goes on: made
/// again whenever the Host's choice or the Players change.
#[derive(Default)]
pub struct Maker {
    /// The choice, the Players, and the SHA-1 of the ROM they are made from.
    wanted: Option<(RandomizerChoice, Vec<PlayerKey>, Option<String>)>,
    status: Option<Arc<Mutex<Status>>>,
}

impl Maker {
    /// Starts making the games for `choice` and `players` from the base ROM
    /// with SHA-1 `base` (`None` until one is chosen), unless those games
    /// are made or being made already. Only then is `reader` called, for
    /// what reads the base ROM in the background. A `choice` of `None` (not
    /// randomized) stops wanting any.
    pub fn want<R>(
        &mut self,
        choice: Option<&RandomizerChoice>,
        players: &[PlayerKey],
        base: Option<&str>,
        reader: impl FnOnce() -> R,
    ) where
        R: FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
    {
        let wanted = choice.map(|c| (c.clone(), players.to_vec(), base.map(str::to_owned)));
        if wanted == self.wanted {
            return;
        }
        self.wanted = wanted;
        self.status = None;
        let Some((choice, players, _)) = self.wanted.clone() else {
            return;
        };
        let status = Arc::new(Mutex::new(Status::Working("Getting the randomizer ready…".to_owned())));
        self.status = Some(status.clone());
        let read_base = reader();
        std::thread::spawn(move || {
            let set = |s: Status| *status.lock().unwrap_or_else(|e| e.into_inner()) = s;
            let progress = |line: String| set(Status::Working(line));
            let result = read_base().and_then(|base| make(&choice, &base, &players, &progress));
            set(match result {
                Ok(games) => Status::Done(games.iter().map(|g| sha1_hex(g)).collect()),
                Err(why) => Status::Failed(why),
            });
        });
    }

    fn status(&self) -> Option<Status> {
        self.status
            .as_ref()
            .map(|s| s.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    /// The made games' SHA-1s, once made.
    pub fn made(&self) -> Vec<String> {
        match self.status() {
            Some(Status::Done(sha1s)) => sha1s,
            _ => Vec::new(),
        }
    }

    /// One line for setup, while there is something to say; after a
    /// failure, a button to try again.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        match self.status() {
            Some(Status::Working(line)) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(line);
                });
            }
            Some(Status::Failed(why)) => {
                ui.colored_label(ui.visuals().error_fg_color, why);
                if ui.button("Try again").clicked() {
                    // Wanted again on the next frame, so it starts over.
                    self.wanted = None;
                    self.status = None;
                }
            }
            Some(Status::Done(_)) => {
                ui.weak("Every Player's randomized game is made.");
            }
            None => {}
        }
    }
}
