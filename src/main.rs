mod app;
mod model;
mod tmux;
mod ui;

use std::io::{self, stdout};

use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::{App, Flow};
use ui::draw_ui;

/// RAII guard that puts the terminal into raw / alternate-screen mode and
/// restores it on drop, even if the loop panics or returns early.
struct TerminalSetup;

impl TerminalSetup {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalSetup {
    fn drop(&mut self) {
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = match App::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error initializing: {}", e);
            return Ok(());
        }
    };

    let _setup = TerminalSetup::new()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    loop {
        terminal.draw(|f| draw_ui(f, &mut app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if let Flow::Exit = app.handle_key(key.code) {
                break;
            }
        }
    }

    Ok(())
}
