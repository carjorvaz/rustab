use clap::{ArgGroup, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "rustab",
    version,
    about = "Browser tab management from the terminal"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, ValueEnum)]
pub enum OutputFormat {
    Tsv,
    Json,
}

#[derive(Subcommand)]
pub enum Command {
    /// List open tabs across all browsers
    List {
        /// Output format
        #[arg(short, long, default_value = "tsv")]
        format: OutputFormat,
        /// Filter by browser (e.g. chrome, firefox, brave)
        #[arg(short, long)]
        browser: Option<String>,
    },
    /// List browser windows
    Windows {
        /// Output format
        #[arg(short, long, default_value = "tsv")]
        format: OutputFormat,
        /// Filter by browser (e.g. chrome, firefox, brave)
        #[arg(short, long)]
        browser: Option<String>,
    },
    /// Close tabs by ID (`prefix.pid.id`, or legacy `prefix.id`)
    Close {
        /// Tab IDs to close; reads from stdin if none given
        tab_ids: Vec<String>,
    },
    /// Move tabs to another window
    #[command(group(ArgGroup::new("target").required(true).args(["to_window", "to_tab"])))]
    Move {
        /// Target window ID (`prefix.pid.w.id`, `prefix.w.id`, or raw browser window ID)
        #[arg(long, value_name = "WINDOW_ID", conflicts_with = "to_tab")]
        to_window: Option<String>,
        /// Move tabs to the window containing this tab ID
        #[arg(long, value_name = "TAB_ID", conflicts_with = "to_window")]
        to_tab: Option<String>,
        /// Target index in the destination window (`-1` appends)
        #[arg(long, default_value_t = -1)]
        index: i64,
        /// Tab IDs to move; reads from stdin if none given
        tab_ids: Vec<String>,
    },
    /// Activate (focus) a tab by ID
    Activate {
        /// Tab ID (`prefix.pid.id`, or legacy `prefix.id`)
        tab_id: String,
    },
    /// Open a URL in a new tab
    Open {
        /// URL to open
        url: String,
        /// Target browser (uses first responsive connected browser if not specified)
        #[arg(short, long)]
        browser: Option<String>,
        /// Target window ID (`prefix.pid.w.id`, `prefix.w.id`, or raw browser window ID)
        #[arg(long, value_name = "WINDOW_ID")]
        window: Option<String>,
        /// Target index in the destination window
        #[arg(long)]
        index: Option<i64>,
    },
    /// Show connected browsers
    Clients,
    /// Diagnose native messaging, extension, and mediator connectivity
    Doctor {
        /// Filter by browser (e.g. chrome, firefox, brave)
        #[arg(short, long)]
        browser: Option<String>,
    },
    /// List read-only synced tabs discovered from local browser state
    Synced {
        #[command(subcommand)]
        command: SyncedCommand,
    },
    /// Install native messaging manifests for detected browsers
    Install {
        /// Path to rustab-mediator binary (auto-detected if not specified)
        #[arg(long)]
        mediator_path: Option<PathBuf>,
        /// Chrome extension ID (for development/sideloaded extensions)
        #[arg(long)]
        chrome_extension_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum SyncedCommand {
    /// List synced tabs
    List {
        /// Output format
        #[arg(short, long, default_value = "tsv")]
        format: OutputFormat,
        /// Filter by browser (currently: orion)
        #[arg(short, long)]
        browser: Option<String>,
        /// Read the newest non-empty archived sync snapshot instead of current state
        #[arg(long)]
        archived: bool,
    },
}
