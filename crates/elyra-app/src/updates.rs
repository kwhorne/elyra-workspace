//! Self-update from GitHub releases.
//!
//! 1. `latest_release` finds the newest release and its DMG + checksum.
//! 2. `prepare` downloads the DMG, checks the SHA-256, copies the app out of
//!    the image next to the running one, and verifies the copy: a valid
//!    signature from the same Developer ID team as the running app, accepted
//!    by Gatekeeper (notarized), with the expected version.
//! 3. `install` swaps the staged app into place with two renames (rolling
//!    back on failure); `relaunch_after_exit` starts it once we have quit.
//!
//! Everything here blocks; run it on a background thread.

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const RELEASES_API: &str =
    "https://api.github.com/repos/kwhorne/elyra-workspace/releases/latest";

/// Releases are built for Apple Silicon only.
const ARCH: &str = "arm64";

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    /// The release page.
    pub url: String,
    pub dmg: Option<Asset>,
    pub checksum: Option<Asset>,
}

/// Parse "v1.2.3" / "1.2.3-beta.1" into comparable numbers (pre-release
/// versions sort before the release).
fn version_key(version: &str) -> Option<(Vec<u64>, bool)> {
    let version = version.trim().trim_start_matches('v');
    let (core, pre) = match version.split_once('-') {
        Some((core, _)) => (core, true),
        None => (version, false),
    };
    let numbers = core
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    Some((numbers, !pre))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (version_key(candidate), version_key(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

pub fn parse_release(json: &Value) -> Release {
    let version = json["tag_name"]
        .as_str()
        .unwrap_or_default()
        .trim_start_matches('v')
        .to_string();
    let assets: Vec<Asset> = json["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(Asset {
                name: a["name"].as_str()?.to_string(),
                url: a["browser_download_url"].as_str()?.to_string(),
            })
        })
        .collect();
    let dmg = assets
        .iter()
        .find(|a| a.name.ends_with(&format!("-{ARCH}.dmg")))
        .cloned();
    let checksum = dmg.as_ref().and_then(|dmg| {
        assets
            .iter()
            .find(|a| a.name == format!("{}.sha256", dmg.name))
            .cloned()
    });
    Release {
        version,
        url: json["html_url"].as_str().unwrap_or_default().to_string(),
        dmg,
        checksum,
    }
}

fn curl(args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("curl")
        .args(args)
        .output()
        .context("running curl")?;
    if !output.status.success() {
        bail!(
            "download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// Fetch the latest release.
pub fn latest_release() -> Result<Release> {
    let body = curl(&[
        "-fsSL",
        "-m",
        "15",
        "-H",
        "Accept: application/vnd.github+json",
        RELEASES_API,
    ])
    .context("no published release found")?;
    Ok(parse_release(&serde_json::from_slice(&body)?))
}

/// The `.app` bundle containing an executable, if it is one.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

/// Why the running app can't update itself, or `None` when it can.
pub fn self_update_blocker(bundle: Option<&Path>) -> Option<&'static str> {
    let Some(bundle) = bundle else {
        return Some("this is a development build");
    };
    let path = bundle.to_string_lossy();
    if path.contains("/AppTranslocation/") || path.starts_with("/Volumes/") {
        return Some("the app is running from the disk image; move it to Applications first");
    }
    let parent = bundle.parent()?;
    let probe = parent.join(format!(".elyra-update-probe-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            None
        }
        Err(_) => Some("the folder the app is in is not writable"),
    }
}

/// The Developer ID team that signed a bundle (`None` for ad-hoc builds).
pub fn team_id(bundle: &Path) -> Option<String> {
    let output = Command::new("codesign")
        .args(["-dv", "--verbose=2"])
        .arg(bundle)
        .output()
        .ok()?;
    // codesign prints the details on stderr.
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|team| !team.is_empty() && *team != "not set")
        .map(str::to_string)
}

fn bundle_version(bundle: &Path) -> Option<String> {
    let output = Command::new("plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(bundle.join("Contents/Info.plist"))
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn sha256(path: &Path) -> Result<String> {
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()?;
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("could not checksum {}", path.display()))
}

fn run(command: &mut Command, what: &str) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("running {what}"))?;
    if !output.status.success() {
        bail!(
            "{what} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// The hidden sibling the new version is staged in before the swap.
pub fn staged_path(bundle: &Path) -> PathBuf {
    let name = bundle.file_name().unwrap_or_default().to_string_lossy();
    bundle.with_file_name(format!(".{name}.update"))
}

/// Check a staged app: same team, valid signature, notarized, right version.
pub fn verify(staged: &Path, team: &str, version: &str) -> Result<()> {
    run(
        Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(staged),
        "signature check",
    )?;
    match team_id(staged) {
        Some(found) if found == team => {}
        Some(found) => bail!("the update is signed by team {found}, expected {team}"),
        None => bail!("the update is not signed with a Developer ID"),
    }
    run(
        Command::new("spctl")
            .args(["--assess", "--type", "execute"])
            .arg(staged),
        "Gatekeeper check",
    )?;
    match bundle_version(staged) {
        Some(found) if found == version => Ok(()),
        found => bail!(
            "the update reports version {}, expected {version}",
            found.unwrap_or_else(|| "unknown".into())
        ),
    }
}

/// Download, check and stage `release` next to `bundle`. Returns the staged
/// app, ready for `install`.
pub fn prepare(release: &Release, bundle: &Path, team: &str, work: &Path) -> Result<PathBuf> {
    let dmg_asset = release
        .dmg
        .as_ref()
        .ok_or_else(|| anyhow!("the release has no {ARCH} disk image"))?;
    let checksum_asset = release
        .checksum
        .as_ref()
        .ok_or_else(|| anyhow!("the release has no checksum"))?;
    std::fs::create_dir_all(work)?;
    let dmg = work.join(&dmg_asset.name);
    run(
        Command::new("curl")
            .args(["-fsSL", "-m", "900", "-o"])
            .arg(&dmg)
            .arg(&dmg_asset.url),
        "download",
    )?;
    let expected = String::from_utf8_lossy(&curl(&["-fsSL", "-m", "60", &checksum_asset.url])?)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let actual = sha256(&dmg)?;
    if expected.is_empty() || expected != actual {
        bail!("the download is damaged (checksum mismatch)");
    }

    let mount = work.join("mount");
    let _ = run(
        Command::new("hdiutil")
            .args(["detach", "-quiet", "-force"])
            .arg(&mount),
        "detach",
    );
    std::fs::create_dir_all(&mount)?;
    run(
        Command::new("hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-quiet",
                "-mountpoint",
            ])
            .arg(&mount)
            .arg(&dmg),
        "opening the disk image",
    )?;
    let staged = staged_path(bundle);
    let result = (|| -> Result<()> {
        let app = std::fs::read_dir(&mount)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or_else(|| anyhow!("no app in the disk image"))?;
        if staged.exists() {
            std::fs::remove_dir_all(&staged)?;
        }
        run(
            Command::new("ditto").arg(&app).arg(&staged),
            "copying the app",
        )?;
        Ok(())
    })();
    let _ = run(
        Command::new("hdiutil")
            .args(["detach", "-quiet", "-force"])
            .arg(&mount),
        "detach",
    );
    let _ = std::fs::remove_file(&dmg);
    result?;
    if let Err(err) = verify(&staged, team, &release.version) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(err);
    }
    Ok(staged)
}

/// Swap the staged app into place. The old version is moved aside first and
/// restored if the swap fails.
pub fn install(staged: &Path, bundle: &Path) -> Result<()> {
    if !staged.exists() {
        bail!("the update is no longer staged");
    }
    let name = bundle.file_name().unwrap_or_default().to_string_lossy();
    let backup = bundle.with_file_name(format!(".{name}.old"));
    if backup.exists() {
        std::fs::remove_dir_all(&backup)?;
    }
    std::fs::rename(bundle, &backup).context("moving the current version aside")?;
    if let Err(err) = std::fs::rename(staged, bundle) {
        let _ = std::fs::rename(&backup, bundle);
        return Err(err).context("moving the new version into place");
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

/// Start `bundle` again once this process has exited.
pub fn relaunch_after_exit(bundle: &Path) -> Result<()> {
    let mut open = format!("open -n {}", shell_quote(&bundle.to_string_lossy()));
    // Keep development settings (isolated data directory, background window).
    for key in ["ELYRA_HOME", "ELYRA_NO_ACTIVATE"] {
        if let Ok(value) = std::env::var(key) {
            open.push_str(&format!(" --env {key}={}", shell_quote(&value)));
        }
    }
    let script = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; {open}",
        pid = std::process::id()
    );
    Command::new("/bin/sh")
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("scheduling the relaunch")?;
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0.0", "1.0.0-beta.2"));
        assert!(!is_newer("1.0.0-beta.2", "1.0.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn picks_the_disk_image_and_its_checksum() {
        let release = parse_release(&json!({
            "tag_name": "v0.2.0",
            "html_url": "https://example.com/r",
            "assets": [
                { "name": "Elyra-Workspace-0.2.0-arm64.dmg.sha256", "browser_download_url": "https://x/sum" },
                { "name": "Elyra-Workspace-0.2.0-arm64.dmg", "browser_download_url": "https://x/dmg" },
                { "name": "notes.txt", "browser_download_url": "https://x/notes" }
            ]
        }));
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.dmg.unwrap().url, "https://x/dmg");
        assert_eq!(release.checksum.unwrap().url, "https://x/sum");
        assert!(
            parse_release(&json!({ "tag_name": "v1", "assets": [] }))
                .dmg
                .is_none()
        );
    }

    #[test]
    fn finds_bundles_and_blockers() {
        let exe = Path::new("/Applications/Elyra Workspace.app/Contents/MacOS/elyra");
        assert_eq!(
            bundle_of(exe),
            Some(PathBuf::from("/Applications/Elyra Workspace.app"))
        );
        assert_eq!(bundle_of(Path::new("/Users/me/target/release/elyra")), None);
        assert!(self_update_blocker(None).is_some());
        assert!(
            self_update_blocker(Some(Path::new(
                "/private/var/folders/x/AppTranslocation/y/Elyra Workspace.app"
            )))
            .is_some()
        );
        assert_eq!(
            staged_path(Path::new("/Applications/Elyra Workspace.app")),
            PathBuf::from("/Applications/.Elyra Workspace.app.update")
        );
    }

    #[test]
    fn installs_by_swapping_bundles() {
        let dir = std::env::temp_dir().join(format!("elyra-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = dir.join("Elyra Workspace.app");
        let staged = staged_path(&bundle);
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        std::fs::write(bundle.join("Contents/version"), "old").unwrap();
        std::fs::create_dir_all(staged.join("Contents")).unwrap();
        std::fs::write(staged.join("Contents/version"), "new").unwrap();
        install(&staged, &bundle).unwrap();
        assert_eq!(
            std::fs::read_to_string(bundle.join("Contents/version")).unwrap(),
            "new"
        );
        assert!(!staged.exists() && !dir.join(".Elyra Workspace.app.old").exists());
        assert!(install(&staged, &bundle).is_err(), "nothing staged");
        assert_eq!(
            std::fs::read_to_string(bundle.join("Contents/version")).unwrap(),
            "new"
        );
        assert!(
            self_update_blocker(Some(&bundle)).is_none(),
            "temp dir is writable"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Uses the locally built release bundle when there is one.
    #[test]
    fn verifies_a_signed_bundle() {
        let bundle = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/release/bundle/Elyra Workspace.app");
        let Some(team) = bundle.exists().then(|| team_id(&bundle)).flatten() else {
            return; // not built or ad-hoc signed
        };
        let version = bundle_version(&bundle).unwrap();
        assert!(verify(&bundle, "WRONGTEAM", &version).is_err());
        assert!(verify(&bundle, &team, "9.9.9").is_err());
        // Only a notarized build passes Gatekeeper.
        let notarized = Command::new("spctl")
            .args(["--assess", "--type", "execute"])
            .arg(&bundle)
            .status()
            .is_ok_and(|s| s.success());
        assert_eq!(verify(&bundle, &team, &version).is_ok(), notarized);
    }
}
