//! Where the program, its models and the user's profiles live.

use std::path::{Path, PathBuf};

/// Beside the program, this file keeps profiles in the program's folder (the portable zip).
pub const PORTABLE_FILE: &str = "portable.txt";

const EXAMPLES: [(&str, &str); 2] = [
    ("example_paradise_lost.xml", include_str!("../profiles/example_paradise_lost.xml")),
    ("example_lincoln_memorial.xml", include_str!("../profiles/example_lincoln_memorial.xml")),
];

pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default()
}

pub fn models_dir() -> PathBuf {
    let beside = exe_dir().join("models");
    #[cfg(debug_assertions)]
    if !beside.is_dir() {
        return Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    }
    beside
}

pub fn profiles_dir() -> PathBuf {
    let exe = exe_dir();
    if exe.join(PORTABLE_FILE).is_file() {
        return exe.join("profiles");
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| exe.clone());
    base.join("AutoSplitter").join("profiles")
}

/// Creates the profile folder with the example profiles the first time.
/// Examples are never written again, so PBs saved in them stay.
pub fn ensure_profiles(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    for (name, xml) in EXAMPLES {
        std::fs::write(dir.join(name), xml)?;
    }
    Ok(())
}

/// Profile files in list order: names that start with a digit first, then by name;
/// a `paradise_lost.xml` goes to the top, as in earlier versions.
pub fn list_profiles(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")))
        .collect();
    let key = |p: &PathBuf| {
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        (name != "paradise_lost.xml", !name.starts_with(|c: char| c.is_ascii_digit()), name)
    };
    files.sort_by_key(key);
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_once_and_sorts() {
        let dir = std::env::temp_dir().join(format!("autosplitter-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ensure_profiles(&dir).unwrap();
        std::fs::remove_file(dir.join("example_lincoln_memorial.xml")).unwrap();
        ensure_profiles(&dir).unwrap();
        assert!(!dir.join("example_lincoln_memorial.xml").exists(), "a deleted example stays deleted");
        for name in ["b.xml", "1_setup.xml", "Paradise_Lost.xml", "notes.txt"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let names: Vec<_> = list_profiles(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Paradise_Lost.xml", "1_setup.xml", "b.xml", "example_paradise_lost.xml"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
