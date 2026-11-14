//! What happened to the code: errors fixed, stuck turns, rewritten intent,
//! undone instructions and the search for the turn that broke something.

use super::*;

/// The project's checks after a turn: passed or failed, with the output
/// (the last line shows while folded, when they failed).
#[allow(clippy::too_many_arguments)]
/// An instruction taken back.
/// The turn worked on errors: keep them as a regression test.
pub(super) fn error_fixed_card(
    id: ItemId,
    errors: &[String],
    test: Option<&elyra_core::KeptTest>,
    running: bool,
    cx: &Context<ThreadView>,
) -> AnyElement {
    use elyra_core::KeptTest;
    let muted = cx.theme().muted_foreground;
    let (icon, color) = match test {
        Some(KeptTest::Guards { .. }) => (IconName::CircleCheck, cx.theme().success),
        Some(KeptTest::PassesWithoutFix { .. } | KeptTest::FailsWithFix { .. }) => {
            (IconName::TriangleAlert, cx.theme().warning)
        }
        _ => (IconName::FlaskConical, muted),
    };
    let button = |label: &'static str, tooltip: &'static str| {
        Button::new(SharedString::from(format!("keep-test-{id}")))
            .xsmall()
            .outline()
            .label(label)
            .tooltip(tooltip)
            .disabled(running)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.session
                    .update(cx, |session, cx| session.keep_as_test(id, cx))
            }))
    };
    v_flex()
        .w_full()
        .p_2()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(Icon::new(icon).small().text_color(color))
                .child(div().font_weight(FontWeight::MEDIUM).child(if errors.len() == 1 {
                    "This turn worked on an error"
                } else {
                    "This turn worked on errors"
                })),
        )
        .children(errors.iter().map(|error| {
            div()
                .text_xs()
                .text_color(muted)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(error.clone())
        }))
        .when_some(test, |this, test| {
            this.child(
                div()
                    .text_xs()
                    .whitespace_normal()
                    .text_color(match test {
                        KeptTest::Guards { .. } => cx.theme().success,
                        KeptTest::PassesWithoutFix { .. } | KeptTest::FailsWithFix { .. } => {
                            cx.theme().warning
                        }
                        _ => muted,
                    })
                    .child(crate::keep_test::describe(test)),
            )
        })
        .child(h_flex().gap_2().map(|this| match test {
            None => this.child(button(
                "Keep as test",
                "Have the agent turn it into a regression test; Elyra then checks that it fails without the fix",
            )),
            Some(KeptTest::PassesWithoutFix { .. } | KeptTest::FailsWithFix { .. }) => this.child(
                button("Ask the agent to fix the test", "Send the agent what was wrong with it"),
            ),
            Some(KeptTest::NotVerified { .. }) => {
                this.child(button("Try again", "Ask the agent for the test again"))
            }
            Some(_) => this,
        }))
        .into_any_element()
}

/// The thrash guard saw the agent going in circles: start over, stop, or
/// let it run (while the turn runs and nobody has decided).
pub(super) fn stuck_card(
    id: ItemId,
    reason: &str,
    detail: &str,
    outcome: Option<elyra_core::StuckOutcome>,
    session: &ThreadSession,
    cx: &Context<ThreadView>,
) -> AnyElement {
    use elyra_core::StuckOutcome;
    let muted = cx.theme().muted_foreground;
    let open = !detail.is_empty() && outcome.is_none();
    let answer = |key: &'static str, label: &'static str, outcome: StuckOutcome| {
        Button::new(SharedString::from(format!("stuck-{key}-{id}")))
            .xsmall()
            .label(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.session
                    .update(cx, |session, cx| session.resolve_stuck(id, outcome, cx))
            }))
    };
    let escalation = crate::preferences::Preferences::global(cx)
        .provider(session.thread.provider)
        .escalation_model
        .trim()
        .to_string();
    let start_over_tip = if escalation.is_empty() {
        "Stop it, put the files back to how they were before this turn, and have it try again at the highest effort".to_string()
    } else {
        format!(
            "Stop it, put the files back to how they were before this turn, and have {escalation} try again"
        )
    };
    v_flex()
        .w_full()
        .p_2()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(if outcome.is_none() {
            cx.theme().warning
        } else {
            cx.theme().border
        })
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(
                    Icon::new(IconName::RotateCcwClock)
                        .small()
                        .text_color(cx.theme().warning),
                )
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child("The agent is going in circles"),
                ),
        )
        .child(
            div()
                .text_xs()
                .whitespace_normal()
                .text_color(muted)
                .child(format!("{reason}.")),
        )
        .when(open, |this| {
            this.child(
                div()
                    .p_1()
                    .rounded_sm()
                    .bg(cx.theme().muted)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .child(detail.to_string()),
            )
        })
        .map(|this| match outcome {
            None if session.running => this.child(
                h_flex()
                    .gap_2()
                    .child(
                        answer("over", "Start over", StuckOutcome::StartedOver)
                            .primary()
                            .tooltip(start_over_tip),
                    )
                    .child(
                        answer("stop", "Stop", StuckOutcome::Stopped)
                            .outline()
                            .tooltip("Stop the turn and leave the files as they are"),
                    )
                    .child(
                        answer("run", "Let it run", StuckOutcome::LetRun)
                            .ghost()
                            .tooltip("It's fine: don't ask again this turn"),
                    ),
            ),
            None => this,
            Some(outcome) => this.child(div().text_xs().text_color(muted).child(match outcome {
                StuckOutcome::StartedOver => {
                    "Stopped and started over from the files before this turn."
                }
                StuckOutcome::Stopped => "Stopped.",
                StuckOutcome::LetRun => "Left to carry on.",
            })),
        })
        .into_any_element()
}

/// The turn took out lines written for other instructions.
pub(super) fn rewrote_card(
    id: ItemId,
    instructions: &[elyra_core::RewrittenIntent],
    cx: &Context<ThreadView>,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let mut card = v_flex()
        .w_full()
        .p_2()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().warning.opacity(0.5))
        .bg(cx.theme().warning.opacity(0.05))
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .small()
                        .text_color(cx.theme().warning),
                )
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child("This turn took out lines written for other instructions"),
                ),
        );
    for (index, intent) in instructions.iter().enumerate() {
        let (thread, asked) = (intent.thread, intent.asked);
        card = card.child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .whitespace_normal()
                        .child(format!("“{}”", intent.instruction)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(crate::intent_guard::summary(intent)),
                )
                .child(
                    v_flex()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(cx.theme().danger.opacity(0.06))
                        .text_xs()
                        .font_family(cx.theme().mono_font_family.clone())
                        .children(intent.lines.iter().take(3).map(|line| {
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(format!("- {line}"))
                        })),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(match asked {
                            None => Button::new(SharedString::from(format!("rewrite-ask-{id}-{index}")))
                                .xsmall()
                                .outline()
                                .icon(IconName::MessagesSquare)
                                .label("Ask the author")
                                .tooltip("Ask the agent that wrote these lines, as it was then, whether the new code still does what it was asked to do")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.ask_rewrite_author(id, index, window, cx)
                                })),
                            Some(chat) => Button::new(SharedString::from(format!("rewrite-answer-{id}-{index}")))
                                .xsmall()
                                .outline()
                                .icon(IconName::MessagesSquare)
                                .label("Open the answer")
                                .on_click(move |_, window, cx| {
                                    if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
                                        let workspace = hub.workspace.clone();
                                        let _ = workspace.update(cx, |this, cx| {
                                            this.show_side_chat(chat, window, cx)
                                        });
                                    }
                                }),
                        })
                        .child(
                            Button::new(SharedString::from(format!("rewrite-open-{id}-{index}")))
                                .xsmall()
                                .ghost()
                                .label("Open its thread")
                                .on_click(move |_, window, cx| {
                                    if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
                                        let workspace = hub.workspace.clone();
                                        let _ = workspace
                                            .update(cx, |this, cx| this.activate(thread, window, cx));
                                    }
                                }),
                        ),
                ),
        );
    }
    card.child(
        div()
            .text_xs()
            .text_color(muted)
            .child("The agent hears of it with your next message."),
    )
    .into_any_element()
}

pub(super) fn undo_card(
    id: ItemId,
    instruction: &str,
    reverted: &[String],
    kept: &[String],
    put_back: bool,
    running: bool,
    cx: &Context<ThreadView>,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let headline = instruction
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();
    let list = |paths: &[String]| {
        paths
            .iter()
            .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let prompt = crate::undo::finish_prompt(instruction, kept, reverted);
    v_flex()
        .w_full()
        .p_2()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(Icon::new(IconName::Undo2).small().text_color(muted))
                .child(div().font_weight(FontWeight::MEDIUM).child(if put_back {
                    "Undone, then put back"
                } else {
                    "Undone"
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(format!("“{headline}”")),
                ),
        )
        .when(!reverted.is_empty(), |this| {
            this.child(div().text_xs().text_color(muted).child(format!(
                "Back as before it: {}",
                list(reverted)
            )))
        })
        .when(!kept.is_empty(), |this| {
            this.child(div().text_xs().text_color(cx.theme().warning).child(format!(
                "Changed again since, left as they are: {}",
                list(kept)
            )))
        })
        .when(!put_back, |this| {
            this.child(
                h_flex()
                    .gap_2()
                    .when(!kept.is_empty(), |this| {
                        this.child(
                            Button::new(SharedString::from(format!("undo-finish-{id}")))
                                .xsmall()
                                .primary()
                                .label("Ask the agent to finish")
                                .tooltip("Have the agent take the instruction's part out of the files left as they are")
                                .disabled(running)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let prompt = elyra_provider::Prompt::text(prompt.clone());
                                    this.session
                                        .update(cx, |session, cx| session.submit(prompt, cx));
                                })),
                        )
                    })
                    .child(
                        Button::new(SharedString::from(format!("undo-put-back-{id}")))
                            .xsmall()
                            .ghost()
                            .label("Put it back")
                            .tooltip("Bring back what the undo took away; what changed since stays")
                            .disabled(running)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.session
                                    .update(cx, |session, cx| session.put_back_undo(id, cx));
                            })),
                    ),
            )
        })
        .into_any_element()
}

/// What "Who broke it?" found.
pub(super) fn bisect_card(
    command: &str,
    verdict: &elyra_core::BisectVerdict,
    cx: &Context<ThreadView>,
) -> AnyElement {
    use elyra_core::BisectVerdict;
    let muted = cx.theme().muted_foreground;
    let BisectVerdict::Found {
        thread,
        instruction,
        thread_title,
        agent,
        at,
        output,
        runs,
        ..
    } = verdict
    else {
        return h_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .text_color(muted)
            .child(Icon::new(IconName::Search).small())
            .child(crate::bisect::describe(command, verdict))
            .into_any_element();
    };
    let thread = *thread;
    let headline = instruction
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("(an empty message)")
        .to_string();
    let tail: Vec<&str> = output
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(6)
        .collect();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    let prompt = format!(
        "The project's checks (`{command}`) started failing with the changes made for this message:\n“{headline}”\n\nThey passed before that turn and fail after it. The failing output ends with:\n```\n{}\n```\nFind and fix the cause.",
        output
            .lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
    let open = move |window: &mut Window, cx: &mut App| {
        if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
            let workspace = hub.workspace.clone();
            let _ = workspace.update(cx, |this, cx| this.activate(thread, window, cx));
        }
    };
    v_flex()
        .w_full()
        .p_2()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().warning)
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(Icon::new(IconName::Search).small().text_color(cx.theme().warning))
                .child(div().font_weight(FontWeight::MEDIUM).child("Broken by"))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(format!("“{headline}”")),
                ),
        )
        .child(div().text_xs().text_color(muted).child(format!(
            "{thread_title} · {agent} · {} · `{command}` passed before that turn and failed after it ({runs} runs)",
            at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
        )))
        .when(!tail.is_empty(), |this| {
            this.child(
                div()
                    .p_1()
                    .rounded_sm()
                    .bg(cx.theme().muted)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .child(tail),
            )
        })
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("bisect-open-{thread}")))
                        .xsmall()
                        .ghost()
                        .label("Open thread")
                        .on_click(move |_, window, cx| open(window, cx)),
                )
                .child(
                    Button::new(SharedString::from(format!("bisect-send-{thread}")))
                        .xsmall()
                        .primary()
                        .label("Send the failure there")
                        .tooltip("Ask the thread that broke it to fix it, with the failing output")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let Some(app) = this.session.read(cx).app_entity() else {
                                return;
                            };
                            if let Some(session) = app.update(cx, |app, cx| app.session(thread, cx)) {
                                let prompt = elyra_provider::Prompt::text(prompt.clone());
                                session.update(cx, |session, cx| session.submit(prompt, cx));
                            }
                            open(window, cx);
                        })),
                ),
        )
        .into_any_element()
}
