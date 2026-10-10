//! elyra-index: a tree-sitter symbol index per project.
//!
//! From Elyra Desktop, where it was written; Elyra Workspace offers it to every
//! agent as the gateway's `symbols` tool. Source files are parsed with
//! tree-sitter and the grammars' `tags.scm` queries turn definitions
//! (`@definition.function`, `.method`, `.class`, `.interface`, `.module`, …)
//! and references (`@reference.call`, `.class`, `.implementation`, …) into
//! rows of a SQLite database, a rebuildable cache stored outside the project.
//! The `symbols` tool answers "where is X defined, who calls X" from it, and
//! the `read` tool uses [`extract`] for outlines of large files in languages
//! the regex outlines do not cover.
//!
//! Supported: Rust, TypeScript, TSX, JavaScript, Python, Go, PHP and C.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use streaming_iterator::StreamingIterator;
use thiserror::Error;
use tree_sitter::{Parser, Query, QueryCursor};

const SCHEMA_VERSION: i64 = 1;
/// Files larger than this are not parsed.
pub const MAX_FILE_BYTES: u64 = 1_000_000;
const MAX_SIGNATURE_CHARS: usize = 120;
const MAX_TEXT_CHARS: usize = 160;
/// Directories skipped even when no `.gitignore` excludes them.
const SKIPPED_DIRS: [&str; 8] = [
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    ".next",
    ".svelte-kit",
];

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("symbol index database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("symbol index I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tree-sitter language error: {0}")]
    Language(#[from] tree_sitter::LanguageError),
    #[error("tree-sitter query error: {0}")]
    Query(#[from] tree_sitter::QueryError),
    #[error("parse failed")]
    Parse,
}

// ── languages ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Php,
    C,
}

impl Language {
    pub const ALL: [Language; 8] = [
        Language::Rust,
        Language::TypeScript,
        Language::Tsx,
        Language::JavaScript,
        Language::Python,
        Language::Go,
        Language::Php,
        Language::C,
    ];

    /// Language of a file by extension.
    pub fn from_path(path: &Path) -> Option<Language> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Language::Rust,
            "ts" | "mts" | "cts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "py" | "pyi" => Language::Python,
            "go" => Language::Go,
            "php" => Language::Php,
            "c" | "h" => Language::C,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::Php => "php",
            Language::C => "c",
        }
    }

    pub fn parse_name(name: &str) -> Option<Language> {
        Language::ALL.into_iter().find(|l| l.name() == name)
    }

    pub fn grammar(self) -> tree_sitter::Language {
        match self {
            Language::Rust => tree_sitter::Language::new(tree_sitter_rust::LANGUAGE),
            Language::TypeScript => {
                tree_sitter::Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT)
            }
            Language::Tsx => tree_sitter::Language::new(tree_sitter_typescript::LANGUAGE_TSX),
            Language::JavaScript => tree_sitter::Language::new(tree_sitter_javascript::LANGUAGE),
            Language::Python => tree_sitter::Language::new(tree_sitter_python::LANGUAGE),
            Language::Go => tree_sitter::Language::new(tree_sitter_go::LANGUAGE),
            Language::Php => tree_sitter::Language::new(tree_sitter_php::LANGUAGE_PHP),
            Language::C => tree_sitter::Language::new(tree_sitter_c::LANGUAGE),
        }
    }

    /// The grammar's `tags.scm`; TypeScript inherits JavaScript's patterns.
    pub fn tags_query(self) -> String {
        match self {
            Language::Rust => tree_sitter_rust::TAGS_QUERY.to_string(),
            Language::TypeScript | Language::Tsx => format!(
                "{}\n{}",
                tree_sitter_javascript::TAGS_QUERY,
                tree_sitter_typescript::TAGS_QUERY
            ),
            Language::JavaScript => tree_sitter_javascript::TAGS_QUERY.to_string(),
            Language::Python => tree_sitter_python::TAGS_QUERY.to_string(),
            Language::Go => tree_sitter_go::TAGS_QUERY.to_string(),
            Language::Php => tree_sitter_php::TAGS_QUERY.to_string(),
            Language::C => tree_sitter_c::TAGS_QUERY.to_string(),
        }
    }
}

static QUERIES: LazyLock<Mutex<HashMap<Language, Arc<Query>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn compiled_query(language: Language) -> Result<Arc<Query>, IndexError> {
    if let Some(query) = QUERIES.lock().get(&language) {
        return Ok(query.clone());
    }
    let grammar = language.grammar();
    let query = match Query::new(&grammar, &language.tags_query()) {
        Ok(query) => query,
        // A combined query can fail when the JavaScript patterns name nodes the
        // TypeScript grammar renamed; fall back to the language's own file.
        Err(_) if matches!(language, Language::TypeScript | Language::Tsx) => {
            Query::new(&grammar, tree_sitter_typescript::TAGS_QUERY)?
        }
        Err(err) => return Err(err.into()),
    };
    let query = Arc::new(query);
    QUERIES.lock().insert(language, query.clone());
    Ok(query)
}

// ── extraction ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Definition {
    pub name: String,
    /// `function`, `method`, `class`, `interface`, `module`, `macro`, `type`, `constant`, …
    pub kind: String,
    /// 1-based.
    pub line: u32,
    /// 0-based column of the definition node.
    pub col: u32,
    pub end_line: u32,
    /// Name of the innermost enclosing definition (class, module, impl target).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    /// First line of the definition, trimmed.
    pub signature: String,
    #[serde(skip)]
    start_byte: usize,
    #[serde(skip)]
    end_byte: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub name: String,
    /// `call`, `class`, `implementation`, `type`, `module`, …
    pub kind: String,
    pub line: u32,
    pub col: u32,
    /// The source line, trimmed.
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extraction {
    pub definitions: Vec<Definition>,
    pub references: Vec<Reference>,
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Parse `source` as `language` and collect definitions and references.
pub fn extract(language: Language, source: &str) -> Result<Extraction, IndexError> {
    let mut parser = Parser::new();
    parser.set_language(&language.grammar())?;
    let tree = parser.parse(source, None).ok_or(IndexError::Parse)?;
    let query = compiled_query(language)?;
    let names = query.capture_names();
    let bytes = source.as_bytes();
    let lines: Vec<&str> = source.split('\n').collect();

    let mut definitions: Vec<Definition> = Vec::new();
    let mut references: Vec<Reference> = Vec::new();
    let mut seen_defs: std::collections::HashSet<(usize, String)> =
        std::collections::HashSet::new();
    let mut seen_refs: std::collections::HashSet<(usize, String)> =
        std::collections::HashSet::new();
    // Blocks that enclose definitions without being one themselves (Rust `impl` blocks).
    let mut containers: Vec<(usize, usize, String)> = Vec::new();

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        let mut tag: Option<(&str, tree_sitter::Node)> = None;
        let mut name_node: Option<tree_sitter::Node> = None;
        for capture in m.captures {
            let capture_name = names[capture.index as usize];
            if capture_name == "name" {
                name_node = Some(capture.node);
            } else if capture_name.starts_with("definition.")
                || capture_name.starts_with("reference.")
            {
                tag = Some((capture_name, capture.node));
            }
        }
        let (Some((tag, node)), Some(name_node)) = (tag, name_node) else {
            continue;
        };
        let Ok(name) = name_node.utf8_text(bytes) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let start = node.start_position();
        if let Some(kind) = tag.strip_prefix("definition.") {
            if !seen_defs.insert((node.start_byte(), name.to_string())) {
                continue;
            }
            let signature = lines
                .get(start.row)
                .map(|l| truncate_chars(l.trim(), MAX_SIGNATURE_CHARS))
                .unwrap_or_default();
            definitions.push(Definition {
                name: name.to_string(),
                kind: kind.to_string(),
                line: start.row as u32 + 1,
                col: start.column as u32,
                end_line: node.end_position().row as u32 + 1,
                container: None,
                signature,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
            });
        } else if let Some(kind) = tag.strip_prefix("reference.") {
            if kind == "implementation" {
                // `impl Trait for Type { .. }`: methods inside belong to `Type`.
                let container_name = node
                    .child_by_field_name("type")
                    .and_then(|n| n.utf8_text(bytes).ok())
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .unwrap_or(name)
                    .to_string();
                containers.push((node.start_byte(), node.end_byte(), container_name));
            }
            let name_start = name_node.start_position();
            if !seen_refs.insert((name_node.start_byte(), name.to_string())) {
                continue;
            }
            references.push(Reference {
                name: name.to_string(),
                kind: kind.to_string(),
                line: name_start.row as u32 + 1,
                col: name_start.column as u32,
                text: lines
                    .get(name_start.row)
                    .map(|l| truncate_chars(l.trim(), MAX_TEXT_CHARS))
                    .unwrap_or_default(),
            });
        }
    }

    // Containers: the innermost other definition whose range encloses this one.
    definitions.sort_by_key(|d| (d.start_byte, std::cmp::Reverse(d.end_byte)));
    let mut ranges: Vec<(usize, usize, String)> = definitions
        .iter()
        .map(|d| (d.start_byte, d.end_byte, d.name.clone()))
        .collect();
    ranges.extend(containers);
    for def in &mut definitions {
        let mut best: Option<&(usize, usize, String)> = None;
        for range in &ranges {
            if range.0 <= def.start_byte
                && range.1 >= def.end_byte
                && !(range.0 == def.start_byte && range.1 == def.end_byte)
                && best.is_none_or(|b| range.1 - range.0 < b.1 - b.0)
            {
                best = Some(range);
            }
        }
        def.container = best.map(|b| b.2.clone());
    }
    references.sort_by_key(|r| (r.line, r.col));
    Ok(Extraction {
        definitions,
        references,
    })
}

// ── index ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReport {
    pub indexed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub files: usize,
    pub symbols: usize,
    pub references: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub files: usize,
    pub symbols: usize,
    pub references: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<String>,
    pub path: String,
    pub root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolHit {
    /// Path relative to the project root, `/` separated.
    pub file: String,
    pub language: String,
    pub name: String,
    pub kind: String,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceHit {
    pub file: String,
    pub language: String,
    pub name: String,
    pub kind: String,
    pub line: u32,
    pub col: u32,
    pub text: String,
    /// Name of the definition enclosing the reference, when one is indexed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
}

/// `<index_dir>/symbols/<encoded root>.sqlite` (`/`, `\` and `:` become `-`).
pub fn default_db_path(index_dir: &Path, root: &Path) -> PathBuf {
    let text = root.to_string_lossy();
    let stripped = text
        .strip_prefix('/')
        .or_else(|| text.strip_prefix('\\'))
        .unwrap_or(&text);
    let encoded: String = stripped
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') {
                '-'
            } else {
                c
            }
        })
        .collect();
    index_dir
        .join("symbols")
        .join(format!("--{encoded}--.sqlite"))
}

pub struct SymbolIndex {
    conn: Connection,
    path: PathBuf,
    root: PathBuf,
}

impl std::fmt::Debug for SymbolIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SymbolIndex")
            .field("path", &self.path)
            .field("root", &self.root)
            .finish()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn to_posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl SymbolIndex {
    /// Open (creating) the database at `db_path` for the project at `root`.
    pub fn open(root: &Path, db_path: &Path) -> Result<SymbolIndex, IndexError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let mut index = SymbolIndex {
            conn,
            path: db_path.to_path_buf(),
            root: root.to_path_buf(),
        };
        index.ensure_schema()?;
        Ok(index)
    }

    pub fn open_in_memory(root: &Path) -> Result<SymbolIndex, IndexError> {
        let mut index = SymbolIndex {
            conn: Connection::open_in_memory()?,
            path: PathBuf::from(":memory:"),
            root: root.to_path_buf(),
        };
        index.ensure_schema()?;
        Ok(index)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn ensure_schema(&mut self) -> Result<(), IndexError> {
        let has_meta = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
            [],
            |r| r.get::<_, i64>(0),
        )? > 0;
        let version: Option<i64> = if has_meta {
            self.conn
                .query_row(
                    "SELECT value FROM meta WHERE key = 'schema_version'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .and_then(|v| v.parse().ok())
        } else {
            None
        };
        if version.is_some_and(|v| v != SCHEMA_VERSION) {
            self.conn.execute_batch(
				"DROP TABLE IF EXISTS symbols; DROP TABLE IF EXISTS refs; DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS meta;",
			)?;
        }
        self.conn.execute_batch(&format!(
			"CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
			 INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', '{SCHEMA_VERSION}');
			 CREATE TABLE IF NOT EXISTS files(
				path TEXT PRIMARY KEY, language TEXT NOT NULL, size INTEGER NOT NULL, mtime_ms INTEGER NOT NULL);
			 CREATE TABLE IF NOT EXISTS symbols(
				file TEXT NOT NULL, name TEXT NOT NULL, name_lower TEXT NOT NULL, kind TEXT NOT NULL,
				line INTEGER NOT NULL, col INTEGER NOT NULL, end_line INTEGER NOT NULL,
				container TEXT, signature TEXT NOT NULL);
			 CREATE INDEX IF NOT EXISTS symbols_name ON symbols(name_lower);
			 CREATE INDEX IF NOT EXISTS symbols_file ON symbols(file, line);
			 CREATE TABLE IF NOT EXISTS refs(
				file TEXT NOT NULL, name TEXT NOT NULL, name_lower TEXT NOT NULL, kind TEXT NOT NULL,
				line INTEGER NOT NULL, col INTEGER NOT NULL, text TEXT NOT NULL);
			 CREATE INDEX IF NOT EXISTS refs_name ON refs(name_lower);
			 CREATE INDEX IF NOT EXISTS refs_file ON refs(file);"
		))?;
        Ok(())
    }

    fn walk(&self) -> Vec<(PathBuf, Language, u64, i64)> {
        let mut files = Vec::new();
        let mut builder = ignore::WalkBuilder::new(&self.root);
        builder
            .hidden(true)
            .ignore(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .parents(true)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| {
                !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| SKIPPED_DIRS.contains(&name))
            });
        for entry in builder.build().filter_map(Result::ok) {
            let path = entry.path();
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let Some(language) = Language::from_path(path) else {
                continue;
            };
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
            let mtime = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            files.push((path.to_path_buf(), language, meta.len(), mtime));
        }
        files
    }

    /// Index new and changed files, drop rows of deleted files.
    pub fn refresh(&mut self) -> Result<RefreshReport, IndexError> {
        let mut report = RefreshReport::default();
        let on_disk = self.walk();
        let stored: HashMap<String, (i64, i64)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT path, size, mtime_ms FROM files")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get::<_, i64>(1)?, r.get::<_, i64>(2)?),
                ))
            })?;
            rows.filter_map(Result::ok).collect()
        };
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (path, language, size, mtime) in &on_disk {
            let rel = to_posix(path.strip_prefix(&self.root).unwrap_or(path));
            seen.insert(rel.clone());
            if stored.get(&rel) == Some(&(*size as i64, *mtime)) {
                report.unchanged += 1;
                continue;
            }
            match self.index_file(path, &rel, *language, *size as i64, *mtime) {
                Ok(()) => report.indexed += 1,
                Err(err) => report.errors.push(format!("{rel}: {err}")),
            }
        }
        for path in stored.keys().filter(|p| !seen.contains(*p)) {
            self.remove_file(path)?;
            report.removed += 1;
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('last_refresh_ms', ?1)",
            params![now_ms().to_string()],
        )?;
        let stats = self.stats()?;
        report.files = stats.files;
        report.symbols = stats.symbols;
        report.references = stats.references;
        Ok(report)
    }

    /// Refresh unless the last refresh is younger than `max_age`.
    pub fn refresh_if_stale(
        &mut self,
        max_age: Duration,
    ) -> Result<Option<RefreshReport>, IndexError> {
        let last: Option<i64> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'last_refresh_ms'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|v| v.parse().ok());
        if let Some(last) = last
            && now_ms() - last < max_age.as_millis() as i64
        {
            return Ok(None);
        }
        self.refresh().map(Some)
    }

    /// Re-index one file (after an edit); removes its rows when it no longer exists or is unsupported.
    pub fn refresh_file(&mut self, path: &Path) -> Result<bool, IndexError> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        let rel = to_posix(absolute.strip_prefix(&self.root).unwrap_or(&absolute));
        let (Some(language), Ok(meta)) =
            (Language::from_path(&absolute), std::fs::metadata(&absolute))
        else {
            self.remove_file(&rel)?;
            return Ok(false);
        };
        if meta.len() > MAX_FILE_BYTES {
            self.remove_file(&rel)?;
            return Ok(false);
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.index_file(&absolute, &rel, language, meta.len() as i64, mtime)?;
        Ok(true)
    }

    fn remove_file(&mut self, rel: &str) -> Result<(), IndexError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM symbols WHERE file = ?1", params![rel])?;
        tx.execute("DELETE FROM refs WHERE file = ?1", params![rel])?;
        tx.execute("DELETE FROM files WHERE path = ?1", params![rel])?;
        tx.commit()?;
        Ok(())
    }

    fn index_file(
        &mut self,
        path: &Path,
        rel: &str,
        language: Language,
        size: i64,
        mtime: i64,
    ) -> Result<(), IndexError> {
        let source = std::fs::read_to_string(path)?;
        let extraction = extract(language, &source)?;
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM symbols WHERE file = ?1", params![rel])?;
        tx.execute("DELETE FROM refs WHERE file = ?1", params![rel])?;
        {
            let mut insert_symbol = tx.prepare(
				"INSERT INTO symbols(file, name, name_lower, kind, line, col, end_line, container, signature)
				 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
			)?;
            for def in &extraction.definitions {
                insert_symbol.execute(params![
                    rel,
                    def.name,
                    def.name.to_lowercase(),
                    def.kind,
                    def.line,
                    def.col,
                    def.end_line,
                    def.container,
                    def.signature,
                ])?;
            }
            let mut insert_ref = tx.prepare(
				"INSERT INTO refs(file, name, name_lower, kind, line, col, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
			)?;
            for reference in &extraction.references {
                insert_ref.execute(params![
                    rel,
                    reference.name,
                    reference.name.to_lowercase(),
                    reference.kind,
                    reference.line,
                    reference.col,
                    reference.text,
                ])?;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO files(path, language, size, mtime_ms) VALUES (?1, ?2, ?3, ?4)",
            params![rel, language.name(), size, mtime],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Definitions named `name` (exact, case-insensitive) or containing it.
    pub fn definitions(
        &self,
        name: &str,
        exact: bool,
        path_prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SymbolHit>, IndexError> {
        let needle = name.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let pattern = if exact {
            needle.clone()
        } else {
            format!("%{}%", escape_like(&needle))
        };
        let prefix = path_prefix.map(|p| {
            format!(
                "{}%",
                escape_like(p.replace('\\', "/").trim_end_matches('/'))
            )
        });
        let limit = if limit == 0 { 50 } else { limit } as i64;
        let mut stmt = self.conn.prepare(
			"SELECT s.file, f.language, s.name, s.kind, s.line, s.col, s.end_line, s.container, s.signature
			 FROM symbols s JOIN files f ON f.path = s.file
			 WHERE (CASE WHEN ?2 THEN s.name_lower = ?1 ELSE s.name_lower LIKE ?1 ESCAPE '\\' END)
			   AND (?3 IS NULL OR s.file LIKE ?3 ESCAPE '\\')
			 ORDER BY (s.name_lower = ?4) DESC, s.file, s.line
			 LIMIT ?5",
		)?;
        let rows = stmt.query_map(params![pattern, exact, prefix, needle, limit], |r| {
            Ok(SymbolHit {
                file: r.get(0)?,
                language: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                line: r.get(4)?,
                col: r.get(5)?,
                end_line: r.get(6)?,
                container: r.get(7)?,
                signature: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// References to `name` (exact, case-insensitive), with the enclosing definition when known.
    pub fn references(
        &self,
        name: &str,
        path_prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ReferenceHit>, IndexError> {
        let needle = name.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let prefix = path_prefix.map(|p| {
            format!(
                "{}%",
                escape_like(p.replace('\\', "/").trim_end_matches('/'))
            )
        });
        let limit = if limit == 0 { 100 } else { limit } as i64;
        let mut stmt = self.conn.prepare(
			"SELECT r.file, f.language, r.name, r.kind, r.line, r.col, r.text,
			        (SELECT s.name FROM symbols s WHERE s.file = r.file AND s.line <= r.line AND s.end_line >= r.line
			         ORDER BY (s.end_line - s.line) ASC LIMIT 1)
			 FROM refs r JOIN files f ON f.path = r.file
			 WHERE r.name_lower = ?1 AND (?2 IS NULL OR r.file LIKE ?2 ESCAPE '\\')
			 ORDER BY r.file, r.line
			 LIMIT ?3",
		)?;
        let rows = stmt.query_map(params![needle, prefix, limit], |r| {
            Ok(ReferenceHit {
                file: r.get(0)?,
                language: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                line: r.get(4)?,
                col: r.get(5)?,
                text: r.get(6)?,
                container: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Definitions of one file in line order (a structural outline).
    pub fn outline(&self, file: &str) -> Result<Vec<SymbolHit>, IndexError> {
        let rel = file.replace('\\', "/");
        let mut stmt = self.conn.prepare(
			"SELECT s.file, f.language, s.name, s.kind, s.line, s.col, s.end_line, s.container, s.signature
			 FROM symbols s JOIN files f ON f.path = s.file WHERE s.file = ?1 ORDER BY s.line, s.col",
		)?;
        let rows = stmt.query_map(params![rel], |r| {
            Ok(SymbolHit {
                file: r.get(0)?,
                language: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                line: r.get(4)?,
                col: r.get(5)?,
                end_line: r.get(6)?,
                container: r.get(7)?,
                signature: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn stats(&self) -> Result<IndexStats, IndexError> {
        let count = |sql: &str| -> Result<usize, IndexError> {
            Ok(self.conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as usize)
        };
        let last_refresh = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'last_refresh_ms'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(IndexStats {
            files: count("SELECT COUNT(*) FROM files")?,
            symbols: count("SELECT COUNT(*) FROM symbols")?,
            references: count("SELECT COUNT(*) FROM refs")?,
            last_refresh,
            path: self.path.display().to_string(),
            root: self.root.display().to_string(),
        })
    }
}

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// What to look up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Definitions,
    References,
    All,
}

impl Mode {
    pub fn parse(value: Option<&str>) -> Result<Mode, String> {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("all") => Ok(Mode::All),
            Some("definitions" | "definition" | "defs" | "def") => Ok(Mode::Definitions),
            Some("references" | "reference" | "refs" | "ref" | "callers") => Ok(Mode::References),
            Some(other) => Err(format!(
                "Unknown mode \"{other}\"; use definitions, references or all"
            )),
        }
    }
}

/// Don't rescan the project more often than this.
pub const REFRESH_MAX_AGE: Duration = Duration::from_secs(3);

/// Look `name` up in the index of the project at `root` (refreshing it when
/// stale) and answer in text for an agent: definitions with signatures,
/// references with the enclosing definition, and the index size.
pub fn lookup(
    root: &Path,
    db_path: &Path,
    name: &str,
    mode: Mode,
    exact: bool,
    path: Option<&str>,
    limit: usize,
) -> Result<String, IndexError> {
    let limit = if limit == 0 { 50 } else { limit };
    let mut index = SymbolIndex::open(root, db_path)?;
    let refreshed = index.refresh_if_stale(REFRESH_MAX_AGE)?;
    let stats = index.stats()?;
    if stats.files == 0 {
        return Ok("The symbol index is empty: no Rust, TypeScript, JavaScript, Python, Go, PHP or C files under \
		 the project directory (respecting .gitignore). Use grep instead."
			.into());
    }
    let path_prefix = path.map(|p| p.trim_start_matches("./").trim_end_matches('/'));
    let mut sections: Vec<String> = Vec::new();
    if mode != Mode::References {
        let definitions = index.definitions(name, exact, path_prefix, limit)?;
        if definitions.is_empty() && exact {
            // Nothing exact: partial matches still help with a typo or a shorter name.
            let partial = index.definitions(name, false, path_prefix, limit)?;
            sections.push(if partial.is_empty() {
                format!("No definition of \"{name}\" in the index.")
            } else {
                format!(
                    "No definition named exactly \"{name}\". Partial matches:\n{}",
                    format_definitions(&partial)
                )
            });
        } else if definitions.is_empty() {
            sections.push(format!("No definition matching \"{name}\" in the index."));
        } else {
            let more = if definitions.len() >= limit {
                " (limit reached)"
            } else {
                ""
            };
            sections.push(format!(
                "Definitions ({}{more}):\n{}",
                definitions.len(),
                format_definitions(&definitions)
            ));
        }
    }
    if mode != Mode::Definitions {
        let references = index.references(name, path_prefix, limit)?;
        sections.push(if references.is_empty() {
            format!("No references to \"{name}\" in the index.")
        } else {
            let more = if references.len() >= limit {
                " (limit reached)"
            } else {
                ""
            };
            format!(
                "References ({}{more}):\n{}",
                references.len(),
                format_references(&references)
            )
        });
    }
    let mut footer = format!(
        "Index: {} files, {} definitions, {} references",
        stats.files, stats.symbols, stats.references
    );
    if let Some(report) = refreshed
        && (report.indexed > 0 || report.removed > 0)
    {
        footer.push_str(&format!(
            " ({} re-indexed, {} removed)",
            report.indexed, report.removed
        ));
    }
    sections.push(footer);
    Ok(sections.join("\n\n"))
}

/// Plain-text rendering of definitions for tools and the CLI.
pub fn format_definitions(hits: &[SymbolHit]) -> String {
    hits.iter()
        .map(|h| {
            let container = h
                .container
                .as_deref()
                .map(|c| format!(" in {c}"))
                .unwrap_or_default();
            format!(
                "{}:{}:{}  {} {}{}  {}",
                h.file,
                h.line,
                h.col + 1,
                h.kind,
                h.name,
                container,
                h.signature
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Plain-text rendering of references for tools and the CLI.
pub fn format_references(hits: &[ReferenceHit]) -> String {
    hits.iter()
        .map(|h| {
            let container = h
                .container
                .as_deref()
                .map(|c| format!(" in {c}"))
                .unwrap_or_default();
            format!(
                "{}:{}:{}  {}{}  {}",
                h.file,
                h.line,
                h.col + 1,
                h.kind,
                container,
                h.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST_SRC: &str = r#"
pub struct Session {
	id: String,
}

impl Session {
	pub fn new(id: &str) -> Self {
		Session { id: id.to_string() }
	}

	pub fn open(&self) -> bool {
		helper(&self.id)
	}
}

fn helper(id: &str) -> bool {
	!id.is_empty()
}

pub trait Store {
	fn save(&self);
}
"#;

    const TS_SRC: &str = r#"
export interface Options { limit: number }
export class Runtime {
	start(): void { this.tick(); }
	tick(): void {}
}
export function boot(options: Options): Runtime {
	const rt = new Runtime();
	rt.start();
	return rt;
}
"#;

    const PY_SRC: &str = "class Agent:\n    def run(self):\n        return helper()\n\ndef helper():\n    return 1\n";

    #[test]
    fn detects_languages() {
        assert_eq!(
            Language::from_path(Path::new("a/b.rs")),
            Some(Language::Rust)
        );
        assert_eq!(Language::from_path(Path::new("x.tsx")), Some(Language::Tsx));
        assert_eq!(
            Language::from_path(Path::new("x.mjs")),
            Some(Language::JavaScript)
        );
        assert_eq!(Language::from_path(Path::new("x.php")), Some(Language::Php));
        assert_eq!(Language::from_path(Path::new("x.h")), Some(Language::C));
        assert_eq!(Language::from_path(Path::new("x.md")), None);
        assert_eq!(Language::parse_name("go"), Some(Language::Go));
        for language in Language::ALL {
            compiled_query(language).unwrap_or_else(|e| panic!("{}: {e}", language.name()));
        }
    }

    #[test]
    fn extracts_rust_definitions_references_and_containers() {
        let extraction = extract(Language::Rust, RUST_SRC).unwrap();
        let names: Vec<(&str, &str, Option<&str>)> = extraction
            .definitions
            .iter()
            .map(|d| (d.kind.as_str(), d.name.as_str(), d.container.as_deref()))
            .collect();
        assert!(names.contains(&("class", "Session", None)), "{names:?}");
        assert!(
            names.contains(&("method", "new", Some("Session"))),
            "{names:?}"
        );
        assert!(
            names.contains(&("method", "open", Some("Session"))),
            "{names:?}"
        );
        assert!(names.contains(&("function", "helper", None)), "{names:?}");
        assert!(names.contains(&("interface", "Store", None)), "{names:?}");
        let new_def = extraction
            .definitions
            .iter()
            .find(|d| d.name == "new")
            .unwrap();
        assert_eq!(new_def.line, 7);
        assert_eq!(new_def.signature, "pub fn new(id: &str) -> Self {");
        let calls: Vec<&str> = extraction
            .references
            .iter()
            .filter(|r| r.kind == "call")
            .map(|r| r.name.as_str())
            .collect();
        assert!(calls.contains(&"helper"), "{calls:?}");
        assert!(calls.contains(&"is_empty"), "{calls:?}");
        let implementation = extraction
            .references
            .iter()
            .find(|r| r.kind == "implementation")
            .unwrap();
        assert_eq!(implementation.name, "Session");
    }

    #[test]
    fn extracts_typescript_and_python() {
        let ts = extract(Language::TypeScript, TS_SRC).unwrap();
        let kinds: Vec<(&str, &str)> = ts
            .definitions
            .iter()
            .map(|d| (d.kind.as_str(), d.name.as_str()))
            .collect();
        assert!(kinds.contains(&("interface", "Options")), "{kinds:?}");
        assert!(kinds.contains(&("class", "Runtime")), "{kinds:?}");
        assert!(kinds.contains(&("method", "start")), "{kinds:?}");
        assert!(kinds.contains(&("function", "boot")), "{kinds:?}");
        assert!(
            ts.references
                .iter()
                .any(|r| r.name == "Runtime" && r.kind == "class")
        );
        assert!(
            ts.references
                .iter()
                .any(|r| r.name == "start" && r.kind == "call")
        );

        let py = extract(Language::Python, PY_SRC).unwrap();
        let run = py.definitions.iter().find(|d| d.name == "run").unwrap();
        assert_eq!(run.container.as_deref(), Some("Agent"));
        assert!(
            py.references
                .iter()
                .any(|r| r.name == "helper" && r.kind == "call")
        );
    }

    #[test]
    fn indexes_a_project_incrementally() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/dep")).unwrap();
        std::fs::write(root.join("src/lib.rs"), RUST_SRC).unwrap();
        std::fs::write(root.join("src/app.ts"), TS_SRC).unwrap();
        std::fs::write(
            root.join("node_modules/dep/index.js"),
            "function ignored() {}",
        )
        .unwrap();
        std::fs::write(root.join("README.md"), "# not code").unwrap();

        let mut index = SymbolIndex::open_in_memory(root).unwrap();
        let report = index.refresh().unwrap();
        assert_eq!(report.indexed, 2, "{report:?}");
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.files, 2);

        let defs = index.definitions("Session", true, None, 0).unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].file, "src/lib.rs");
        assert_eq!(defs[0].kind, "class");
        let fuzzy = index.definitions("run", false, None, 0).unwrap();
        assert!(fuzzy.iter().any(|d| d.name == "Runtime"));
        let scoped = index
            .definitions("Runtime", true, Some("src/lib"), 0)
            .unwrap();
        assert!(scoped.is_empty());

        let refs = index.references("helper", None, 0).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].container.as_deref(), Some("open"));
        assert!(refs[0].text.contains("helper(&self.id)"));

        let outline = index.outline("src/app.ts").unwrap();
        assert_eq!(outline.first().map(|h| h.name.as_str()), Some("Options"));

        // Unchanged files are skipped, edits are picked up, deletions are dropped.
        let second = index.refresh().unwrap();
        assert_eq!((second.indexed, second.unchanged), (0, 2));
        std::fs::write(root.join("src/lib.rs"), "fn renamed() {}\n").unwrap();
        // Force a different mtime/size fingerprint even on coarse filesystems.
        assert!(index.refresh_file(&root.join("src/lib.rs")).unwrap());
        assert!(
            index
                .definitions("Session", true, None, 0)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            index.definitions("renamed", true, None, 0).unwrap().len(),
            1
        );
        std::fs::remove_file(root.join("src/app.ts")).unwrap();
        let third = index.refresh().unwrap();
        assert_eq!(third.removed, 1);
        assert_eq!(index.stats().unwrap().files, 1);
        assert!(
            index
                .refresh_if_stale(Duration::from_secs(60))
                .unwrap()
                .is_none()
        );

        let text = format_definitions(&index.definitions("renamed", true, None, 0).unwrap());
        assert!(text.starts_with("src/lib.rs:1:1  function renamed  fn renamed() {}"));
        assert_eq!(
            default_db_path(Path::new("/agent/index"), Path::new("/Users/kh/proj")),
            PathBuf::from("/agent/index/symbols/--Users-kh-proj--.sqlite")
        );
    }

    #[test]
    fn looks_symbols_up_for_an_agent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), RUST_SRC).unwrap();
        let db = dir.path().join("index.sqlite");
        let text = lookup(dir.path(), &db, "helper", Mode::All, true, None, 0).unwrap();
        assert!(text.contains("Definitions (1):\nlib.rs:"), "{text}");
        assert!(text.contains("References (1):\nlib.rs:"), "{text}");
        assert!(text.contains("in open"), "the caller: {text}");
        let partial = lookup(dir.path(), &db, "sess", Mode::Definitions, true, None, 0).unwrap();
        assert!(partial.contains("Partial matches"), "{partial}");
        assert_eq!(Mode::parse(Some("callers")), Ok(Mode::References));
        assert!(Mode::parse(Some("everything")).is_err());
    }
}
