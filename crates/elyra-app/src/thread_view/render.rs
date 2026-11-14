//! Drawing the thread: where it runs, the controls under the composer, an
//! interrupted turn, the queue, the empty state and the whole view.

use super::*;

pub(super) fn context_meter(session: &ThreadSession, cx: &App) -> AnyElement {
    let (Some(used), Some(window)) = (session.usage.context_tokens, session.usage.context_window)
    else {
        return div().into_any_element();
    };
    let ratio = (used as f32 / window.max(1) as f32).clamp(0., 1.);
    let color = if ratio > 0.85 {
        cx.theme().danger
    } else if ratio > 0.6 {
        cx.theme().warning
    } else {
        cx.theme().muted_foreground
    };
    let cost = session
        .usage
        .cost_usd
        .map(|c| format!(" · ${c:.2}"))
        .unwrap_or_default();
    let tooltip = format!(
        "Context: {} of {} tokens{cost}. Click to see what fills it.",
        format_tokens(used),
        format_tokens(window)
    );
    h_flex()
        .id("context-meter")
        .gap_1()
        .px_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(
            div()
                .w(px(36.))
                .h(px(4.))
                .rounded_full()
                .bg(cx.theme().muted)
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .w(relative(ratio.max(0.03)))
                        .bg(color),
                ),
        )
        .child(format!("{}%", (ratio * 100.).round() as u32))
        .cursor_pointer()
        .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::ShowContext), cx))
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
        })
        .into_any_element()
}

impl Render for ThreadView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_forms(window, cx);
        self.sync_rows(cx);
        self.sync_reply(cx);
        let controls = self.render_controls(cx);
        let queue = self.render_queue(cx);
        let interrupted = self.render_interrupted(cx);
        let attachments = self.render_attachments(cx);
        let page_errors = self.render_page_errors(cx);
        let route = self.render_route(cx);
        let popup = self.render_popup(cx);
        let find = self.render_find(cx);
        let width = crate::preferences::Preferences::global(cx).chat_width;
        let rows = list(
            self.list.clone(),
            cx.processor(|this, ix: usize, window, cx| this.render_list_row(ix, window, cx)),
        )
        .size_full();

        v_flex()
            .key_context("Composer")
            .on_action(cx.listener(Self::interrupt))
            .on_action(cx.listener(Self::open_find))
            .size_full()
            .child(self.render_location(cx))
            .children(find)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(rows)
                    .vertical_scrollbar(&self.list),
            )
            .child(
                div().w_full().px_6().pb_4().child(
                    v_flex()
                        .id("composer-box")
                        .w_full()
                        .when(width > 0., |this| this.max_w(px(width)))
                        .mx_auto()
                        .gap_1()
                        .children(interrupted)
                        .children(popup)
                        .children(queue)
                        .children(self.felagi.clone())
                        .children(page_errors)
                        .children(route)
                        .child(
                            v_flex()
                                .id("composer-frame")
                                .w_full()
                                .p_2()
                                .gap_2()
                                .rounded_xl()
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().background)
                                .shadow_sm()
                                .drag_over::<ExternalPaths>(|style, _, _, cx| {
                                    style.border_color(cx.theme().ring)
                                })
                                .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                                    this.attach_paths(paths.paths(), window, cx)
                                }))
                                .children(attachments)
                                .child(Textarea::new(&self.composer).appearance(false))
                                .child(controls),
                        ),
                ),
            )
    }
}

impl ThreadView {
    /// Which project, folder and branch the thread works in: tabs from
    /// several projects look alike otherwise.
    pub(super) fn render_location(&self, cx: &Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let project = &session.project;
        let color = project
            .color
            .as_deref()
            .and_then(crate::sidebar::parse_color)
            .unwrap_or(cx.theme().muted_foreground);
        let dir = session.working_dir();
        let shown = match dirs::home_dir()
            .and_then(|home| dir.strip_prefix(home).ok().map(|rest| rest.to_path_buf()))
        {
            Some(rest) => format!("~/{}", rest.display()),
            None => dir.display().to_string(),
        };
        let worktree = matches!(session.thread.environment, Environment::Worktree { .. });
        let reveal = dir.clone();
        h_flex()
            .flex_none()
            .w_full()
            .px_4()
            .py_1()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(match project.icon.as_deref().filter(|i| !i.is_empty()) {
                Some(icon) => div().flex_none().child(icon.to_string()).into_any_element(),
                None => div()
                    .flex_none()
                    .size(px(8.))
                    .rounded_full()
                    .bg(color)
                    .into_any_element(),
            })
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(project.name.clone()),
            )
            .child(
                div()
                    .id("thread-location")
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .font_family(cx.theme().mono_font_family.clone())
                    .cursor_pointer()
                    .hover(|this| this.text_color(cx.theme().foreground))
                    .child(shown)
                    .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
            )
            .children(self.branch.clone().map(|branch| {
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(Icon::new(IconName::GitBranch).xsmall())
                    .child(branch)
            }))
            .when(worktree, |this| {
                this.child(
                    div()
                        .flex_none()
                        .px_1()
                        .rounded_sm()
                        .bg(cx.theme().muted)
                        .child("worktree"),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_controls(&self, cx: &Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let caps = session.capabilities();
        let busy = session.running || session.preparing.is_some();
        let started = session.has_started();
        let provider = session.thread.provider;
        let current_model = session.thread.model.clone().unwrap_or_default();
        let model_label: SharedString = session
            .models
            .iter()
            .find(|m| m.id == current_model)
            .map(|m| m.label.clone())
            .unwrap_or_else(|| {
                if current_model.is_empty() {
                    "Default model".into()
                } else {
                    current_model.clone()
                }
            })
            .into();
        let models = session.models.clone();
        let prefs = crate::preferences::Preferences::global(cx);
        let enabled = prefs.enabled_providers();
        let custom_launch = prefs
            .launch(ProviderKind::CustomAcp, None)
            .executable
            .is_some();
        let presets = prefs.starred_models.clone();
        let accounts: Vec<String> = prefs
            .provider(provider)
            .accounts()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let account = session.thread.account.clone();
        let current_preset = crate::preferences::ModelPreset {
            provider: provider.as_str().to_string(),
            model: current_model.clone(),
            effort: session.thread.effort.clone(),
        };
        let starred = prefs.is_starred(&current_preset);
        let weak_account = self.session.downgrade();
        let effort = session.thread.effort.clone().unwrap_or_default();
        let effort_label: SharedString = caps
            .efforts
            .iter()
            .find(|(id, _)| *id == effort)
            .map(|(_, label)| format!("Effort: {label}"))
            .unwrap_or_else(|| "Effort".into())
            .into();
        let efforts = caps.efforts;
        let mode = session.thread.permission_mode;
        let can_choose_env = !started
            && self.is_git_repo
            && matches!(session.thread.environment, Environment::Local);
        let env_label: SharedString = match &session.thread.environment {
            Environment::Local if session.use_worktree => "New worktree".into(),
            Environment::Local => "Local".into(),
            Environment::Worktree { branch, .. } => branch.clone().into(),
        };
        let env_icon =
            if session.use_worktree || !matches!(session.thread.environment, Environment::Local) {
                IconName::GitBranch
            } else {
                IconName::Folder
            };
        let has_text =
            !self.composer.read(cx).value().trim().is_empty() || !self.attachments.is_empty();
        let weak = self.session.downgrade();
        let (weak_model, weak_effort, weak_mode) = (weak.clone(), weak.clone(), weak.clone());

        // Options wrap on narrow panels; the meter and Send stay right.
        let options = h_flex()
            .flex_1()
            .min_w_0()
            .flex_wrap()
            .gap_1()
            .child(
                Button::new("attach")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Paperclip)
                    .tooltip("Attach files or images (or paste/drop them)")
                    .on_click(cx.listener(|this, _, window, cx| this.pick_files(window, cx))),
            )
            .child(
                Button::new("provider")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Bot)
                    .label(provider.label())
                    .dropdown_caret(!started)
                    .disabled(started)
                    .dropdown_menu(move |mut menu, _, _| {
                        for kind in enabled.iter().copied() {
                            let session = weak.clone();
                            let installed = elyra_provider::is_installed(kind)
                                || (kind == ProviderKind::CustomAcp && custom_launch);
                            let label = if installed {
                                kind.label().to_string()
                            } else {
                                format!("{} (not installed)", kind.label())
                            };
                            menu = menu.item(
                                PopupMenuItem::new(label)
                                    .checked(kind == provider)
                                    .disabled(!installed)
                                    .on_click(move |_, _, cx| {
                                        let _ =
                                            session.update(cx, |s, cx| s.set_provider(kind, cx));
                                    }),
                            );
                        }
                        menu
                    }),
            )
            .child(
                Button::new("model")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Sparkles)
                    .label(model_label)
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        menu = menu.scrollable(true).max_h(px(420.));
                        if !presets.is_empty() {
                            menu = menu.label("Starred");
                            for preset in &presets {
                                let kind = ProviderKind::parse(&preset.provider);
                                let label = format!(
                                    "★ {} · {}",
                                    kind.map(|k| k.label()).unwrap_or("?"),
                                    if preset.model.is_empty() {
                                        "Default"
                                    } else {
                                        &preset.model
                                    }
                                );
                                let other_provider = kind != Some(provider);
                                let session = weak_model.clone();
                                let preset = preset.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(label)
                                        .disabled(other_provider && started)
                                        .on_click(move |_, _, cx| {
                                            let _ = session
                                                .update(cx, |s, cx| s.apply_preset(&preset, cx));
                                        }),
                                );
                            }
                            menu = menu.separator();
                        }
                        for model in &models {
                            let session = weak_model.clone();
                            let id = model.id.clone();
                            menu = menu.item(
                                PopupMenuItem::new(model.label.clone())
                                    .checked(current_model == id)
                                    .on_click(move |_, _, cx| {
                                        let id = id.clone();
                                        let _ =
                                            session.update(cx, |s, cx| s.set_model(Some(id), cx));
                                    }),
                            );
                        }
                        let preset = current_preset.clone();
                        menu.separator().item(
                            PopupMenuItem::new(if starred {
                                "Unstar this model"
                            } else {
                                "Star this model"
                            })
                            .on_click(move |_, _, cx| {
                                let preset = preset.clone();
                                crate::preferences::update(cx, |p| {
                                    if p.is_starred(&preset) {
                                        p.starred_models.retain(|s| {
                                            !(s.provider == preset.provider
                                                && s.model == preset.model)
                                        });
                                    } else {
                                        p.starred_models.push(preset);
                                    }
                                });
                            }),
                        )
                    }),
            )
            .child({
                let debug = session.thread.debug_mode;
                let button = Button::new("debug-mode")
                    .xsmall()
                    .icon(IconName::Bug)
                    .tooltip(if debug {
                        "Debug mode on: reproduce first, then fix. Click to turn off."
                    } else {
                        "Debug mode: reproduce the bug before fixing it"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.session
                            .update(cx, |session, cx| session.set_debug_mode(!debug, cx));
                    }));
                if debug {
                    button.warning().label("Debug")
                } else {
                    button.ghost()
                }
            })
            .when(!accounts.is_empty(), |this| {
                let label: SharedString = account
                    .clone()
                    .unwrap_or_else(|| "Default account".into())
                    .into();
                this.child(
                    Button::new("account")
                        .ghost()
                        .xsmall()
                        .icon(IconName::CircleUser)
                        .label(label)
                        .dropdown_caret(!started)
                        .disabled(started)
                        .dropdown_menu(move |mut menu, _, _| {
                            let options =
                                std::iter::once(None).chain(accounts.iter().cloned().map(Some));
                            for option in options {
                                let session = weak_account.clone();
                                let label =
                                    option.clone().unwrap_or_else(|| "Default account".into());
                                menu = menu.item(
                                    PopupMenuItem::new(label)
                                        .checked(option == account)
                                        .on_click(move |_, _, cx| {
                                            let option = option.clone();
                                            let _ = session
                                                .update(cx, |s, cx| s.set_account(option, cx));
                                        }),
                                );
                            }
                            menu
                        }),
                )
            })
            .when(!efforts.is_empty(), |this| {
                this.child(
                    Button::new("effort")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Gauge)
                        .label(effort_label)
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            let options =
                                std::iter::once(("", "Default")).chain(efforts.iter().copied());
                            for (id, label) in options {
                                let session = weak_effort.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(label).checked(effort == id).on_click(
                                        move |_, _, cx| {
                                            let _ = session.update(cx, |s, cx| {
                                                s.set_effort(Some(id.to_string()), cx)
                                            });
                                        },
                                    ),
                                );
                            }
                            menu
                        }),
                )
            })
            .when(caps.permission_modes, |this| {
                this.child(
                    Button::new("permission")
                        .ghost()
                        .xsmall()
                        .icon(match mode {
                            PermissionMode::FullAccess => IconName::ShieldOff,
                            PermissionMode::Plan => IconName::NotebookPen,
                            _ => IconName::ShieldCheck,
                        })
                        .label(mode.label())
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            for option in PermissionMode::ALL {
                                let session = weak_mode.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(option.label())
                                        .checked(option == mode)
                                        .on_click(move |_, _, cx| {
                                            let _ = session.update(cx, |s, cx| {
                                                s.set_permission_mode(option, cx)
                                            });
                                        }),
                                );
                            }
                            menu
                        }),
                )
            })
            .child(
                Button::new("environment")
                    .ghost()
                    .xsmall()
                    .icon(env_icon)
                    .label(env_label)
                    .disabled(!can_choose_env)
                    .tooltip(if can_choose_env {
                        "Toggle between the project checkout and an isolated Git worktree"
                    } else {
                        "Where this thread works"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.use_worktree = !session.use_worktree;
                            cx.notify();
                        });
                    })),
            );
        let actions = h_flex()
            .flex_none()
            .gap_1()
            .child(context_meter(session, cx))
            .when(busy, |this| {
                this.child(
                    Button::new("stop")
                        .small()
                        .danger()
                        .icon(IconName::Square)
                        .label("Stop")
                        .tooltip("Interrupt the agent (⌃C)")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |session, cx| session.interrupt(cx));
                        })),
                )
            })
            .when(!busy || has_text, |this| {
                this.child(
                    Button::new("send")
                        .small()
                        .primary()
                        .icon(if busy {
                            IconName::ListPlus
                        } else {
                            IconName::ArrowUp
                        })
                        .label(if busy { "Queue" } else { "Send" })
                        .disabled(!has_text)
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                )
            });
        h_flex()
            .w_full()
            .gap_1()
            .items_end()
            .child(options)
            .child(actions)
            .into_any_element()
    }

    pub(super) fn render_interrupted(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session = self.session.read(cx);
        if session.thread.status != elyra_core::ThreadStatus::Interrupted || session.running {
            return None;
        }
        Some(
            h_flex()
                .w_full()
                .px_3()
                .py_2()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().warning)
                .bg(cx.theme().warning.opacity(0.08))
                .text_sm()
                .child(
                    Icon::new(IconName::CirclePause)
                        .small()
                        .text_color(cx.theme().warning),
                )
                .child(
                    div()
                        .flex_1()
                        .child("The last turn was interrupted when the app quit."),
                )
                .child(
                    Button::new("resume-interrupted")
                        .small()
                        .primary()
                        .label("Resume")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.resume_interrupted(cx));
                        })),
                )
                .child(
                    Button::new("dismiss-interrupted")
                        .small()
                        .ghost()
                        .label("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.dismiss_interrupted(cx));
                        })),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_queue(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session = self.session.read(cx);
        if session.queued.is_empty() && session.provider_queue.is_empty() {
            return None;
        }
        let can_steer = session.capabilities().steer && session.running;
        let rows = session.queued.iter().enumerate().map(|(index, prompt)| {
            let preview: String = prompt
                .text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            h_flex()
                .id(("queued", index))
                .gap_2()
                .text_sm()
                .child(
                    Icon::new(IconName::Clock)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(preview),
                )
                .when(can_steer, |this| {
                    this.child(
                        Button::new(("steer", index))
                            .ghost()
                            .xsmall()
                            .label("Send now")
                            .tooltip("Deliver into the running turn")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.session.update(cx, |s, cx| s.steer_queued(index, cx));
                            })),
                    )
                })
                .child(
                    Button::new(("edit-queued", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Pencil)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let prompt =
                                this.session.update(cx, |s, cx| s.remove_queued(index, cx));
                            if let Some(prompt) = prompt {
                                this.edit_message(prompt.text, window, cx);
                            }
                        })),
                )
                .child(
                    Button::new(("drop-queued", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.remove_queued(index, cx));
                        })),
                )
        });
        let pending = session.provider_queue.iter().map(|text| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!(
                    "↳ steering: {}",
                    text.chars().take(120).collect::<String>()
                ))
        });
        Some(
            v_flex()
                .w_full()
                .px_2()
                .gap_1()
                .children(rows)
                .children(pending)
                .into_any_element(),
        )
    }

    pub(super) fn empty_state(&self, project_name: String, cwd: String, cx: &App) -> AnyElement {
        v_flex()
            .pt(px(120.))
            .items_center()
            .gap_2()
            .child(
                Icon::new(IconName::Bot)
                    .large()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("What should we do in {project_name}?")),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(cwd),
            )
            .when_some(self.agent_hint.clone(), |this, hint| {
                this.child(
                    h_flex()
                        .pt_4()
                        .max_w(px(560.))
                        .gap_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::ChartColumn).xsmall())
                        .child(hint),
                )
            })
            .into_any_element()
    }
}
