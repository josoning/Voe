use clap::{ArgAction, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "voe",
    version,
    about = "Voe - A modular and extensible version control system",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub subcommand: VoeCommand,

    #[arg(
        short = 'v',
        long = "verbose",
        help = "Increase verbosity",
        action = ArgAction::Count,
    )]
    pub verbose: u8,
}

#[derive(Subcommand, Debug)]
pub enum VoeCommand {
    /// Initialize a new Voe repository
    Init {
        /// Directory to initialize (defaults to current directory)
        #[arg(index = 1)]
        path: Option<String>,

        /// Author name (skip interactive prompt)
        #[arg(long = "name", value_name = "NAME")]
        name: Option<String>,

        /// Author email (skip interactive prompt)
        #[arg(long = "email", value_name = "EMAIL")]
        email: Option<String>,
    },

    /// View and edit masks (list, split, merge)
    ///
    /// Use `voe shell` to enter the interactive shell where every
    /// maskmanager subcommand is available without the "voe" prefix.
    #[command(alias = "maskmgr")]
    Maskmanager {
        #[command(subcommand)]
        action: Option<MaskmanagerAction>,
    },

    /// Manage the staging area: stage and unstage masks
    #[command(alias = "stagemgr")]
    Stagemanager {
        #[command(subcommand)]
        action: Option<StagemanagerAction>,
    },

    /// Manage branches: switch, create, delete, rename, list
    #[command(alias = "branchmgr")]
    Branchmanager {
        #[command(subcommand)]
        action: BranchmanagerAction,
    },

    /// Create a commit from staged changes
    Commit {
        /// Commit message
        #[arg(short = 'm', long = "message")]
        message: Option<String>,
    },

    /// Show the working tree status
    Status,

    /// Show commit history
    Log {
        /// Limit number of commits shown
        #[arg(short = 'n', long = "max-count", value_name = "N")]
        max_count: Option<String>,

        /// Show one line per commit
        #[arg(short = '1', long = "oneline", action = ArgAction::SetTrue)]
        oneline: bool,
    },

    /// Move HEAD to a previous commit (soft or mixed)
    ///
    /// Soft: Only HEAD moves. Index and working tree stay as-is, so
    /// old staged changes become staged relative to the new HEAD.
    /// Mixed: HEAD moves and the index is cleared (default). Working tree
    /// stays as-is, so old staged changes become unstaged.
    ///
    /// For destructive hard resets use `voe rollback --force`.
    Reset {
        /// Commit OID (full or short), or HEAD
        #[arg(index = 1)]
        target: String,

        /// Soft reset: move HEAD only, keep index and working tree
        #[arg(long = "soft", action = ArgAction::SetTrue)]
        soft: bool,

        /// Mixed reset: move HEAD and clear index (default)
        #[arg(long = "mixed", action = ArgAction::SetTrue)]
        mixed: bool,
    },

    /// DESTRUCTIVE: hard reset — discard all local changes
    ///
    /// HARD reset: move HEAD, clear the index, and overwrite the working tree.
    ///
    /// This command discards ALL uncommitted changes — both staged and
    /// unstaged — and is provided as a distinct verb (`voe rollback`) to
    /// guard against accidental destruction.
    ///
    /// For non-destructive HEAD movement use `voe reset --soft` or `voe reset`.
    Rollback {
        /// Commit OID (full or short), or HEAD
        #[arg(index = 1)]
        target: String,

        /// Actually perform the destructive rollback (required)
        #[arg(short = 'f', long = "force", action = ArgAction::SetTrue)]
        force: bool,
    },

    /// Merge another branch into the current branch
    Merge {
        /// Branch identifier (or alias) to merge into the current branch
        #[arg(index = 1)]
        target: Option<String>,

        /// Merge commit message
        #[arg(short = 'm', long = "message")]
        message: Option<String>,

        /// Abort an in-progress merge and restore the pre-merge state
        #[arg(long = "abort", action = ArgAction::SetTrue)]
        abort: bool,
    },

    /// Manage Voe plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },

    /// Launch the interactive Voe shell (full-screen REPL)
    ///
    /// Enter a full-screen interactive session where every builtin and
    /// plugin command is available without typing the "voe" prefix.
    /// Type `exit`, `quit`, or `q` to return to your regular prompt.
    Shell,
}

#[derive(Subcommand, Debug)]
pub enum BranchmanagerAction {
    /// Switch to a branch, alias, or commit and update the working tree
    Switch {
        /// Branch, alias, or commit OID to switch to
        #[arg(index = 1)]
        target: String,

        /// Overwrite uncommitted changes in the working tree
        #[arg(short = 'f', long = "force", action = ArgAction::SetTrue)]
        force: bool,
    },

    /// Create a new branch at the current HEAD
    Create {
        /// Branch identifier (e.g. feature@dev)
        #[arg(index = 1)]
        name: String,
    },

    /// Delete a branch
    Delete {
        /// Branch identifier or alias to delete
        #[arg(index = 1)]
        name: String,
    },

    /// Rename a branch
    Rename {
        /// Current branch identifier
        #[arg(index = 1)]
        name: String,

        /// New name for the branch
        #[arg(index = 2)]
        new: String,
    },

    /// Print the name of the currently checked-out branch
    Current,

    /// Add a label to a branch
    #[command(name = "add-label")]
    AddLabel {
        /// Branch identifier
        #[arg(index = 1)]
        name: String,

        /// Label to add
        #[arg(index = 2)]
        label: String,
    },

    /// Remove a label from a branch
    #[command(name = "remove-label")]
    RemoveLabel {
        /// Branch identifier
        #[arg(index = 1)]
        name: String,

        /// Label to remove
        #[arg(index = 2)]
        label: String,
    },

    /// List all branches
    #[command(alias = "ls")]
    List,
}

#[derive(Subcommand, Debug)]
pub enum PluginAction {
    /// List installed plugins
    List,
    /// Install a plugin from a path
    Install {
        /// Path to the plugin shared library
        #[arg(index = 1)]
        path: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum StagemanagerAction {
    /// Show every mask currently staged (index)
    #[command(alias = "ls", alias = "l")]
    List,

    /// Stage the n-th unstaged mask (1-based)
    #[command(alias = "stage", alias = "a")]
    Add {
        /// 1-based index into the unstaged list
        #[arg(index = 1)]
        n: usize,
    },

    /// Remove the n-th staged mask from the index (1-based index into staged list)
    #[command(alias = "unstage", alias = "rm", alias = "r")]
    Remove {
        /// 1-based index into the staged list
        #[arg(index = 1)]
        n: usize,
    },

    /// Re-scan the working tree and reload the index view
    #[command(alias = "refresh")]
    Reload,
}

#[derive(Subcommand, Debug)]
pub enum MaskmanagerAction {
    /// Show every known mask (unstaged + current staging area)
    #[command(alias = "ls", alias = "l")]
    List,

    /// Split the n-th mask into single-change masks (auto-syncs index if staged)
    Split {
        /// 1-based index into the full mask list (same as `list` output)
        #[arg(index = 1)]
        n: usize,
    },

    /// Merge multiple masks into one multi-change mask
    #[command(alias = "m")]
    Merge {
        /// 1-based indices (full mask list), two or more required
        #[arg(index = 1, num_args = 2..)]
        indices: Vec<usize>,
    },

    /// Re-scan the working tree and reload the view
    #[command(alias = "refresh")]
    Reload,
}
