//! Rendering: the main three-pane layout, the modal overlays, and the keymap
//! tables that both the help panel and the status bar are rendered from.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Mode};
use crate::model::Target;
use crate::tmux::best_layout_for;

// ── Key bindings ───────────────────────────────────────────────────────────
// Single source of truth for the help panel and the status bar, so the two
// views can't drift apart when a binding is added or changed.

struct Binding {
    /// Keys as shown in the help panel's left column, e.g. "j/k ↑↓".
    help_keys: &'static str,
    /// Description shown in the help panel.
    help_desc: &'static str,
    /// Compact ("key", "label") pair for the status bar; `None` to omit it there.
    status: Option<(&'static str, &'static str)>,
}

/// Navigation keys — top group of the help panel.
const NAV_BINDINGS: &[Binding] = &[
    Binding { help_keys: "j/k ↑↓", help_desc: "Navigate",         status: None },
    Binding { help_keys: "←/→",    help_desc: "Collapse/Expand",  status: None },
    Binding { help_keys: "Space",  help_desc: "Toggle collapse",  status: None },
    Binding { help_keys: "Enter",  help_desc: "Focus & exit TUI", status: Some(("Enter", "focus")) },
];

/// Action keys — bottom group of the help panel.
const ACTION_BINDINGS: &[Binding] = &[
    Binding { help_keys: "m",     help_desc: "Move pane",                  status: Some(("m", "move")) },
    Binding { help_keys: "s",     help_desc: "Swap pane",                  status: Some(("s", "swap")) },
    Binding { help_keys: "e",     help_desc: "Equalize pane layout",       status: Some(("e", "equalize")) },
    Binding { help_keys: "r",     help_desc: "Rename session/window/pane", status: Some(("r", "rename")) },
    Binding { help_keys: "n",     help_desc: "New window in session",      status: Some(("n", "new window")) },
    Binding { help_keys: "N",     help_desc: "New session",                status: Some(("N", "new session")) },
    Binding { help_keys: "p",     help_desc: "Toggle preview/help",        status: Some(("p", "preview")) },
    Binding { help_keys: "x/d",   help_desc: "Kill item",                  status: Some(("x", "kill")) },
    Binding { help_keys: "q/Esc", help_desc: "Quit",                       status: None },
];

/// Render the help panel body (two groups separated by a divider).
fn help_text() -> String {
    let row = |b: &Binding| format!("{:<8}: {}", b.help_keys, b.help_desc);
    let nav: Vec<String> = NAV_BINDINGS.iter().map(row).collect();
    let actions: Vec<String> = ACTION_BINDINGS.iter().map(row).collect();
    format!("{}\n─────────────────────────\n{}", nav.join("\n"), actions.join("\n"))
}

/// Render the compact normal-mode hint string for the status bar.
fn status_hints() -> String {
    ACTION_BINDINGS
        .iter()
        .chain(NAV_BINDINGS.iter())
        .filter_map(|b| b.status)
        .map(|(key, label)| format!("{}={}", key, label))
        .collect::<Vec<_>>()
        .join("  ")
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub fn draw_ui(f: &mut Frame, app: &mut App) {
    let active_border = Style::default().fg(Color::Yellow);
    let dim_border = Style::default().fg(Color::DarkGray);

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(2)])
        .split(f.size());

    // Header
    let header = Line::from(vec![
        Span::styled(" tmux-organizer ", Style::default().bg(Color::Red).fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw("  Session · Window · Pane manager"),
    ]);
    f.render_widget(Paragraph::new(header), main_layout[0]);

    // Body split
    let body_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(main_layout[1]);

    // Build list items
    let is_move_pane = matches!(app.mode, Mode::MovePaneSelect { .. });
    let is_move_window = matches!(app.mode, Mode::MoveWindowSelect { .. });
    let is_swap = matches!(app.mode, Mode::SwapSelect { .. });
    let is_select = is_move_pane || is_move_window || is_swap;

    let list_items: Vec<ListItem> = app
        .flat_items
        .iter()
        .map(|item| {
            let style = match &item.target {
                Target::Session(_) => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                Target::Window { .. } => Style::default().fg(Color::LightYellow),
                Target::Pane { session, window_index, id } => {
                    let active = app
                        .find_pane(session, *window_index, id)
                        .map(|p| p.active)
                        .unwrap_or(false);
                    if active {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    }
                }
            };
            ListItem::new(item.label.as_str()).style(style)
        })
        .collect();

    let tree_title = if is_move_pane {
        " Choose Target to Move Pane "
    } else if is_move_window {
        " Choose Target to Move Window "
    } else if is_swap {
        " Choose Target Pane to Swap "
    } else {
        " Sessions, Windows & Panes "
    };

    let tree_title_style = if is_select {
        Style::default().fg(Color::LightMagenta).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let tree = List::new(list_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(tree_title)
                .title_style(tree_title_style)
                .border_style(if is_select { active_border } else { dim_border }),
        )
        .highlight_style(Style::default().bg(Color::Red).fg(Color::White).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");

    f.render_stateful_widget(tree, body_layout[0], &mut app.list_state);

    // Right panel
    let right_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(5)])
        .split(body_layout[1]);

    // Details
    let details = match app.current_target() {
        Some(Target::Session(name)) => {
            let (nw, np) = app
                .find_session(&name)
                .map(|s| (s.windows.len(), s.windows.iter().map(|w| w.panes.len()).sum::<usize>()))
                .unwrap_or((0, 0));
            format!("Type: Session\n\nName: {}\nWindows: {}\nTotal Panes: {}", name, nw, np)
        }
        Some(Target::Window { session, index }) => {
            let (name, np) = app
                .find_window(&session, index)
                .map(|w| (w.name.clone(), w.panes.len()))
                .unwrap_or_else(|| (String::new(), 0));
            format!(
                "Type: Window\n\nSession: {}\nIndex: {}\nName: {}\nPanes: {}\n\nPress [e] to equalize pane layout",
                session, index, name, np
            )
        }
        Some(Target::Pane { session, window_index, id }) => match app.find_pane(&session, window_index, &id) {
            Some(p) => {
                let title_line = if !p.title.is_empty() && p.title != p.current_command {
                    format!("Title: {}\n", p.title)
                } else {
                    String::new()
                };
                format!(
                    "Type: Pane\n\nPane ID: {}\nIndex: {}\nActive: {}\nCommand: {}\n{}Path: {}\n\nWindow: {}\nSession: {}",
                    id,
                    p.index,
                    if p.active { "Yes ●" } else { "No" },
                    p.current_command,
                    title_line,
                    p.current_path,
                    window_index,
                    session
                )
            }
            None => format!("Type: Pane\n\nPane ID: {}\n(no longer available)", id),
        },
        None => "No item selected.\n\nPress [n] to create a new window\nor [N] for a new session.".to_string(),
    };

    f.render_widget(
        Paragraph::new(details)
            .block(Block::default().borders(Borders::ALL).title(" Details ").border_style(dim_border))
            .wrap(Wrap { trim: true }),
        right_layout[0],
    );

    if app.show_preview {
        app.update_preview();
        let preview_text = app.preview_content.as_deref().unwrap_or("No preview available.");
        let pane_label = match &app.last_previewed_pane_id {
            Some(pane_id) => format!("Previewing Pane {}", pane_id),
            None => "Preview".to_string(),
        };

        f.render_widget(
            Paragraph::new(preview_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} (Press 'p' for Help) ", pane_label))
                        .border_style(dim_border),
                )
                .style(Style::default().fg(Color::Gray)),
            right_layout[1],
        );
    } else {
        f.render_widget(
            Paragraph::new(help_text())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Help (Press 'p' for Preview) ")
                        .border_style(dim_border),
                )
                .wrap(Wrap { trim: true }),
            right_layout[1],
        );
    }

    f.render_widget(Paragraph::new(status_line(&app.mode)), main_layout[2]);

    draw_overlays(f, &app.mode, f.size());
}

/// Build the bottom status bar line for the current mode.
fn status_line(mode: &Mode) -> Line<'_> {
    match mode {
        Mode::Normal => Line::from(vec![
            Span::styled("NORMAL", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(format!("  {}", status_hints())),
        ]),
        Mode::MovePaneSelect { src_label, .. } => Line::from(vec![
            Span::styled("MOVE PANE ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::styled(src_label, Style::default().fg(Color::Yellow)),
            Span::raw("  →  navigate to target then:  v=split-v  h=split-h  w=new-window  Esc=cancel"),
        ]),
        Mode::MoveWindowSelect { src_label, .. } => Line::from(vec![
            Span::styled("MOVE WINDOW ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::styled(src_label, Style::default().fg(Color::Yellow)),
            Span::raw("  →  navigate to target then:  Enter=move (new window)  m=merge (join all panes)  Esc=cancel"),
        ]),
        Mode::SwapSelect { src_label, .. } => Line::from(vec![
            Span::styled("SWAP ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::styled(src_label, Style::default().fg(Color::Yellow)),
            Span::raw("  →  navigate to target pane then Enter=swap  Esc=cancel"),
        ]),
        Mode::RenameInput { target, .. } => Line::from(vec![
            Span::styled("RENAME ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(format!("{}  →  type new name then Enter=save  Esc=cancel", target.kind())),
        ]),
        Mode::KillConfirm { label, .. } => Line::from(vec![
            Span::styled("KILL? ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(label, Style::default().fg(Color::Yellow)),
            Span::raw("  →  y=yes  n=cancel"),
        ]),
        Mode::NewSession { .. } => Line::from(vec![
            Span::styled("NEW SESSION", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("  →  type session name then Enter=create  Esc=cancel"),
        ]),
        Mode::NewWindow { session, .. } => Line::from(vec![
            Span::styled("NEW WINDOW", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(format!(" in [{}]  →  type window name (optional) then Enter=create  Esc=cancel", session)),
        ]),
        Mode::EqualizeConfirm { window_name, num_panes, .. } => Line::from(vec![
            Span::styled("EQUALIZE ", Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD)),
            Span::styled(window_name, Style::default().fg(Color::Yellow)),
            Span::raw(format!(" ({} panes)  →  Enter=auto  t=tiled  h=horizontal  v=vertical  m=main-h  Esc=cancel", num_panes)),
        ]),
        Mode::Error(err) => Line::from(vec![
            Span::styled("ERROR ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(err, Style::default().fg(Color::LightRed)),
            Span::raw("  →  Enter/Esc to dismiss"),
        ]),
    }
}

fn draw_overlays(f: &mut Frame, mode: &Mode, size: Rect) {
    let yellow_border = Style::default().fg(Color::Yellow);
    let red_border = Style::default().fg(Color::Red);
    let cyan_border = Style::default().fg(Color::Cyan);
    let blue_border = Style::default().fg(Color::LightBlue);

    match mode {
        Mode::RenameInput { target, input } => {
            let kind = target.kind();
            let area = centered_rect(60, 28, size);
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(format!("Enter new name for {}:\n\n▶ {}_\n\n[Enter] save  [Esc] cancel", kind, input))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" ✏ Rename {} ", kind))
                            .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                            .border_style(yellow_border),
                    )
                    .wrap(Wrap { trim: true }),
                area,
            );
        }
        Mode::KillConfirm { label, .. } => {
            let area = centered_rect(62, 30, size);
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(format!("⚠  Kill and permanently destroy:\n\n  {}\n\nThis cannot be undone!\n\n[y / Enter] kill     [n / Esc] cancel", label))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" ✖ Confirm Kill ")
                            .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                            .border_style(red_border),
                    )
                    .wrap(Wrap { trim: true }),
                area,
            );
        }
        Mode::NewSession { input } => {
            let area = centered_rect(60, 28, size);
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(format!("Enter name for the new session:\n\n▶ {}_\n\n[Enter] create  [Esc] cancel", input))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" ✚ New Session ")
                            .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                            .border_style(cyan_border),
                    )
                    .wrap(Wrap { trim: true }),
                area,
            );
        }
        Mode::NewWindow { session, input } => {
            let area = centered_rect(60, 28, size);
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(format!("New window in session [{}]\n\nName (optional, leave blank for default):\n▶ {}_\n\n[Enter] create  [Esc] cancel", session, input))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" ✚ New Window ")
                            .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                            .border_style(cyan_border),
                    )
                    .wrap(Wrap { trim: true }),
                area,
            );
        }
        Mode::EqualizeConfirm { window_name, num_panes, .. } => {
            let layout_name = best_layout_for(*num_panes);
            let area = centered_rect(65, 40, size);
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(format!(
                    "Equalize layout for window [{}] ({} panes)\n\nRecommended layout: {}\n\nPick a layout:\n  Enter / y  →  auto (recommended: {})\n  t          →  tiled (grid)\n  h          →  even-horizontal (columns)\n  v          →  even-vertical (rows)\n  m          →  main-horizontal (1 big + rows)\n\n[Esc / n]   →  cancel",
                    window_name, num_panes, layout_name, layout_name
                ))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" ⟺ Equalize Panes ")
                        .title_style(Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD))
                        .border_style(blue_border),
                )
                .wrap(Wrap { trim: true }),
                area,
            );
        }
        Mode::Error(err) => {
            let area = centered_rect(60, 30, size);
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(format!("tmux returned an error:\n\n{}\n\n[Enter / Esc] dismiss", err))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" ✖ Error ")
                            .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                            .border_style(red_border),
                    )
                    .wrap(Wrap { trim: true }),
                area,
            );
        }
        _ => {}
    }
}
