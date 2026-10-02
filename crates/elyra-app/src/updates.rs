//! Release check against GitHub. Only checks and links to the download;
//! installing stays with the user.

use serde_json::Value;

pub const RELEASES_API: &str =
    "https://api.github.com/repos/kwhorne/elyra-workspace/releases/latest";

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
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

/// Fetch the latest release (blocking; run in the background).
pub fn latest_release() -> anyhow::Result<Release> {
    let output = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "-m",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            RELEASES_API,
        ])
        .output()?;
    anyhow::ensure!(output.status.success(), "no published release found");
    let json: Value = serde_json::from_slice(&output.stdout)?;
    Ok(Release {
        version: json["tag_name"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches('v')
            .to_string(),
        url: json["html_url"].as_str().unwrap_or_default().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0.0", "1.0.0-beta.2"));
        assert!(!is_newer("1.0.0-beta.2", "1.0.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }
}
