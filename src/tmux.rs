//! All interaction with the `tmux` binary: running commands, fetching the
//! session tree, parsing its output, and the small layout heuristic.

use crate::model::{Pane, Session, Target, Window};

/// Format string passed to `tmux list-panes -F`. Fields are tab-separated and
/// parsed positionally by [`parse_tmux_state`]; keep the two in sync.
const LIST_FORMAT: &str = "#{session_name}\t#{window_index}\t#{window_name}\t#{pane_index}\t#{pane_id}\t#{pane_active}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_title}\t#{window_active}";

/// Run `tmux` with the given arguments, returning stdout on success or the
/// trimmed stderr on failure.
pub fn run_tmux(args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("tmux")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Fetch the full session/window/pane tree from tmux.
pub fn fetch_tmux_state() -> Result<Vec<Session>, String> {
    let output = run_tmux(&["list-panes", "-a", "-F", LIST_FORMAT])?;
    Ok(parse_tmux_state(&output))
}

/// Parse the tab-separated `list-panes -F` output into a session tree.
///
/// Pure (no I/O), so it is unit-tested directly. Lines with fewer than the
/// expected number of fields are skipped; windows and panes are sorted by
/// index.
pub fn parse_tmux_state(output: &str) -> Vec<Session> {
    let mut sessions: Vec<Session> = Vec::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 10 {
            continue;
        }
        let session_name = parts[0].to_string();
        let window_index = parts[1].parse::<usize>().unwrap_or(0);
        let window_name = parts[2].to_string();
        let pane_index = parts[3].parse::<usize>().unwrap_or(0);
        let pane_id = parts[4].to_string();
        let pane_active = parts[5] == "1";
        let pane_command = parts[6].to_string();
        let pane_path = parts[7].to_string();
        let pane_title = parts[8].to_string();
        let window_active = parts[9] == "1";

        let pane = Pane {
            id: pane_id,
            index: pane_index,
            active: pane_active,
            current_command: pane_command,
            current_path: pane_path,
            title: pane_title,
        };

        let s_idx = match sessions.iter().position(|s| s.name == session_name) {
            Some(idx) => idx,
            None => {
                sessions.push(Session { name: session_name.clone(), windows: Vec::new() });
                sessions.len() - 1
            }
        };

        let w_idx = match sessions[s_idx].windows.iter().position(|w| w.index == window_index) {
            Some(idx) => idx,
            None => {
                sessions[s_idx].windows.push(Window {
                    name: window_name.clone(),
                    index: window_index,
                    panes: Vec::new(),
                    active: window_active,
                });
                sessions[s_idx].windows.len() - 1
            }
        };

        sessions[s_idx].windows[w_idx].active = window_active;
        sessions[s_idx].windows[w_idx].panes.push(pane);
    }

    for s in &mut sessions {
        s.windows.sort_by_key(|w| w.index);
        for w in &mut s.windows {
            w.panes.sort_by_key(|p| p.index);
        }
    }

    sessions
}

/// Focus the given target in tmux (select pane/window as needed, then switch
/// the attached client to its session).
pub fn switch_to_target(target: &Target) -> Result<String, String> {
    match target {
        Target::Session(name) => run_tmux(&["switch-client", "-t", name]),
        Target::Window { session, index } => {
            run_tmux(&["select-window", "-t", &format!("{}:{}", session, index)])
                .and_then(|_| run_tmux(&["switch-client", "-t", session]))
        }
        Target::Pane { session, window_index, id } => {
            run_tmux(&["select-pane", "-t", id])
                .and_then(|_| run_tmux(&["select-window", "-t", &format!("{}:{}", session, window_index)]))
                .and_then(|_| run_tmux(&["switch-client", "-t", session]))
        }
    }
}

/// Choose the layout name that distributes `num_panes` panes most evenly.
pub fn best_layout_for(num_panes: usize) -> &'static str {
    match num_panes {
        0 | 1 => "even-horizontal",
        2 => "even-vertical",
        _ => "tiled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_layout_picks_by_pane_count() {
        assert_eq!(best_layout_for(0), "even-horizontal");
        assert_eq!(best_layout_for(1), "even-horizontal");
        assert_eq!(best_layout_for(2), "even-vertical");
        assert_eq!(best_layout_for(3), "tiled");
        assert_eq!(best_layout_for(42), "tiled");
    }

    #[test]
    fn parse_groups_panes_into_windows_and_sessions() {
        let out = "main\t0\teditor\t0\t%1\t1\tnvim\t/home\tnvim\t1\n\
                   main\t0\teditor\t1\t%2\t0\tbash\t/home\tbash\t1\n\
                   main\t1\tshell\t0\t%3\t0\tzsh\t/tmp\tzsh\t0\n\
                   work\t0\tlogs\t0\t%4\t1\ttail\t/var\ttail\t1\n";
        let sessions = parse_tmux_state(out);

        assert_eq!(sessions.len(), 2);

        let main = &sessions[0];
        assert_eq!(main.name, "main");
        assert_eq!(main.windows.len(), 2);
        assert_eq!(main.windows[0].name, "editor");
        assert_eq!(main.windows[0].panes.len(), 2);
        assert!(main.windows[0].active);
        assert_eq!(main.windows[0].panes[0].id, "%1");
        assert!(main.windows[0].panes[0].active);
        assert_eq!(main.windows[0].panes[0].current_path, "/home");
        assert_eq!(main.windows[1].name, "shell");
        assert!(!main.windows[1].active);

        assert_eq!(sessions[1].name, "work");
        assert_eq!(sessions[1].windows[0].panes.len(), 1);
    }

    #[test]
    fn parse_sorts_windows_and_panes_by_index() {
        // Arrive out of order; expect them sorted by index.
        let out = "s\t2\tw2\t1\t%b\t0\ta\t/\ta\t1\n\
                   s\t2\tw2\t0\t%a\t0\tb\t/\tb\t1\n\
                   s\t0\tw0\t0\t%c\t0\tc\t/\tc\t0\n";
        let s = &parse_tmux_state(out)[0];
        assert_eq!(s.windows[0].index, 0);
        assert_eq!(s.windows[1].index, 2);
        assert_eq!(s.windows[1].panes[0].index, 0);
        assert_eq!(s.windows[1].panes[0].id, "%a");
        assert_eq!(s.windows[1].panes[1].index, 1);
    }

    #[test]
    fn parse_skips_malformed_lines() {
        let out = "too\tfew\tfields\n\
                   s\t0\tw\t0\t%1\t1\tbash\t/\tbash\t1\n";
        let sessions = parse_tmux_state(out);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].windows[0].panes.len(), 1);
    }

    #[test]
    fn parse_empty_output_is_empty() {
        assert!(parse_tmux_state("").is_empty());
    }
}
