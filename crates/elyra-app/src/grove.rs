//! Elyra Grove, the local development environment (`*.test` sites, a
//! request timeline, dev processes, mail): what Elyra Workspace asks it
//! through its CLI (`grove … --json`). Grove is optional; without it these
//! return nothing.
//!
//! Every function here blocks on a `grove` process, so call them on the
//! background executor.

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long the list of sites is trusted before asking Grove again.
const SITES_TTL: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Site {
    pub name: String,
    pub hostname: String,
    pub path: PathBuf,
    pub driver: String,
    #[serde(default)]
    pub secure: bool,
}

impl Site {
    pub fn url(&self) -> String {
        format!(
            "{}://{}/",
            if self.secure { "https" } else { "http" },
            self.hostname
        )
    }

    /// An app Grove runs (Laravel, PHP, a proxied dev server), not a folder
    /// it merely parks and would serve as static files.
    pub fn is_app(&self) -> bool {
        self.driver != "static"
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Request {
    pub id: u64,
    pub site: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    #[serde(default)]
    pub duration_ms: u64,
}

impl Request {
    pub fn line(&self) -> String {
        format!("{} {} → {}", self.method, self.path, self.status)
    }
}

/// The `grove` binary: on PATH, or where Grove installs it.
pub fn executable() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join("grove"))
        .find(|path| path.is_file())
    {
        return Some(path);
    }
    let home = dirs::home_dir().unwrap_or_default();
    [
        home.join(".grove/bin/grove"),
        PathBuf::from("/Applications/Grove.app/Contents/MacOS/grove"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

/// `grove <args> --json`, returning its `data`.
fn run(args: &[&str]) -> Result<Value> {
    let grove = executable().context("Grove is not installed")?;
    let output = std::process::Command::new(&grove)
        .args(args)
        .arg("--json")
        .output()
        .with_context(|| format!("running {}", grove.display()))?;
    let value: Value = serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "grove {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    })?;
    if value["ok"] != true {
        bail!(
            "grove {}: {}",
            args.join(" "),
            value["error"].as_str().unwrap_or("failed")
        );
    }
    Ok(value["data"].clone())
}

static SITES: Mutex<Option<(Instant, Vec<Site>)>> = Mutex::new(None);

/// Every site Grove serves (cached for a minute).
pub fn sites() -> Vec<Site> {
    if let Ok(cache) = SITES.lock()
        && let Some((at, sites)) = cache.as_ref()
        && at.elapsed() < SITES_TTL
    {
        return sites.clone();
    }
    let sites: Vec<Site> = run(&["list"])
        .ok()
        .and_then(|data| serde_json::from_value(data["sites"].clone()).ok())
        .unwrap_or_default();
    if let Ok(mut cache) = SITES.lock() {
        *cache = Some((Instant::now(), sites.clone()));
    }
    sites
}

/// What the cache already knows, without asking Grove (for code that
/// mustn't block, such as starting an agent).
pub fn cached_sites() -> Vec<Site> {
    SITES
        .lock()
        .ok()
        .and_then(|cache| cache.as_ref().map(|(_, sites)| sites.clone()))
        .unwrap_or_default()
}

/// The site serving `dir`: the one whose folder is `dir` or holds it.
pub fn site_for(sites: &[Site], dir: &Path) -> Option<Site> {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    sites
        .iter()
        .filter(|site| {
            let path = site
                .path
                .canonicalize()
                .unwrap_or_else(|_| site.path.clone());
            dir.starts_with(&path)
        })
        .max_by_key(|site| site.path.as_os_str().len())
        .cloned()
}

/// The app Grove runs for `dir`, if any.
pub fn app_for(dir: &Path) -> Option<Site> {
    site_for(&sites(), dir).filter(Site::is_app)
}

/// Recent requests to `site`, newest first.
pub fn requests(site: &str, limit: usize) -> Result<Vec<Request>> {
    let data = run(&["requests", site, "--limit", &limit.to_string()])?;
    Ok(serde_json::from_value(data["requests"].clone())?)
}

/// Server errors (5xx) among `requests` newer than `seen`, oldest first, and
/// the new watermark. The first look (`seen` is `None`) only sets the mark:
/// errors from before aren't news.
pub fn new_server_errors(seen: Option<u64>, requests: Vec<Request>) -> (u64, Vec<Request>) {
    let newest = requests.iter().map(|r| r.id).max().unwrap_or(0);
    let Some(seen) = seen else {
        return (newest, Vec::new());
    };
    let mut fresh: Vec<_> = requests
        .into_iter()
        .filter(|r| r.id > seen && r.status >= 500)
        .collect();
    fresh.sort_by_key(|r| r.id);
    (seen.max(newest), fresh)
}

/// Whether `grove dev` runs `site`'s dev processes.
pub fn dev_running(site: &str) -> bool {
    run(&["dev", "list"])
        .ok()
        .and_then(|data| data["dev_sites"].as_array().cloned())
        .is_some_and(|sites| sites.iter().any(|s| s == site))
}

pub fn set_dev(site: &str, on: bool) -> Result<()> {
    run(&["dev", if on { "start" } else { "stop" }, site]).map(|_| ())
}

/// How many mails Grove has caught (from every site).
pub fn mail_count() -> usize {
    run(&["mail"])
        .ok()
        .and_then(|data| data["mail"].as_array().map(Vec::len))
        .unwrap_or(0)
}

/// `grove explain <id>`: the request, the SQL and mail around it and the
/// error log, as text for an agent.
pub fn explain(id: u64) -> Result<String> {
    let data = run(&["explain", &id.to_string()])?;
    Ok(format_explain(&data["explain"]))
}

const MAX_BODY: usize = 2_000;
const MAX_QUERIES: usize = 40;
const MAX_LOG: usize = 8_000;

/// The explain bundle as readable text.
pub fn format_explain(explain: &Value) -> String {
    let short = |value: &Value| match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let mut out = format!("{}\n", short(&explain["summary"]));
    let request = &explain["request"];
    out.push_str(&format!(
        "\nRequest {}: {} {}://{}{} → {}\n",
        request["id"],
        short(&request["method"]),
        if request["https"] == true {
            "https"
        } else {
            "http"
        },
        short(&request["host"]),
        short(&request["path"]),
        request["status"],
    ));
    if let Some(body) = request["body"].as_str().filter(|b| !b.is_empty()) {
        let body: String = body.chars().take(MAX_BODY).collect();
        out.push_str(&format!("Body: {body}\n"));
    }
    let chain = &explain["chain"];
    let queries = chain["queries"].as_array().cloned().unwrap_or_default();
    if !queries.is_empty() {
        out.push_str(&format!("\nSQL ({}):\n", queries.len()));
        for query in queries.iter().take(MAX_QUERIES) {
            let sql = query
                .get("sql")
                .or_else(|| query.get("query"))
                .map(short)
                .unwrap_or_else(|| short(query));
            out.push_str(&format!("- {sql}\n"));
        }
    }
    let emails = chain["emails"].as_array().cloned().unwrap_or_default();
    if !emails.is_empty() {
        out.push_str(&format!("\nMail sent ({}):\n", emails.len()));
        for email in &emails {
            out.push_str(&format!(
                "- {} to {}\n",
                short(&email["subject"]),
                short(&email["to"])
            ));
        }
    }
    let logs = explain["logs"].as_array().cloned().unwrap_or_default();
    if !logs.is_empty() {
        let text: String = logs
            .iter()
            .map(|entry| {
                entry
                    .get("message")
                    .or_else(|| entry.get("text"))
                    .map(short)
                    .unwrap_or_else(|| short(entry))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text: String = text.chars().take(MAX_LOG).collect();
        out.push_str(&format!("\nError log:\n{text}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Request, Site, format_explain, new_server_errors, site_for};
    use serde_json::json;
    use std::path::PathBuf;

    fn site(name: &str, path: &str, driver: &str) -> Site {
        Site {
            name: name.into(),
            hostname: format!("{name}.test"),
            path: PathBuf::from(path),
            driver: driver.into(),
            secure: true,
        }
    }

    #[test]
    fn finds_the_site_serving_a_folder() {
        let sites = [
            site("code", "/nonexistent/code", "static"),
            site("shop", "/nonexistent/code/shop", "laravel"),
        ];
        let found = site_for(&sites, &PathBuf::from("/nonexistent/code/shop/app")).unwrap();
        assert_eq!(found.name, "shop");
        assert!(found.is_app());
        assert_eq!(found.url(), "https://shop.test/");
        assert!(
            !site_for(&sites, &PathBuf::from("/nonexistent/code"))
                .unwrap()
                .is_app()
        );
        assert!(site_for(&sites, &PathBuf::from("/elsewhere")).is_none());
    }

    /// Against the Grove installed on this Mac: `cargo test -p elyra-app live_grove -- --ignored`.
    #[test]
    #[ignore]
    fn live_grove() {
        let sites = super::sites();
        assert!(!sites.is_empty(), "Grove lists no sites");
        let home = dirs::home_dir().unwrap();
        let app = super::app_for(&home.join("Code/elyra-web")).expect("elyra-web is a Grove app");
        println!("app: {} {} {}", app.name, app.url(), app.driver);
        let requests = super::requests("vidrplay", 5).unwrap();
        println!(
            "requests: {:?}",
            requests.iter().map(Request::line).collect::<Vec<_>>()
        );
        if let Some(first) = requests.first() {
            println!("explain:\n{}", super::explain(first.id).unwrap());
        }
        println!(
            "dev vidrplay: {}  mail: {}",
            super::dev_running("vidrplay"),
            super::mail_count()
        );
    }

    #[test]
    fn picks_new_server_errors() {
        let request = |id, status| Request {
            id,
            site: "shop".into(),
            method: "GET".into(),
            path: format!("/{id}"),
            status,
            duration_ms: 1,
        };
        // The first look only sets the mark.
        let (seen, fresh) = new_server_errors(None, vec![request(5, 500), request(4, 200)]);
        assert_eq!((seen, fresh.len()), (5, 0));
        let (seen, fresh) = new_server_errors(
            Some(seen),
            vec![
                request(8, 503),
                request(7, 200),
                request(6, 500),
                request(5, 500),
            ],
        );
        assert_eq!(seen, 8);
        assert_eq!(fresh.iter().map(|r| r.id).collect::<Vec<_>>(), [6, 8]);
        assert_eq!(fresh[0].line(), "GET /6 → 500");
        let (seen, fresh) = new_server_errors(Some(seen), Vec::new());
        assert_eq!((seen, fresh.len()), (8, 0));
    }

    #[test]
    fn formats_an_explain_bundle() {
        let explain = json!({
            "summary": "POST /checkout → 500 on shop · 2 queries, 0 emails · 1 error log entries",
            "request": {"id": 7, "method": "POST", "host": "shop.test", "path": "/checkout",
                        "https": true, "status": 500, "body": "{\"qty\":2}"},
            "chain": {"queries": [{"sql": "select * from carts"}, "update carts set qty = 2"], "emails": []},
            "logs": [{"message": "ErrorException: Undefined index: price in CartController.php:42"}],
        });
        let text = format_explain(&explain);
        assert!(text.starts_with("POST /checkout → 500 on shop"));
        assert!(text.contains("Request 7: POST https://shop.test/checkout → 500"));
        assert!(text.contains("Body: {\"qty\":2}"));
        assert!(text.contains("SQL (2):\n- select * from carts\n- update carts set qty = 2\n"));
        assert!(text.contains("Error log:\nErrorException: Undefined index: price"));
        assert!(!text.contains("Mail sent"));
    }
}
