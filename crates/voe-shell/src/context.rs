//! Shell execution context.
//!
//! A [`ShellContext`] is created when the shell starts and is threaded
//! through every command invocation.  It carries:
//! - A [`voe_repo_api::command::CommandContext`] so commands can access the
//!   repository, flags, and positional arguments.
//! - The [`CommandRegistry`](voe_commands::registry::CommandRegistry)
//!   that knows about every registered command (builtin + plugin).
//! - Shell-local state such as the prompt label, command history, and
//!   whether the shell should terminate after the current command.
//!
//! The context is intentionally cheap to construct and does **not**
//! carry the terminal guard — that lives on the stack of the REPL loop
//! so it is dropped at the right moment.

use std::collections::HashMap;

use voe_commands::registry::CommandRegistry;
use voe_repo_api::command::{CommandContext, CommandInfo};
use voe_repo_api::repository::RepoManager;

/// Runtime context shared by every command executed inside the shell.
///
/// Unlike a global static, each `ShellContext` is independent — running
/// two shells in parallel (e.g. for testing or nested use) is safe.
pub struct ShellContext<'a> {
    /// Access to repository operations, passed through to
    /// [`voe_repo_api::command::CommandContext`].
    pub repo_manager: Option<&'a dyn RepoManager>,

    /// All commands registered in this shell (builtin + plugin).
    pub registry: &'a CommandRegistry,

    /// User-supplied arguments for the current command dispatch.  The
    /// REPL loop populates this before handing control to the command.
    pub args: HashMap<String, String>,

    /// Boolean flags parsed from the command line (e.g. `--force`).
    pub flags: Vec<String>,

    /// Raw token strings for the current command (excluding the command
    /// name itself).  Commands that implement `clap_command()` can
    /// parse these directly for type-safe argument access.
    pub raw_args: Vec<String>,

    /// The prompt text, e.g. `"voe"`, `"maskmanagmrger"`.  Commands can read
    /// this for display purposes; the REPL loop owns the actual drawing.
    pub prompt: String,

    /// When set to `true` the REPL loop exits cleanly after the current
    /// command finishes.  Commands such as `exit` / `quit` flip this.
    pub should_exit: bool,

    /// Command history (trimmed to the most recent N entries).  Used by
    /// the completion helper and reserved for future line-editing support.
    pub history: Vec<String>,
}

impl<'a> ShellContext<'a> {
    /// Construct a new shell context with the default "voe" prompt.
    pub fn new(repo_manager: Option<&'a dyn RepoManager>, registry: &'a CommandRegistry) -> Self {
        Self {
            repo_manager,
            registry,
            args: HashMap::new(),
            flags: Vec::new(),
            raw_args: Vec::new(),
            prompt: "voe".to_string(),
            should_exit: false,
            history: Vec::new(),
        }
    }

    /// Set the prompt label (e.g. when entering a sub-shell like maskmanagmrger).
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = prompt.into();
        self
    }

    /// Convert the shell context into a [`CommandContext`] suitable for
    /// dispatching a registered command.
    ///
    /// This is how the REPL bridges the voe-shell layer to the existing
    /// voe-commands layer — a single `.as_command_ctx()` call and the
    /// command system does not know it came from an interactive shell.
    pub fn as_command_ctx(&mut self) -> CommandContext<'a> {
        CommandContext {
            repo_manager: self.repo_manager,
            args: self.args.clone(),
            flags: self.flags.clone(),
            raw_args: self.raw_args.clone(),
        }
    }

    /// Record a successfully parsed line in the history buffer.
    ///
    /// Duplicates are suppressed (we do not record the same line twice in
    /// a row) and the buffer is capped at 100 entries to bound memory.
    pub fn record_history(&mut self, line: &str) {
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        if self.history.last().map(|s| s.as_str()) == Some(trimmed.as_str()) {
            return;
        }
        self.history.push(trimmed);
        if self.history.len() > 100 {
            self.history.remove(0);
        }
    }

    /// Return the list of all known command names, sorted.
    pub fn command_names(&self) -> Vec<&str> {
        self.registry.names()
    }

    /// Return the list of all registered command metadata.
    pub fn command_infos(&self) -> Vec<CommandInfo> {
        self.registry.infos()
    }
}
