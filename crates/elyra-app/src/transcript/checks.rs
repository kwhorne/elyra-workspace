//! What proves a turn: its checks, the invariants and testing the tests.

use super::*;

/// A check entry's fields, for its row.
pub(super) struct Check<'a> {
    pub(super) command: &'a str,
    pub(super) passed: bool,
    pub(super) exit_code: Option<i32>,
    pub(super) duration_ms: u64,
    pub(super) output: &'a str,
    /// Invariants that held just before (shown on this row).
    pub(super) invariants: usize,
}

/// Whether checks ran after the item `id` in the same turn.
pub(super) fn checks_follow(items: &[TranscriptItem], id: ItemId) -> bool {
    let Some(at) = items.iter().position(|i| i.id == id) else {
        return false;
    };
    items[at + 1..]
        .iter()
        .take_while(|i| !matches!(i.content, ItemContent::User { .. }))
        .any(|i| matches!(i.content, ItemContent::Check { .. }))
}

/// How many invariants held right before the checks `id` (same turn).
pub(super) fn invariants_held_before(items: &[TranscriptItem], id: ItemId) -> usize {
    let Some(at) = items.iter().position(|i| i.id == id) else {
        return 0;
    };
    items[..at]
        .iter()
        .rev()
        .take_while(|i| {
            !matches!(
                i.content,
                ItemContent::User { .. } | ItemContent::Check { .. }
            )
        })
        .find_map(|i| match &i.content {
            ItemContent::Invariants { checked, broken } if broken.is_empty() => Some(checked.len()),
            _ => None,
        })
        .unwrap_or(0)
}

pub(super) fn check_row(
    id: ItemId,
    check: Check,
    open: bool,
    can_bisect: bool,
    cx: &Context<ThreadView>,
) -> AnyElement {
    let Check {
        command,
        passed,
        exit_code,
        duration_ms,
        output,
        invariants,
    } = check;
    let bisect_command = command.to_string();
    let mut summary = vec![
        command.to_string(),
        format!("{:.1}s", duration_ms as f64 / 1000.0),
    ];
    if !passed && let Some(code) = exit_code {
        summary.push(format!("exit {code}"));
    }
    let last_line = (!passed && !open)
        .then(|| output.lines().rev().find(|l| !l.trim().is_empty()))
        .flatten()
        .map(|line| line.chars().take(160).collect::<String>());
    let (icon, color, label) = if passed {
        (IconName::CircleCheck, cx.theme().success, "Checks passed")
    } else {
        (IconName::CircleX, cx.theme().danger, "Checks failed")
    };
    v_flex()
        .w_full()
        .rounded_md()
        .border_1()
        .border_color(if passed { cx.theme().border } else { color })
        .child(
            h_flex()
                .id(SharedString::from(format!("check-{id}")))
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .cursor_pointer()
                .hover(|this| this.bg(cx.theme().muted))
                .text_sm()
                .child(Icon::new(icon).small().text_color(color))
                .child(div().font_weight(FontWeight::MEDIUM).child(label))
                .when(invariants > 0, |this| {
                    this.child(
                        h_flex()
                            .flex_none()
                            .gap_0p5()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                Icon::new(IconName::ShieldCheck)
                                    .xsmall()
                                    .text_color(cx.theme().success),
                            )
                            .child(if invariants == 1 {
                                "1 invariant".to_string()
                            } else {
                                format!("{invariants} invariants")
                            }),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(summary.join(" · ")),
                )
                .when(can_bisect, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("bisect-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Search)
                            .label("Who broke it?")
                            .tooltip("Run the checks against the states before earlier turns in this folder to find the one that made them fail")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                let command = bisect_command.clone();
                                this.session.update(cx, |session, cx| {
                                    session.find_breaking_change(Some(command), cx)
                                });
                            })),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_expanded(id, cx))),
        )
        .when_some(last_line, |this, line| {
            this.child(
                div()
                    .px_2()
                    .pb_1()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(line),
            )
        })
        .when(open && !output.is_empty(), |this| {
            this.child(
                div()
                    .p_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(mono_block(truncate(output, MAX_RESULT_CHARS), cx)),
            )
        })
        .into_any_element()
}

/// The project's invariants after a turn: held, or what broke.
pub(super) fn invariants_card(
    checked: &[String],
    broken: &[elyra_core::BrokenInvariant],
    cx: &Context<ThreadView>,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    if broken.is_empty() {
        return h_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .child(
                Icon::new(IconName::ShieldCheck)
                    .small()
                    .text_color(cx.theme().success),
            )
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(if checked.len() == 1 {
                        "Invariant held".to_string()
                    } else {
                        format!("{} invariants held", checked.len())
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_xs()
                    .text_color(muted)
                    .child(checked.join(" · ")),
            )
            .into_any_element();
    }
    let mut names: Vec<&str> = broken.iter().map(|b| b.name.as_str()).collect();
    names.dedup();
    v_flex()
        .w_full()
        .p_2()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().danger)
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(
                    Icon::new(IconName::ShieldAlert)
                        .small()
                        .text_color(cx.theme().danger),
                )
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(if names.len() == 1 {
                            format!("Invariant broken: {}", names[0])
                        } else {
                            format!("{} invariants broken", names.len())
                        }),
                ),
        )
        .children(names.iter().map(|name| {
            let mine: Vec<elyra_core::BrokenInvariant> =
                broken.iter().filter(|b| b.name == *name).cloned().collect();
            let why = mine.first().map(|b| b.why.clone()).unwrap_or_default();
            v_flex()
                .gap_0p5()
                .text_xs()
                .when(names.len() > 1, |this| {
                    this.child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(name.to_string()),
                    )
                })
                .when(!why.is_empty(), |this| {
                    this.child(div().whitespace_normal().text_color(muted).child(why))
                })
                .children(
                    crate::invariants::describe(&mine)
                        .into_iter()
                        .take(8)
                        .map(|line| {
                            div()
                                .whitespace_normal()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(line)
                        }),
                )
                .into_any_element()
        }))
        .into_any_element()
}

/// An invariant the agent proposed: what it checks, whether the code holds
/// it now, and saving it.
pub(super) fn invariant_proposal_card(
    id: ItemId,
    invariant: &elyra_core::Invariant,
    holds: Option<&[elyra_core::BrokenInvariant]>,
    outcome: Option<elyra_core::ProposalOutcome>,
    cx: &Context<ThreadView>,
) -> AnyElement {
    use elyra_core::ProposalOutcome;
    let muted = cx.theme().muted_foreground;
    let pending = outcome.is_none();
    let title = match outcome {
        None => "Proposed check".to_string(),
        Some(ProposalOutcome::Created) => format!("Saved as a check in {}", crate::invariants::DIR),
        Some(ProposalOutcome::Dismissed) => "Check dismissed".to_string(),
    };
    let rule = |label: &'static str, value: &str| {
        h_flex()
            .gap_2()
            .text_xs()
            .child(div().w(px(64.)).flex_none().text_color(muted).child(label))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(value.to_string()),
            )
    };
    let decide = |key: &'static str, label: &'static str, save: bool| {
        Button::new(SharedString::from(format!("invariant-{key}-{id}")))
            .small()
            .label(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.session
                    .update(cx, |session, cx| session.decide_invariant(id, save, cx))
            }))
    };
    v_flex()
        .w_full()
        .p_3()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(if pending { cx.theme().info } else { cx.theme().border })
        .when(pending, |this| this.bg(cx.theme().info.opacity(0.06)))
        .child(
            h_flex()
                .gap_2()
                .child(Icon::new(IconName::ShieldCheck).small().text_color(cx.theme().info))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
        )
        .child(div().text_sm().whitespace_normal().child(invariant.name.clone()))
        .child(
            v_flex()
                .gap_0p5()
                .child(rule(
                    "Files",
                    if invariant.paths.is_empty() { "all" } else { &invariant.paths },
                ))
                .when_some(invariant.require.as_deref(), |this, v| this.child(rule("Must have", v)))
                .when_some(invariant.forbid.as_deref(), |this, v| {
                    this.child(rule("New lines without", v))
                })
                .when_some(invariant.command.as_deref(), |this, v| this.child(rule("Runs", v))),
        )
        .when(!invariant.why.is_empty(), |this| {
            this.child(
                div()
                    .text_xs()
                    .whitespace_normal()
                    .text_color(muted)
                    .child(invariant.why.clone()),
            )
        })
        .child(match holds {
            None => h_flex()
                .gap_2()
                .text_xs()
                .text_color(muted)
                .child(Spinner::new().xsmall())
                .child("Trying it on the code as it is…")
                .into_any_element(),
            Some([]) => h_flex()
                .gap_1()
                .text_xs()
                .text_color(cx.theme().success)
                .child(Icon::new(IconName::CircleCheck).xsmall())
                .child("The code holds it now.")
                .into_any_element(),
            Some(found) => v_flex()
                .gap_0p5()
                .text_xs()
                .child(
                    div()
                        .text_color(cx.theme().warning)
                        .whitespace_normal()
                        .child(format!(
                            "The code doesn't hold it in {} place{} yet; once saved, only files a turn changes are checked:",
                            found.len(),
                            if found.len() == 1 { "" } else { "s" }
                        )),
                )
                .children(crate::invariants::describe(found).into_iter().take(5).map(|line| {
                    div()
                        .whitespace_normal()
                        .text_color(muted)
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(line)
                }))
                .into_any_element(),
        })
        .when(pending, |this| {
            this.child(
                h_flex()
                    .gap_2()
                    .child(decide("save", "Save as check", true).primary())
                    .child(decide("dismiss", "Dismiss", false).ghost()),
            )
        })
        .into_any_element()
}

/// The mutation gate: small changes to the lines the instruction added, and
/// whether the checks caught each.
pub(super) fn mutation_card(
    id: ItemId,
    command: &str,
    mutants: &[elyra_core::MutantResult],
    state: &elyra_core::GateState,
    asked: bool,
    running: bool,
    cx: &Context<ThreadView>,
) -> AnyElement {
    use elyra_core::GateState;
    let muted = cx.theme().muted_foreground;
    let caught = mutants.iter().filter(|m| m.caught == Some(true)).count();
    let missed = mutants.iter().filter(|m| m.caught == Some(false)).count();
    let total = mutants.len();
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let (title, color) = match state {
        GateState::Running => (
            format!(
                "Testing the tests: {total} small change{} to the new lines…",
                plural(total)
            ),
            muted,
        ),
        GateState::Done if missed == 0 => (
            format!("The tests caught all {total} change{}", plural(total)),
            cx.theme().success,
        ),
        GateState::Done => (
            format!(
                "The tests missed {missed} of {total} change{}",
                plural(total)
            ),
            cx.theme().warning,
        ),
        GateState::Stopped => (
            format!("Testing the tests was stopped ({caught} caught, {missed} missed)"),
            muted,
        ),
        GateState::Failed { .. } => ("Couldn't test the tests".to_string(), muted),
    };
    let mono = cx.theme().mono_font_family.clone();
    v_flex()
        .w_full()
        .p_2()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(if missed > 0 && !asked {
            cx.theme().warning
        } else {
            cx.theme().border
        })
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .map(|this| match state {
                    GateState::Running => this.child(Spinner::new().small()),
                    GateState::Done if missed == 0 => {
                        this.child(Icon::new(IconName::CircleCheck).small().text_color(color))
                    }
                    GateState::Done => {
                        this.child(Icon::new(IconName::TriangleAlert).small().text_color(color))
                    }
                    _ => this.child(Icon::new(IconName::FlaskConical).small().text_color(muted)),
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .when(matches!(state, GateState::Running), |this| {
                    this.child(
                        Button::new(SharedString::from(format!("mutation-stop-{id}")))
                            .xsmall()
                            .ghost()
                            .label("Stop")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.session.update(cx, |session, cx| session.stop_mutation(cx))
                            })),
                    )
                }),
        )
        .when_some(
            match state {
                GateState::Failed { reason } => Some(reason.clone()),
                _ => None,
            },
            |this, reason| {
                this.child(
                    div()
                        .text_xs()
                        .whitespace_normal()
                        .text_color(muted)
                        .child(reason),
                )
            },
        )
        .children(mutants.iter().map(|mutant| {
            let (icon, color) = match mutant.caught {
                Some(true) => (IconName::CircleCheck, cx.theme().success),
                Some(false) => (IconName::CircleX, cx.theme().warning),
                None => (IconName::CircleDashed, muted),
            };
            v_flex()
                .gap_0p5()
                .text_xs()
                .child(
                    h_flex()
                        .gap_2()
                        .child(Icon::new(icon).xsmall().text_color(color))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_color(muted)
                                .child(format!("{}:{}", mutant.path, mutant.line)),
                        )
                        .child(div().flex_none().text_color(color).child(match mutant.caught {
                            Some(true) => format!("{}: caught", mutant.change),
                            Some(false) => format!("{}: no test failed", mutant.change),
                            None => mutant.change.clone(),
                        })),
                )
                .child(
                    div()
                        .pl_5()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(mono.clone())
                        .child(mutant.code.clone()),
                )
                .into_any_element()
        }))
        .when(missed > 0, |this| {
            this.child(if asked {
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Asked for tests that catch these; this runs again once they pass.")
                    .into_any_element()
            } else {
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("mutation-ask-{id}")))
                            .small()
                            .primary()
                            .label("Ask for tests")
                            .tooltip("Have the agent add tests that fail with these changes, without changing the code")
                            .disabled(running)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.session
                                    .update(cx, |session, cx| session.ask_for_tests(id, cx))
                            })),
                    )
                    .child(div().flex_1().min_w_0().text_xs().whitespace_normal().text_color(muted).child(format!(
                        "`{command}` still passed with {}.",
                        if missed == 1 { "this change" } else { "these changes" }
                    )))
                    .into_any_element()
            })
        })
        .into_any_element()
}
