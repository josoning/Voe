//! Interactive REPL (Read-Eval-Print Loop) for the Voe shell.
//!
//! The REPL is the main entry point of the framework.  It:
//! 1. Creates a [`TerminalGuard`](crate::terminal::TerminalGuard) so the
//!    session can span the full screen.
//! 2. Uses [`reedline`] for line editing — providing Emacs-style
//!    keybindings, persistent history, fish-style autosuggestions, and
//!    graceful handling of Ctrl-D / Ctrl-C / Ctrl-L.
//! 3. Parses each line into tokens, dispatches the first token as a
//!    command name and the rest as arguments.
//! 4. Looks up the command in the registry (builtin + plugin) and
//!    invokes it through the standard [`voe_repo_api::command::Command`]
//!    trait — so the existing `voe init`, `voe status`, ... commands
//!    work unchanged inside the shell.
//! 5. Continues until `exit`/`quit`/Ctrl-D or a fatal error.

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;

use clap::{Arg, ArgAction, Command as ClapCommand};
use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};
use shlex::split as shlex_split;

use crate::context::ShellContext;
use crate::io::println_heading;
use crate::shell_err;
use crate::terminal::TerminalGuard;
use voe_commands::registry::CommandRegistry;
use voe_repo_api::command::CommandContext;
use voe_repo_api::repository::RepoManager;
use voe_types::error::VoeError;

/// Result type for shell-level parse/dispatch errors.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Command '{name}' not found.  Type 'help' for a list.")]
    UnknownCommand { name: String },

    #[error(transparent)]
    Command(#[from] VoeError),
}

/// Top-level shell handle.
///
/// Construct with [`Shell::new`], customise it, then call
/// [`Shell::run`] to enter the REPL.
pub struct Shell {
    registry: Arc<CommandRegistry>,
    repo_manager: Option<Box<dyn RepoManager>>,
    prompt: String,
    banner: Option<String>,
}

impl Shell {
    /// Create a new shell that contains only the commands registered
    /// in the given `registry`.  A default "voe" banner is printed at
    /// startup (suppressed with [`Shell::quiet`]).
    pub fn new(
        registry: Arc<CommandRegistry>,
        repo_manager: Option<Box<dyn RepoManager>>,
    ) -> Self {
        Self {
            registry,
            repo_manager,
            prompt: "voe".to_string(),
            banner: None,
        }
    }

    /// Override the prompt label — useful when the shell is entered
    /// from a sub-context (e.g. maskmanager).
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = prompt.into();
        self
    }

    /// Print a custom banner on startup instead of the default one.
    /// Set the banner to an empty string for no output at all.
    pub fn with_banner(mut self, banner: impl Into<String>) -> Self {
        self.banner = Some(banner.into());
        self
    }

    /// Suppress the default startup banner.
    pub fn quiet(mut self) -> Self {
        self.banner = Some(String::new());
        self
    }

    /// Run the REPL until the user exits or a fatal error occurs.
    ///
    /// On success returns `Ok(())`.  On an unhandled shell-level error
    /// (e.g. we could not even initialise the terminal) returns
    /// `Err(ShellError)`.  Individual command errors are printed inline
    /// and do **not** abort the REPL — only unrecoverable problems
    /// surface as `Err`.
    ///
    /// Uses [`reedline`] for interactive line editing (Emacs-style
    /// keybindings, history, autosuggestions) so users get a modern
    /// shell experience out of the box.  The terminal itself is still
    /// managed by [`TerminalGuard`] so the session remains full-screen
    /// and fully restored on exit.
    pub fn run(self) -> Result<(), ShellError> {
        // --- terminal setup ---
        let _guard = match TerminalGuard::enter_fullscreen() {
            Ok(g) => g,
            Err(_) => {
                // Non-TTY (CI, pipe, …) — fall back to line mode so we
                // still get cursor restore guarantees.  reedline will
                // degrade gracefully in this case.
                TerminalGuard::enter_line_mode()?
            }
        };

        // --- banner ---
        if let Some(banner) = &self.banner {
            if !banner.is_empty() {
                println_heading(banner);
            }
        } else {
            println_heading("Voe shell — type 'help' for commands, 'exit' to quit.");
        }

        // Grab references out of the owned values so the context can
        // borrow them for the lifetime of the loop.
        let repo_mgr_ref: Option<&dyn RepoManager> = self.repo_manager.as_ref().map(|b| b.as_ref());

        let mut ctx =
            ShellContext::new(repo_mgr_ref, &self.registry).with_prompt(self.prompt.clone());

        // --- reedline setup ---
        //
        // reedline owns raw-mode switching and line-editing state; the
        // TerminalGuard above only manages alternate-screen + cursor
        // visibility.  The two layers are independent and coexist cleanly.
        let mut rl = Reedline::create();

        // --- REPL loop ---
        loop {
            let prompt = DefaultPrompt {
                left_prompt: DefaultPromptSegment::Basic(format!("{}> ", ctx.prompt)),
                right_prompt: DefaultPromptSegment::Empty,
            };

            match rl.read_line(&prompt) {
                Ok(Signal::Success(line)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    ctx.record_history(trimmed);

                    match dispatch_line(trimmed, &mut ctx) {
                        Ok(_) => {
                            if ctx.should_exit {
                                break;
                            }
                        }
                        Err(ShellError::UnknownCommand { name }) => {
                            let suggestions = suggest_commands(&name, ctx.command_names());
                            if suggestions.is_empty() {
                                shell_err!("Unknown command: '{}'. Type 'help' for a list.", name);
                            } else {
                                shell_err!(
                                    "Unknown command: '{}'. Did you mean {}?",
                                    name,
                                    suggestions.join(", ")
                                );
                            }
                        }
                        Err(ShellError::Command(e)) => {
                            shell_err!("{}", e);
                        }
                        Err(ShellError::Io(e)) => {
                            shell_err!("IO error: {}", e);
                            break;
                        }
                    }
                }

                // Ctrl-D — clean exit, newline the prompt line.
                Ok(Signal::CtrlD) => {
                    let _ = writeln!(io::stdout());
                    break;
                }

                // Ctrl-C — interrupt the current operation but keep
                // the shell alive.  Users expect to "cancel and retry"
                // rather than exit the whole session.
                Ok(Signal::CtrlC) => {
                    shell_err!("Interrupted.");
                }

                // Catch-all for future Signal variants (reedline marks
                // Signal as non_exhaustive so we must handle unknowns).
                Ok(_) => {}

                Err(e) => {
                    // reedline failed fatally — this is unrecoverable
                    // (e.g. stdin closed mid-read, broken tty).
                    shell_err!("Read error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Line parsing & dispatch

/// Parse a single input line and dispatch it to the right handler.
///
/// Routing order:
/// 1. Shell builtins (`help`, `exit`, `quit`, `clear`) — these are
///    handled directly so they can terminate the REPL or mess with the
///    terminal.
/// 2. Commands registered in [`voe_commands::registry::CommandRegistry`] —
///    these are executed via the standard [`Command`] trait.
fn dispatch_line(line: &str, ctx: &mut ShellContext<'_>) -> Result<(), ShellError> {
    // shlex::split is a drop-in POSIX-compatible tokenizer: it handles
    // single/double quotes, backslash escapes, and comments.  Returns
    // None for unterminated quotes — treat that as "not a parseable
    // command" and let the user try again.
    let Some(tokens) = shlex_split(line) else {
        return Ok(());
    };
    if tokens.is_empty() {
        return Ok(());
    }

    let cmd = tokens[0].to_lowercase();
    let rest = &tokens[1..];

    // --- shell builtins ---
    match cmd.as_str() {
        "help" | "h" | "?" => {
            print_help(ctx);
            return Ok(());
        }
        "exit" | "quit" | "q" => {
            ctx.should_exit = true;
            return Ok(());
        }
        "clear" | "cls" => {
            let _ = crate::terminal::clear_screen();
            return Ok(());
        }
        _ => {}
    }

    // --- registered commands ---
    let Some(_) = ctx.registry.get(&cmd) else {
        return Err(ShellError::UnknownCommand { name: cmd });
    };

    // Store raw tokens for commands that implement clap_command().
    ctx.raw_args = rest.to_vec();

    // Also populate the legacy HashMap-based args for backward compat.
    let (args, flags) = tokens_to_args(rest);
    ctx.args = args;
    ctx.flags = flags;

    let mut cc: CommandContext<'_> = ctx.as_command_ctx();
    ctx.registry
        .execute(&cmd, &mut cc)
        .map_err(ShellError::Command)
}

/// Convert remaining tokens into the `(HashMap args, Vec flags)` shape
/// expected by [`CommandContext`].
///
/// - Tokens like `--name=foo` or `--name foo` become `args["name"] = "foo"`.
/// - Tokens like `--force`, `-v` become `flags`.
/// - Bare positional tokens are stored under `args["__pos"]` joined
///   with unit separator (0x1f) — matching the existing convention in
///   `voe-cli/src/dispatcher.rs`.
fn tokens_to_args(tokens: &[String]) -> (HashMap<String, String>, Vec<String>) {
    let mut args: HashMap<String, String> = HashMap::new();
    let mut flags: Vec<String> = Vec::new();
    let mut pos: Vec<String> = Vec::new();

    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];

        if let Some(eq) = t.find('=') {
            // --key=value
            let key = t[2..eq].to_string();
            let val = t[eq + 1..].to_string();
            args.insert(key, val);
            i += 1;
        } else if t.starts_with("--") && t.len() > 2 {
            // --key value  OR  --flag (no value)
            let key = t[2..].to_string();
            if i + 1 < tokens.len() && !tokens[i + 1].starts_with('-') {
                args.insert(key, tokens[i + 1].clone());
                i += 2;
            } else {
                flags.push(key);
                i += 1;
            }
        } else if t.starts_with('-') && t.len() == 2 {
            // Short flag: -f
            flags.push(t[1..].to_string());
            i += 1;
        } else {
            pos.push(t.clone());
            i += 1;
        }
    }

    if !pos.is_empty() {
        args.insert("__pos".to_string(), pos.join("\x1f"));
    }

    (args, flags)
}

// ---------------------------------------------------------------------------
// Help + completion helpers

/// Print a formatted help block listing every registered command plus
/// the shell builtins.  Column widths are computed dynamically so the
/// table always lines up nicely.
fn print_help(ctx: &ShellContext<'_>) {
    let mut rows: Vec<(&str, &str)> = Vec::new();

    rows.push(("help, h, ?", "Show this help"));
    rows.push(("exit, quit, q", "Exit the shell"));
    rows.push(("clear, cls", "Clear the terminal screen"));

    for info in ctx.command_infos() {
        // Clamp long descriptions so the help block stays readable on
        // narrow terminals (e.g. 80 columns).
        rows.push((info.name, info.description));
    }

    let widest = rows.iter().map(|(n, _)| n.len()).max().unwrap_or(0);

    println_heading("Commands:");
    for (name, desc) in rows {
        use owo_colors::OwoColorize;
        println!("  {:<width$}  {}", name.bold().cyan(), desc, width = widest,);
    }
    println!();
}

/// Given a partial (possibly unknown) command name, return the list of
/// registered names that are close matches.  Used to produce "Did you
/// mean …?" suggestions.
///
/// Uses case-insensitive prefix matching first, then falls back to
/// simple Levenshtein-distance-ish similarity (no external dep — we
/// keep it tiny).
pub fn suggest_commands(input: &str, names: Vec<&str>) -> Vec<String> {
    let input = input.to_lowercase();
    let mut out: Vec<String> = names
        .iter()
        .filter(|n| n.to_lowercase().starts_with(&input))
        .map(|n| (*n).to_string())
        .collect();

    if out.is_empty() {
        // Fallback: edit distance ≤ 3.
        for n in &names {
            let d = levenshtein(&input, &n.to_lowercase());
            if d <= 3 {
                out.push((*n).to_string());
            }
        }
    }
    out.truncate(5);
    out
}

/// Tiny Levenshtein distance — kept on purpose to avoid pulling in a
/// string-distance crate for a one-off helper.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate().take(n + 1) {
        *cell = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1])
            };
        }
    }
    dp[m][n]
}

// ---------------------------------------------------------------------------
// Public surface: a thin wrapper around clap so commands can use the
// same derive-based API they already know, without the shell having to
// re-derive Subcommand every time.

/// Build a `clap::Command` from the name and description so that
/// individual module authors can reuse clap's help-formatting machinery
/// from inside their shell-specific subcommand.
pub fn make_clap_command(name: &'static str, description: &'static str) -> ClapCommand {
    ClapCommand::new(name).about(description).arg(
        Arg::new("help")
            .short('h')
            .long("help")
            .action(ArgAction::HelpShort),
    )
}
