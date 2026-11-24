//! New versions from GitHub Releases. A download is used only when the minisign-signed
//! SHA256SUMS of its release lists it with the same checksum.

use sha2::Digest;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const RELEASES: &str = "https://github.com/KimP0ssible-no/AutoSplitter/releases";
const LATEST_API: &str = "https://api.github.com/repos/KimP0ssible-no/AutoSplitter/releases/latest";

/// The public half of the key that signs every release's SHA256SUMS.
pub const RELEASE_KEY: &str = include_str!("../packaging/minisign.pub");

/// `v1.2.3` or `1.2.3`; anything else (pre-releases included) is not a version.
pub fn parse_version(tag: &str) -> Option<(u32, u32, u32)> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.').map(|p| p.parse().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

fn agent(max_redirects: u32) -> ureq::Agent {
    let config = ureq::Agent::config_builder();
    #[cfg(windows)]
    let config = {
        use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
        config.tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
    };
    config
        .max_redirects(max_redirects)
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(concat!("AutoSplitter/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// The newest release: its tag and release notes.
pub fn latest() -> Result<(String, String), String> {
    let response = agent(5).get(LATEST_API).header("Accept", "application/vnd.github+json").call().map_err(|e| e.to_string())?;
    let body = response.into_body().into_with_config().limit(4 << 20).read_to_vec().map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let tag = json["tag_name"].as_str().ok_or("no release found")?.to_string();
    Ok((tag, json["body"].as_str().unwrap_or_default().to_string()))
}

/// Where downloads wait until they are installed; emptied at the next start.
fn download_dir() -> PathBuf {
    std::env::temp_dir().join("autosplitter-update")
}

/// Download `asset` of release `tag` into `dir`, but only if SHA256SUMS is signed by `key`
/// and lists the file with the same checksum.
pub fn download_verified(releases: &str, tag: &str, asset: &str, dir: &Path, key: &str) -> Result<PathBuf, String> {
    let base = format!("{releases}/download/{tag}");
    let agent = agent(10);
    let small = |name: &str| -> Result<Vec<u8>, String> {
        let response = agent.get(format!("{base}/{name}")).call().map_err(|e| format!("cannot download {name}: {e}"))?;
        response.into_body().into_with_config().limit(1 << 20).read_to_vec().map_err(|e| e.to_string())
    };
    let sums = small("SHA256SUMS")?;
    let signature = String::from_utf8(small("SHA256SUMS.minisig")?).map_err(|e| e.to_string())?;
    let signed = minisign_verify::PublicKey::decode(key).and_then(|key| key.verify(&sums, &minisign_verify::Signature::decode(&signature)?, false));
    if signed.is_err() {
        return Err("SHA256SUMS is not signed with the AutoSplitter release key".into());
    }
    let sums = String::from_utf8(sums).map_err(|e| e.to_string())?;
    let expected = sums
        .lines()
        .find_map(|line| line.split_once("  ").filter(|(_, name)| *name == asset).map(|(hash, _)| hash.to_ascii_lowercase()))
        .ok_or_else(|| format!("{asset} is not in SHA256SUMS"))?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(asset);
    save_checked(&agent, &format!("{base}/{asset}"), &path, &expected).map_err(|e| format!("cannot download {asset}: {e}"))?;
    Ok(path)
}

/// Download `url` to `path`; it is kept only if its SHA-256 is `expected` (hex).
fn save_checked(agent: &ureq::Agent, url: &str, path: &Path, expected: &str) -> Result<(), String> {
    // The agent's 15 seconds also cover reading the body: too short for tens of MB.
    let response = agent.get(url).config().timeout_global(Some(Duration::from_secs(600))).build().call().map_err(|e| e.to_string())?;
    let mut body = response.into_body().into_with_config().limit(512 << 20).reader();
    let result = (|| {
        let (mut file, mut hash) = (std::fs::File::create(path)?, sha2::Sha256::new());
        let mut buf = vec![0; 64 * 1024];
        loop {
            match body.read(&mut buf)? {
                0 => break,
                n => {
                    hash.update(&buf[..n]);
                    file.write_all(&buf[..n])?;
                }
            }
        }
        file.flush()?;
        Ok::<_, std::io::Error>(hash.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>())
    })();
    let result = match result {
        Ok(actual) if actual.eq_ignore_ascii_case(expected) => Ok(()),
        Ok(_) => Err("the download does not match its published checksum".to_string()),
        Err(e) => Err(e.to_string()),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

/// How this copy was installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// The setup program (its uninstaller sits beside the program).
    Setup,
    /// The portable zip (`portable.txt` beside the program).
    Portable,
    /// A copied folder or a development build: the user downloads new versions by hand.
    Manual,
}

impl InstallKind {
    pub fn of(exe_dir: &Path) -> Self {
        if exe_dir.join(crate::paths::PORTABLE_FILE).is_file() {
            Self::Portable
        } else if exe_dir.join("unins000.exe").is_file() {
            Self::Setup
        } else {
            Self::Manual
        }
    }

    pub fn asset(self, version: &str) -> Option<String> {
        match self {
            Self::Setup => Some(format!("autosplitter-{version}-x64-setup.exe")),
            Self::Portable => Some(format!("autosplitter-{version}-x64-portable.zip")),
            Self::Manual => None,
        }
    }
}

/// Downloads and installs `tag`, then starts the new version. The caller quits straight after.
pub fn install(kind: InstallKind, tag: &str, exe_dir: &Path) -> Result<(), String> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let asset = kind.asset(version).ok_or("this copy cannot update itself: download the new version from the release page")?;
    let file = download_verified(RELEASES, tag, &asset, &download_dir(), RELEASE_KEY)?;
    match kind {
        // No questions, close the program, start it again afterwards (/RELAUNCH=1, see packaging/windows/autosplitter.iss).
        InstallKind::Setup => shell_open(&file, "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS /RELAUNCH=1"),
        _ => {
            swap_in(&file, exe_dir)?;
            std::process::Command::new(exe_dir.join("AutoSplitter.exe")).spawn().map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}

/// Unpacks `zip` over `dir`. Files in use cannot be overwritten on Windows but can be renamed,
/// so each old file moves to `<name>.old` first; [`clean_up`] removes those at the next start.
fn swap_in(zip: &Path, dir: &Path) -> Result<(), String> {
    let e = |e: &dyn std::fmt::Display| e.to_string();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip).map_err(|x| e(&x))?).map_err(|x| e(&x))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|x| e(&x))?;
        // enclosed_name refuses ../ and absolute paths.
        let Some(rel) = entry.enclosed_name() else { continue };
        if entry.is_dir() || rel.starts_with("profiles") {
            continue;
        }
        let dest = dir.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|x| e(&x))?;
        }
        let (new, old) = (dest.with_added_extension("new"), dest.with_added_extension("old"));
        std::io::copy(&mut entry, &mut std::fs::File::create(&new).map_err(|x| e(&x))?).map_err(|x| e(&x))?;
        if dest.exists() {
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&dest, &old).map_err(|x| format!("{}: {x}", dest.display()))?;
        }
        std::fs::rename(&new, &dest).map_err(|x| e(&x))?;
    }
    Ok(())
}

/// At start-up: remove what the last update left behind.
pub fn clean_up(exe_dir: &Path) {
    let _ = std::fs::remove_dir_all(download_dir());
    fn walk(dir: &Path, depth: u32) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() && depth > 0 {
                walk(&path, depth - 1);
            } else if path.extension().is_some_and(|x| x == "old") {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    walk(exe_dir, 1);
}

/// Start `program` through the shell, so one that needs administrator rights gets its prompt.
#[cfg(windows)]
pub fn shell_open(program: &Path, args: &str) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::HSTRING;
    // SAFETY: both strings are NUL-terminated HSTRINGs that outlive the call.
    let started = unsafe { ShellExecuteW(None, None, &HSTRING::from(program.as_os_str()), &HSTRING::from(args), None, SW_SHOWNORMAL) };
    // Values up to 32 are errors (ShellExecute's documented convention).
    if started.0 as usize <= 32 {
        return Err(format!("{} did not start (error {})", program.display(), started.0 as usize));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn shell_open(program: &Path, _: &str) -> Result<(), String> {
    Err(format!("{} only runs on Windows", program.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v2.0.10"), Some((2, 0, 10)));
        assert_eq!(parse_version("v2.0.0-rc1"), None);
        assert!(is_newer("v2.0.0", "1.3.7"));
        assert!(!is_newer("v2.0.0", "2.0.0"));
    }

    #[test]
    fn install_kinds_and_swap() {
        let dir = std::env::temp_dir().join(format!("autosplitter-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("profiles")).unwrap();
        assert_eq!(InstallKind::of(&dir), InstallKind::Manual);
        std::fs::write(dir.join("portable.txt"), "").unwrap();
        assert_eq!(InstallKind::of(&dir), InstallKind::Portable);
        std::fs::write(dir.join("AutoSplitter.exe"), "old").unwrap();
        std::fs::write(dir.join("profiles/mine.xml"), "pb").unwrap();

        let zip_path = dir.join("new.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        for (name, body) in [("AutoSplitter.exe", "new"), ("models/a.onnx", "model"), ("profiles/mine.xml", "lost"), ("../escape.txt", "x")] {
            z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        swap_in(&zip_path, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("AutoSplitter.exe")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(dir.join("models/a.onnx")).unwrap(), "model");
        assert_eq!(std::fs::read_to_string(dir.join("profiles/mine.xml")).unwrap(), "pb", "profiles are never replaced");
        assert!(dir.join("AutoSplitter.exe.old").exists() && !dir.parent().unwrap().join("escape.txt").exists());
        clean_up(&dir);
        assert!(!dir.join("AutoSplitter.exe.old").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A local server standing in for github.com: answers each request for `/<name>`
    /// from `files`, 404 for anything else, until `requests` are served.
    fn serve(files: Vec<(&'static str, Vec<u8>)>, requests: usize) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let n = stream.read(&mut request).unwrap();
                let line = String::from_utf8_lossy(&request[..n]).lines().next().unwrap_or_default().to_string();
                let path = line.split(' ').nth(1).unwrap_or_default().to_string();
                match files.iter().find(|(name, _)| path == format!("/r/download/v1.2.3/{name}")) {
                    Some((_, body)) => {
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                        stream.write_all(body).unwrap();
                    }
                    None => write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap(),
                }
            }
        });
        format!("{base}/r")
    }

    const SUMS: &[u8] = include_bytes!("../testdata/update/SHA256SUMS");
    const SIG: &[u8] = include_bytes!("../testdata/update/SHA256SUMS.minisig");
    const TEST_KEY: &str = include_str!("../testdata/update/test.pub");
    const OTHER_KEY: &str = include_str!("../testdata/update/other.pub");
    const ASSET: &[u8] = include_bytes!("../testdata/update/asset.bin");

    fn download(files: Vec<(&'static str, Vec<u8>)>, requests: usize, key: &str) -> (Result<PathBuf, String>, PathBuf) {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("autosplitter-dl-{}-{n}", std::process::id()));
        (download_verified(&serve(files, requests), "v1.2.3", "asset.bin", &dir, key), dir)
    }

    #[test]
    fn downloads_only_what_the_signed_list_vouches_for() {
        let good = || vec![("SHA256SUMS", SUMS.to_vec()), ("SHA256SUMS.minisig", SIG.to_vec()), ("asset.bin", ASSET.to_vec())];
        let (ok, dir) = download(good(), 3, TEST_KEY);
        assert_eq!(std::fs::read(ok.unwrap()).unwrap(), ASSET);
        std::fs::remove_dir_all(dir).unwrap();

        // Wrong key, edited list, swapped file: nothing is left on disk.
        assert!(download(good(), 2, OTHER_KEY).0.unwrap_err().contains("not signed"));
        let mut edited = good();
        edited[0].1.push(b'\n');
        assert!(download(edited, 2, TEST_KEY).0.unwrap_err().contains("not signed"));
        let mut swapped = good();
        swapped[2].1 = b"something else".to_vec();
        let (bad, dir) = download(swapped, 3, TEST_KEY);
        assert!(bad.unwrap_err().contains("does not match"));
        assert!(!dir.join("asset.bin").exists());
    }

    #[test]
    fn release_key_is_a_minisign_key() {
        assert!(minisign_verify::PublicKey::decode(RELEASE_KEY).is_ok());
    }
}
