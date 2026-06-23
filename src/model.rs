//! Domain types describing the tmux session tree and what the cursor points at.

#[derive(Clone, Debug)]
pub struct Pane {
    pub id: String,
    pub index: usize,
    pub active: bool,
    pub current_command: String,
    pub current_path: String,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct Window {
    pub name: String,
    pub index: usize,
    pub panes: Vec<Pane>,
    pub active: bool,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub name: String,
    pub windows: Vec<Window>,
}

/// What the cursor currently points at in the tree.
///
/// This holds only *addressing* information (enough to build a tmux target).
/// Display fields (command, title, pane count, …) are looked up from the live
/// [`Session`] tree on demand, so they can never go stale relative to it.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Session(String),
    Window { session: String, index: usize },
    Pane {
        session: String,
        window_index: usize,
        id: String,
    },
}

impl Target {
    /// The name of the session this target lives in.
    pub fn session(&self) -> &str {
        match self {
            Target::Session(name) => name,
            Target::Window { session, .. } => session,
            Target::Pane { session, .. } => session,
        }
    }

    /// The tmux target string for this item: `sess:`, `sess:idx`, or a pane id.
    pub fn tmux_id(&self) -> String {
        match self {
            Target::Session(name) => format!("{}:", name),
            Target::Window { session, index } => format!("{}:{}", session, index),
            Target::Pane { id, .. } => id.clone(),
        }
    }

    /// Human label for the kind of target, e.g. for status text.
    pub fn kind(&self) -> &'static str {
        match self {
            Target::Session(_) => "Session",
            Target::Window { .. } => "Window",
            Target::Pane { .. } => "Pane",
        }
    }
}

/// One row of the flattened, navigable tree: a precomputed display label plus
/// the target it points at.
#[derive(Clone, Debug)]
pub struct NavItem {
    pub label: String,
    pub target: Target,
}
