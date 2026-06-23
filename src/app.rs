//! Application state, the in-memory tree lookups, and per-mode key handling.

use std::collections::HashSet;

use crossterm::event::KeyCode;
use ratatui::widgets::ListState;

use crate::model::{NavItem, Pane, Session, Target, Window};
use crate::tmux::{best_layout_for, fetch_tmux_state, run_tmux, switch_to_target};

/// Whether the event loop should keep running or exit the TUI.
pub enum Flow {
    Continue,
    Exit,
}

/// The interaction mode. Each variant carries exactly the data its screen needs.
#[derive(Clone, Debug)]
pub enum Mode {
    Normal,
    MovePaneSelect {
        src_pane_id: String,
        src_label: String,
    },
    MoveWindowSelect {
        src_session: String,
        src_window_index: usize,
        src_label: String,
    },
    SwapSelect {
        src_pane_id: String,
        src_label: String,
    },
    RenameInput {
        target: Target,
        input: String,
    },
    KillConfirm {
        target: Target,
        label: String,
    },
    NewSession {
        input: String,
    },
    NewWindow {
        session: String,
        input: String,
    },
    EqualizeConfirm {
        session: String,
        window_index: usize,
        window_name: String,
        num_panes: usize,
    },
    Error(String),
}

pub struct App {
    pub(crate) sessions: Vec<Session>,
    collapsed_sessions: HashSet<String>,
    collapsed_windows: HashSet<(String, usize)>,
    pub(crate) flat_items: Vec<NavItem>,
    pub(crate) list_state: ListState,
    pub(crate) mode: Mode,
    pub(crate) preview_content: Option<String>,
    pub(crate) show_preview: bool,
    pub(crate) last_previewed_pane_id: Option<String>,
}

impl App {
    pub fn new() -> Result<Self, String> {
        let sessions = fetch_tmux_state()?;
        let collapsed_sessions = HashSet::new();
        let collapsed_windows = HashSet::new();
        let flat_items = rebuild_flat_items(&sessions, &collapsed_sessions, &collapsed_windows);
        let mut list_state = ListState::default();
        if !flat_items.is_empty() {
            list_state.select(Some(0));
        }
        let mut app = Self {
            sessions,
            collapsed_sessions,
            collapsed_windows,
            flat_items,
            list_state,
            mode: Mode::Normal,
            preview_content: None,
            show_preview: true,
            last_previewed_pane_id: None,
        };
        app.update_preview();
        Ok(app)
    }

    // ── State refresh ──────────────────────────────────────────────────────

    /// Rebuild the flattened list from the in-memory session tree, without
    /// touching tmux. Use this for pure UI-state changes (collapse/expand) so
    /// navigating the tree never spawns a subprocess.
    fn rebuild_items(&mut self) {
        self.flat_items =
            rebuild_flat_items(&self.sessions, &self.collapsed_sessions, &self.collapsed_windows);
        let len = self.flat_items.len();
        if len == 0 {
            self.list_state.select(None);
        } else {
            let curr = self.list_state.selected().unwrap_or(0);
            self.list_state.select(Some(curr.min(len - 1)));
        }
    }

    /// Re-fetch the full state from tmux and rebuild the list. Use this only
    /// after a mutating command (kill, new, rename, move, equalize) has changed
    /// tmux's state. The preview is invalidated so it re-captures next frame.
    fn reload(&mut self) -> Result<(), String> {
        self.sessions = fetch_tmux_state()?;
        self.rebuild_items();
        self.last_previewed_pane_id = None;
        Ok(())
    }

    /// Run a mutating tmux command and, on success, reload state. Any tmux or
    /// reload error is surfaced through [`Mode::Error`].
    fn run_then_reload(&mut self, args: &[&str]) {
        match run_tmux(args) {
            Ok(_) => {
                if let Err(e) = self.reload() {
                    self.mode = Mode::Error(e);
                }
            }
            Err(e) => self.mode = Mode::Error(e),
        }
    }

    // ── Tree lookups ─────────────────────────────────────────────────────────

    pub(crate) fn find_session(&self, name: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.name == name)
    }

    pub(crate) fn find_window(&self, session: &str, index: usize) -> Option<&Window> {
        self.find_session(session)?.windows.iter().find(|w| w.index == index)
    }

    pub(crate) fn find_pane(&self, session: &str, window_index: usize, id: &str) -> Option<&Pane> {
        self.find_window(session, window_index)?.panes.iter().find(|p| p.id == id)
    }

    /// Initial value to seed a rename input with, looked up from current state.
    fn rename_seed(&self, target: &Target) -> String {
        match target {
            Target::Session(name) => name.clone(),
            Target::Window { session, index } => {
                self.find_window(session, *index).map(|w| w.name.clone()).unwrap_or_default()
            }
            Target::Pane { session, window_index, id } => match self.find_pane(session, *window_index, id) {
                Some(p) if !p.title.is_empty() => p.title.clone(),
                Some(p) => p.current_command.clone(),
                None => String::new(),
            },
        }
    }

    /// Resolve the window a `[e]qualize` would act on, plus its pane count.
    fn equalize_target_info(&self) -> Option<(String, usize, String, usize)> {
        let (session, window_index) = match self.current_target()? {
            Target::Window { session, index } => (session, index),
            Target::Pane { session, window_index, .. } => (session, window_index),
            Target::Session(_) => return None,
        };
        let w = self.find_window(&session, window_index)?;
        Some((session, window_index, w.name.clone(), w.panes.len()))
    }

    // ── Selection / navigation ────────────────────────────────────────────────

    pub(crate) fn current_target(&self) -> Option<Target> {
        let idx = self.list_state.selected()?;
        self.flat_items.get(idx).map(|item| item.target.clone())
    }

    fn current_label(&self) -> Option<&str> {
        let idx = self.list_state.selected()?;
        self.flat_items.get(idx).map(|item| item.label.as_str())
    }

    fn next(&mut self) {
        if self.flat_items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => if i >= self.flat_items.len() - 1 { 0 } else { i + 1 },
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.flat_items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => if i == 0 { self.flat_items.len() - 1 } else { i - 1 },
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    /// Collapse (`true`) or expand (`false`) the current session/window node.
    fn set_collapsed(&mut self, collapse: bool) {
        if let Some(target) = self.current_target() {
            match target {
                Target::Session(name) => {
                    if collapse {
                        self.collapsed_sessions.insert(name);
                    } else {
                        self.collapsed_sessions.remove(&name);
                    }
                }
                Target::Window { session, index } => {
                    let key = (session, index);
                    if collapse {
                        self.collapsed_windows.insert(key);
                    } else {
                        self.collapsed_windows.remove(&key);
                    }
                }
                Target::Pane { .. } => {}
            }
            self.rebuild_items();
        }
    }

    /// Toggle the collapsed state of the current session/window node.
    fn toggle_collapsed(&mut self) {
        if let Some(target) = self.current_target() {
            match target {
                Target::Session(name) => {
                    if !self.collapsed_sessions.insert(name.clone()) {
                        self.collapsed_sessions.remove(&name);
                    }
                }
                Target::Window { session, index } => {
                    let key = (session, index);
                    if !self.collapsed_windows.insert(key.clone()) {
                        self.collapsed_windows.remove(&key);
                    }
                }
                Target::Pane { .. } => {}
            }
            self.rebuild_items();
        }
    }

    // ── Preview ────────────────────────────────────────────────────────────────

    /// Resolve the pane whose contents should be previewed for a target: the
    /// pane itself, or the active (else first) pane of a window/session.
    fn find_active_pane_for_target(&self, target: &Target) -> Option<String> {
        match target {
            Target::Pane { id, .. } => Some(id.clone()),
            Target::Window { session, index } => {
                let w = self.find_window(session, *index)?;
                let p = w.panes.iter().find(|p| p.active).or_else(|| w.panes.first())?;
                Some(p.id.clone())
            }
            Target::Session(name) => {
                let s = self.find_session(name)?;
                let w = s.windows.iter().find(|w| w.active).or_else(|| s.windows.first())?;
                let p = w.panes.iter().find(|p| p.active).or_else(|| w.panes.first())?;
                Some(p.id.clone())
            }
        }
    }

    pub(crate) fn update_preview(&mut self) {
        if !self.show_preview {
            return;
        }
        let target = match self.current_target() {
            Some(t) => t,
            None => {
                self.preview_content = None;
                self.last_previewed_pane_id = None;
                return;
            }
        };

        if let Some(id) = self.find_active_pane_for_target(&target) {
            if Some(id.clone()) != self.last_previewed_pane_id || self.preview_content.is_none() {
                let content = match run_tmux(&["capture-pane", "-pt", &id]) {
                    Ok(c) => c,
                    Err(e) => format!("Error capturing pane preview: {}", e),
                };
                self.preview_content = Some(content);
                self.last_previewed_pane_id = Some(id);
            }
        } else {
            self.preview_content = None;
            self.last_previewed_pane_id = None;
        }
    }

    // ── Key handling ───────────────────────────────────────────────────────────

    /// Dispatch a keypress to the handler for the current mode.
    ///
    /// The current mode is moved out (replaced with [`Mode::Normal`]); each
    /// handler is responsible for restoring its mode if the app should stay in
    /// it, so the common "return to Normal" cases need no extra work.
    pub fn handle_key(&mut self, key: KeyCode) -> Flow {
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.on_normal(key),
            Mode::MovePaneSelect { src_pane_id, src_label } => {
                self.on_move_pane(key, src_pane_id, src_label)
            }
            Mode::MoveWindowSelect { src_session, src_window_index, src_label } => {
                self.on_move_window(key, src_session, src_window_index, src_label)
            }
            Mode::SwapSelect { src_pane_id, src_label } => self.on_swap(key, src_pane_id, src_label),
            Mode::RenameInput { target, input } => self.on_rename(key, target, input),
            Mode::KillConfirm { target, label } => self.on_kill_confirm(key, target, label),
            Mode::NewSession { input } => self.on_new_session(key, input),
            Mode::NewWindow { session, input } => self.on_new_window(key, session, input),
            Mode::EqualizeConfirm { session, window_index, window_name, num_panes } => {
                self.on_equalize(key, session, window_index, window_name, num_panes)
            }
            Mode::Error(msg) => self.on_error(key, msg),
        }
    }

    fn on_normal(&mut self, key: KeyCode) -> Flow {
        match key {
            KeyCode::Char('q') | KeyCode::Esc => return Flow::Exit,
            KeyCode::Up | KeyCode::Char('k') => self.previous(),
            KeyCode::Down | KeyCode::Char('j') => self.next(),
            KeyCode::Left | KeyCode::Char('h') => self.set_collapsed(true),
            KeyCode::Right | KeyCode::Char('l') => self.set_collapsed(false),
            KeyCode::Char(' ') => self.toggle_collapsed(),
            KeyCode::Enter => {
                if let Some(target) = self.current_target() {
                    match switch_to_target(&target) {
                        Ok(_) => return Flow::Exit,
                        Err(e) => self.mode = Mode::Error(e),
                    }
                }
            }
            KeyCode::Char('m') => {
                if let Some(target) = self.current_target() {
                    match target {
                        Target::Pane { id, .. } => {
                            let label = self.current_label().unwrap_or("Pane").to_string();
                            self.mode = Mode::MovePaneSelect { src_pane_id: id, src_label: label };
                        }
                        Target::Window { session, index } => {
                            let name = self.find_window(&session, index)
                                .map(|w| w.name.clone())
                                .unwrap_or_default();
                            let label = format!("Window {}: {}", index, name);
                            self.mode = Mode::MoveWindowSelect {
                                src_session: session,
                                src_window_index: index,
                                src_label: label,
                            };
                        }
                        Target::Session(_) => {}
                    }
                }
            }
            KeyCode::Char('s') => {
                if let Some(Target::Pane { id, .. }) = self.current_target() {
                    let label = self.current_label().unwrap_or("Pane").to_string();
                    self.mode = Mode::SwapSelect { src_pane_id: id, src_label: label };
                }
            }
            KeyCode::Char('r') => {
                if let Some(target) = self.current_target() {
                    let input = self.rename_seed(&target);
                    self.mode = Mode::RenameInput { target, input };
                }
            }
            KeyCode::Char('x') | KeyCode::Char('d') => {
                if let Some(target) = self.current_target() {
                    let label = self.current_label().unwrap_or("Item").to_string();
                    self.mode = Mode::KillConfirm { target, label };
                }
            }
            KeyCode::Char('n') => {
                let session = self
                    .current_target()
                    .map(|t| t.session().to_string())
                    .unwrap_or_else(|| {
                        self.sessions.first().map(|s| s.name.clone()).unwrap_or_default()
                    });
                self.mode = Mode::NewWindow { session, input: String::new() };
            }
            KeyCode::Char('N') => self.mode = Mode::NewSession { input: String::new() },
            KeyCode::Char('e') => {
                if let Some((session, window_index, window_name, num_panes)) =
                    self.equalize_target_info()
                {
                    if num_panes <= 1 {
                        self.mode = Mode::Error(
                            "Window has only 1 pane — nothing to equalize.".to_string(),
                        );
                    } else {
                        self.mode = Mode::EqualizeConfirm {
                            session,
                            window_index,
                            window_name,
                            num_panes,
                        };
                    }
                }
            }
            KeyCode::Char('p') => {
                self.show_preview = !self.show_preview;
                if self.show_preview {
                    self.last_previewed_pane_id = None;
                    self.update_preview();
                }
            }
            _ => {}
        }
        Flow::Continue
    }

    fn on_move_pane(&mut self, key: KeyCode, src_pane_id: String, src_label: String) -> Flow {
        match key {
            KeyCode::Esc => return Flow::Continue,
            KeyCode::Up | KeyCode::Char('k') => self.previous(),
            KeyCode::Down | KeyCode::Char('j') => self.next(),
            KeyCode::Left => self.set_collapsed(true),
            KeyCode::Right => self.set_collapsed(false),
            KeyCode::Char(' ') => self.toggle_collapsed(),
            KeyCode::Char('v') => {
                if let Some(target) = self.current_target() {
                    let t = target.tmux_id();
                    match run_tmux(&["join-pane", "-h", "-s", &src_pane_id, "-t", &t]) {
                        Ok(_) => return Flow::Exit,
                        Err(e) => {
                            self.mode = Mode::Error(e);
                            return Flow::Continue;
                        }
                    }
                }
            }
            KeyCode::Char('h') => {
                if let Some(target) = self.current_target() {
                    let t = target.tmux_id();
                    match run_tmux(&["join-pane", "-v", "-s", &src_pane_id, "-t", &t]) {
                        Ok(_) => return Flow::Exit,
                        Err(e) => {
                            self.mode = Mode::Error(e);
                            return Flow::Continue;
                        }
                    }
                }
            }
            KeyCode::Char('w') => {
                if let Some(target) = self.current_target() {
                    let target_session = target.session().to_string();
                    match run_tmux(&["break-pane", "-d", "-s", &src_pane_id, "-P", "-F", "#{window_id}"]) {
                        Ok(new_window_id) => {
                            let wid = new_window_id.trim().to_string();
                            match run_tmux(&["move-window", "-d", "-s", &wid, "-t", &format!("{}:", target_session)]) {
                                Ok(_) => return Flow::Exit,
                                Err(e) => {
                                    self.mode = Mode::Error(e);
                                    return Flow::Continue;
                                }
                            }
                        }
                        Err(e) => {
                            self.mode = Mode::Error(e);
                            return Flow::Continue;
                        }
                    }
                }
            }
            _ => {}
        }
        self.mode = Mode::MovePaneSelect { src_pane_id, src_label };
        Flow::Continue
    }

    fn on_move_window(
        &mut self,
        key: KeyCode,
        src_session: String,
        src_window_index: usize,
        src_label: String,
    ) -> Flow {
        match key {
            KeyCode::Esc => return Flow::Continue,
            KeyCode::Up | KeyCode::Char('k') => self.previous(),
            KeyCode::Down | KeyCode::Char('j') => self.next(),
            KeyCode::Left => self.set_collapsed(true),
            KeyCode::Right => self.set_collapsed(false),
            KeyCode::Char(' ') => self.toggle_collapsed(),
            KeyCode::Enter => {
                if let Some(target) = self.current_target() {
                    let src = format!("{}:{}", src_session, src_window_index);
                    let dst = format!("{}:", target.session());
                    match run_tmux(&["move-window", "-d", "-s", &src, "-t", &dst]) {
                        Ok(_) => return Flow::Exit,
                        Err(e) => {
                            self.mode = Mode::Error(e);
                            return Flow::Continue;
                        }
                    }
                }
            }
            KeyCode::Char('m') => {
                if let Some(target) = self.current_target() {
                    let (dst_session, dst_index) = match &target {
                        Target::Window { session, index } => (session.clone(), *index),
                        Target::Pane { session, window_index, .. } => (session.clone(), *window_index),
                        Target::Session(_) => {
                            self.mode = Mode::Error(
                                "Must select a target Window or Pane to merge.".to_string(),
                            );
                            return Flow::Continue;
                        }
                    };
                    if src_session == dst_session && src_window_index == dst_index {
                        self.mode = Mode::Error("Cannot merge a window into itself.".to_string());
                        return Flow::Continue;
                    }
                    // Snapshot the source pane ids; this ends the borrow of
                    // `self` before we start mutating `self.mode` below.
                    let pane_ids: Option<Vec<String>> = self
                        .find_window(&src_session, src_window_index)
                        .map(|w| w.panes.iter().map(|p| p.id.clone()).collect());
                    if let Some(pane_ids) = pane_ids {
                        let target_str = format!("{}:{}", dst_session, dst_index);
                        for pid in pane_ids {
                            if let Err(e) = run_tmux(&["join-pane", "-d", "-s", &pid, "-t", &target_str]) {
                                self.mode =
                                    Mode::Error(format!("Failed to merge pane {}: {}", pid, e));
                                return Flow::Continue;
                            }
                        }
                        return Flow::Exit;
                    }
                }
            }
            _ => {}
        }
        self.mode = Mode::MoveWindowSelect { src_session, src_window_index, src_label };
        Flow::Continue
    }

    fn on_swap(&mut self, key: KeyCode, src_pane_id: String, src_label: String) -> Flow {
        match key {
            KeyCode::Esc => return Flow::Continue,
            KeyCode::Up | KeyCode::Char('k') => self.previous(),
            KeyCode::Down | KeyCode::Char('j') => self.next(),
            KeyCode::Left => self.set_collapsed(true),
            KeyCode::Right => self.set_collapsed(false),
            KeyCode::Char(' ') => self.toggle_collapsed(),
            KeyCode::Enter => {
                if let Some(Target::Pane { id: dst, .. }) = self.current_target() {
                    match run_tmux(&["swap-pane", "-s", &src_pane_id, "-t", &dst]) {
                        Ok(_) => return Flow::Exit,
                        Err(e) => {
                            self.mode = Mode::Error(e);
                            return Flow::Continue;
                        }
                    }
                }
            }
            _ => {}
        }
        self.mode = Mode::SwapSelect { src_pane_id, src_label };
        Flow::Continue
    }

    fn on_rename(&mut self, key: KeyCode, target: Target, mut input: String) -> Flow {
        match key {
            KeyCode::Esc => return Flow::Continue,
            KeyCode::Enter => {
                let trimmed = input.trim().to_string();
                if trimmed.is_empty() {
                    self.mode = Mode::RenameInput { target, input };
                    return Flow::Continue;
                }
                match &target {
                    Target::Session(name) => {
                        self.run_then_reload(&["rename-session", "-t", name, &trimmed])
                    }
                    Target::Window { session, index } => {
                        let t = format!("{}:{}", session, index);
                        self.run_then_reload(&["rename-window", "-t", &t, &trimmed]);
                    }
                    Target::Pane { id, .. } => {
                        self.run_then_reload(&["select-pane", "-t", id, "-T", &trimmed])
                    }
                }
                return Flow::Continue;
            }
            KeyCode::Char(c) => input.push(c),
            KeyCode::Backspace => {
                input.pop();
            }
            _ => {}
        }
        self.mode = Mode::RenameInput { target, input };
        Flow::Continue
    }

    fn on_kill_confirm(&mut self, key: KeyCode, target: Target, label: String) -> Flow {
        match key {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => match &target {
                Target::Session(name) => self.run_then_reload(&["kill-session", "-t", name]),
                Target::Window { session, index } => {
                    let t = format!("{}:{}", session, index);
                    self.run_then_reload(&["kill-window", "-t", &t]);
                }
                Target::Pane { id, .. } => self.run_then_reload(&["kill-pane", "-t", id]),
            },
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {}
            _ => self.mode = Mode::KillConfirm { target, label },
        }
        Flow::Continue
    }

    fn on_new_session(&mut self, key: KeyCode, mut input: String) -> Flow {
        match key {
            KeyCode::Esc => return Flow::Continue,
            KeyCode::Enter => {
                let trimmed = input.trim().to_string();
                if trimmed.is_empty() {
                    self.mode = Mode::NewSession { input };
                    return Flow::Continue;
                }
                self.run_then_reload(&["new-session", "-d", "-s", &trimmed]);
                return Flow::Continue;
            }
            KeyCode::Char(c) => input.push(c),
            KeyCode::Backspace => {
                input.pop();
            }
            _ => {}
        }
        self.mode = Mode::NewSession { input };
        Flow::Continue
    }

    fn on_new_window(&mut self, key: KeyCode, session: String, mut input: String) -> Flow {
        match key {
            KeyCode::Esc => return Flow::Continue,
            KeyCode::Enter => {
                let trimmed = input.trim().to_string();
                let target = format!("{}:", session);
                if trimmed.is_empty() {
                    self.run_then_reload(&["new-window", "-d", "-t", &target]);
                } else {
                    self.run_then_reload(&["new-window", "-d", "-t", &target, "-n", &trimmed]);
                }
                return Flow::Continue;
            }
            KeyCode::Char(c) => input.push(c),
            KeyCode::Backspace => {
                input.pop();
            }
            _ => {}
        }
        self.mode = Mode::NewWindow { session, input };
        Flow::Continue
    }

    fn on_equalize(
        &mut self,
        key: KeyCode,
        session: String,
        window_index: usize,
        window_name: String,
        num_panes: usize,
    ) -> Flow {
        let layout = match key {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => return Flow::Continue,
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => best_layout_for(num_panes),
            KeyCode::Char('t') => "tiled",
            KeyCode::Char('h') => "even-horizontal",
            KeyCode::Char('v') => "even-vertical",
            KeyCode::Char('m') => "main-horizontal",
            _ => {
                self.mode = Mode::EqualizeConfirm { session, window_index, window_name, num_panes };
                return Flow::Continue;
            }
        };
        let target = format!("{}:{}", session, window_index);
        self.run_then_reload(&["select-layout", "-t", &target, layout]);
        Flow::Continue
    }

    fn on_error(&mut self, key: KeyCode, msg: String) -> Flow {
        match key {
            KeyCode::Esc | KeyCode::Enter => {
                let _ = self.reload();
            }
            _ => self.mode = Mode::Error(msg),
        }
        Flow::Continue
    }
}

/// Flatten the session tree into a displayable, navigable list, honouring the
/// collapsed-session and collapsed-window sets.
pub fn rebuild_flat_items(
    sessions: &[Session],
    collapsed_sessions: &HashSet<String>,
    collapsed_windows: &HashSet<(String, usize)>,
) -> Vec<NavItem> {
    let mut flat_items = Vec::new();
    for s in sessions {
        let is_collapsed = collapsed_sessions.contains(&s.name);
        let icon = if is_collapsed { "▶" } else { "▼" };
        flat_items.push(NavItem {
            label: format!("{} Session: {}", icon, s.name),
            target: Target::Session(s.name.clone()),
        });

        if is_collapsed {
            continue;
        }

        for w in &s.windows {
            let w_key = (s.name.clone(), w.index);
            let is_w_collapsed = collapsed_windows.contains(&w_key);
            let w_icon = if is_w_collapsed { "▶" } else { "▼" };
            flat_items.push(NavItem {
                label: format!("  {} Window {}: {}", w_icon, w.index, w.name),
                target: Target::Window { session: s.name.clone(), index: w.index },
            });

            if is_w_collapsed {
                continue;
            }

            let num_panes = w.panes.len();
            for (p_idx, p) in w.panes.iter().enumerate() {
                let prefix = if p_idx == num_panes - 1 { "    └─ " } else { "    ├─ " };
                let active_indicator = if p.active { " ●" } else { "" };
                // Show the title if it differs from the command, else just the command.
                let label_content = if !p.title.is_empty() && p.title != p.current_command {
                    format!("{} [{}]", p.current_command, p.title)
                } else {
                    p.current_command.clone()
                };
                flat_items.push(NavItem {
                    label: format!(
                        "{}{} ({}): {}{}",
                        prefix, p.index, p.id, label_content, active_indicator
                    ),
                    target: Target::Pane {
                        session: s.name.clone(),
                        window_index: w.index,
                        id: p.id.clone(),
                    },
                });
            }
        }
    }
    flat_items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pane, Window};

    fn pane(id: &str, index: usize) -> Pane {
        Pane {
            id: id.into(),
            index,
            active: false,
            current_command: "bash".into(),
            current_path: "/".into(),
            title: String::new(),
        }
    }

    fn sample() -> Vec<Session> {
        vec![Session {
            name: "main".into(),
            windows: vec![
                Window { name: "w0".into(), index: 0, active: true, panes: vec![pane("%1", 0), pane("%2", 1)] },
                Window { name: "w1".into(), index: 1, active: false, panes: vec![pane("%3", 0)] },
            ],
        }]
    }

    #[test]
    fn expanded_lists_session_windows_and_panes() {
        let items = rebuild_flat_items(&sample(), &HashSet::new(), &HashSet::new());
        // 1 session + 2 windows + (2 + 1) panes = 6.
        assert_eq!(items.len(), 6);
        assert!(matches!(items[0].target, Target::Session(_)));
        assert!(matches!(items[1].target, Target::Window { index: 0, .. }));
        assert!(matches!(items[2].target, Target::Pane { .. }));
    }

    #[test]
    fn collapsed_session_hides_all_descendants() {
        let mut collapsed = HashSet::new();
        collapsed.insert("main".to_string());
        let items = rebuild_flat_items(&sample(), &collapsed, &HashSet::new());
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0].target, Target::Session(_)));
    }

    #[test]
    fn collapsed_window_hides_only_its_panes() {
        let mut collapsed_windows = HashSet::new();
        collapsed_windows.insert(("main".to_string(), 0)); // hide w0's 2 panes
        let items = rebuild_flat_items(&sample(), &HashSet::new(), &collapsed_windows);
        // session + w0 + w1 + w1's single pane = 4.
        assert_eq!(items.len(), 4);
    }
}
