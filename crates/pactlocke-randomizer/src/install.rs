//! Getting UPR FVX: the official release zip for this machine, downloaded
//! from GitHub only when a Player asks for it, checked against its pinned
//! SHA-256 and unpacked into the app's data folder. Each zip carries its own
//! small Java runtime, so nothing else is installed.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{FVX_VERSION, hex};

/// One pinned release zip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    pub file: &'static str,
    /// As GitHub publishes it for the release.
    pub sha256: &'static str,
    pub size: u64,
}

impl Asset {
    pub fn url(&self) -> String {
        format!(
            "https://github.com/upr-fvx/universal-pokemon-randomizer-fvx/releases/download/vFVX{FVX_VERSION}/{}",
            self.file
        )
    }
}

const WINDOWS: Asset = Asset {
    file: "UPR_FVX-v1_6_3-Windows.zip",
    sha256: "43bf2badbaa06b35bdab10ef438a1b2db4a78dd66d2683ec3c60e1f18edc60e9",
    size: 41_474_900,
};
const LINUX_X86: Asset = Asset {
    file: "UPR_FVX-v1_6_3-Linux_x86.zip",
    sha256: "2a84f84d68be25dab1584000ab68587c435b330ab2225ea18ffe330e4909587e",
    size: 51_098_313,
};
const LINUX_ARM: Asset = Asset {
    file: "UPR_FVX-v1_6_3-Linux_ARM.zip",
    sha256: "66d240c25983a7e3b25cfa1f2d6969ec72d2f731c8d6d55316e17f2161c91da2",
    size: 49_632_566,
};
const MAC_X86: Asset = Asset {
    file: "UPR_FVX-v1_6_3-Mac_x86.zip",
    sha256: "d7b8eab369a2c6ed10e0ca9fef7c06c1860b22187ebbb1a2d4c13faa2e522a0f",
    size: 43_327_064,
};
const MAC_ARM: Asset = Asset {
    file: "UPR_FVX-v1_6_3-Mac_ARM.zip",
    sha256: "621de58bc014997cf937e8e9f790f0aff72d2909f6c9c7d4c758ecf679393e98",
    size: 41_365_492,
};

/// The release zip for this operating system and processor; `None` where FVX ships none.
pub fn asset_for_this_machine() -> Option<Asset> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some(WINDOWS),
        ("linux", "x86_64") => Some(LINUX_X86),
        ("linux", "aarch64") => Some(LINUX_ARM),
        ("macos", "x86_64") => Some(MAC_X86),
        ("macos", "aarch64") => Some(MAC_ARM),
        _ => None,
    }
}

/// An unpacked, checked FVX: its jar and the Java runtime that came with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub dir: PathBuf,
    pub java: PathBuf,
    pub jar: PathBuf,
}

/// Why FVX couldn't be got.
#[derive(Debug)]
pub enum InstallError {
    /// FVX ships no build for this operating system and processor.
    NoBuild,
    Download(String),
    /// The download isn't the pinned release.
    WrongFile {
        sha256: String,
    },
    /// The zip didn't hold a jar and a Java runtime.
    Incomplete,
    Io(std::io::Error),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::NoBuild => write!(f, "the randomizer has no build for this computer"),
            InstallError::Download(why) => write!(f, "the randomizer couldn't be downloaded: {why}"),
            InstallError::WrongFile { sha256 } => write!(
                f,
                "the randomizer download isn't the FVX {FVX_VERSION} release (SHA-256 {sha256}), so it wasn't used"
            ),
            InstallError::Incomplete => write!(f, "the randomizer download is missing its program or Java"),
            InstallError::Io(e) => write!(f, "the randomizer couldn't be saved: {e}"),
        }
    }
}

impl From<std::io::Error> for InstallError {
    fn from(e: std::io::Error) -> Self {
        InstallError::Io(e)
    }
}

/// Written last, so a half-unpacked folder is never taken for an install.
const DONE: &str = ".pactlocke-checked";

fn folder(data_dir: &Path) -> PathBuf {
    data_dir.join("randomizer").join(format!("fvx-{FVX_VERSION}"))
}

/// FVX, if it was installed under `data_dir` before.
pub fn installed(data_dir: &Path) -> Option<Installed> {
    let dir = folder(data_dir);
    dir.join(DONE).is_file().then(|| locate(&dir)).flatten()
}

/// Installs FVX under `data_dir` from `zip`, the bytes `fetch` returns for
/// the asset's URL, unless it already is. Only call this when a Player asked
/// for a randomized Run.
pub fn install(
    data_dir: &Path,
    asset: Asset,
    fetch: impl FnOnce(&str) -> Result<Vec<u8>, String>,
) -> Result<Installed, InstallError> {
    if let Some(done) = installed(data_dir) {
        return Ok(done);
    }
    let zip = fetch(&asset.url()).map_err(InstallError::Download)?;
    let sha256 = hex(&Sha256::digest(&zip));
    if sha256 != asset.sha256 {
        return Err(InstallError::WrongFile { sha256 });
    }
    let dir = folder(data_dir);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    unpack(&zip, &dir)?;
    let found = locate(&dir).ok_or(InstallError::Incomplete)?;
    std::fs::write(dir.join(DONE), asset.sha256)?;
    Ok(found)
}

/// Downloads `url` over HTTPS.
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let response = reqwest::blocking::get(url).map_err(|e| e.to_string())?;
    let mut response = response.error_for_status().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn unpack(zip: &[u8], dir: &Path) -> Result<(), InstallError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|_| InstallError::Incomplete)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|_| InstallError::Incomplete)?;
        // `enclosed_name` refuses paths that would leave the folder.
        let Some(relative) = entry.enclosed_name() else {
            return Err(InstallError::Incomplete);
        };
        let path = dir.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&path)?;
        std::io::copy(&mut entry, &mut out)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}

/// The jar and the bundled `bin/java` anywhere under `dir`, since each
/// platform's zip lays its runtime out its own way.
fn locate(dir: &Path) -> Option<Installed> {
    let java_name = if cfg!(windows) { "java.exe" } else { "java" };
    let (mut java, mut jar) = (None, None);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(here) = stack.pop() {
        for entry in std::fs::read_dir(&here).ok()?.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                stack.push(path);
            } else if name == "UPR-FVX.jar" {
                jar = Some(path);
            } else if name == java_name && here.file_name().is_some_and(|d| d == "bin") {
                java = Some(path);
            }
        }
    }
    Some(Installed {
        dir: jar.as_ref()?.parent()?.to_path_buf(),
        java: java?,
        jar: jar?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in files {
            out.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(bytes).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    fn pinned_to(zip: &[u8]) -> Asset {
        let sha256: &'static str = Box::leak(hex(&Sha256::digest(zip)).into_boxed_str());
        Asset {
            sha256,
            size: zip.len() as u64,
            ..WINDOWS
        }
    }

    const JAVA: &str = if cfg!(windows) {
        "java/bin/java.exe"
    } else {
        "java/bin/java"
    };

    #[test]
    fn installs_once_and_finds_the_jar_and_java() {
        let data = tempfile::tempdir().unwrap();
        let zip = zip_of(&[("UPR-FVX.jar", b"jar"), (JAVA, b"java")]);
        let asset = pinned_to(&zip);
        let got = install(data.path(), asset, |url| {
            assert!(url.ends_with(asset.file));
            Ok(zip.clone())
        })
        .unwrap();
        assert!(got.jar.ends_with("UPR-FVX.jar") && got.java.ends_with(JAVA));
        assert_eq!(installed(data.path()), Some(got.clone()));
        let again = install(data.path(), asset, |_| panic!("no second download")).unwrap();
        assert_eq!(again, got);
    }

    #[test]
    fn a_download_that_isnt_the_pinned_release_is_refused() {
        let data = tempfile::tempdir().unwrap();
        let zip = zip_of(&[("UPR-FVX.jar", b"jar"), (JAVA, b"java")]);
        let got = install(data.path(), WINDOWS, |_| Ok(zip.clone()));
        assert!(matches!(got, Err(InstallError::WrongFile { .. })));
        assert_eq!(installed(data.path()), None);
    }

    #[test]
    fn a_zip_without_java_isnt_an_install() {
        let data = tempfile::tempdir().unwrap();
        let zip = zip_of(&[("UPR-FVX.jar", b"jar")]);
        let got = install(data.path(), pinned_to(&zip), |_| Ok(zip.clone()));
        assert!(matches!(got, Err(InstallError::Incomplete)));
        assert_eq!(installed(data.path()), None);
    }

    #[test]
    fn every_supported_machine_has_a_pinned_zip() {
        for asset in [WINDOWS, LINUX_X86, LINUX_ARM, MAC_X86, MAC_ARM] {
            assert_eq!(asset.sha256.len(), 64);
            assert!(asset.url().ends_with(asset.file) && asset.file.contains("1_6_3"));
        }
    }
}
