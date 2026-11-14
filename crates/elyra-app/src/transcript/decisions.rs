//! What waits on the user: proposals, approvals, plans and their
//! cross-examination, and questions.

use super::*;

/// An automation the agent proposed: what, when and where, with Create,
/// Edit and Dismiss until the user decides.
pub(super) fn proposal_card(
    id: ItemId,
    automation: &elyra_core::Automation,
    outcome: Option<elyra_core::ProposalOutcome>,
    session: &ThreadSession,
    cx: &Context<ThreadView>,
) -> AnyElement {
    use elyra_core::ProposalOutcome;
    let pending = outcome.is_none();
    let title = match outcome {
        None => "Proposed automation",
        Some(ProposalOutcome::Created) => "Automation created",
        Some(ProposalOutcome::Dismissed) => "Automation proposal dismissed",
    };
    let next = automation
        .next_run_at
        .map(|t| {
            format!(
                " · next {}",
                t.with_timezone(&chrono::Local).format("%a %d.%m %H:%M")
            )
        })
        .unwrap_or_default();
    let project = if automation.project_id == session.project.id {
        session.project.name.clone()
    } else {
        "another project".to_string()
    };
    let mut runs = format!(
        "{} in {project} · {}",
        automation.provider.label(),
        automation.permission_mode.label()
    );
    if let Some(model) = &automation.model {
        runs.push_str(&format!(" · {model}"));
    }
    let mut card = v_flex()
        .w_full()
        .p_3()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(if pending {
            cx.theme().info
        } else {
            cx.theme().border
        })
        .when(pending, |this| this.bg(cx.theme().info.opacity(0.06)))
        .child(
            h_flex()
                .gap_2()
                .child(
                    Icon::new(IconName::CalendarClock)
                        .small()
                        .text_color(cx.theme().info),
                )
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
        )
        .child(
            v_flex()
                .gap_1()
                .text_sm()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(automation.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{}{next}", automation.schedule.describe())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(runs),
                ),
        )
        .child(mono_block(truncate(&automation.prompt, 1200), cx));
    if pending {
        card = card.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("proposal-create-{id}")))
                        .primary()
                        .small()
                        .label("Create")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.create_proposal(id, window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("proposal-edit-{id}")))
                        .small()
                        .label("Edit…")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_proposal(id, window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("proposal-dismiss-{id}")))
                        .ghost()
                        .small()
                        .label("Dismiss")
                        .on_click(cx.listener(move |this, _, _, cx| this.dismiss_proposal(id, cx))),
                ),
        );
    }
    card.into_any_element()
}

fn decided_line(label: &str, color: Hsla, tool_name: &str, input: &Value, cx: &App) -> AnyElement {
    h_flex()
        .gap_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::ShieldCheck).xsmall().text_color(color))
        .child(format!("{label}: {tool_name}"))
        .child(
            div()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(tool_summary(tool_name, input)),
        )
        .into_any_element()
}

fn attention_card(cx: &App) -> Div {
    v_flex()
        .w_full()
        .p_3()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().warning)
        .bg(cx.theme().warning.opacity(0.08))
}

pub(super) fn approval_card(
    id: ItemId,
    request_id: &str,
    tool_name: &str,
    description: &Option<String>,
    input: &Value,
    decision: Option<ApprovalDecision>,
    cx: &Context<ThreadView>,
) -> AnyElement {
    if let Some(decision) = decision {
        let (label, color) = match decision {
            ApprovalDecision::Allowed => ("Allowed", cx.theme().success),
            ApprovalDecision::AllowedForSession => ("Allowed for session", cx.theme().success),
            ApprovalDecision::Denied => ("Denied", cx.theme().danger),
        };
        return decided_line(label, color, tool_name, input, cx);
    }
    let summary = tool_summary(tool_name, input);
    let (allow, session_allow, deny) = (
        request_id.to_string(),
        request_id.to_string(),
        request_id.to_string(),
    );
    attention_card(cx)
        .child(
            h_flex()
                .gap_2()
                .child(
                    Icon::new(IconName::ShieldAlert)
                        .small()
                        .text_color(cx.theme().warning),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{tool_name} wants permission")),
                ),
        )
        .when_some(description.clone(), |this, d| {
            this.child(div().text_sm().child(d))
        })
        .when(!summary.is_empty(), |this| {
            this.child(mono_block(summary, cx))
        })
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("allow-{id}")))
                        .primary()
                        .small()
                        .label("Allow")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.respond(&allow, ApprovalDecision::Allowed, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("allow-session-{id}")))
                        .small()
                        .label("Allow for session")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.respond(&session_allow, ApprovalDecision::AllowedForSession, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("deny-{id}")))
                        .ghost()
                        .small()
                        .label("Deny")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.respond(&deny, ApprovalDecision::Denied, cx)
                        })),
                ),
        )
        .into_any_element()
}

/// `examined`: a cross-examination was started (or can't be: a side chat).
pub(super) fn plan_card(
    id: ItemId,
    request_id: &str,
    input: &Value,
    decision: Option<ApprovalDecision>,
    provider: elyra_core::ProviderKind,
    examined: bool,
    cx: &Context<ThreadView>,
) -> AnyElement {
    let plan = input["plan"].as_str().unwrap_or("").to_string();
    let pending = decision.is_none();
    let title = match decision {
        None => "Plan ready for review",
        Some(ApprovalDecision::Denied) => "Plan sent back for changes",
        Some(_) => "Plan approved",
    };
    let mut card = v_flex()
        .w_full()
        .p_3()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(if pending {
            cx.theme().info
        } else {
            cx.theme().border
        })
        .when(pending, |this| this.bg(cx.theme().info.opacity(0.06)))
        .child(
            h_flex()
                .gap_2()
                .child(
                    Icon::new(IconName::NotebookPen)
                        .small()
                        .text_color(cx.theme().info),
                )
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
        )
        .when(!plan.is_empty(), |this| {
            this.child(
                TextView::markdown(SharedString::from(format!("plan-{id}")), plan.clone())
                    .selectable(true),
            )
        });
    if pending {
        let (auto, ask, keep) = (
            request_id.to_string(),
            request_id.to_string(),
            request_id.to_string(),
        );
        card = card.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("plan-auto-{id}")))
                        .primary()
                        .small()
                        .label("Approve, accept edits")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.approve_plan(&auto, Some(PermissionMode::AcceptEdits), cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("plan-ask-{id}")))
                        .small()
                        .label("Approve, ask for edits")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.approve_plan(&ask, Some(PermissionMode::Ask), cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("plan-keep-{id}")))
                        .ghost()
                        .small()
                        .label("Keep planning")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.approve_plan(&keep, None, cx)),
                        ),
                )
                .when(!examined, |this| {
                    let examiners: Vec<elyra_core::ProviderKind> =
                        crate::preferences::Preferences::global(cx)
                            .enabled_providers()
                            .into_iter()
                            .filter(|kind| *kind != provider)
                            .collect();
                    if examiners.is_empty() {
                        return this;
                    }
                    let (view, request) = (cx.weak_entity(), request_id.to_string());
                    this.child(
                        Button::new(SharedString::from(format!("plan-examine-{id}")))
                            .ghost()
                            .small()
                            .icon(IconName::MessagesSquare)
                            .label("Cross-examine")
                            .tooltip("Have another agent check the plan against the code before anything is written")
                            .dropdown_caret(true)
                            .dropdown_menu(move |mut menu, _, _| {
                                for kind in &examiners {
                                    let (view, request, kind) = (view.clone(), request.clone(), *kind);
                                    menu = menu.item(PopupMenuItem::new(kind.label()).on_click(
                                        move |_, _, cx| {
                                            let _ = view.update(cx, |this, cx| {
                                                this.session.update(cx, |session, cx| {
                                                    session.cross_examine(&request, kind, cx)
                                                })
                                            });
                                        },
                                    ));
                                }
                                menu
                            }),
                    )
                }),
        );
    }
    card.into_any_element()
}

/// Another agent's objections to a plan, each to take or leave, and the
/// way back to the planning agent.
#[allow(clippy::too_many_arguments)]
pub(super) fn cross_examination_card(
    id: ItemId,
    request_id: &str,
    examiner: elyra_core::ProviderKind,
    chat: elyra_core::ThreadId,
    objections: &[elyra_core::Objection],
    state: &elyra_core::ExamState,
    session: &ThreadSession,
    cx: &Context<ThreadView>,
) -> AnyElement {
    use elyra_core::ExamState;
    let muted = cx.theme().muted_foreground;
    let decidable = matches!(state, ExamState::Done) && session.plan_pending(request_id);
    let taken = objections.iter().filter(|o| o.taken == Some(true)).count();
    let title = match state {
        ExamState::Running => format!("{} is cross-examining the plan…", examiner.label()),
        ExamState::Done | ExamState::Sent if objections.is_empty() => {
            format!("{} found nothing to change in the plan", examiner.label())
        }
        ExamState::Done | ExamState::Sent => format!(
            "{} objects to the plan ({})",
            examiner.label(),
            objections.len()
        ),
        ExamState::Failed { .. } => format!("{}'s cross-examination", examiner.label()),
    };
    let open_chat = move |window: &mut Window, cx: &mut App| {
        if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
            let workspace = hub.workspace.clone();
            let _ = workspace.update(cx, |this, cx| this.show_side_chat(chat, window, cx));
        }
    };
    v_flex()
        .w_full()
        .p_2()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(if decidable && !objections.is_empty() {
            cx.theme().info
        } else {
            cx.theme().border
        })
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .map(|this| match state {
                    ExamState::Running => this.child(Spinner::new().small()),
                    _ => this.child(
                        Icon::new(IconName::MessagesSquare)
                            .small()
                            .text_color(cx.theme().info),
                    ),
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .child(
                    Button::new(SharedString::from(format!("exam-chat-{id}")))
                        .xsmall()
                        .ghost()
                        .label("Open the chat")
                        .on_click(move |_, window, cx| open_chat(window, cx)),
                ),
        )
        .when_some(
            match state {
                ExamState::Failed { reason } => Some(reason.clone()),
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
        .children(objections.iter().enumerate().map(|(at, objection)| {
            let choice = |key: &'static str, label: &'static str, take: bool| {
                let chosen = objection.taken == Some(take);
                Button::new(SharedString::from(format!("exam-{key}-{id}-{at}")))
                    .xsmall()
                    .label(label)
                    .map(|b| if chosen { b.primary() } else { b.outline() })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.session
                            .update(cx, |session, cx| session.set_objection(id, at, take, cx))
                    }))
            };
            v_flex()
                .gap_0p5()
                .pl_2()
                .border_l_2()
                .border_color(match objection.taken {
                    Some(true) => cx.theme().success,
                    Some(false) => cx.theme().border,
                    None => cx.theme().warning,
                })
                .child(
                    div()
                        .text_sm()
                        .whitespace_normal()
                        .when(objection.taken == Some(false), |this| {
                            this.text_color(muted)
                        })
                        .child(objection.point.clone()),
                )
                .when(!objection.why.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(muted)
                            .child(objection.why.clone()),
                    )
                })
                .when(!objection.instead.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(muted)
                            .child(format!("Instead: {}", objection.instead)),
                    )
                })
                .when(decidable, |this| {
                    this.child(
                        h_flex()
                            .pt_0p5()
                            .gap_2()
                            .child(choice("take", "Take it", true))
                            .child(choice("keep", "Keep the plan", false)),
                    )
                })
                .into_any_element()
        }))
        .when(decidable && !objections.is_empty(), |this| {
            this.child(
                h_flex().gap_2().child(
                    Button::new(SharedString::from(format!("exam-revise-{id}")))
                        .small()
                        .primary()
                        .label(if taken == 0 {
                            "Revise the plan".to_string()
                        } else {
                            format!("Revise the plan ({taken})")
                        })
                        .disabled(taken == 0)
                        .tooltip("Send the planning agent back with the points you took")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.session
                                .update(cx, |session, cx| session.revise_plan(id, cx))
                        })),
                ),
            )
        })
        .when(matches!(state, ExamState::Sent), |this| {
            this.child(div().text_xs().text_color(muted).child(format!(
                "Sent back to the planning agent with {taken} point{}.",
                if taken == 1 { "" } else { "s" }
            )))
        })
        .into_any_element()
}

fn answer_summary(request: &QuestionRequest, answer: &QuestionAnswer) -> String {
    match answer {
        QuestionAnswer::Answers { answers } => request
            .questions
            .iter()
            .zip(answers)
            .map(|(q, a)| {
                let label = if q.header.is_empty() {
                    q.question.as_str()
                } else {
                    q.header.as_str()
                };
                if label.is_empty() {
                    a.clone()
                } else {
                    format!("{label}: {a}")
                }
            })
            .collect::<Vec<_>>()
            .join(" · "),
        QuestionAnswer::Confirmed { confirmed } => {
            format!(
                "{}: {}",
                request.title,
                if *confirmed { "Yes" } else { "No" }
            )
        }
        QuestionAnswer::Cancelled => "Question dismissed".to_string(),
    }
}

pub(super) fn question_card(
    id: ItemId,
    request: &QuestionRequest,
    answer: Option<&QuestionAnswer>,
    form: Option<&QuestionForm>,
    cx: &Context<ThreadView>,
) -> AnyElement {
    if let Some(answer) = answer {
        return h_flex()
            .gap_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::MessageCircleQuestionMark).xsmall())
            .child(div().flex_1().child(answer_summary(request, answer)))
            .into_any_element();
    }
    let request_id = request.request_id.clone();
    let title = if request.title.is_empty() {
        "The agent has a question".to_string()
    } else {
        request.title.clone()
    };
    let mut card = attention_card(cx).child(
        h_flex()
            .gap_2()
            .child(
                Icon::new(IconName::MessageCircleQuestionMark)
                    .small()
                    .text_color(cx.theme().warning),
            )
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
    );
    if request.kind == QuestionKind::Confirm {
        let header = request
            .questions
            .first()
            .map(|q| q.header.clone())
            .unwrap_or_default();
        let (yes, no) = (request_id.clone(), request_id.clone());
        return card
            .when(!header.is_empty(), |this| {
                this.child(div().text_sm().child(header))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("q-yes-{id}")))
                            .primary()
                            .small()
                            .label("Yes")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.answer(&yes, QuestionAnswer::Confirmed { confirmed: true }, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("q-no-{id}")))
                            .small()
                            .label("No")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.answer(&no, QuestionAnswer::Confirmed { confirmed: false }, cx)
                            })),
                    ),
            )
            .into_any_element();
    }
    for (qi, question) in request.questions.iter().enumerate() {
        let mut block = v_flex().gap_1();
        let show_question = request.kind == QuestionKind::Choice && !question.question.is_empty();
        if show_question || !question.header.is_empty() {
            block = block.child(
                h_flex()
                    .gap_2()
                    .when(!question.header.is_empty(), |this| {
                        this.child(
                            div()
                                .px_1p5()
                                .rounded_sm()
                                .bg(cx.theme().muted)
                                .text_xs()
                                .child(question.header.clone()),
                        )
                    })
                    .when(show_question, |this| {
                        this.child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(question.question.clone()),
                        )
                    }),
            );
        }
        for (oi, option) in question.options.iter().enumerate() {
            let selected =
                form.is_some_and(|f| f.selected.get(qi).is_some_and(|s| s.contains(&oi)));
            let rid = request_id.clone();
            let multi = question.multi_select;
            let icon = match (multi, selected) {
                (true, true) => IconName::SquareCheck,
                (true, false) => IconName::Square,
                (false, true) => IconName::CircleDot,
                (false, false) => IconName::Circle,
            };
            block = block.child(
                h_flex()
                    .id(SharedString::from(format!("q-{id}-{qi}-{oi}")))
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if selected {
                        cx.theme().ring
                    } else {
                        cx.theme().border
                    })
                    .when(selected, |this| this.bg(cx.theme().accent))
                    .cursor_pointer()
                    .child(Icon::new(icon).xsmall())
                    .child(
                        v_flex()
                            .child(div().text_sm().child(option.label.clone()))
                            .when(!option.description.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(option.description.clone()),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_option(&rid, qi, oi, multi, cx)
                    })),
            );
        }
        if let Some(input) = form.and_then(|f| f.texts.get(qi)) {
            block = block.child(Input::new(input).small());
        }
        card = card.child(block);
    }
    let (submit, dismiss) = (request_id.clone(), request_id);
    card.child(
        h_flex()
            .gap_2()
            .child(
                Button::new(SharedString::from(format!("q-submit-{id}")))
                    .primary()
                    .small()
                    .label("Submit")
                    .on_click(cx.listener(move |this, _, _, cx| this.submit_question(&submit, cx))),
            )
            .child(
                Button::new(SharedString::from(format!("q-dismiss-{id}")))
                    .ghost()
                    .small()
                    .label("Dismiss")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.answer(&dismiss, QuestionAnswer::Cancelled, cx)
                    })),
            ),
    )
    .into_any_element()
}
