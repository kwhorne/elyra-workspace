//! GitHub through the `gh` CLI (uses the user's own authentication).

use anyhow::{Result, bail};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn gh(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("gh")
        .args(args)
        .current_dir(repo)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|err| anyhow::anyhow!("GitHub CLI (`gh`) is not available: {err}"))?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn available() -> bool {
    Command::new("gh")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    /// "pass", "fail", "pending" or "skipped".
    pub state: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub created_at: String,
    /// File and line for inline review comments.
    pub path: Option<String>,
    pub line: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub author: String,
    pub state: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub is_draft: bool,
    pub url: String,
    pub head: String,
    pub base: String,
    pub review_decision: String,
    pub mergeable: String,
    pub body: String,
    pub additions: u64,
    pub deletions: u64,
    pub checks: Vec<Check>,
    pub comments: Vec<Comment>,
    pub reviews: Vec<Review>,
}

const PR_FIELDS: &str = "number,title,state,isDraft,url,headRefName,baseRefName,reviewDecision,mergeable,body,additions,deletions,statusCheckRollup,comments,reviews";

fn login(value: &Value) -> String {
    value["author"]["login"]
        .as_str()
        .unwrap_or("ghost")
        .to_string()
}

fn check_state(check: &Value) -> String {
    let conclusion = check["conclusion"].as_str().unwrap_or("");
    let status = check["status"].as_str().unwrap_or("");
    let state = check["state"].as_str().unwrap_or("");
    match (conclusion, status, state) {
        ("SUCCESS", ..) | ("NEUTRAL", ..) | (_, _, "SUCCESS") => "pass",
        ("SKIPPED", ..) => "skipped",
        ("FAILURE", ..)
        | ("TIMED_OUT", ..)
        | ("CANCELLED", ..)
        | ("ACTION_REQUIRED", ..)
        | (_, _, "FAILURE")
        | (_, _, "ERROR") => "fail",
        _ => "pending",
    }
    .to_string()
}

pub(crate) fn parse_pr(json: &Value) -> PullRequest {
    PullRequest {
        number: json["number"].as_u64().unwrap_or(0),
        title: json["title"].as_str().unwrap_or("").into(),
        state: json["state"].as_str().unwrap_or("").into(),
        is_draft: json["isDraft"].as_bool().unwrap_or(false),
        url: json["url"].as_str().unwrap_or("").into(),
        head: json["headRefName"].as_str().unwrap_or("").into(),
        base: json["baseRefName"].as_str().unwrap_or("").into(),
        review_decision: json["reviewDecision"].as_str().unwrap_or("").into(),
        mergeable: json["mergeable"].as_str().unwrap_or("").into(),
        body: json["body"].as_str().unwrap_or("").into(),
        additions: json["additions"].as_u64().unwrap_or(0),
        deletions: json["deletions"].as_u64().unwrap_or(0),
        checks: json["statusCheckRollup"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| Check {
                name: c["name"]
                    .as_str()
                    .or(c["context"].as_str())
                    .unwrap_or("check")
                    .into(),
                state: check_state(c),
                url: c["detailsUrl"]
                    .as_str()
                    .or(c["targetUrl"].as_str())
                    .map(str::to_string),
            })
            .collect(),
        comments: json["comments"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| Comment {
                author: login(c),
                body: c["body"].as_str().unwrap_or("").into(),
                created_at: c["createdAt"].as_str().unwrap_or("").into(),
                path: None,
                line: None,
            })
            .collect(),
        reviews: json["reviews"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| Review {
                author: login(r),
                state: r["state"].as_str().unwrap_or("").into(),
                body: r["body"].as_str().unwrap_or("").into(),
            })
            .collect(),
    }
}

/// The pull request for the checked-out branch, if any.
pub fn pr_for_branch(repo: &Path) -> Result<Option<PullRequest>> {
    match gh(repo, &["pr", "view", "--json", PR_FIELDS]) {
        Ok(text) => Ok(Some(parse_pr(&serde_json::from_str(&text)?))),
        Err(err) if err.to_string().contains("no pull requests found") => Ok(None),
        Err(err) => Err(err),
    }
}

pub fn pr(repo: &Path, number: u64) -> Result<PullRequest> {
    let text = gh(
        repo,
        &["pr", "view", &number.to_string(), "--json", PR_FIELDS],
    )?;
    Ok(parse_pr(&serde_json::from_str(&text)?))
}

/// Inline review comments (file and line).
pub fn review_comments(repo: &Path, number: u64) -> Result<Vec<Comment>> {
    let text = gh(
        repo,
        &[
            "api",
            &format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments"),
        ],
    )?;
    let json: Value = serde_json::from_str(&text)?;
    Ok(json
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| Comment {
            author: c["user"]["login"].as_str().unwrap_or("ghost").into(),
            body: c["body"].as_str().unwrap_or("").into(),
            created_at: c["created_at"].as_str().unwrap_or("").into(),
            path: c["path"].as_str().map(str::to_string),
            line: c["line"].as_u64().or(c["original_line"].as_u64()),
        })
        .collect())
}

/// Who `gh` is signed in as.
pub fn signed_in_as() -> Result<String> {
    let login = gh(&std::env::temp_dir(), &["api", "user", "--jq", ".login"])?;
    let login = login.trim();
    if login.is_empty() {
        bail!("`gh` is not signed in");
    }
    Ok(login.to_string())
}

/// Where a pull request stands, for following up a review.
#[derive(Clone, Debug, PartialEq)]
pub struct PrProgress {
    pub title: String,
    pub url: String,
    /// OPEN, MERGED or CLOSED.
    pub state: String,
    pub head: String,
    /// Its commits, oldest first.
    pub commits: Vec<String>,
    /// The commit `me` last reviewed, if they did.
    pub my_review: Option<String>,
    /// `me` is asked to review (again).
    pub requested: bool,
}

impl PrProgress {
    /// Commits pushed after `since` (all of them when it isn't in the PR any
    /// more, after a rebase).
    pub fn commits_after(&self, since: &str) -> usize {
        match self.commits.iter().position(|c| c == since) {
            Some(at) => self.commits.len() - at - 1,
            None => self.commits.len(),
        }
    }
}

/// `slug` is `owner/name`.
pub fn pr_progress(slug: &str, number: u64, me: &str) -> Result<PrProgress> {
    let text = gh(
        &std::env::temp_dir(),
        &[
            "pr",
            "view",
            &number.to_string(),
            "--repo",
            slug,
            "--json",
            "title,url,state,headRefOid,commits,reviews,reviewRequests",
        ],
    )?;
    Ok(parse_progress(&serde_json::from_str(&text)?, me))
}

pub(crate) fn parse_progress(json: &Value, me: &str) -> PrProgress {
    let mine = |author: &Value| {
        author["login"]
            .as_str()
            .is_some_and(|l| l.eq_ignore_ascii_case(me))
    };
    PrProgress {
        title: json["title"].as_str().unwrap_or("").into(),
        url: json["url"].as_str().unwrap_or("").into(),
        state: json["state"].as_str().unwrap_or("").into(),
        head: json["headRefOid"].as_str().unwrap_or("").into(),
        commits: json["commits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["oid"].as_str().map(str::to_string))
            .collect(),
        my_review: json["reviews"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| mine(&r["author"]) && r["state"] != "PENDING")
            .filter_map(|r| r["commit"]["oid"].as_str().map(str::to_string))
            .next_back(),
        requested: json["reviewRequests"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|r| {
                r["login"]
                    .as_str()
                    .is_some_and(|l| l.eq_ignore_ascii_case(me))
            }),
    }
}

/// One of `me`'s comments on a pull request, with the answers to it.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewPoint {
    pub path: Option<String>,
    pub line: Option<u64>,
    pub body: String,
    /// (author, reply).
    pub replies: Vec<(String, String)>,
}

/// `me`'s inline comments and review summaries on a pull request, with the
/// replies to the inline ones.
pub fn review_points(slug: &str, number: u64, me: &str) -> Result<Vec<ReviewPoint>> {
    let dir = std::env::temp_dir();
    let comments = gh(
        &dir,
        &[
            "api",
            "--paginate",
            &format!("repos/{slug}/pulls/{number}/comments"),
        ],
    )?;
    let reviews = gh(
        &dir,
        &[
            "api",
            "--paginate",
            &format!("repos/{slug}/pulls/{number}/reviews"),
        ],
    )?;
    Ok(parse_points(
        &concat_pages(&comments),
        &concat_pages(&reviews),
        me,
    ))
}

/// `gh api --paginate` prints one JSON array per page.
fn concat_pages(text: &str) -> Value {
    let mut all = Vec::new();
    let stream = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    for page in stream.flatten() {
        if let Value::Array(items) = page {
            all.extend(items);
        }
    }
    Value::Array(all)
}

pub(crate) fn parse_points(comments: &Value, reviews: &Value, me: &str) -> Vec<ReviewPoint> {
    let mine = |c: &Value| {
        c["user"]["login"]
            .as_str()
            .is_some_and(|l| l.eq_ignore_ascii_case(me))
    };
    let empty = Vec::new();
    let comments = comments.as_array().unwrap_or(&empty);
    let mut points: Vec<ReviewPoint> = reviews
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter(|r| mine(r) && !r["body"].as_str().unwrap_or("").trim().is_empty())
        .map(|r| ReviewPoint {
            path: None,
            line: None,
            body: r["body"].as_str().unwrap_or("").trim().to_string(),
            replies: Vec::new(),
        })
        .collect();
    for comment in comments
        .iter()
        .filter(|c| mine(c) && c["in_reply_to_id"].is_null())
    {
        let id = comment["id"].as_u64();
        points.push(ReviewPoint {
            path: comment["path"].as_str().map(str::to_string),
            line: comment["line"]
                .as_u64()
                .or(comment["original_line"].as_u64()),
            body: comment["body"].as_str().unwrap_or("").trim().to_string(),
            replies: comments
                .iter()
                .filter(|r| r["in_reply_to_id"].as_u64() == id && id.is_some())
                .map(|r| {
                    (
                        r["user"]["login"].as_str().unwrap_or("ghost").to_string(),
                        r["body"].as_str().unwrap_or("").trim().to_string(),
                    )
                })
                .collect(),
        });
    }
    points
}

/// Create a PR for the current branch; returns its URL.
pub fn create_pr(
    repo: &Path,
    title: &str,
    body: &str,
    draft: bool,
    base: Option<&str>,
) -> Result<String> {
    let mut args = vec!["pr", "create", "--title", title, "--body", body];
    if draft {
        args.push("--draft");
    }
    if let Some(base) = base.filter(|b| !b.is_empty()) {
        args.extend(["--base", base]);
    }
    Ok(gh(repo, &args)?
        .lines()
        .last()
        .unwrap_or("")
        .trim()
        .to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}

pub fn merge_pr(repo: &Path, number: u64, method: MergeMethod, delete_branch: bool) -> Result<()> {
    let number = number.to_string();
    let flag = match method {
        MergeMethod::Squash => "--squash",
        MergeMethod::Merge => "--merge",
        MergeMethod::Rebase => "--rebase",
    };
    let mut args = vec!["pr", "merge", number.as_str(), flag];
    if delete_branch {
        args.push("--delete-branch");
    }
    gh(repo, &args)?;
    Ok(())
}

pub fn set_pr_state(repo: &Path, number: u64, action: &str) -> Result<()> {
    // action: "close" | "reopen" | "ready"
    gh(repo, &["pr", action, &number.to_string()])?;
    Ok(())
}

pub fn comment_pr(repo: &Path, number: u64, body: &str) -> Result<()> {
    gh(
        repo,
        &["pr", "comment", &number.to_string(), "--body", body],
    )?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    PullRequest,
    Issue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboxItem {
    pub kind: ItemKind,
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: String,
    pub is_draft: bool,
    pub updated_at: String,
    pub url: String,
    pub labels: Vec<String>,
}

pub(crate) fn parse_items(kind: ItemKind, json: &Value) -> Vec<InboxItem> {
    json.as_array()
        .into_iter()
        .flatten()
        .map(|item| InboxItem {
            kind,
            number: item["number"].as_u64().unwrap_or(0),
            title: item["title"].as_str().unwrap_or("").into(),
            author: login(item),
            state: item["state"].as_str().unwrap_or("").into(),
            is_draft: item["isDraft"].as_bool().unwrap_or(false),
            updated_at: item["updatedAt"].as_str().unwrap_or("").into(),
            url: item["url"].as_str().unwrap_or("").into(),
            labels: item["labels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| l["name"].as_str().map(str::to_string))
                .collect(),
        })
        .collect()
}

/// Open or closed pull requests and issues of the repository at `repo`.
pub fn list_items(repo: &Path, kind: ItemKind, open: bool, limit: usize) -> Result<Vec<InboxItem>> {
    let limit = limit.to_string();
    let state = if open { "open" } else { "closed" };
    let (command, fields) = match kind {
        ItemKind::PullRequest => (
            "pr",
            "number,title,author,state,isDraft,updatedAt,url,labels",
        ),
        ItemKind::Issue => ("issue", "number,title,author,state,updatedAt,url,labels"),
    };
    let text = gh(
        repo,
        &[
            command, "list", "--state", state, "--limit", &limit, "--json", fields,
        ],
    )?;
    Ok(parse_items(kind, &serde_json::from_str(&text)?))
}

/// Issue body and comments as Markdown.
pub fn issue_markdown(repo: &Path, number: u64) -> Result<String> {
    let text = gh(
        repo,
        &[
            "issue",
            "view",
            &number.to_string(),
            "--json",
            "title,body,comments",
        ],
    )?;
    let json: Value = serde_json::from_str(&text)?;
    let mut out = json["body"].as_str().unwrap_or("").to_string();
    for comment in json["comments"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "\n\n---\n**{}**: {}",
            login(comment),
            comment["body"].as_str().unwrap_or("")
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_pull_request_json() {
        let pr = parse_pr(&json!({
            "number": 12, "title": "Add parser", "state": "OPEN", "isDraft": false,
            "url": "https://github.com/a/b/pull/12", "headRefName": "feat", "baseRefName": "main",
            "reviewDecision": "CHANGES_REQUESTED", "mergeable": "MERGEABLE", "body": "Body",
            "additions": 10, "deletions": 2,
            "statusCheckRollup": [
                {"__typename": "CheckRun", "name": "test", "status": "COMPLETED", "conclusion": "FAILURE", "detailsUrl": "u"},
                {"__typename": "StatusContext", "context": "lint", "state": "SUCCESS"},
                {"__typename": "CheckRun", "name": "build", "status": "IN_PROGRESS", "conclusion": ""}
            ],
            "comments": [{"author": {"login": "ann"}, "body": "nit", "createdAt": "t"}],
            "reviews": [{"author": {"login": "bob"}, "state": "CHANGES_REQUESTED", "body": "fix"}]
        }));
        assert_eq!(pr.number, 12);
        let states: Vec<_> = pr.checks.iter().map(|c| c.state.as_str()).collect();
        assert_eq!(states, ["fail", "pass", "pending"]);
        assert_eq!(pr.comments[0].author, "ann");
        assert_eq!(pr.reviews[0].state, "CHANGES_REQUESTED");
    }

    #[test]
    fn parses_inbox_items() {
        let items = parse_items(
            ItemKind::Issue,
            &json!([{"number": 3, "title": "Bug", "author": {"login": "c"}, "state": "OPEN", "updatedAt": "t", "url": "u", "labels": [{"name": "bug"}]}]),
        );
        assert_eq!((items[0].number, items[0].labels[0].as_str()), (3, "bug"));
    }

    #[test]
    fn follows_a_pull_request_after_a_review() {
        let json = serde_json::json!({
            "title": "Fix totals", "url": "https://github.com/o/r/pull/7", "state": "OPEN",
            "headRefOid": "c3",
            "commits": [{"oid": "c1"}, {"oid": "c2"}, {"oid": "c3"}],
            "reviews": [
                {"author": {"login": "KH"}, "state": "CHANGES_REQUESTED", "commit": {"oid": "c1"}},
                {"author": {"login": "dev"}, "state": "COMMENTED", "commit": {"oid": "c2"}},
                {"author": {"login": "kh"}, "state": "PENDING", "commit": {"oid": "c3"}}
            ],
            "reviewRequests": [{"login": "kh"}]
        });
        let progress = super::parse_progress(&json, "kh");
        assert_eq!(
            progress.my_review.as_deref(),
            Some("c1"),
            "the last submitted one"
        );
        assert!(progress.requested);
        assert_eq!(progress.commits_after("c1"), 2);
        assert_eq!(progress.commits_after("gone"), 3, "after a rebase");

        let comments = serde_json::json!([
            {"id": 1, "user": {"login": "kh"}, "path": "a.php", "line": 4, "body": "Null check?", "in_reply_to_id": null},
            {"id": 2, "user": {"login": "dev"}, "body": "Added.", "in_reply_to_id": 1},
            {"id": 3, "user": {"login": "dev"}, "path": "b.php", "line": 9, "body": "Not mine", "in_reply_to_id": null}
        ]);
        let reviews = serde_json::json!([
            {"user": {"login": "kh"}, "body": "Please fix 1-3 before merge."},
            {"user": {"login": "kh"}, "body": ""}
        ]);
        let points = super::parse_points(&comments, &reviews, "kh");
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].body, "Please fix 1-3 before merge.");
        assert_eq!(points[1].path.as_deref(), Some("a.php"));
        assert_eq!(
            points[1].replies,
            [("dev".to_string(), "Added.".to_string())]
        );
        assert_eq!(
            super::concat_pages("[1,2]\n[3]"),
            serde_json::json!([1, 2, 3])
        );
    }
}
