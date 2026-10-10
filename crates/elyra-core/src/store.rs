use crate::model::*;
use crate::orchestration::*;
use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Ordered, append-only schema migrations. Never edit a released entry;
/// add a new one instead.
const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        path TEXT NOT NULL UNIQUE,
        created_at TEXT NOT NULL
    );
    CREATE TABLE threads (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        title TEXT NOT NULL,
        provider TEXT NOT NULL,
        model TEXT,
        permission_mode TEXT NOT NULL,
        provider_session_id TEXT,
        environment TEXT NOT NULL,
        status TEXT NOT NULL,
        archived INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE INDEX threads_project ON threads(project_id, updated_at DESC);
    CREATE TABLE transcript_items (
        id TEXT PRIMARY KEY,
        thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        seq INTEGER NOT NULL,
        content TEXT NOT NULL,
        created_at TEXT NOT NULL,
        UNIQUE(thread_id, seq)
    );
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
"#,
    r#"
    ALTER TABLE threads ADD COLUMN effort TEXT;
"#,
    r#"
    ALTER TABLE threads ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE threads ADD COLUMN done INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE threads ADD COLUMN read_at TEXT;
    ALTER TABLE threads ADD COLUMN last_activity_at TEXT;
    ALTER TABLE projects ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE projects ADD COLUMN icon TEXT;
    ALTER TABLE projects ADD COLUMN color TEXT;
"#,
    r#"
    ALTER TABLE threads ADD COLUMN parent_id TEXT;
    ALTER TABLE threads ADD COLUMN notes TEXT;
    ALTER TABLE threads ADD COLUMN recap TEXT;
    ALTER TABLE threads ADD COLUMN pinned_items TEXT;
    ALTER TABLE projects ADD COLUMN space TEXT;
    ALTER TABLE projects ADD COLUMN instructions TEXT;
"#,
    r#"
    ALTER TABLE threads ADD COLUMN account TEXT;
    ALTER TABLE threads ADD COLUMN fork_context TEXT;
"#,
    r#"
    ALTER TABLE threads ADD COLUMN goal TEXT;
    ALTER TABLE threads ADD COLUMN goal_status TEXT;
    ALTER TABLE threads ADD COLUMN goal_runs INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE threads ADD COLUMN debug_mode INTEGER NOT NULL DEFAULT 0;
    CREATE TABLE automations (id TEXT PRIMARY KEY, data TEXT NOT NULL);
    CREATE TABLE automation_runs (
        id TEXT PRIMARY KEY,
        automation_id TEXT NOT NULL,
        started_at TEXT NOT NULL,
        data TEXT NOT NULL
    );
    CREATE INDEX automation_runs_by_automation ON automation_runs (automation_id, started_at);
    CREATE TABLE tasks (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, data TEXT NOT NULL);
    CREATE TABLE mcp_clients (id TEXT PRIMARY KEY, data TEXT NOT NULL);
    CREATE TABLE audit_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        at TEXT NOT NULL,
        data TEXT NOT NULL
    );
"#,
    r#"
    ALTER TABLE threads ADD COLUMN budget_usd REAL;
    ALTER TABLE threads ADD COLUMN race_id TEXT;
"#,
    r#"
    ALTER TABLE threads ADD COLUMN felagi_issue TEXT;
"#,
    r#"
    ALTER TABLE projects ADD COLUMN check_command TEXT;
"#,
    r#"
    CREATE TABLE line_origins (
        project_id TEXT NOT NULL,
        path TEXT NOT NULL,
        hash INTEGER NOT NULL,
        thread_id TEXT NOT NULL,
        item_id TEXT NOT NULL,
        at TEXT NOT NULL
    );
    CREATE INDEX line_origins_lookup ON line_origins (project_id, path, hash);
"#,
];

const PROJECT_COLUMNS: &str =
    "id, name, path, created_at, pinned, icon, color, space, instructions, check_command";
const THREAD_COLUMNS: &str = "id, project_id, title, provider, model, permission_mode,
    provider_session_id, environment, status, archived, created_at, updated_at, effort,
    pinned, done, read_at, last_activity_at, parent_id, notes, recap, pinned_items, account,
    fork_context, goal, goal_status, goal_runs, debug_mode, budget_usd, race_id, felagi_issue";

pub struct Store {
    pub(crate) conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)
                .with_context(|| format!("migration {}", index + 1))?;
            tx.pragma_update(None, "user_version", (index + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    // ---- projects -------------------------------------------------------

    pub fn add_project(&self, path: &Path) -> Result<Project> {
        let path = path
            .canonicalize()
            .with_context(|| format!("resolving {}", path.display()))?;
        if let Some(existing) = self.project_by_path(&path)? {
            return Ok(existing);
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let project = Project {
            id: new_id(),
            name,
            path,
            created_at: Utc::now(),
            pinned: false,
            icon: None,
            color: None,
            space: None,
            instructions: None,
            check_command: None,
        };
        self.conn.execute(
            "INSERT INTO projects (id, name, path, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                project.id.to_string(),
                project.name,
                project.path.to_string_lossy(),
                project.created_at.to_rfc3339()
            ],
        )?;
        Ok(project)
    }

    fn project_by_path(&self, path: &Path) -> Result<Option<Project>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE path = ?1"),
                params![path.to_string_lossy()],
                project_from_row,
            )
            .optional()?)
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {PROJECT_COLUMNS} FROM projects ORDER BY name COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([], project_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Point a project at another folder, keeping its threads and history.
    pub fn relocate_project(&self, id: ProjectId, path: &Path) -> Result<Project> {
        let path = path
            .canonicalize()
            .with_context(|| format!("resolving {}", path.display()))?;
        if let Some(existing) = self.project_by_path(&path)?
            && existing.id != id
        {
            anyhow::bail!(
                "{} is already the project \"{}\"",
                path.display(),
                existing.name
            );
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let updated = self.conn.execute(
            "UPDATE projects SET path = ?2, name = ?3 WHERE id = ?1",
            params![id.to_string(), path.to_string_lossy(), name],
        )?;
        anyhow::ensure!(updated == 1, "project {id} not found");
        Ok(self
            .project_by_path(&path)?
            .expect("project was just updated"))
    }

    /// Persist name, pinning and appearance.
    pub fn update_project(&self, project: &Project) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET name = ?2, pinned = ?3, icon = ?4, color = ?5, space = ?6,
                instructions = ?7, check_command = ?8 WHERE id = ?1",
            params![
                project.id.to_string(),
                project.name,
                project.pinned,
                project.icon,
                project.color,
                project.space,
                project.instructions,
                project.check_command
            ],
        )?;
        Ok(())
    }

    pub fn remove_project(&self, id: ProjectId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM projects WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    // ---- threads --------------------------------------------------------

    pub fn create_thread(
        &self,
        project_id: ProjectId,
        provider: ProviderKind,
        model: Option<String>,
        permission_mode: PermissionMode,
        environment: Environment,
    ) -> Result<Thread> {
        let now = Utc::now();
        let thread = Thread {
            id: new_id(),
            project_id,
            title: "New thread".into(),
            provider,
            model,
            effort: None,
            permission_mode,
            provider_session_id: None,
            environment,
            status: ThreadStatus::Idle,
            archived: false,
            pinned: false,
            done: false,
            read_at: None,
            last_activity_at: None,
            parent_id: None,
            notes: None,
            recap: None,
            pinned_items: Vec::new(),
            account: None,
            fork_context: None,
            goal: None,
            goal_status: None,
            goal_runs: 0,
            debug_mode: false,
            budget_usd: None,
            race_id: None,
            felagi_issue: None,
            created_at: now,
            updated_at: now,
        };
        self.conn.execute(
            "INSERT INTO threads (id, project_id, title, provider, model, permission_mode,
                provider_session_id, environment, status, archived, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                thread.id.to_string(),
                thread.project_id.to_string(),
                thread.title,
                thread.provider.as_str(),
                thread.model,
                thread.permission_mode.as_str(),
                thread.provider_session_id,
                serde_json::to_string(&thread.environment)?,
                thread.status.as_str(),
                thread.archived,
                thread.created_at.to_rfc3339(),
                thread.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(thread)
    }

    pub fn update_thread(&self, thread: &Thread) -> Result<()> {
        self.conn.execute(
            "UPDATE threads SET title = ?2, model = ?3, permission_mode = ?4,
                provider_session_id = ?5, environment = ?6, status = ?7, archived = ?8,
                updated_at = ?9, effort = ?10, provider = ?11, pinned = ?12, done = ?13,
                read_at = ?14, last_activity_at = ?15, parent_id = ?16, notes = ?17, recap = ?18,
                pinned_items = ?19, account = ?20, fork_context = ?21, goal = ?22,
                goal_status = ?23, goal_runs = ?24, debug_mode = ?25, budget_usd = ?26,
                race_id = ?27, felagi_issue = ?28
             WHERE id = ?1",
            params![
                thread.id.to_string(),
                thread.title,
                thread.model,
                thread.permission_mode.as_str(),
                thread.provider_session_id,
                serde_json::to_string(&thread.environment)?,
                thread.status.as_str(),
                thread.archived,
                thread.updated_at.to_rfc3339(),
                thread.effort,
                thread.provider.as_str(),
                thread.pinned,
                thread.done,
                thread.read_at.map(|t| t.to_rfc3339()),
                thread.last_activity_at.map(|t| t.to_rfc3339()),
                thread.parent_id.map(|id| id.to_string()),
                thread.notes,
                thread.recap,
                serde_json::to_string(&thread.pinned_items)?,
                thread.account,
                thread.fork_context,
                thread.goal,
                thread.goal_status.map(|s| s.as_str()),
                thread.goal_runs,
                thread.debug_mode,
                thread.budget_usd,
                thread.race_id.map(|id| id.to_string()),
                thread.felagi_issue,
            ],
        )?;
        Ok(())
    }

    pub fn threads(&self, include_archived: bool) -> Result<Vec<Thread>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {THREAD_COLUMNS} FROM threads WHERE archived = 0 OR ?1
                 ORDER BY updated_at DESC"
        ))?;
        let rows = stmt.query_map(params![include_archived], thread_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn archived_threads(&self) -> Result<Vec<Thread>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {THREAD_COLUMNS} FROM threads WHERE archived = 1 ORDER BY updated_at DESC"
        ))?;
        let rows = stmt.query_map([], thread_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Threads whose messages contain `query` (case-insensitive), newest first.
    pub fn search_messages(&self, query: &str, limit: usize) -> Result<Vec<(ThreadId, String)>> {
        let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = self.conn.prepare(
            "SELECT thread_id, content FROM transcript_items
             WHERE content LIKE ?1 ESCAPE '\\'
               AND (json_extract(content, '$.kind') IN ('user', 'assistant'))
             ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            let content: String = row.get(1)?;
            Ok((parse_uuid(row, 0)?, content))
        })?;
        let mut results = Vec::new();
        for row in rows {
            let (thread, content) = row?;
            let text = serde_json::from_str::<ItemContent>(&content)
                .ok()
                .and_then(|c| match c {
                    ItemContent::User { text, .. } | ItemContent::Assistant { text, .. } => {
                        Some(text)
                    }
                    _ => None,
                })
                .unwrap_or_default();
            results.push((thread, snippet(&text, query)));
        }
        Ok(results)
    }

    pub fn delete_thread(&self, id: ThreadId) -> Result<()> {
        self.conn
            .execute("DELETE FROM threads WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    // ---- transcript -----------------------------------------------------

    pub fn append_item(&self, thread_id: ThreadId, content: ItemContent) -> Result<TranscriptItem> {
        let seq: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM transcript_items WHERE thread_id = ?1",
            params![thread_id.to_string()],
            |row| row.get(0),
        )?;
        let item = TranscriptItem {
            id: new_id(),
            thread_id,
            seq,
            content,
            created_at: Utc::now(),
        };
        self.conn.execute(
            "INSERT INTO transcript_items (id, thread_id, seq, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                item.id.to_string(),
                thread_id.to_string(),
                item.seq,
                serde_json::to_string(&item.content)?,
                item.created_at.to_rfc3339(),
            ],
        )?;
        Ok(item)
    }

    /// Append many items in one transaction (imports and forks), keeping
    /// their original timestamps when given.
    pub fn append_items(
        &self,
        thread_id: ThreadId,
        items: impl IntoIterator<Item = (ItemContent, Option<DateTime<Utc>>)>,
    ) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM transcript_items WHERE thread_id = ?1",
            params![thread_id.to_string()],
            |row| row.get(0),
        )?;
        let mut count = 0;
        {
            let mut insert = tx.prepare(
                "INSERT INTO transcript_items (id, thread_id, seq, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (content, created_at) in items {
                seq += 1;
                count += 1;
                insert.execute(params![
                    new_id().to_string(),
                    thread_id.to_string(),
                    seq,
                    serde_json::to_string(&content)?,
                    created_at.unwrap_or_else(Utc::now).to_rfc3339(),
                ])?;
            }
        }
        tx.commit()?;
        Ok(count)
    }

    pub fn update_item(&self, item: &TranscriptItem) -> Result<()> {
        self.conn.execute(
            "UPDATE transcript_items SET content = ?2 WHERE id = ?1",
            params![item.id.to_string(), serde_json::to_string(&item.content)?],
        )?;
        Ok(())
    }

    pub fn transcript(&self, thread_id: ThreadId) -> Result<Vec<TranscriptItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, thread_id, seq, content, created_at FROM transcript_items
             WHERE thread_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], |row| {
            let content: String = row.get(3)?;
            Ok(TranscriptItem {
                id: parse_uuid(row, 0)?,
                thread_id: parse_uuid(row, 1)?,
                seq: row.get(2)?,
                content: serde_json::from_str(&content).unwrap_or(ItemContent::Notice {
                    text: "Unreadable transcript entry".into(),
                    is_error: true,
                }),
                created_at: parse_time(row, 4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- settings -------------------------------------------------------

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

// ---- orchestration: JSON documents keyed by id --------------------------

impl Store {
    fn put_doc(&self, table: &str, id: Uuid, data: &impl serde::Serialize) -> Result<()> {
        self.conn.execute(
            &format!("INSERT OR REPLACE INTO {table} (id, data) VALUES (?1, ?2)"),
            params![id.to_string(), serde_json::to_string(data)?],
        )?;
        Ok(())
    }

    fn docs<T: serde::de::DeserializeOwned>(
        &self,
        sql: &str,
        args: impl rusqlite::Params,
    ) -> Result<Vec<T>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(args, |row| row.get::<_, String>(0))?;
        let mut docs = Vec::new();
        for json in rows {
            match serde_json::from_str(&json?) {
                Ok(doc) => docs.push(doc),
                Err(err) => log::warn!("skipping unreadable record: {err}"),
            }
        }
        Ok(docs)
    }

    pub fn automations(&self) -> Result<Vec<Automation>> {
        let mut list: Vec<Automation> = self.docs("SELECT data FROM automations", [])?;
        list.sort_by_key(|a| a.created_at);
        Ok(list)
    }

    pub fn save_automation(&self, automation: &Automation) -> Result<()> {
        self.put_doc("automations", automation.id, automation)
    }

    pub fn delete_automation(&self, id: AutomationId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM automations WHERE id = ?1",
            params![id.to_string()],
        )?;
        self.conn.execute(
            "DELETE FROM automation_runs WHERE automation_id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    pub fn save_run(&self, run: &AutomationRun) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO automation_runs (id, automation_id, started_at, data)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                run.id.to_string(),
                run.automation_id.to_string(),
                run.started_at.to_rfc3339(),
                serde_json::to_string(run)?
            ],
        )?;
        Ok(())
    }

    /// Most recent runs first.
    pub fn runs(&self, automation_id: AutomationId, limit: usize) -> Result<Vec<AutomationRun>> {
        self.docs(
            "SELECT data FROM automation_runs WHERE automation_id = ?1
             ORDER BY started_at DESC LIMIT ?2",
            params![automation_id.to_string(), limit as i64],
        )
    }

    pub fn tasks(&self) -> Result<Vec<Task>> {
        let mut list: Vec<Task> = self.docs("SELECT data FROM tasks", [])?;
        list.sort_by_key(|t| t.created_at);
        Ok(list)
    }

    pub fn save_task(&self, task: &Task) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO tasks (id, project_id, data) VALUES (?1, ?2, ?3)",
            params![
                task.id.to_string(),
                task.project_id.to_string(),
                serde_json::to_string(task)?
            ],
        )?;
        Ok(())
    }

    pub fn delete_task(&self, id: TaskId) -> Result<()> {
        self.conn
            .execute("DELETE FROM tasks WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn mcp_clients(&self) -> Result<Vec<McpClient>> {
        let mut list: Vec<McpClient> = self.docs("SELECT data FROM mcp_clients", [])?;
        list.sort_by_key(|c| c.created_at);
        Ok(list)
    }

    pub fn save_mcp_client(&self, client: &McpClient) -> Result<()> {
        self.put_doc("mcp_clients", client.id, client)
    }

    pub fn delete_mcp_client(&self, id: Uuid) -> Result<()> {
        self.conn.execute(
            "DELETE FROM mcp_clients WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    // ---- line origins ---------------------------------------------------

    /// Lines a turn added (repository-relative path, [`line_hash`]): the turn
    /// is the user message `item` in `thread`.
    pub fn record_line_origins(
        &self,
        project: ProjectId,
        thread: ThreadId,
        item: ItemId,
        at: DateTime<Utc>,
        lines: &[(String, i64)],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO line_origins (project_id, path, hash, thread_id, item_id, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (path, hash) in lines {
                insert.execute(params![
                    project.to_string(),
                    path,
                    hash,
                    thread.to_string(),
                    item.to_string(),
                    at.to_rfc3339()
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The turns that wrote a line with this content in this file, newest first.
    pub fn line_origins(
        &self,
        project: ProjectId,
        path: &str,
        hash: i64,
    ) -> Result<Vec<LineOrigin>> {
        let mut stmt = self.conn.prepare(
            "SELECT thread_id, item_id, at, COUNT(*) FROM line_origins
             WHERE project_id = ?1 AND path = ?2 AND hash = ?3
             GROUP BY thread_id, item_id ORDER BY at DESC",
        )?;
        let rows = stmt.query_map(params![project.to_string(), path, hash], origin_from_row)?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect())
    }

    /// The turns that wrote lines in this file, newest first, with how many.
    pub fn file_origins(
        &self,
        project: ProjectId,
        path: &str,
        limit: usize,
    ) -> Result<Vec<LineOrigin>> {
        let mut stmt = self.conn.prepare(
            "SELECT thread_id, item_id, MAX(at), COUNT(*) FROM line_origins
             WHERE project_id = ?1 AND path = ?2
             GROUP BY thread_id, item_id ORDER BY MAX(at) DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![project.to_string(), path, limit as i64],
            origin_from_row,
        )?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect())
    }

    /// Record a gateway call; keeps the newest 5000 entries.
    pub fn audit(&self, entry: &AuditEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit_log (at, data) VALUES (?1, ?2)",
            params![entry.at.to_rfc3339(), serde_json::to_string(entry)?],
        )?;
        self.conn.execute(
            "DELETE FROM audit_log WHERE id <= (SELECT MAX(id) FROM audit_log) - 5000",
            [],
        )?;
        Ok(())
    }

    /// Newest first.
    pub fn audit_log(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        self.docs(
            "SELECT data FROM audit_log ORDER BY id DESC LIMIT ?1",
            params![limit as i64],
        )
    }

    /// Turn summaries across all threads, for usage statistics:
    /// (thread provider, finished at, duration ms, cost).
    pub fn turn_stats(&self) -> Result<Vec<TurnStat>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.provider, i.created_at,
                    json_extract(i.content, '$.duration_ms'),
                    json_extract(i.content, '$.cost_usd')
             FROM transcript_items i JOIN threads t ON t.id = i.thread_id
             WHERE json_extract(i.content, '$.kind') = 'turn_summary'",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TurnStat {
                provider: row.get(0)?,
                at: parse_time(row, 1)?,
                duration_ms: row.get::<_, Option<i64>>(2)?.map(|d| d.max(0) as u64),
                cost_usd: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn parse_uuid(row: &Row, index: usize) -> rusqlite::Result<Uuid> {
    let text: String = row.get(index)?;
    Uuid::parse_str(&text).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(err))
    })
}

fn parse_time(row: &Row, index: usize) -> rusqlite::Result<DateTime<Utc>> {
    let text: String = row.get(index)?;
    DateTime::parse_from_rfc3339(&text)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        })
}

/// The part of `text` around the first match of `query`.
fn snippet(text: &str, query: &str) -> String {
    let lower = text.to_lowercase();
    let position = lower.find(&query.to_lowercase()).unwrap_or(0);
    let start = text[..position.min(text.len())]
        .char_indices()
        .rev()
        .nth(40)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let excerpt: String = text[start..].chars().take(120).collect();
    let excerpt = excerpt.replace('\n', " ");
    if start > 0 {
        format!("…{excerpt}")
    } else {
        excerpt
    }
}

fn parse_optional_time(row: &Row, index: usize) -> rusqlite::Result<Option<DateTime<Utc>>> {
    let text: Option<String> = row.get(index)?;
    Ok(text
        .and_then(|text| DateTime::parse_from_rfc3339(&text).ok())
        .map(|time| time.with_timezone(&Utc)))
}

fn project_from_row(row: &Row) -> rusqlite::Result<Project> {
    let path: String = row.get(2)?;
    Ok(Project {
        id: parse_uuid(row, 0)?,
        name: row.get(1)?,
        path: PathBuf::from(path),
        created_at: parse_time(row, 3)?,
        pinned: row.get(4)?,
        icon: row.get(5)?,
        color: row.get(6)?,
        space: row.get(7)?,
        instructions: row.get(8)?,
        check_command: row.get(9)?,
    })
}

fn thread_from_row(row: &Row) -> rusqlite::Result<Thread> {
    let provider: String = row.get(3)?;
    let permission_mode: String = row.get(5)?;
    let environment: String = row.get(7)?;
    let status: String = row.get(8)?;
    Ok(Thread {
        id: parse_uuid(row, 0)?,
        project_id: parse_uuid(row, 1)?,
        title: row.get(2)?,
        provider: ProviderKind::parse(&provider).unwrap_or(ProviderKind::Claude),
        model: row.get(4)?,
        effort: row.get(12)?,
        permission_mode: PermissionMode::parse(&permission_mode),
        provider_session_id: row.get(6)?,
        environment: serde_json::from_str(&environment).unwrap_or(Environment::Local),
        status: ThreadStatus::parse(&status),
        archived: row.get(9)?,
        pinned: row.get(13)?,
        done: row.get(14)?,
        read_at: parse_optional_time(row, 15)?,
        last_activity_at: parse_optional_time(row, 16)?,
        parent_id: row
            .get::<_, Option<String>>(17)?
            .and_then(|id| Uuid::parse_str(&id).ok()),
        notes: row.get(18)?,
        recap: row.get(19)?,
        pinned_items: row
            .get::<_, Option<String>>(20)?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default(),
        account: row.get(21)?,
        fork_context: row.get(22)?,
        goal: row.get(23)?,
        goal_status: row
            .get::<_, Option<String>>(24)?
            .and_then(|s| GoalStatus::parse(&s)),
        goal_runs: row.get(25)?,
        debug_mode: row.get(26)?,
        budget_usd: row.get(27)?,
        race_id: row
            .get::<_, Option<String>>(28)?
            .and_then(|id| Uuid::parse_str(&id).ok()),
        felagi_issue: row.get(29)?,
        created_at: parse_time(row, 10)?,
        updated_at: parse_time(row, 11)?,
    })
}

fn origin_from_row(row: &Row) -> rusqlite::Result<Option<LineOrigin>> {
    let thread: String = row.get(0)?;
    let item: String = row.get(1)?;
    let at: String = row.get(2)?;
    let lines: i64 = row.get(3)?;
    Ok((|| {
        Some(LineOrigin {
            thread: Uuid::parse_str(&thread).ok()?,
            item: Uuid::parse_str(&item).ok()?,
            at: DateTime::parse_from_rfc3339(&at).ok()?.with_timezone(&Utc),
            lines: lines as usize,
        })
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FNV-1a: a hash that stays the same across Rust versions.
    fn fingerprint(sql: &str) -> u64 {
        sql.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    /// Fingerprints of the released migrations. A user's database has run
    /// them already, so editing one would leave their schema behind; add a
    /// new migration instead, and its fingerprint here.
    const RELEASED: &[u64] = &[
        0xdef6_a70e_381d_b908,
        0xb4b1_4f36_c158_9d7d,
        0x8d5d_cf46_bb66_b83c,
        0x249a_30b7_7de5_7741,
        0x6977_b9a8_aee3_3666,
        0x1a65_6b3c_775e_c71e,
        0x534f_4e32_21e0_c19d,
        0xf69a_d7c2_003d_7105,
        0x3228_ebaa_3834_b92a,
        0x42f8_ac9c_0227_c9c2,
    ];

    #[test]
    fn released_migrations_are_unchanged() {
        let actual: Vec<u64> = MIGRATIONS.iter().map(|sql| fingerprint(sql)).collect();
        assert!(
            actual.len() >= RELEASED.len(),
            "a released migration was removed"
        );
        for (index, (actual, released)) in actual.iter().zip(RELEASED).enumerate() {
            assert_eq!(
                actual,
                released,
                "migration {} was edited after release; add a new one instead",
                index + 1
            );
        }
        assert_eq!(
            actual.len(),
            RELEASED.len(),
            "add the new migration's fingerprint to RELEASED: {:#x}",
            actual.last().unwrap()
        );
    }

    #[test]
    fn upgrades_databases_from_every_version() {
        for version in 1..=MIGRATIONS.len() {
            // A database left by an older release, with rows written in the
            // first version's columns.
            let conn = Connection::open_in_memory().unwrap();
            for sql in &MIGRATIONS[..version] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", version as i64)
                .unwrap();
            let project = new_id();
            let thread = new_id();
            let now = Utc::now().to_rfc3339();
            let environment = serde_json::to_string(&Environment::Worktree {
                path: "/tmp/elyra-wt".into(),
                branch: "elyra/fix".into(),
            })
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, name, path, created_at) VALUES (?1, 'shop', '/tmp/shop', ?2)",
                params![project.to_string(), now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO threads (id, project_id, title, provider, model, permission_mode,
                    provider_session_id, environment, status, archived, created_at, updated_at)
                 VALUES (?1, ?2, 'Fix cart', 'claude', 'opus', 'accept_edits', 'sess', ?3,
                    'idle', 0, ?4, ?4)",
                params![thread.to_string(), project.to_string(), environment, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO transcript_items (id, thread_id, seq, content, created_at)
                 VALUES (?1, ?2, 1, '{\"kind\":\"user\",\"text\":\"hi\"}', ?3)",
                params![new_id().to_string(), thread.to_string(), now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('theme', 'dark')",
                [],
            )
            .unwrap();

            let store = Store::from_connection(conn)
                .unwrap_or_else(|err| panic!("upgrading from version {version}: {err:#}"));
            let current: i64 = store
                .conn
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(current as usize, MIGRATIONS.len(), "from version {version}");

            let projects = store.projects().unwrap();
            assert_eq!(
                (projects.len(), projects[0].id, projects[0].name.as_str()),
                (1, project, "shop"),
                "from version {version}"
            );
            let threads = store.threads(false).unwrap();
            assert_eq!(threads.len(), 1, "from version {version}");
            let loaded = &threads[0];
            assert_eq!(
                (
                    loaded.title.as_str(),
                    loaded.provider,
                    loaded.model.as_deref(),
                    loaded.permission_mode,
                    loaded.provider_session_id.as_deref(),
                ),
                (
                    "Fix cart",
                    ProviderKind::Claude,
                    Some("opus"),
                    PermissionMode::AcceptEdits,
                    Some("sess"),
                ),
                "from version {version}"
            );
            assert!(
                matches!(&loaded.environment, Environment::Worktree { branch, .. } if branch == "elyra/fix"),
                "from version {version}"
            );
            assert!(
                !loaded.pinned
                    && !loaded.done
                    && loaded.goal_runs == 0
                    && loaded.felagi_issue.is_none(),
                "new columns take their defaults (from version {version})"
            );
            assert_eq!(
                store.transcript(thread).unwrap()[0].content,
                ItemContent::User {
                    text: "hi".into(),
                    checkpoint: None
                },
                "from version {version}"
            );
            assert_eq!(store.setting("theme").unwrap().as_deref(), Some("dark"));
            // The upgraded database takes new rows in every column.
            let mut updated = loaded.clone();
            updated.felagi_issue = Some("ACM-1".into());
            updated.budget_usd = Some(1.0);
            store.update_thread(&updated).unwrap();
            assert_eq!(store.threads(false).unwrap()[0], updated);
        }
    }

    #[test]
    fn reopening_a_database_keeps_it() {
        let dir = std::env::temp_dir().join(format!("elyra-reopen-{}", std::process::id()));
        let path = dir.join("state.db");
        {
            let store = Store::open(&path).unwrap();
            store.set_setting("theme", "dark").unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.setting("theme").unwrap().as_deref(), Some("dark"));
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn project_thread_and_transcript_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let dir = std::env::temp_dir();
        let project = store.add_project(&dir).unwrap();
        assert_eq!(
            store.add_project(&dir).unwrap().id,
            project.id,
            "path is unique"
        );

        let mut thread = store
            .create_thread(
                project.id,
                ProviderKind::Claude,
                None,
                PermissionMode::Ask,
                Environment::Local,
            )
            .unwrap();
        thread.title = "Fix bug".into();
        thread.effort = Some("high".into());
        thread.pinned = true;
        thread.last_activity_at = Some(Utc::now());
        assert!(thread.is_unread());
        thread.read_at = Some(Utc::now());
        assert!(!thread.is_unread());
        thread.provider = ProviderKind::Elyra;
        thread.provider_session_id = Some("abc".into());
        store.update_thread(&thread).unwrap();
        let loaded = store.threads(false).unwrap();
        assert_eq!(loaded, vec![thread.clone()]);

        store
            .append_item(
                thread.id,
                ItemContent::User {
                    text: "hi".into(),
                    checkpoint: None,
                },
            )
            .unwrap();
        let mut second = store
            .append_item(
                thread.id,
                ItemContent::Assistant {
                    text: "he".into(),
                    parent_tool_use_id: None,
                },
            )
            .unwrap();
        second.content = ItemContent::Assistant {
            text: "hello".into(),
            parent_tool_use_id: None,
        };
        store.update_item(&second).unwrap();
        let transcript = store.transcript(thread.id).unwrap();
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[1], second);

        let other = std::env::temp_dir().join(format!("elyra-relocate-{}", std::process::id()));
        std::fs::create_dir_all(&other).unwrap();
        let moved = store.relocate_project(project.id, &other).unwrap();
        assert_eq!(moved.id, project.id);
        assert_eq!(moved.path, other.canonicalize().unwrap());
        assert_eq!(
            store.threads(false).unwrap().len(),
            1,
            "threads stay with the project"
        );
        let second = store.add_project(&dir).unwrap();
        assert!(
            store.relocate_project(second.id, &other).is_err(),
            "paths stay unique"
        );
        std::fs::remove_dir_all(&other).unwrap();

        let hits = store.search_messages("HELLO", 10).unwrap();
        assert_eq!(hits.len(), 1, "case-insensitive message search");
        assert_eq!(hits[0].1, "hello");
        let mut child = store
            .create_thread(
                project.id,
                ProviderKind::Claude,
                None,
                PermissionMode::Ask,
                Environment::Local,
            )
            .unwrap();
        child.parent_id = Some(thread.id);
        child.notes = Some("remember".into());
        child.pinned_items = vec![second.id];
        child.account = Some("work".into());
        child.fork_context = Some("earlier".into());
        store.update_thread(&child).unwrap();
        let loaded = store.threads(false).unwrap();
        let loaded = loaded.iter().find(|t| t.id == child.id).unwrap();
        assert_eq!(
            (
                loaded.parent_id,
                loaded.notes.as_deref(),
                loaded.pinned_items.len(),
                loaded.account.as_deref(),
                loaded.fork_context.as_deref(),
            ),
            (
                Some(thread.id),
                Some("remember"),
                1,
                Some("work"),
                Some("earlier")
            )
        );
        store.delete_thread(child.id).unwrap();

        let mut renamed = store.projects().unwrap()[0].clone();
        renamed.name = "Renamed".into();
        renamed.icon = Some("🚀".into());
        renamed.check_command = Some("cargo test".into());
        renamed.pinned = true;
        store.update_project(&renamed).unwrap();
        assert_eq!(store.projects().unwrap()[0], renamed);

        let mut archived = store.threads(false).unwrap()[0].clone();
        archived.archived = true;
        store.update_thread(&archived).unwrap();
        assert!(store.threads(false).unwrap().is_empty());
        assert_eq!(store.archived_threads().unwrap().len(), 1);

        store.remove_project(project.id).unwrap();
        assert!(store.threads(true).unwrap().is_empty(), "threads cascade");
    }

    #[test]
    fn stores_orchestration_records() {
        let store = Store::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("elyra-orch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = store.add_project(&dir).unwrap();

        let mut automation = Automation::new(
            "Nightly".into(),
            project.id,
            ProviderKind::Claude,
            "Run the tests".into(),
            Schedule::Daily {
                time: "02:00".into(),
                tz: "UTC".into(),
            },
        );
        store.save_automation(&automation).unwrap();
        automation.runs = 3;
        store.save_automation(&automation).unwrap();
        assert_eq!(store.automations().unwrap(), vec![automation.clone()]);
        let run = AutomationRun {
            id: new_id(),
            automation_id: automation.id,
            thread_id: None,
            started_at: Utc::now(),
            finished_at: None,
            status: RunStatus::Running,
            message: None,
        };
        store.save_run(&run).unwrap();
        assert_eq!(store.runs(automation.id, 10).unwrap(), vec![run]);
        store.delete_automation(automation.id).unwrap();
        assert!(store.runs(automation.id, 10).unwrap().is_empty());

        let task = Task::new(project.id, "Add dark mode".into());
        store.save_task(&task).unwrap();
        assert_eq!(store.tasks().unwrap(), vec![task.clone()]);
        store.delete_task(task.id).unwrap();
        assert!(store.tasks().unwrap().is_empty());

        for n in 0..3 {
            store
                .audit(&AuditEntry {
                    at: Utc::now(),
                    client: "Claude Desktop".into(),
                    tool: format!("tool{n}"),
                    detail: String::new(),
                    ok: true,
                })
                .unwrap();
        }
        assert_eq!(store.audit_log(2).unwrap()[0].tool, "tool2", "newest first");

        let mut thread = store
            .create_thread(
                project.id,
                ProviderKind::Claude,
                None,
                PermissionMode::Ask,
                Environment::Local,
            )
            .unwrap();
        thread.goal = Some("Green CI".into());
        thread.goal_status = Some(GoalStatus::Active);
        thread.goal_runs = 2;
        thread.debug_mode = true;
        thread.budget_usd = Some(2.5);
        let race = Uuid::new_v4();
        thread.race_id = Some(race);
        thread.felagi_issue = Some("ACM-231".into());
        store.update_thread(&thread).unwrap();
        let loaded = store
            .threads(false)
            .unwrap()
            .into_iter()
            .find(|t| t.id == thread.id)
            .unwrap();
        assert_eq!(
            (
                loaded.goal.as_deref(),
                loaded.goal_status,
                loaded.goal_runs,
                loaded.debug_mode
            ),
            (Some("Green CI"), Some(GoalStatus::Active), 2, true)
        );
        assert_eq!(loaded.budget_usd, Some(2.5));
        assert_eq!(loaded.race_id, Some(race));
        assert_eq!(loaded.felagi_issue.as_deref(), Some("ACM-231"));
        store
            .append_item(
                thread.id,
                ItemContent::TurnSummary {
                    duration_ms: Some(1200),
                    cost_usd: Some(0.5),
                    is_error: false,
                    context_tokens: None,
                    context_window: None,
                },
            )
            .unwrap();
        let stats = store.turn_stats().unwrap();
        assert_eq!(
            (
                stats[0].provider.as_str(),
                stats[0].duration_ms,
                stats[0].cost_usd
            ),
            ("claude", Some(1200), Some(0.5))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remembers_which_turn_wrote_a_line() {
        let store = Store::open_in_memory().unwrap();
        let (project, thread) = (new_id(), new_id());
        let (first, second) = (new_id(), new_id());
        let hash = line_hash("    return $customer->address?->street;").unwrap();
        let earlier = Utc::now() - chrono::Duration::hours(1);
        store
            .record_line_origins(
                project,
                thread,
                first,
                earlier,
                &[("app/Order.php".into(), hash)],
            )
            .unwrap();
        store
            .record_line_origins(
                project,
                thread,
                second,
                Utc::now(),
                &[("app/Order.php".into(), hash), ("app/Order.php".into(), 7)],
            )
            .unwrap();
        let origins = store.line_origins(project, "app/Order.php", hash).unwrap();
        assert_eq!(origins.len(), 2);
        assert_eq!(origins[0].item, second, "newest first");
        assert!(
            store
                .line_origins(project, "other.php", hash)
                .unwrap()
                .is_empty()
        );
        let file = store.file_origins(project, "app/Order.php", 10).unwrap();
        assert_eq!((file[0].item, file[0].lines), (second, 2));
        assert_eq!(line_hash("  }  "), None, "too common to track");
        assert_eq!(line_hash("return x;"), line_hash("\treturn x;  "));
    }
}
