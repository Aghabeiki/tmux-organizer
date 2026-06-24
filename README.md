# tmux-organizer

A terminal UI for browsing and reorganizing every tmux session, window, and
pane from a collapsible tree — focus, move, swap, merge, equalize, rename,
create, and kill, with a live pane preview.

```
┌ Sessions, Windows & Panes ──────┐┌ Details ───────────────┐
│ ▼ Session: main                 ││ Type: Pane             │
│   ▼ Window 0: editor            ││ Pane ID: %1            │
│     ├─ 0 (%1): nvim ●           ││ Active: Yes ●          │
│   ▶ Window 1: shell             ││ Command: nvim          │
└─────────────────────────────────┘└────────────────────────┘
 NORMAL  m=move s=swap e=equalize r=rename n=new window …
```

## Install

```sh
cargo install tmux-organizer
```

Installs to `~/.cargo/bin/tmux-organizer` — make sure that's on your `$PATH`.
Requires `tmux` and Rust to build.

## tmux integration

Add a popup launcher to `~/.config/tmux/tmux.conf`:

```tmux
bind-key m display-popup -E -w 85% -h 85% "$HOME/.cargo/bin/tmux-organizer"
```

`prefix + m` opens it; `Enter` jumps to the selected target and closes the popup.

## Keys

| Key | Action |
| --- | --- |
| `j`/`k`, `↑`/`↓` | navigate |
| `←`/`→`, `Space` | collapse / expand |
| `Enter` | focus target & exit |
| `m` / `s` | move pane or window / swap pane |
| `e` | equalize layout |
| `r` / `n` / `N` | rename / new window / new session |
| `x`, `d` | kill |
| `p` | toggle preview / help |
| `q`, `Esc` | quit |

For `m` and `s`, navigate to a target then confirm — the status bar shows the keys.

## License

[AGPL-3.0-or-later](LICENSE) © Amin Aghabeiki
