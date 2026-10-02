/// A parsed unified diff for a single file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub hunks: Vec<DiffHunk>,
    pub binary: bool,
}

impl FileDiff {
    pub fn additions(&self) -> usize {
        self.count(DiffLineKind::Added)
    }

    pub fn deletions(&self) -> usize {
        self.count(DiffLineKind::Removed)
    }

    fn count(&self, kind: DiffLineKind) -> usize {
        self.hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| line.kind == kind)
            .count()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
}

/// Parse `@@ -a,b +c,d @@` hunk ranges into their starting line numbers.
fn parse_hunk_header(header: &str) -> Option<(u32, u32)> {
    let inner = header.strip_prefix("@@ ")?;
    let mut parts = inner.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let start = |range: &str| range.split(',').next()?.parse::<u32>().ok();
    Some((start(old)?, start(new)?))
}

pub fn parse_unified_diff(path: &str, text: &str) -> FileDiff {
    let mut diff = FileDiff {
        path: path.to_string(),
        ..Default::default()
    };
    let (mut old_no, mut new_no) = (0u32, 0u32);
    for line in text.lines() {
        if line.starts_with("Binary files ") {
            diff.binary = true;
            continue;
        }
        if line.starts_with("@@") {
            if let Some((old, new)) = parse_hunk_header(line) {
                old_no = old;
                new_no = new;
            }
            diff.hunks.push(DiffHunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = diff.hunks.last_mut() else {
            // File headers (diff --git, index, ---/+++) precede the first hunk.
            continue;
        };
        let (kind, rest) = match line.as_bytes().first() {
            Some(b'+') => (DiffLineKind::Added, &line[1..]),
            Some(b'-') => (DiffLineKind::Removed, &line[1..]),
            Some(b' ') => (DiffLineKind::Context, &line[1..]),
            Some(b'\\') => continue, // "\ No newline at end of file"
            _ => (DiffLineKind::Context, line),
        };
        let (old_line, new_line) = match kind {
            DiffLineKind::Added => {
                new_no += 1;
                (None, Some(new_no - 1))
            }
            DiffLineKind::Removed => {
                old_no += 1;
                (Some(old_no - 1), None)
            }
            DiffLineKind::Context => {
                old_no += 1;
                new_no += 1;
                (Some(old_no - 1), Some(new_no - 1))
            }
        };
        hunk.lines.push(DiffLine {
            kind,
            old_line,
            new_line,
            text: rest.to_string(),
        });
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hunks_with_line_numbers() {
        let text = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n\\ No newline at end of file\n";
        let diff = parse_unified_diff("f", text);
        assert_eq!(diff.hunks.len(), 1);
        let lines = &diff.hunks[0].lines;
        assert_eq!(lines.len(), 4);
        assert_eq!(
            (lines[1].kind, lines[1].old_line, lines[1].new_line),
            (DiffLineKind::Removed, Some(2), None)
        );
        assert_eq!(
            (lines[2].kind, lines[2].old_line, lines[2].new_line),
            (DiffLineKind::Added, None, Some(2))
        );
        assert_eq!((lines[3].old_line, lines[3].new_line), (Some(3), Some(3)));
        assert_eq!((diff.additions(), diff.deletions()), (1, 1));
    }
}
