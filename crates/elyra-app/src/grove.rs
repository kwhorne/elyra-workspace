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

/// A branch Grove runs beside the main checkout (`grove try`): its own git
/// worktree, its own copy of the database, migrated, and its own site.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TryRecord {
    /// The site it was made from.
    pub site: String,
    pub branch: String,
    /// The worktree.
    pub path: PathBuf,
    pub url: String,
    /// `mysql`, `sqlite` or `none`.
    #[serde(default)]
    pub engine: String,
    #[serde(default)]
    pub database: String,
}

/// The output of a `grove` command that doesn't speak JSON, or its error.
fn run_plain(args: &[&str]) -> Result<String> {
    let grove = executable().context("Grove is not installed")?;
    let output = std::process::Command::new(&grove)
        .args(args)
        .output()
        .with_context(|| format!("running {}", grove.display()))?;
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_string();
    if !output.status.success() {
        let stderr = text(&output.stderr);
        bail!(
            "grove {}: {}",
            args.join(" "),
            if stderr.is_empty() {
                text(&output.stdout)
            } else {
                stderr
            }
        );
    }
    Ok(text(&output.stdout))
}

/// Every branch Grove runs beside a checkout.
pub fn tries() -> Vec<TryRecord> {
    run_plain(&["try", "--list", "--json"])
        .ok()
        .and_then(|json| {
            serde_json::from_str::<std::collections::BTreeMap<String, TryRecord>>(&json).ok()
        })
        .map(|tries| tries.into_values().collect())
        .unwrap_or_default()
}

/// The try whose worktree is `path`, if Grove runs one there.
pub fn try_at(path: &Path) -> Option<TryRecord> {
    tries().into_iter().find(|t| t.path == path)
}

/// `grove try --new <branch>`: a new branch from `site`'s current commit in
/// its own worktree, with a migrated copy of the database, served at its own
/// address. Takes a while: Grove installs dependencies and migrates.
pub fn start_try(site: &str, branch: &str) -> Result<TryRecord> {
    run_plain(&["try", "--new", branch, "--site", site])?;
    // The try is a new site: learn it now, for code that reads the cache.
    if let Ok(mut cache) = SITES.lock() {
        *cache = None;
    }
    sites();
    tries()
        .into_iter()
        .find(|t| t.site == site && t.branch == branch)
        .context("Grove started the branch but doesn't list it")
}

/// Take a try down: its site, its database copy and its worktree. The branch
/// and its commits stay.
pub fn end_try(record: &TryRecord) -> Result<()> {
    run_plain(&[
        "try",
        "--done",
        &record.branch,
        "--site",
        &record.site,
        "--force",
    ])
    .map(|_| ())
}

/// The database a project's `.env` points to.
#[derive(Clone, Debug, PartialEq)]
pub enum Database {
    Sqlite(PathBuf),
    /// One of Grove's servers: `mysql`, `postgres` or `elyrasql`.
    Server {
        engine: String,
        name: String,
    },
}

/// `KEY=value` lines of a `.env` file (quotes removed; comments skipped).
fn parse_env(text: &str) -> std::collections::HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim().trim_matches('"').trim_matches('\'');
            Some((key.trim().to_string(), value.to_string()))
        })
        .collect()
}

/// Which database `env` (a project's `.env` in `dir`) uses. A MySQL
/// connection on ElyraSQL's port is ElyraSQL: it speaks MySQL's protocol.
pub fn database_from_env(
    env: &std::collections::HashMap<String, String>,
    dir: &Path,
    elyrasql_port: Option<u16>,
) -> Option<Database> {
    let connection = env.get("DB_CONNECTION").map(String::as_str).unwrap_or("");
    match connection {
        "sqlite" => {
            let file = env
                .get("DB_DATABASE")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("database/database.sqlite"));
            Some(Database::Sqlite(if file.is_absolute() {
                file
            } else {
                dir.join(file)
            }))
        }
        "mysql" | "mariadb" | "pgsql" => {
            let name = env.get("DB_DATABASE").filter(|n| !n.is_empty())?.clone();
            let engine = if connection == "pgsql" {
                "postgres"
            } else {
                let port = env
                    .get("DB_PORT")
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(3306);
                if elyrasql_port == Some(port) {
                    "elyrasql"
                } else {
                    "mysql"
                }
            };
            Some(Database::Server {
                engine: engine.into(),
                name,
            })
        }
        _ => None,
    }
}

/// The database of the app in `dir`, from its `.env`.
pub fn database_of(dir: &Path) -> Option<Database> {
    let env = parse_env(&std::fs::read_to_string(dir.join(".env")).ok()?);
    let elyrasql_port = run(&["service", "list"]).ok().and_then(|data| {
        data["services"].as_array()?.iter().find_map(|s| {
            (s["key"] == "elyrasql" && s["installed"] == true)
                .then(|| s["port"].as_u64().map(|p| p as u16))
                .flatten()
        })
    });
    database_from_env(&env, dir, elyrasql_port)
}

/// Take a snapshot of `database`; the returned reference restores it. SQLite
/// files are copied under `into`; Grove's servers are dumped by Grove.
pub fn snapshot_database(database: &Database, into: &Path, note: &str) -> Result<String> {
    match database {
        Database::Sqlite(file) => {
            std::fs::create_dir_all(into)?;
            let copy = into.join(format!("{}.sqlite", uuid::Uuid::new_v4().simple()));
            std::fs::copy(file, &copy).with_context(|| format!("copying {}", file.display()))?;
            Ok(format!("sqlite:{}", copy.display()))
        }
        Database::Server { engine, name } => {
            let data = run(&[
                "db", "snapshot", "--engine", engine, "--db", name, "--note", note,
            ])?;
            let message = data
                .as_str()
                .or_else(|| data["message"].as_str())
                .unwrap_or_default();
            // "snapshot <id> created (…)"
            let id = message
                .split_whitespace()
                .nth(1)
                .filter(|_| message.starts_with("snapshot "))
                .with_context(|| format!("unexpected answer from grove db snapshot: {message}"))?;
            Ok(format!("grove:{id}"))
        }
    }
}

/// Put a database back as a snapshot found it.
pub fn restore_database(database: &Database, reference: &str) -> Result<()> {
    match (database, reference.split_once(':')) {
        (Database::Sqlite(file), Some(("sqlite", copy))) => {
            std::fs::copy(copy, file).with_context(|| format!("restoring {}", file.display()))?;
            // Journal files from after the snapshot would replay newer writes.
            for suffix in ["-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", file.display()));
            }
            Ok(())
        }
        (Database::Server { .. }, Some(("grove", id))) => run(&["db", "restore", id]).map(|_| ()),
        _ => bail!("the snapshot {reference} doesn't belong to this database"),
    }
}

/// Delete a snapshot that is no longer needed.
pub fn drop_snapshot(reference: &str) {
    match reference.split_once(':') {
        Some(("sqlite", copy)) => {
            let _ = std::fs::remove_file(copy);
        }
        Some(("grove", id)) => {
            let _ = run(&["db", "rm", id]);
        }
        _ => {}
    }
}

/// Send a recorded request again (`grove replay --same-data`: from the same
/// data every time) and return its new status.
pub fn replay(id: u64) -> Result<u16> {
    let data = run(&["replay", &id.to_string(), "--same-data"])?;
    let replayed = if data["replayed_same_data"].is_object() {
        &data["replayed_same_data"]
    } else {
        &data["replayed"]
    };
    replayed["status"]
        .as_u64()
        .map(|status| status as u16)
        .context("grove replay gave no status")
}

/// What a replay says about a request that failed before.
pub fn replay_note(line: &str, status: Result<u16, String>) -> (String, bool) {
    let before = line.rsplit(' ').next().unwrap_or("");
    let request = line.rsplit_once(" → ").map_or(line, |(request, _)| request);
    match status {
        Ok(status) if status < 500 => (
            format!("Replayed {request}: was {before}, now {status} ✓"),
            false,
        ),
        Ok(status) => (format!("Replayed {request}: still {status}"), true),
        Err(err) => (format!("Couldn't replay {request}: {err}"), true),
    }
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
        if let Some(get) = requests.iter().find(|r| r.method == "GET") {
            println!("replay {} → {:?}", get.line(), super::replay(get.id));
            // Leave no --same-data baseline behind.
            let _ = super::run(&["replay", &get.id.to_string(), "--forget"]);
        }
        println!(
            "dev vidrplay: {}  mail: {}",
            super::dev_running("vidrplay"),
            super::mail_count()
        );
    }

    /// Needs a Grove app named `elyratryshop` in a git repository:
    /// `cargo test -p elyra-app live_grove_try -- --ignored`.
    #[test]
    #[ignore]
    fn live_grove_try() {
        let record = super::start_try("elyratryshop", "elyra/livetest").expect("try starts");
        println!(
            "try: {} {} {}",
            record.url,
            record.path.display(),
            record.engine
        );
        assert_eq!(super::try_at(&record.path), Some(record.clone()));
        let site = super::app_for(&record.path).expect("the try is a site");
        println!("site for worktree: {} {}", site.name, site.url());
        let status = std::process::Command::new("curl")
            .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", &record.url])
            .output()
            .unwrap();
        println!(
            "GET {} → {}",
            record.url,
            String::from_utf8_lossy(&status.stdout)
        );
        super::end_try(&record).expect("try ends");
        assert!(super::try_at(&record.path).is_none());
        assert!(!record.path.exists());
    }

    #[test]
    fn reads_the_database_from_env() {
        use super::{Database, database_from_env, parse_env};
        let dir = PathBuf::from("/app");
        let env = parse_env("# local\nDB_CONNECTION=sqlite\n");
        assert_eq!(
            database_from_env(&env, &dir, None),
            Some(Database::Sqlite(PathBuf::from(
                "/app/database/database.sqlite"
            )))
        );
        let env = parse_env("DB_CONNECTION=mysql\nDB_PORT=3307\nDB_DATABASE=\"shop\"\n");
        assert_eq!(
            database_from_env(&env, &dir, Some(3307)),
            Some(Database::Server {
                engine: "elyrasql".into(),
                name: "shop".into()
            })
        );
        assert_eq!(
            database_from_env(&env, &dir, Some(3310)),
            Some(Database::Server {
                engine: "mysql".into(),
                name: "shop".into()
            })
        );
        let env = parse_env("DB_CONNECTION=pgsql\nDB_DATABASE=shop\n");
        assert_eq!(
            database_from_env(&env, &dir, None),
            Some(Database::Server {
                engine: "postgres".into(),
                name: "shop".into()
            })
        );
        assert_eq!(
            database_from_env(&parse_env("DB_CONNECTION=mysql\n"), &dir, None),
            None
        );
    }

    #[test]
    fn snapshots_and_restores_sqlite() {
        use super::{Database, drop_snapshot, restore_database, snapshot_database};
        let dir = std::env::temp_dir().join(format!("elyra-grove-db-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("app.sqlite");
        std::fs::write(&file, "before").unwrap();
        let database = Database::Sqlite(file.clone());
        let reference = snapshot_database(&database, &dir.join("snaps"), "test").unwrap();
        std::fs::write(&file, "after").unwrap();
        std::fs::write(dir.join("app.sqlite-wal"), "journal").unwrap();
        restore_database(&database, &reference).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "before");
        assert!(!dir.join("app.sqlite-wal").exists());
        drop_snapshot(&reference);
        assert!(!PathBuf::from(reference.trim_start_matches("sqlite:")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn says_what_a_replay_found() {
        use super::replay_note;
        assert_eq!(
            replay_note("POST /checkout → 500", Ok(200)),
            ("Replayed POST /checkout: was 500, now 200 ✓".into(), false)
        );
        assert_eq!(
            replay_note("POST /checkout → 500", Ok(502)),
            ("Replayed POST /checkout: still 502".into(), true)
        );
        assert!(replay_note("GET / → 500", Err("no request".into())).1);
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
