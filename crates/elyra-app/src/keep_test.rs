//! From an error to a lasting check: after a turn worked on an error (a
//! server error from Grove, or one the app logged), Keep as test has the agent
//! turn it into a regression test. Elyra then runs the project's checks in a
//! temporary checkout twice: with the fix, where the test must pass, and with
//! the folder as it was before the fix plus only the new test files, where it
//! must fail. Only then does the test guard the fix.

use elyra_core::KeptTest;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// How the message asking for the test starts (to tell its turn apart).
pub const ASK: &str = "Keep the error this thread fixed as a regression test";

/// The message asking for the test.
pub fn prompt(errors: &[String], detail: &str) -> String {
    let detail: String = detail.chars().take(6_000).collect();
    format!(
        "{ASK}, so it can't come back unnoticed:\n\n<error>\n{}\n\n{}\n</error>\n\nWrite a test in the project's own test suite that reproduces exactly the condition that failed (the request, input or data), named after what it guards. Only add or change test files: don't change the fix or other code. Elyra then runs the checks with the fix, where the test must pass, and against the code from before the fix, where it must fail.",
        errors.join("\n"),
        detail.trim()
    )
}

/// Whether a path is a test (by the usual conventions).
pub fn is_test(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let in_dir =
        |dir: &str| path.starts_with(&format!("{dir}/")) || path.contains(&format!("/{dir}/"));
    in_dir("tests")
        || in_dir("test")
        || in_dir("spec")
        || in_dir("__tests__")
        || name.ends_with("Test.php")
        || name.ends_with("_test.go")
        || name.ends_with("_test.rs")
        || name.ends_with("_spec.rb")
        || (name.starts_with("test_") && name.ends_with(".py"))
        || [".test.", ".spec."].iter().any(|part| name.contains(part))
}

/// Run the checks with and without the fix. `folder` is the thread's folder
/// relative to the repository. Blocking.
#[allow(clippy::too_many_arguments)]
pub fn verify(
    root: &Path,
    folder: &Path,
    fix_before: &str,
    test_before: &str,
    after: &str,
    command: Option<&str>,
    cancel: &Arc<AtomicBool>,
) -> KeptTest {
    let not_verified = |reason: String| KeptTest::NotVerified { reason };
    let Some(command) = command else {
        return not_verified(
            "The project has no check command to run the test with (Context tab).".into(),
        );
    };
    let changed = elyra_git::undo::changed_paths(root, test_before, after).unwrap_or_default();
    let files: Vec<String> = changed.iter().filter(|p| is_test(p)).cloned().collect();
    if files.is_empty() {
        return not_verified(if changed.is_empty() {
            "The turn didn't change any files.".into()
        } else {
            format!(
                "None of the files the turn changed looks like a test ({}), so they can't be run apart from the fix.",
                changed.join(", ")
            )
        });
    }
    let run = || -> anyhow::Result<KeptTest> {
        let scratch = elyra_git::scratch::ScratchCheckout::new(root)?;
        let dir = scratch.dir().join(folder);
        // With the fix: the folder as the test's turn left it.
        scratch.checkout(after)?;
        let with = crate::checks::run(&dir, command, cancel);
        if !with.passed {
            let output: String = with
                .output
                .chars()
                .rev()
                .take(3_000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            return Ok(KeptTest::FailsWithFix {
                files: files.clone(),
                output,
            });
        }
        // Without it: as before the fix, plus only the new test files.
        scratch.checkout(fix_before)?;
        for path in &files {
            let target = scratch.dir().join(path);
            match elyra_git::undo::content_at(root, after, path) {
                Some(content) => {
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&target, content)?;
                }
                None => {
                    let _ = std::fs::remove_file(&target);
                }
            }
        }
        let without = crate::checks::run(&dir, command, cancel);
        Ok(if without.passed {
            KeptTest::PassesWithoutFix {
                files: files.clone(),
            }
        } else {
            KeptTest::Guards {
                files: files.clone(),
            }
        })
    };
    run().unwrap_or_else(|err| not_verified(format!("{err:#}")))
}

/// What the card says.
pub fn describe(test: &KeptTest) -> String {
    match test {
        KeptTest::Writing => "The agent is writing the test…".into(),
        KeptTest::Checking => "Running the checks with and without the fix…".into(),
        KeptTest::Guards { files } => format!(
            "Kept: {} passes with the fix and fails without it.",
            files.join(", ")
        ),
        KeptTest::PassesWithoutFix { files } => format!(
            "{} passes without the fix too, so it doesn't catch the error.",
            files.join(", ")
        ),
        KeptTest::FailsWithFix { files, .. } => {
            format!("{} fails with the fix as well.", files.join(", "))
        }
        KeptTest::NotVerified { reason } => format!("Not checked: {reason}"),
    }
}

/// What the agent is asked when the test doesn't guard the fix.
pub fn retry_prompt(test: &KeptTest) -> Option<String> {
    match test {
        KeptTest::PassesWithoutFix { files } => Some(format!(
            "{ASK}: the test you wrote ({}) also passes against the code from before the fix, so it would not catch the error coming back. Make it reproduce the exact condition that failed, so it fails there and passes now. Only change test files.",
            files.join(", ")
        )),
        KeptTest::FailsWithFix { files, output } => Some(format!(
            "{ASK}: the test you wrote ({}) fails with the fix as well. Make it pass with the fix (and fail without it). Only change test files.\n\nThe checks said:\n```\n{}\n```",
            files.join(", "),
            output.trim()
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{describe, is_test, prompt, retry_prompt, verify};
    use elyra_core::KeptTest;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn knows_a_test_when_it_sees_one() {
        for path in [
            "tests/Feature/CartTest.php",
            "app/tests/x.py",
            "src/cart.test.ts",
            "web/__tests__/cart.js",
            "pkg/cart_test.go",
            "spec/cart_spec.rb",
            "test_cart.py",
        ] {
            assert!(is_test(path), "{path}");
        }
        for path in [
            "app/Http/Controllers/CartController.php",
            "src/latest.rs",
            "contest/a.rs",
        ] {
            assert!(!is_test(path), "{path}");
        }
        let asked = prompt(
            &["Server error GET /cart → 500".into()],
            "Undefined array key",
        );
        assert!(asked.starts_with(super::ASK));
        assert!(
            asked
                .contains("<error>\nServer error GET /cart → 500\n\nUndefined array key\n</error>")
        );
        let weak = KeptTest::PassesWithoutFix {
            files: vec!["tests/CartTest.php".into()],
        };
        assert!(
            retry_prompt(&weak)
                .unwrap()
                .contains("also passes against the code from before the fix")
        );
        assert!(describe(&weak).contains("doesn't catch the error"));
    }

    #[test]
    fn a_test_guards_the_fix_only_if_it_fails_without_it() {
        let repo = std::env::temp_dir().join(format!("elyra-keep-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("tests")).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "start",
        ]);
        let snap = |name: &str| {
            elyra_git::checkpoint::create(&repo, &format!("refs/elyra/t/{name}"), name).unwrap()
        };
        // The bug, then the fix, then the test.
        std::fs::write(repo.join("price.txt"), "broken\n").unwrap();
        let fix_before = snap("fix-before");
        std::fs::write(repo.join("price.txt"), "fixed\n").unwrap();
        let test_before = snap("test-before");
        std::fs::write(repo.join("tests/price.sh"), "grep -q fixed price.txt\n").unwrap();
        let after = snap("after");
        let cancel = Arc::new(AtomicBool::new(false));
        let run = |command: &str| {
            verify(
                &repo,
                std::path::Path::new(""),
                &fix_before,
                &test_before,
                &after,
                Some(command),
                &cancel,
            )
        };
        assert_eq!(
            run("sh tests/price.sh"),
            KeptTest::Guards {
                files: vec!["tests/price.sh".into()]
            }
        );
        assert_eq!(
            run("true"),
            KeptTest::PassesWithoutFix {
                files: vec!["tests/price.sh".into()]
            }
        );
        assert!(matches!(run("false"), KeptTest::FailsWithFix { .. }));
        assert!(matches!(
            verify(
                &repo,
                std::path::Path::new(""),
                &fix_before,
                &test_before,
                &after,
                None,
                &cancel
            ),
            KeptTest::NotVerified { .. }
        ));
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
