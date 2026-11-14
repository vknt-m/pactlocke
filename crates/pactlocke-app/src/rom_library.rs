//! The ROM library: every ROM the Player has opened, by path and SHA-1,
//! newest first. It never copies a ROM; it only remembers where each one
//! was, so the app can offer the right one for a Run (joining, resuming, a
//! New Run) without the Player browsing for it. Kept as `roms.json` beside
//! the settings.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many ROMs the library remembers.
const KEPT: usize = 64;

/// One ROM the Player has opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownRom {
    pub path: PathBuf,
    /// Lowercase hex.
    pub sha1: String,
    /// What it is, as the app last named it: its status when opened
    /// ("Pokémon FireRed (USA) 1.0: Supported"), or the Run's game when it
    /// was picked for a Run.
    pub game: String,
    /// Whether a Run can be played on it (it has a Game Profile, or plays as
    /// an Untested Romhack): New Run offers only these.
    #[serde(default)]
    pub runs: bool,
}

#[derive(Debug, Default)]
pub struct RomLibrary {
    roms: Vec<KnownRom>,
    /// Where it is kept; `None` keeps it in memory only.
    file: Option<PathBuf>,
}

impl RomLibrary {
    /// The library kept in `file`, or an empty one there.
    pub fn open(file: PathBuf) -> Self {
        let roms = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { roms, file: Some(file) }
    }

    /// Whether the library has been kept before (else it may take over the
    /// old recent-ROMs list).
    pub fn is_kept(&self) -> bool {
        self.file.as_ref().is_some_and(|file| file.exists())
    }

    /// Every ROM, newest first.
    pub fn roms(&self) -> &[KnownRom] {
        &self.roms
    }

    /// A ROM was opened: it goes to the top, with what it is now.
    pub fn remember(&mut self, path: &Path, sha1: &str, game: &str, runs: bool) {
        self.roms.retain(|rom| rom.path != path);
        self.roms.insert(
            0,
            KnownRom {
                path: path.to_path_buf(),
                sha1: sha1.to_owned(),
                game: game.to_owned(),
                runs,
            },
        );
        self.roms.truncate(KEPT);
        self.save();
    }

    /// The ROM with this SHA-1 where the library last saw it: each place it
    /// was, newest first, read by `read` and checked again by SHA-1. A place
    /// whose file is gone or changed is forgotten.
    pub fn locate(&mut self, sha1: &str, read: impl Fn(&Path) -> Option<Vec<u8>>) -> Option<(PathBuf, Vec<u8>)> {
        let candidates: Vec<PathBuf> = self
            .roms
            .iter()
            .filter(|rom| rom.sha1.eq_ignore_ascii_case(sha1))
            .map(|rom| rom.path.clone())
            .collect();
        let mut stale = Vec::new();
        let mut found = None;
        for path in candidates {
            match read(&path) {
                Some(rom) if pactlocke_game::sha1_hex(&rom).eq_ignore_ascii_case(sha1) => {
                    found = Some((path, rom));
                    break;
                }
                _ => stale.push(path),
            }
        }
        if !stale.is_empty() {
            self.roms.retain(|rom| !stale.contains(&rom.path));
            self.save();
        }
        found
    }

    fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let written = serde_json::to_string_pretty(&self.roms)
            .map_err(|e| e.to_string())
            .and_then(|text| std::fs::write(file, text).map_err(|e| e.to_string()));
        if let Err(e) = written {
            log::warn!("The ROM library couldn't be saved to {}: {e}", file.display());
        }
    }
}

/// Where the ROM library is kept.
pub fn path() -> PathBuf {
    crate::settings::path().with_file_name("roms.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRERED: &[u8] = b"a FireRed ROM";
    const EMERALD: &[u8] = b"an Emerald ROM";

    fn sha1(rom: &[u8]) -> String {
        pactlocke_game::sha1_hex(rom)
    }

    /// A folder of ROM files that exists only in the test.
    fn disk(files: &[(&str, &'static [u8])]) -> impl Fn(&Path) -> Option<Vec<u8>> + use<> {
        let files: Vec<(PathBuf, &'static [u8])> = files.iter().map(|(p, r)| (PathBuf::from(p), *r)).collect();
        move |path: &Path| files.iter().find(|(p, _)| p == path).map(|(_, rom)| rom.to_vec())
    }

    #[test]
    fn the_newest_rom_comes_first_and_a_path_is_kept_once() {
        let mut library = RomLibrary::default();
        library.remember(Path::new("fr.gba"), &sha1(FIRERED), "FireRed", true);
        library.remember(Path::new("em.gba"), &sha1(EMERALD), "Emerald", true);
        library.remember(Path::new("fr.gba"), &sha1(FIRERED), "FireRed", true);
        let paths: Vec<&Path> = library.roms().iter().map(|r| r.path.as_path()).collect();
        assert_eq!(paths, [Path::new("fr.gba"), Path::new("em.gba")]);
    }

    #[test]
    fn a_rom_is_found_by_its_sha1_where_it_was() {
        let mut library = RomLibrary::default();
        library.remember(Path::new("roms/fr.gba"), &sha1(FIRERED), "FireRed", true);
        library.remember(Path::new("roms/em.gba"), &sha1(EMERALD), "Emerald", true);
        let read = disk(&[("roms/fr.gba", FIRERED), ("roms/em.gba", EMERALD)]);
        assert_eq!(
            library.locate(&sha1(FIRERED), &read),
            Some((PathBuf::from("roms/fr.gba"), FIRERED.to_vec()))
        );
        assert_eq!(library.locate(&"00".repeat(20), &read), None, "a ROM never opened");
    }

    #[test]
    fn a_rom_moved_or_changed_since_is_forgotten_and_a_copy_elsewhere_still_found() {
        let mut library = RomLibrary::default();
        library.remember(Path::new("old/fr.gba"), &sha1(FIRERED), "FireRed", true);
        library.remember(Path::new("changed/fr.gba"), &sha1(FIRERED), "FireRed", true);
        library.remember(Path::new("gone/fr.gba"), &sha1(FIRERED), "FireRed", true);
        // The newest two places no longer hold it; the oldest still does.
        let read = disk(&[("old/fr.gba", FIRERED), ("changed/fr.gba", EMERALD)]);
        assert_eq!(
            library.locate(&sha1(FIRERED), &read).map(|(p, _)| p),
            Some(PathBuf::from("old/fr.gba"))
        );
        let paths: Vec<&Path> = library.roms().iter().map(|r| r.path.as_path()).collect();
        assert_eq!(paths, [Path::new("old/fr.gba")]);
    }

    #[test]
    fn the_library_is_kept_and_read_back() {
        let file = std::env::temp_dir().join(format!("pactlocke-roms-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let mut library = RomLibrary::open(file.clone());
        assert!(!library.is_kept());
        library.remember(Path::new("fr.gba"), &sha1(FIRERED), "FireRed", true);
        let again = RomLibrary::open(file.clone());
        assert!(again.is_kept());
        assert_eq!(again.roms(), library.roms());
        let _ = std::fs::remove_file(&file);
    }
}
