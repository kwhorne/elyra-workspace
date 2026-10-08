//! Browser journeys: a flow an agent walked through in the thread's browser
//! (open a page, click, fill in, press, wait, expect), saved with the project
//! in `.elyra/journeys/<name>.json` and replayed later — on request, or after
//! every turn that changed files (once the user turned that on) — so a change
//! that breaks the flow is caught like a failing test.
//!
//! This module is the file format and the words around it; replaying lives in
//! `browser_tools`, which drives the page.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where a project keeps its journeys.
pub const DIR: &str = ".elyra/journeys";

/// An element: by CSS selector, by visible text (label, placeholder), or both.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Target {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Target {
    pub fn describe(&self) -> String {
        match (self.text.as_deref(), self.selector.as_deref()) {
            (Some(text), _) => format!("\u{201c}{text}\u{201d}"),
            (None, Some(selector)) => format!("`{selector}`"),
            (None, None) => "the focused element".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Load a page (a local address).
    Open(String),
    Click(Target),
    Fill {
        #[serde(flatten)]
        target: Target,
        value: String,
    },
    Press {
        key: String,
        #[serde(flatten)]
        target: Target,
    },
    /// Wait until it is on the page.
    Wait(Target),
    /// It must be on the page now (after a short wait); the journey's verdict.
    Expect(Target),
}

impl Step {
    pub fn describe(&self) -> String {
        match self {
            Step::Open(url) => format!("open {url}"),
            Step::Click(target) => format!("click {}", target.describe()),
            Step::Fill { target, value } => {
                format!("fill in {} with \u{201c}{value}\u{201d}", target.describe())
            }
            Step::Press { key, target } => format!("press {key} in {}", target.describe()),
            Step::Wait(target) => format!("wait for {}", target.describe()),
            Step::Expect(target) => format!("expect {}", target.describe()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Journey {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub steps: Vec<Step>,
}

impl Journey {
    /// Saved journeys need somewhere to start and something to check.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("a journey needs a name".into());
        }
        if !matches!(self.steps.first(), Some(Step::Open(_))) {
            return Err("a journey starts by opening a page".into());
        }
        if !self.steps.iter().any(|s| matches!(s, Step::Expect(_))) {
            return Err(
                "a journey needs at least one expect step: what must be on the page when it works"
                    .into(),
            );
        }
        Ok(())
    }
}

/// The file a journey is saved in: its name, lowercased, words joined by `-`.
pub fn file_name(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("{}.json", if slug.is_empty() { "journey" } else { &slug })
}

/// A project's journeys, by file name; unreadable files are reported.
pub fn load_all(dir: &Path) -> Vec<(PathBuf, Result<Journey, String>)> {
    let Ok(entries) = std::fs::read_dir(dir.join(DIR)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let journey = std::fs::read_to_string(&path)
                .map_err(|err| err.to_string())
                .and_then(|text| serde_json::from_str(&text).map_err(|err| err.to_string()));
            (path, journey)
        })
        .collect()
}

pub fn find(dir: &Path, name: &str) -> Option<Journey> {
    load_all(dir).into_iter().find_map(|(path, journey)| {
        let journey = journey.ok()?;
        (journey.name.eq_ignore_ascii_case(name)
            || path.file_name().is_some_and(|f| *f == *file_name(name)))
        .then_some(journey)
    })
}

/// Write a journey into the project (replacing one of the same name).
pub fn save(dir: &Path, journey: &Journey) -> Result<PathBuf, String> {
    journey.validate()?;
    let folder = dir.join(DIR);
    std::fs::create_dir_all(&folder).map_err(|err| err.to_string())?;
    let path = folder.join(file_name(&journey.name));
    let text = serde_json::to_string_pretty(journey).map_err(|err| err.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|err| err.to_string())?;
    Ok(path)
}

/// Put a recorded or saved address on `origin` (the thread's own site, such
/// as a worktree's), keeping its path.
pub fn rebase(url: &str, origin: Option<&str>) -> String {
    let Some(origin) = origin else {
        return url.to_string();
    };
    let path = url
        .split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|at| &rest[at..]))
        .unwrap_or("/");
    format!("{}{path}", origin.trim_end_matches('/'))
}

/// `scheme://host[:port]` of a URL.
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split('/').next()?;
    (!host.is_empty()).then(|| format!("{scheme}://{host}"))
}

/// What a replay found.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub passed: bool,
    pub steps_run: usize,
    /// The step that failed and why.
    pub failure: Option<String>,
    pub duration_ms: u64,
}

/// The message that sends a failed journey back to the agent.
pub fn fix_prompt(name: &str, outcome: &Outcome, attempt: u32, attempts: u32) -> String {
    format!(
        "The browser journey \u{201c}{name}\u{201d} failed after your changes, at step {}: {}. \
         It is a saved flow in .elyra/journeys that worked before. Fix the cause in the code \
         (change the journey only if the flow itself was meant to change), then finish your \
         turn; the journeys run again (automatic fix {attempt} of {attempts}).",
        outcome.steps_run + 1,
        outcome.failure.as_deref().unwrap_or("it failed")
    )
}

// ---- replaying after each turn --------------------------------------------------

fn auto_key(project: elyra_core::ProjectId) -> String {
    format!("journeys_after_turn:{project}")
}

/// Whether the project replays its journeys after every turn that changed files.
pub fn replay_after_turn(store: &elyra_core::Store, project: elyra_core::ProjectId) -> bool {
    store.setting(&auto_key(project)).ok().flatten().as_deref() == Some("1")
}

pub fn set_replay_after_turn(store: &elyra_core::Store, project: elyra_core::ProjectId, on: bool) {
    if let Err(err) = store.set_setting(&auto_key(project), if on { "1" } else { "" }) {
        log::warn!("saving the journey setting: {err:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::{Journey, Step, Target, file_name, load_all, rebase, save};

    fn text(text: &str) -> Target {
        Target {
            text: Some(text.into()),
            selector: None,
        }
    }

    #[test]
    fn reads_and_writes_the_file_format() {
        let json = r##"{
            "name": "Checkout with a discount code",
            "steps": [
                {"open": "http://shop.test/products/1"},
                {"click": {"text": "Add to cart"}},
                {"fill": {"text": "Discount code", "value": "SUMMER"}},
                {"press": {"key": "Enter", "selector": "#code"}},
                {"wait": {"text": "Total"}},
                {"expect": {"text": "Total: 90,00"}}
            ]
        }"##;
        let journey: Journey = serde_json::from_str(json).unwrap();
        assert_eq!(journey.steps.len(), 6);
        assert_eq!(
            journey.steps[2],
            Step::Fill {
                target: text("Discount code"),
                value: "SUMMER".into()
            }
        );
        assert_eq!(journey.steps[3].describe(), "press Enter in `#code`");
        assert!(journey.validate().is_ok());

        let dir = std::env::temp_dir().join(format!("elyra-journeys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = save(&dir, &journey).unwrap();
        assert!(path.ends_with(".elyra/journeys/checkout-with-a-discount-code.json"));
        let loaded = load_all(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].1.as_ref().unwrap(), &journey);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_journeys_that_prove_nothing() {
        let mut journey = Journey {
            name: "x".into(),
            description: String::new(),
            steps: vec![Step::Click(text("Go"))],
        };
        assert!(journey.validate().unwrap_err().contains("opening a page"));
        journey.steps.insert(0, Step::Open("http://a.test/".into()));
        assert!(journey.validate().unwrap_err().contains("expect"));
        assert_eq!(file_name("Æ, ø & å!"), "æ-ø-å.json");
    }

    #[test]
    fn replays_on_the_thread_s_own_site() {
        assert_eq!(
            rebase(
                "http://shop.test/cart?x=1",
                Some("http://fix-cart.shop.test")
            ),
            "http://fix-cart.shop.test/cart?x=1"
        );
        assert_eq!(
            rebase("http://localhost:5173", Some("http://localhost:5174")),
            "http://localhost:5174/"
        );
        assert_eq!(rebase("http://shop.test/a", None), "http://shop.test/a");
    }
}
