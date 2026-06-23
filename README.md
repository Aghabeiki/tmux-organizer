# tmux-organizer

A terminal UI for browsing and reorganizing every tmux session, window, and
pane on your server from a single collapsible tree. Built with
[ratatui](https://ratatui.rs) and [crossterm](https://github.com/crossterm-rs/crossterm).

```
┌ Sessions, Windows & Panes ──────┐┌ Details ───────────────┐
│ ▼ Session: main                 ││ Type: Pane             │
│   ▼ Window 0: editor            ││ Pane ID: %1            │
│     ├─ 0 (%1): nvim ●           ││ Active: Yes ●          │
│     └─ 1 (%2): bash             ││ Command: nvim          │
│   ▶ Window 1: shell             │└────────────────────────┘
│ ▼ Session: work                 │┌ Preview ───────────────┐
│   ▼ Window 0: logs              ││ (live pane contents)   │
│     └─ 0 (%4): tail             ││                        │
└─────────────────────────────────┘└────────────────────────┘
 NORMAL  m=move s=swap e=equalize r=rename n=new window …
```

## What it does

- **Tree view** of all sessions → windows → panes, with collapse/expand.
- **Live preview** of the selected pane's contents (toggle with `p`).
- **Focus** any session/window/pane and jump straight to it (`Enter`).
- **Reorganize** without leaving the keyboard: move panes between
  windows/sessions, merge or relocate whole windows, swap two panes, and
  re-balance a window's layout.
- **Manage**: rename, create, and kill sessions/windows/panes, each with a
  confirmation or input prompt.

## Requirements

- [`tmux`](https://github.com/tmux/tmux) on your `PATH` (the app shells out to
  it for all reads and actions).
- A running tmux server with at least one session.
- Rust (2024 edition) to build.

## Install

```sh
# Build a release binary (lands at target/release/tmux-organizer)
cargo build --release

# …or build and run straight from a clone
cargo run --release

# …or install the binary onto your PATH (~/.cargo/bin)
cargo install --path .
```

`Enter` focuses a target via `tmux switch-client`, so the app is most useful
when launched from inside tmux. A convenient integration is a popup binding in
`~/.tmux.conf`:

```tmux
bind-key T display-popup -E "tmux-organizer"
```

Pressing `prefix` + `T` opens the manager in a popup; selecting a target with
`Enter` switches the client and closes the popup.

## Keys

### Navigation

| Key        | Action                          |
| ---------- | ------------------------------- |
| `j`/`k`, `↑`/`↓` | Move selection up/down    |
| `←`/`→`    | Collapse / expand the node      |
| `Space`    | Toggle collapse                 |
| `Enter`    | Focus the target and exit       |
| `p`        | Toggle the preview / help panel |
| `q`, `Esc` | Quit                            |

### Actions (normal mode)

| Key      | Action                                              |
| -------- | --------------------------------------------------- |
| `m`      | Move the selected **pane** or **window** (see below) |
| `s`      | Swap the selected pane with another pane            |
| `e`      | Equalize / re-balance a window's pane layout        |
| `r`      | Rename the selected session / window / pane         |
| `n`      | New window in the selected session                  |
| `N`      | New session                                         |
| `x`, `d` | Kill the selected item (with confirmation)          |

### Move / swap sub-modes

After pressing `m`/`s` you navigate to a target, then:

- **Move pane** (`m` on a pane): `v` split alongside the target, `h` split
  above/below the target, `w` break it out into a new window in the target's
  session.
- **Move window** (`m` on a window): `Enter` move it (as a new window) into the
  target's session, `m` merge it (join all its panes) into the target window.
- **Swap** (`s` on a pane): `Enter` swaps the two panes.
- `Esc` cancels any sub-mode.

### Equalize

Press `e` on a window (or a pane within it) to pick a layout: `Enter` for the
auto-recommended one, or `t` tiled, `h` even-horizontal, `v` even-vertical,
`m` main-horizontal.

## How it works

The app holds no tmux state of its own. On startup and after every mutating
command it runs `tmux list-panes -a` and rebuilds the tree; navigation and
collapse/expand are purely in-memory and never spawn a subprocess. Every action
maps to a single tmux command (`join-pane`, `swap-pane`, `move-window`,
`select-layout`, `kill-*`, `new-*`, `rename-*`), and any error tmux returns is
surfaced in a dismissable dialog.

## Development

```sh
cargo test     # unit tests for parsing and tree flattening
cargo clippy    # lints
cargo run       # debug build
```

Source layout:

| File        | Responsibility                                          |
| ----------- | ------------------------------------------------------- |
| `main.rs`   | Terminal setup (RAII) and the event loop                |
| `model.rs`  | `Session` / `Window` / `Pane` / `Target` data types     |
| `tmux.rs`   | Running tmux, parsing its output, the layout heuristic  |
| `app.rs`    | Application state, tree lookups, per-mode key handling  |
| `ui.rs`     | Rendering the panels, overlays, and keymap tables       |

## License

[GNU AGPL-3.0-or-later](LICENSE) © Amin Aghabeiki
