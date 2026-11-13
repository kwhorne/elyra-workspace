//! Mutants: small, syntax-aware changes to code for testing the tests. Flip a
//! comparison (`==` to `!=`, `<` to `>=`), swap `&&` and `||`, drop a `!`, turn
//! `true` into `false`. Each keeps the code compiling in the usual case, so
//! when the tests still pass with one, no test guards that line.
//!
//! Found with tree-sitter, so operators in strings, comments, generics
//! (`Vec<u8>`) and macros (`println!`) are left alone.

use crate::Language;
use std::collections::HashSet;
use tree_sitter::{Node, Parser};

/// What sort of change a mutant makes, most telling first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// `==` → `!=`, `<` → `>=`, …
    Comparison,
    /// `&&` ↔ `||`, `and` ↔ `or`.
    Logic,
    /// `!x` → `x`, `not x` → `x`.
    Negation,
    /// `true` ↔ `false`.
    Literal,
}

/// One change to a source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mutant {
    /// 1-based.
    pub line: usize,
    /// The bytes replaced.
    pub start: usize,
    pub end: usize,
    pub original: String,
    pub replacement: String,
    pub kind: Kind,
}

impl Mutant {
    /// The source with the change made.
    pub fn apply(&self, source: &str) -> String {
        format!(
            "{}{}{}",
            &source[..self.start],
            self.replacement,
            &source[self.end..]
        )
    }

    /// For people: "`>` → `<=`", "drop `!`".
    pub fn describe(&self) -> String {
        let original = self.original.trim();
        if self.replacement.is_empty() {
            format!("drop `{original}`")
        } else {
            format!("`{original}` → `{}`", self.replacement)
        }
    }
}

fn flipped(operator: &str) -> Option<(&'static str, Kind)> {
    Some(match operator {
        "==" => ("!=", Kind::Comparison),
        "!=" => ("==", Kind::Comparison),
        "===" => ("!==", Kind::Comparison),
        "!==" => ("===", Kind::Comparison),
        "<" => (">=", Kind::Comparison),
        ">" => ("<=", Kind::Comparison),
        "<=" => (">", Kind::Comparison),
        ">=" => ("<", Kind::Comparison),
        "&&" => ("||", Kind::Logic),
        "||" => ("&&", Kind::Logic),
        "and" => ("or", Kind::Logic),
        "or" => ("and", Kind::Logic),
        _ => return None,
    })
}

/// The mutants on `lines` (1-based) of `source`, in order. Nothing when it
/// doesn't parse.
pub fn mutants(language: Language, source: &str, lines: &HashSet<usize>) -> Vec<Mutant> {
    let mut parser = Parser::new();
    if parser.set_language(&language.grammar()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        let line = node.start_position().row + 1;
        if node.child_count() == 0 {
            if lines.contains(&line)
                && let Some(mutant) = mutant(node, source, line)
            {
                found.push(mutant);
            }
            continue;
        }
        // Only descend where a wanted line is.
        let (first, last) = (line, node.end_position().row + 1);
        if lines.iter().any(|l| (first..=last).contains(l)) {
            for i in (0..node.child_count()).rev() {
                if let Some(child) = node.child(i as u32) {
                    stack.push(child);
                }
            }
        }
    }
    found.sort_by_key(|m| (m.line, m.start));
    found
}

fn mutant(node: Node, source: &str, line: usize) -> Option<Mutant> {
    let text = node.utf8_text(source.as_bytes()).ok()?;
    let parent = node.parent()?.kind();
    let make = |end: usize, replacement: &str, kind| Mutant {
        line,
        start: node.start_byte(),
        end,
        original: source[node.start_byte()..end].to_string(),
        replacement: replacement.to_string(),
        kind,
    };
    // A binary operator: `a == b`, `a && b`, Python's `a < b` and `a and b`.
    let binary = parent.contains("binary")
        || parent == "comparison_operator"
        || parent == "boolean_operator";
    if binary
        && !node.is_named()
        && let Some((replacement, kind)) = flipped(text)
    {
        return Some(make(node.end_byte(), replacement, kind));
    }
    // A negation: `!x` (also PHP's), Python's `not x`.
    if !node.is_named()
        && ((text == "!" && matches!(parent, "unary_expression" | "unary_op_expression"))
            || (text == "not" && parent == "not_operator"))
    {
        // `not` with the space after it.
        let end = node
            .next_sibling()
            .map(|next| next.start_byte())
            .unwrap_or(node.end_byte());
        return Some(make(end, "", Kind::Negation));
    }
    // A boolean literal.
    let literal =
        matches!(node.kind(), "true" | "false" | "boolean") || parent.contains("boolean_literal");
    if literal {
        let replacement = match text {
            "true" => "false",
            "false" => "true",
            "True" => "False",
            "False" => "True",
            "TRUE" => "FALSE",
            "FALSE" => "TRUE",
            _ => return None,
        };
        return Some(make(node.end_byte(), replacement, Kind::Literal));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{Kind, mutants};
    use crate::Language;
    use std::collections::HashSet;

    fn on(language: Language, source: &str, lines: &[usize]) -> Vec<(usize, String, String)> {
        let lines: HashSet<usize> = lines.iter().copied().collect();
        mutants(language, source, &lines)
            .into_iter()
            .map(|m| {
                let after = m.apply(source);
                (
                    m.line,
                    m.describe(),
                    after.lines().nth(m.line - 1).unwrap_or("").to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn flips_conditions_on_the_lines_asked_for() {
        let source = "fn discount(total: u32, member: bool) -> u32 {\n    if total > 100 && !member {\n        return 10;\n    }\n    let items: Vec<u8> = vec![];\n    println!(\"a < b\");\n    if total == 0 { 0 } else { 5 }\n}\n";
        let found = on(Language::Rust, source, &[2, 5, 6]);
        assert_eq!(
            found,
            vec![
                (
                    2,
                    "`>` → `<=`".into(),
                    "    if total <= 100 && !member {".into()
                ),
                (
                    2,
                    "`&&` → `||`".into(),
                    "    if total > 100 || !member {".into()
                ),
                (
                    2,
                    "drop `!`".into(),
                    "    if total > 100 && member {".into()
                ),
            ],
            "generics, macros and strings are left alone, and so is line 7"
        );
        assert_eq!(on(Language::Rust, source, &[7])[0].1, "`==` → `!=`");
    }

    #[test]
    fn knows_the_other_languages() {
        let php = "<?php\nif ($order->total() >= 500 || $free === true) {\n    return 0;\n}\n";
        let found = on(Language::Php, php, &[2]);
        let changes: Vec<&str> = found.iter().map(|(_, d, _)| d.as_str()).collect();
        assert_eq!(
            changes,
            [
                "`>=` → `<`",
                "`||` → `&&`",
                "`===` → `!==`",
                "`true` → `false`"
            ]
        );

        let python = "def free(total, member):\n    return total > 500 and not member\n";
        let found = on(Language::Python, python, &[2]);
        assert_eq!(
            found[2],
            (
                2,
                "drop `not`".into(),
                "    return total > 500 and member".into()
            )
        );
        assert_eq!(found[1].1, "`and` → `or`");

        let ts = "const ok = (n: number): boolean => n !== 0 && list.length < max;\n";
        let found = on(Language::TypeScript, ts, &[1]);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(super::Kind::Comparison < Kind::Literal);
    }

    #[test]
    fn nothing_when_it_does_not_parse_or_has_no_conditions() {
        assert!(on(Language::Go, "package main\n\nfunc main() {}\n", &[3]).is_empty());
        assert!(on(Language::C, "", &[1]).is_empty());
    }
}
