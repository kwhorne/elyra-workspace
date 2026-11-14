//! The thread's settings and details: provider, account, model, effort,
//! permission mode, debug mode, title, pin, recap, notes and Félagi issue.

use super::*;

impl ThreadSession {
    /// Change provider; only before the conversation has started.
    pub fn set_provider(&mut self, provider: ProviderKind, cx: &mut Context<Self>) {
        if self.has_started() || self.thread.provider == provider {
            return;
        }
        self.stop_provider();
        self.thread.provider = provider;
        self.thread.model = None;
        self.thread.effort = None;
        self.thread.account = None;
        self.commands.clear();
        self.models = self
            .app
            .read_with(cx, |app, _| cached_models(&app.store, provider))
            .unwrap_or_default();
        self.set_setting("default_provider", provider.as_str(), cx);
        self.save_thread(cx);
        cx.notify();
    }

    /// Pick a provider account; only before the first message, since a
    /// session belongs to the account that started it.
    pub fn set_account(&mut self, account: Option<String>, cx: &mut Context<Self>) {
        if self.has_started() || self.thread.account == account {
            return;
        }
        self.stop_provider();
        self.thread.account = account;
        self.save_thread(cx);
        cx.notify();
    }

    /// Apply a starred provider/model/effort preset.
    pub fn apply_preset(
        &mut self,
        preset: &crate::preferences::ModelPreset,
        cx: &mut Context<Self>,
    ) {
        if let Some(kind) = ProviderKind::parse(&preset.provider) {
            if kind != self.thread.provider {
                if self.has_started() {
                    return;
                }
                self.set_provider(kind, cx);
            }
            self.set_model(Some(preset.model.clone()), cx);
            if preset.effort.is_some() {
                self.set_effort(preset.effort.clone(), cx);
            }
        }
    }

    pub fn set_permission_mode(&mut self, mode: PermissionMode, cx: &mut Context<Self>) {
        self.thread.permission_mode = mode;
        if let Some(provider) = &self.provider
            && let Err(err) = provider.set_permission_mode(mode)
        {
            log::warn!("set_permission_mode failed: {err:#}");
        }
        self.set_setting("default_permission_mode", mode.as_str(), cx);
        self.save_thread(cx);
        cx.notify();
    }

    pub fn set_model(&mut self, model: Option<String>, cx: &mut Context<Self>) {
        self.thread.model = model.filter(|m| !m.is_empty());
        if let Some(provider) = &self.provider
            && let Err(err) = provider.set_model(self.thread.model.as_deref())
        {
            log::warn!("set_model failed: {err:#}");
        }
        let key = format!("default_model.{}", self.thread.provider.as_str());
        let value = self.thread.model.clone().unwrap_or_default();
        self.set_setting(&key, &value, cx);
        self.save_thread(cx);
        cx.notify();
    }

    pub fn set_effort(&mut self, effort: Option<String>, cx: &mut Context<Self>) {
        self.thread.effort = effort.filter(|e| !e.is_empty());
        if let Some(provider) = &self.provider {
            match provider.set_effort(self.thread.effort.as_deref()) {
                Ok(true) => {}
                Ok(false) => self.needs_restart = true,
                Err(err) => log::warn!("set_effort failed: {err:#}"),
            }
        }
        let key = format!("default_effort.{}", self.thread.provider.as_str());
        let value = self.thread.effort.clone().unwrap_or_default();
        self.set_setting(&key, &value, cx);
        self.save_thread(cx);
        cx.notify();
    }

    /// Reproduce-first debugging: changes the agent's instructions, so it
    /// takes effect from the next message.
    pub fn set_debug_mode(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.thread.debug_mode == on {
            return;
        }
        self.thread.debug_mode = on;
        if self.provider.is_some() {
            self.needs_restart = true;
        }
        self.save_thread(cx);
        cx.notify();
    }

    /// Ask the provider for a short title based on the conversation.
    pub fn generate_title(&mut self, cx: &mut Context<Self>) {
        let excerpt: String = self
            .items
            .iter()
            .filter_map(|item| match &item.content {
                ItemContent::User { text, .. } => Some(format!("User: {text}")),
                ItemContent::Assistant {
                    text,
                    parent_tool_use_id: None,
                } => Some(format!("Assistant: {text}")),
                _ => None,
            })
            .take(6)
            .collect::<Vec<_>>()
            .join("\n\n")
            .chars()
            .take(4000)
            .collect();
        if excerpt.is_empty() {
            return;
        }
        let prompt = format!(
            "Write a short title (3-7 words, no quotes, no trailing period) for this coding task conversation. Reply with the title only.\n\n{excerpt}"
        );
        let (kind, cwd) = (self.thread.provider, self.working_dir());
        let settings = crate::preferences::Preferences::global(cx)
            .cli_settings(kind, self.thread.account.as_deref());
        let job = cx
            .background_executor()
            .spawn(async move { elyra_provider::generate_text(kind, &cwd, &prompt, &settings) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(text) => {
                    let title = elyra_provider::clean_title(&text);
                    if !title.is_empty() {
                        this.rename(title, cx);
                    }
                }
                Err(err) => this.notice(format!("Could not generate a title: {err:#}"), true, cx),
            });
        })
        .detach();
    }

    pub fn toggle_pin(&mut self, id: ItemId, cx: &mut Context<Self>) {
        let pinned = &mut self.thread.pinned_items;
        match pinned.iter().position(|item| *item == id) {
            Some(index) => {
                pinned.remove(index);
            }
            None => pinned.push(id),
        }
        self.save_thread(cx);
        cx.notify();
    }

    /// Summarise the conversation so far into `thread.recap`.
    pub fn generate_recap(&mut self, cx: &mut Context<Self>) {
        let mut excerpt: Vec<String> = self
            .items
            .iter()
            .filter_map(|item| match &item.content {
                ItemContent::User { text, .. } => Some(format!("User: {text}")),
                ItemContent::Assistant {
                    text,
                    parent_tool_use_id: None,
                } => Some(format!("Assistant: {text}")),
                _ => None,
            })
            .collect();
        // Keep the most recent part of long conversations.
        let mut total = 0;
        let keep = excerpt
            .iter()
            .rev()
            .take_while(|text| {
                total += text.len();
                total < 16_000
            })
            .count();
        let excerpt = excerpt.split_off(excerpt.len() - keep).join("\n\n");
        if excerpt.is_empty() {
            return;
        }
        self.recapping = true;
        cx.notify();
        let prompt = format!(
            "Summarise this coding conversation for someone picking it up later: the goal, what has been done, decisions made, and what is left. Use at most 8 short bullet points. Reply with the bullets only.\n\n{excerpt}"
        );
        let (kind, cwd) = (self.thread.provider, self.working_dir());
        let settings = crate::preferences::Preferences::global(cx)
            .cli_settings(kind, self.thread.account.as_deref());
        let job = cx
            .background_executor()
            .spawn(async move { elyra_provider::generate_text(kind, &cwd, &prompt, &settings) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.recapping = false;
                match result {
                    Ok(text) => {
                        this.thread.recap = Some(text.trim().to_string());
                        this.save_thread(cx);
                    }
                    Err(err) => this.notice(format!("Could not write a recap: {err:#}"), true, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn set_notes(&mut self, notes: String, cx: &mut Context<Self>) {
        let notes = Some(notes).filter(|n| !n.trim().is_empty());
        if self.thread.notes != notes {
            self.thread.notes = notes;
            self.save_thread(cx);
        }
    }

    /// Work on a Félagi issue from this thread.
    pub fn link_felagi(&mut self, issue: String, cx: &mut Context<Self>) {
        self.thread.felagi_issue = Some(issue);
        self.save_thread(cx);
        cx.notify();
    }

    pub fn rename(&mut self, title: String, cx: &mut Context<Self>) {
        let title = title.trim();
        if title.is_empty() {
            return;
        }
        self.thread.title = title.chars().take(120).collect();
        self.save_thread(cx);
        cx.notify();
    }

    // ---- provider events ----------------------------------------------------
}
