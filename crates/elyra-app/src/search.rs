//! Content search across a working directory (respects .gitignore).

use std::io::Read as _;
use std::path::Path;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// Path relative to the searched root.
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    pub text: String,
}

/// Lines containing `query`. Case-insensitive unless the query has an
/// uppercase letter ("smart case").
pub fn search(root: &Path, query: &str, limit: usize) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let case_sensitive = query.chars().any(char::is_uppercase);
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let mut matches = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    for entry in walker.flatten() {
        if matches.len() >= limit {
            break;
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if entry
            .metadata()
            .map(|m| m.len() > MAX_FILE_BYTES)
            .unwrap_or(true)
        {
            continue;
        }
        let mut bytes = Vec::new();
        if std::fs::File::open(entry.path())
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .is_err()
        {
            continue;
        }
        if bytes.iter().take(8000).any(|b| *b == 0) {
            continue; // binary
        }
        let text = String::from_utf8_lossy(&bytes);
        let relative = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        for (index, line) in text.lines().enumerate() {
            let found = if case_sensitive {
                line.contains(&needle)
            } else {
                line.to_lowercase().contains(&needle)
            };
            if found {
                matches.push(Match {
                    path: relative.clone(),
                    line: index + 1,
                    text: line.trim().chars().take(200).collect(),
                });
                if matches.len() >= limit {
                    break;
                }
            }
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::search;

    #[test]
    fn finds_lines_with_smart_case() {
        let dir = std::env::temp_dir().join(format!("elyra-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn Parse() {}\nlet parse = 1;\n").unwrap();
        std::fs::write(dir.join("bin.dat"), [0u8, 1, 2, b'p']).unwrap();
        let all = search(&dir, "parse", 10);
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].path.as_str(), all[0].line), ("src/a.rs", 1));
        let exact = search(&dir, "Parse", 10);
        assert_eq!(exact.len(), 1);
        assert_eq!(search(&dir, "parse", 1).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
