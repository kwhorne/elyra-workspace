//! Elyra Félagi, the task and time platform: issues to work on, and what was
//! done, their status and the hours, written back. Spoken to over its REST
//! API (`/api/v1`) with a personal token, which lives in the macOS Keychain.
//!
//! Requests go through `curl` (as updates do) with the token handed over on
//! stdin, so it never appears in a process list. Every request function
//! blocks, so call them on the background executor.

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Write as _;
use std::process::{Command, Stdio};

/// Where the connection (not the token) is kept, in the settings table.
pub const SETTINGS_KEY: &str = "felagi";

/// The Keychain item holding the token.
const KEYCHAIN_SERVICE: &str = "Elyra Workspace – Félagi";

/// The statuses an issue moves through, in board order.
pub const STATUSES: &[(&str, &str)] = &[
    ("backlog", "Backlog"),
    ("todo", "To do"),
    ("in_progress", "In progress"),
    ("in_review", "In review"),
    ("on_hold", "On hold"),
    ("done", "Done"),
    ("canceled", "Canceled"),
];

pub fn status_label(status: &str) -> &str {
    STATUSES
        .iter()
        .find(|(key, _)| *key == status)
        .map_or(status, |(_, label)| label)
}

/// A connected Félagi workspace.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    /// Like `https://felagi.example.com`, without `/api/v1`.
    pub url: String,
    pub workspace: String,
    pub user: String,
    /// The user's actor key, `user:3`, for "assigned to me".
    pub actor: String,
    pub can_write: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Issue {
    /// `ACM-231`: the identifier, and what the API takes back.
    pub id: String,
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub estimate_minutes: Option<u64>,
    #[serde(default)]
    pub spent_minutes: Option<u64>,
    #[serde(default)]
    pub project: Option<Value>,
}

impl Issue {
    /// The issue as a first message to an agent.
    pub fn as_prompt(&self) -> String {
        let mut text = format!("{} {}\n", self.id, self.title);
        if let Some(description) = self.description.as_deref().map(html_to_text)
            && !description.trim().is_empty()
        {
            text.push_str(&format!("\n{}\n", description.trim()));
        }
        if !self.acceptance_criteria.is_empty() {
            text.push_str("\nAcceptance criteria:\n");
            for criterion in &self.acceptance_criteria {
                text.push_str(&format!("- {criterion}\n"));
            }
        }
        text
    }
}

/// The running timer, if any.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Timer {
    pub issue: String,
    pub started_at: String,
}

impl Timer {
    /// Whole minutes since the timer started (at least one).
    pub fn minutes(&self) -> u64 {
        chrono::DateTime::parse_from_rfc3339(&self.started_at)
            .map(|start| {
                let seconds =
                    (chrono::Utc::now() - start.with_timezone(&chrono::Utc)).num_seconds();
                ((seconds as f64 / 60.).round() as u64).max(1)
            })
            .unwrap_or(1)
    }
}

/// Félagi's descriptions are sanitised HTML; agents read plain text better.
pub fn html_to_text(html: &str) -> String {
    let mut text = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                match name.as_str() {
                    "br" | "p" | "div" | "h1" | "h2" | "h3" | "h4" | "tr" => text.push('\n'),
                    "li" if !tag.starts_with('/') => text.push_str("\n- "),
                    _ => {}
                }
            }
            _ if in_tag => tag.push(c),
            _ => text.push(c),
        }
    }
    let text = text
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    // Collapse runs of blank lines.
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

/// Plain text (paragraphs, `- ` lists) as the HTML Félagi stores comments in.
pub fn text_to_html(text: &str) -> String {
    let escape = |line: &str| {
        line.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut html = String::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut list: Vec<String> = Vec::new();
    let flush = |html: &mut String, paragraph: &mut Vec<String>, list: &mut Vec<String>| {
        if !paragraph.is_empty() {
            html.push_str(&format!("<p>{}</p>", paragraph.join("<br>")));
            paragraph.clear();
        }
        if !list.is_empty() {
            html.push_str("<ul>");
            for item in list.drain(..) {
                html.push_str(&format!("<li>{item}</li>"));
            }
            html.push_str("</ul>");
        }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut html, &mut paragraph, &mut list);
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            if !paragraph.is_empty() {
                flush(&mut html, &mut paragraph, &mut Vec::new());
            }
            list.push(escape(item));
        } else {
            if !list.is_empty() {
                flush(&mut html, &mut Vec::new(), &mut list);
            }
            paragraph.push(escape(trimmed));
        }
    }
    flush(&mut html, &mut paragraph, &mut list);
    html
}

/// `2h 30m`, as Félagi writes durations.
pub fn format_minutes(minutes: u64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// `2h 30m`, `1.5h`, `90m` or `90` into minutes.
pub fn parse_duration(text: &str) -> Option<u64> {
    let text = text.trim().to_lowercase().replace(',', ".");
    if text.is_empty() {
        return None;
    }
    if let Ok(minutes) = text.parse::<u64>() {
        return Some(minutes);
    }
    let mut total = 0.;
    let mut number = String::new();
    let mut any = false;
    for c in text.chars().chain(std::iter::once(' ')) {
        match c {
            '0'..='9' | '.' => number.push(c),
            'h' | 'm' | 'd' => {
                let value: f64 = number.parse().ok()?;
                total += value
                    * match c {
                        'h' => 60.,
                        'd' => 480.,
                        _ => 1.,
                    };
                number.clear();
                any = true;
            }
            ' ' if number.is_empty() => {}
            ' ' => return None,
            _ => return None,
        }
    }
    (any && total > 0.).then(|| total.round() as u64)
}

/// The saved connection, if any.
pub fn connection(store: &elyra_core::Store) -> Option<Connection> {
    store
        .setting(SETTINGS_KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .filter(|c: &Connection| !c.url.is_empty())
}

pub fn save_connection(store: &elyra_core::Store, connection: Option<&Connection>) {
    let json = connection
        .and_then(|c| serde_json::to_string(c).ok())
        .unwrap_or_default();
    if let Err(err) = store.set_setting(SETTINGS_KEY, &json) {
        log::warn!("saving the Félagi connection: {err:#}");
    }
}

/// The Félagi project an Elyra project's work belongs to.
pub fn linked_project(store: &elyra_core::Store, project: elyra_core::ProjectId) -> Option<u64> {
    store
        .setting(&format!("felagi_project:{project}"))
        .ok()
        .flatten()
        .and_then(|id| id.parse().ok())
}

pub fn link_project(
    store: &elyra_core::Store,
    project: elyra_core::ProjectId,
    felagi: Option<u64>,
) {
    let value = felagi.map(|id| id.to_string()).unwrap_or_default();
    let _ = store.set_setting(&format!("felagi_project:{project}"), &value);
}

// ---- token -------------------------------------------------------------

#[cfg(target_os = "macos")]
pub fn save_token(url: &str, token: &str) -> Result<()> {
    security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, url, token.as_bytes())
        .context("saving the token in the Keychain")
}

#[cfg(target_os = "macos")]
pub fn token(url: &str) -> Option<String> {
    security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, url)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

#[cfg(target_os = "macos")]
pub fn forget_token(url: &str) {
    let _ = security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, url);
}

#[cfg(not(target_os = "macos"))]
pub fn save_token(_: &str, _: &str) -> Result<()> {
    bail!("Félagi tokens are kept in the macOS Keychain")
}

#[cfg(not(target_os = "macos"))]
pub fn token(_: &str) -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn forget_token(_: &str) {}

// ---- requests ------------------------------------------------------------

/// A request to the API; `body` is sent as JSON. Returns the parsed answer,
/// or the API's own message on an error status.
fn request(
    url: &str,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let endpoint = format!("{}/api/v1{path}", url.trim_end_matches('/'));
    let mut command = Command::new("curl");
    command
        .args([
            "-sS",
            "--max-time",
            "30",
            "-X",
            method,
            "-w",
            "\n%{http_code}",
        ])
        .args(["-K", "-"])
        .arg(&endpoint)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("running curl")?;
    {
        let mut stdin = child.stdin.take().context("curl stdin")?;
        // curl's config syntax: one option per line, values quoted.
        let quote = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(stdin, "header = \"Authorization: Bearer {}\"", quote(token))?;
        writeln!(stdin, "header = \"Accept: application/json\"")?;
        if let Some(body) = body {
            writeln!(stdin, "header = \"Content-Type: application/json\"")?;
            writeln!(stdin, "data = \"{}\"", quote(&body.to_string()))?;
        }
    }
    let output = child.wait_with_output().context("waiting for curl")?;
    if !output.status.success() {
        bail!(
            "could not reach Félagi: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, status) = text.rsplit_once('\n').unwrap_or(("", &text));
    let status: u16 = status.trim().parse().unwrap_or(0);
    let value: Value = if body.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body).unwrap_or(Value::Null)
    };
    if !(200..300).contains(&status) {
        let message = value["message"].as_str().unwrap_or("").to_string();
        bail!(match status {
            401 => "Félagi doesn't accept the token (wrong, revoked, expired, or you left the workspace)".to_string(),
            403 if message.is_empty() => "the token may not do that (a read-only token can't write)".to_string(),
            404 => format!("not found in Félagi{}", if message.is_empty() { String::new() } else { format!(": {message}") }),
            _ if message.is_empty() => format!("Félagi answered {status}"),
            _ => format!("Félagi: {message}"),
        });
    }
    Ok(value)
}

/// A client for one connected workspace.
#[derive(Clone)]
pub struct Client {
    url: String,
    token: String,
}

impl Client {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    /// The client for a saved connection, with its token from the Keychain.
    pub fn for_connection(connection: &Connection) -> Option<Self> {
        token(&connection.url).map(|token| Self::new(&connection.url, &token))
    }

    fn get(&self, path: &str) -> Result<Value> {
        request(&self.url, &self.token, "GET", path, None)
    }

    fn send(&self, method: &str, path: &str, body: Value) -> Result<Value> {
        request(&self.url, &self.token, method, path, Some(&body))
    }

    /// Who the token is: checked when connecting.
    pub fn me(&self) -> Result<Connection> {
        let value = self.get("/me")?;
        // `/me` answers without the `data` envelope the other endpoints use.
        let data = if value["data"].is_object() {
            &value["data"]
        } else {
            &value
        };
        let user = &data["user"];
        Ok(Connection {
            url: self.url.clone(),
            workspace: data["workspace"]["name"].as_str().unwrap_or("").to_string(),
            user: user["name"].as_str().unwrap_or("").to_string(),
            actor: format!("user:{}", user["id"]),
            // The abilities sit on the token (older answers had them on top).
            can_write: [&data["token"]["abilities"], &data["abilities"]]
                .into_iter()
                .filter_map(|abilities| abilities.as_array())
                .any(|abilities| abilities.iter().any(|a| a == "write")),
        })
    }

    /// Issues assigned to `assignee` (an actor key), newest activity first,
    /// optionally in one project. Follows pages up to `limit` issues.
    pub fn issues(&self, assignee: &str, project: Option<u64>, limit: usize) -> Result<Vec<Issue>> {
        let mut path = format!("/issues?assignee={assignee}&per_page=100");
        if let Some(project) = project {
            path.push_str(&format!("&project_id={project}"));
        }
        let mut issues = Vec::new();
        let mut next = Some(path);
        while let Some(page) = next.take() {
            let value = self.get(&page)?;
            issues.extend(serde_json::from_value::<Vec<Issue>>(value["data"].clone())?);
            next = value["links"]["next"]
                .as_str()
                .and_then(|link| link.split_once("/api/v1").map(|(_, rest)| rest.to_string()))
                .filter(|_| issues.len() < limit);
        }
        Ok(issues)
    }

    pub fn issue(&self, id: &str) -> Result<Issue> {
        Ok(serde_json::from_value(
            self.get(&format!("/issues/{id}"))?["data"].clone(),
        )?)
    }

    /// Projects: (id, name).
    pub fn projects(&self) -> Result<Vec<(u64, String)>> {
        // `/projects` takes `page` only, so follow the pages.
        let mut projects = Vec::new();
        let mut next = Some("/projects".to_string());
        while let Some(page) = next.take() {
            let value = self.get(&page)?;
            projects.extend(
                value["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| Some((p["id"].as_u64()?, p["name"].as_str()?.to_string()))),
            );
            next = value["links"]["next"]
                .as_str()
                .and_then(|link| link.split_once("/api/v1").map(|(_, rest)| rest.to_string()))
                .filter(|_| projects.len() < 500);
        }
        Ok(projects)
    }

    pub fn set_status(&self, id: &str, status: &str) -> Result<()> {
        self.send(
            "PATCH",
            &format!("/issues/{id}"),
            json!({ "status": status }),
        )
        .map(|_| ())
    }

    pub fn comment(&self, id: &str, body: &str) -> Result<()> {
        self.send(
            "POST",
            &format!("/issues/{id}/comments"),
            json!({ "body": body }),
        )
        .map(|_| ())
    }

    pub fn log_time(&self, id: &str, minutes: u64, note: &str) -> Result<()> {
        self.send(
            "POST",
            &format!("/issues/{id}/time"),
            json!({ "minutes": minutes, "note": note }),
        )
        .map(|_| ())
    }

    pub fn timer(&self) -> Result<Option<Timer>> {
        let data = self.get("/timer")?["data"].clone();
        Ok(serde_json::from_value(data).ok())
    }

    /// Start the clock on an issue (stopping and logging any other timer).
    pub fn start_timer(&self, id: &str) -> Result<()> {
        self.send("POST", &format!("/issues/{id}/timer"), json!({}))
            .map(|_| ())
    }

    /// Throw the running timer away, recording nothing (the hours are logged
    /// with `log_time`, as the user corrected them).
    pub fn discard_timer(&self) -> Result<()> {
        request(&self.url, &self.token, "DELETE", "/timer", None).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::{Issue, format_minutes, html_to_text, parse_duration};
    use serde_json::json;

    /// Against a Félagi with a test issue assigned to the token's user:
    /// `FELAGI_TEST_URL=… FELAGI_TEST_TOKEN_FILE=… FELAGI_TEST_ISSUE=ACM-9
    /// cargo test -p elyra-app live_felagi -- --ignored`. It comments, logs a
    /// minute and moves the issue to In review.
    #[test]
    #[ignore]
    fn live_felagi() {
        let url = std::env::var("FELAGI_TEST_URL").unwrap();
        let token =
            std::fs::read_to_string(std::env::var("FELAGI_TEST_TOKEN_FILE").unwrap()).unwrap();
        let id = std::env::var("FELAGI_TEST_ISSUE").unwrap();
        let client = super::Client::new(&url, token.trim());
        let me = client.me().unwrap();
        println!(
            "me: {} as {} ({}) write={}",
            me.workspace, me.user, me.actor, me.can_write
        );
        let issues = client.issues(&me.actor, None, 300).unwrap();
        println!(
            "{} issues assigned: {:?}",
            issues.len(),
            issues
                .iter()
                .map(|i| (&i.id, &i.status))
                .collect::<Vec<_>>()
        );
        assert!(issues.iter().any(|i| i.id == id));
        let issue = client.issue(&id).unwrap();
        println!("prompt:\n{}", issue.as_prompt());
        println!("projects: {:?}", client.projects().unwrap());
        client.start_timer(&id).unwrap();
        let timer = client.timer().unwrap().expect("a timer runs");
        println!(
            "timer: {} since {} ({} min)",
            timer.issue,
            timer.started_at,
            timer.minutes()
        );
        assert_eq!(timer.issue, id);
        client.discard_timer().unwrap();
        assert!(client.timer().unwrap().is_none());
        client
            .log_time(&id, 1, "Elyra Workspace: integration test")
            .unwrap();
        client
            .comment(
                &id,
                &super::text_to_html("Integration test.\n\n- Comment\n- Time\n- Status"),
            )
            .unwrap();
        client.set_status(&id, "in_review").unwrap();
        let after = client.issue(&id).unwrap();
        println!(
            "after: status {} spent {:?}",
            after.status, after.spent_minutes
        );
        assert_eq!(after.status, "in_review");
    }

    #[test]
    fn reads_durations() {
        assert_eq!(parse_duration("2h 30m"), Some(150));
        assert_eq!(parse_duration("1.5h"), Some(90));
        assert_eq!(parse_duration("1,5h"), Some(90));
        assert_eq!(parse_duration("45"), Some(45));
        assert_eq!(parse_duration("45m"), Some(45));
        assert_eq!(parse_duration("1d"), Some(480));
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(format_minutes(150), "2h 30m");
        assert_eq!(format_minutes(120), "2h");
        assert_eq!(format_minutes(5), "5m");
    }

    #[test]
    fn turns_text_into_html() {
        assert_eq!(
            super::text_to_html(
                "Fixed the <login> loop.\nSafari only.\n\n- Cookie path\n- A test\n\nDone."
            ),
            "<p>Fixed the &lt;login&gt; loop.<br>Safari only.</p><ul><li>Cookie path</li><li>A test</li></ul><p>Done.</p>"
        );
    }

    #[test]
    fn turns_html_into_text() {
        assert_eq!(
            html_to_text(
                "<p>Fix the <strong>login</strong> loop.</p><ul><li>Safari</li><li>Chrome &amp; Edge</li></ul>"
            ),
            "Fix the login loop.\n\n- Safari\n- Chrome & Edge"
        );
    }

    #[test]
    fn writes_an_issue_as_a_prompt() {
        let issue: Issue = serde_json::from_value(json!({
            "id": "ACM-231", "number": 231, "title": "Login loops on Safari",
            "description": "<p>Users get sent back to /login.</p>",
            "acceptance_criteria": ["Safari logs in", "A test covers it"],
            "status": "todo", "priority": "high", "type": "bug",
            "project": {"id": 4, "name": "Portal"}
        }))
        .unwrap();
        assert_eq!(
            issue.as_prompt(),
            "ACM-231 Login loops on Safari\n\nUsers get sent back to /login.\n\nAcceptance criteria:\n- Safari logs in\n- A test covers it\n"
        );
    }
}
