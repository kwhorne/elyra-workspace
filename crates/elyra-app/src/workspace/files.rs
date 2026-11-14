//! The command palette, the large file editor and opening files in an editor.

use super::*;

impl Workspace {
    pub(super) fn open_palette(
        &mut self,
        mode: PaletteMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.active_cwd(cx).or_else(|| {
            let state = self.app.read(cx);
            self.current_project
                .and_then(|id| state.project(id))
                .map(|p| p.path.clone())
        });
        let app = self.app.clone();
        let palette = cx.new(|cx| Palette::new(mode, app, root, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &palette,
            window,
            |this, _, event: &PaletteEvent, window, cx| {
                this.palette = None;
                if let PaletteEvent::Pick(target) = event {
                    this.pick(target.clone(), window, cx);
                } else {
                    this.on_focus_composer(&FocusComposer, window, cx);
                }
                cx.notify();
            },
        ));
        self.palette = Some(palette);
        cx.notify();
    }

    pub(super) fn pick(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Target::Action(action) => {
                window.focus(&self.focus, cx);
                window.dispatch_action(action(), cx);
            }
            Target::Thread(id) => self.activate(id, window, cx),
            Target::Project(id) => self.new_thread_in(id, window, cx),
            Target::Theme(name) => crate::preferences::update(cx, |p| p.theme = name),
            Target::File { path, line } => self.open_file(&path, line, window, cx),
        }
    }

    /// Show a file of the active thread's working directory in the Files tab.
    pub(crate) fn open_file(
        &mut self,
        path: &str,
        line: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_right_tab(RightTab::Files, window, cx);
        if let Some(files) = self.active.and_then(|id| self.files_views.get(&id)) {
            files.update(cx, |files, cx| files.open_file(path, line, window, cx));
        }
    }

    pub(super) fn on_command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteMode::Commands, window, cx);
    }

    pub(super) fn on_find_file(
        &mut self,
        _: &FindFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteMode::Files, window, cx);
    }

    pub(super) fn on_go_to_symbol(
        &mut self,
        _: &GoToSymbol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteMode::Symbols, window, cx);
    }

    pub(super) fn on_search_in_files(
        &mut self,
        _: &SearchInFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteMode::Search, window, cx);
    }

    // ---- panels ---------------------------------------------------------

    pub(super) fn on_show_files(
        &mut self,
        _: &ShowFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_right_tab(RightTab::Files, window, cx);
    }

    pub(super) fn on_open_files_editor(
        &mut self,
        _: &OpenFilesEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.files_sheet.is_some() && window.has_active_sheet(cx) {
            window.close_sheet(cx);
        } else {
            self.open_files_sheet(window, cx);
        }
    }

    /// Show the active thread's Files view in a sheet over half the window,
    /// where the editor has room; the side panel is too narrow for it.
    pub(crate) fn open_files_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active else {
            return;
        };
        if !self.files_views.contains_key(&id) {
            self.show_right_tab(RightTab::Files, window, cx);
        }
        let Some(view) = self.files_views.get(&id).cloned() else {
            return;
        };
        view.update(cx, |view, cx| view.set_in_sheet(true, cx));
        self.files_sheet = Some(id);
        let title = self
            .front_thread(cx)
            .map(|(project, _)| project)
            .unwrap_or_default();
        window.open_sheet(cx, move |sheet, _, _| {
            sheet
                .size(relative(0.5))
                .p_0()
                .title(title.clone())
                .child(div().size_full().child(view.clone()))
        });
        cx.notify();
    }

    /// Open the working directory, or the file open in the Files tab, in the
    /// preferred external editor.
    pub(crate) fn open_in_editor(
        &mut self,
        editor: Option<crate::editors::EditorApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cwd) = self.active_cwd(cx).or_else(|| {
            let state = self.app.read(cx);
            self.current_project
                .and_then(|id| state.project(id))
                .map(|p| p.path.clone())
        }) else {
            return;
        };
        let editor = editor.or_else(|| crate::preferences::Preferences::global(cx).editor());
        let Some(editor) = editor else {
            window.push_notification(
                Notification::warning("No external editor found. Pick one in Settings → General."),
                cx,
            );
            return;
        };
        let file = (self.right_open && self.right_tab == RightTab::Files)
            .then(|| self.active.and_then(|id| self.files_views.get(&id)))
            .flatten()
            .and_then(|files| files.read(cx).open_path());
        let target = file.unwrap_or(cwd);
        if let Err(err) = crate::editors::open(editor, &target, None) {
            window.push_notification(
                Notification::error(format!("Couldn't open {}: {err:#}", editor.name)),
                cx,
            );
        }
    }

    pub(super) fn on_open_in_editor(
        &mut self,
        _: &OpenInEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_in_editor(None, window, cx);
    }

    // ---- browser -----------------------------------------------------------
}
