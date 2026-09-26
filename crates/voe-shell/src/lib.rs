//! Voe Shell — a unified interactive command-line framework for the Voe
//! version control system.
//!
//! The shell provides:
//!
//! - **Full-screen immersive REPL** via the [`terminal`] module, backed
//!   by [`crossterm`] for cross-platform compatibility (bash, zsh, fish,
//!   PowerShell).  The [`TerminalGuard`](terminal::TerminalGuard) RAII
//!   guard guarantees the terminal is always restored on exit — even
//!   after panic or Ctrl-C.
//! - **Unified colored I/O** via the [`io`] module, which wraps
//!   [`owo_colors`] behind a semantic [`ColorScheme`](io::ColorScheme).
//!   Commands say "print an error" not "print red bold text", and colors
//!   are automatically disabled under `NO_COLOR`, `CLICOLOR=0`, or
//!   non-TTY output.
//! - **Extension-friendly dispatch** via the [`repl`] module.  The REPL
//!   consumes tokens, converts them into the standard
//!   [`CommandContext`](voe_repo_api::command::CommandContext) shape, and
//!   dispatches to commands registered in
//!   [`CommandRegistry`](voe_commands::registry::CommandRegistry) — so
//!   every existing `voe ...` command and every future plugin works in
//!   the shell unchanged.
//! - **Isolated state** per [`ShellContext`] so the shell and the main
//!   binary can share repository access but keep history, prompts, and
//!   exit flags separate.
//!
//! # Quick start
//! ```no_run
//! use std::sync::Arc;
//! use voe_shell::repl::Shell;
//! use voe_commands::registry::CommandRegistry;
//! use voe_commands::builtin::register_builtin_commands;
//!
//! let mut registry = CommandRegistry::new();
//! register_builtin_commands(&mut registry);
//! let registry = Arc::new(registry);
//!
//! Shell::new(registry, None)
//!     .with_prompt("voe")
//!     .run()
//!     .unwrap();
//! ```
//!
//! # Macros
//! The crate exports five convenience macros (`shell_info!`, `shell_ok!`,
//! `shell_warn!`, `shell_err!`, `shell_heading!`) that format their
//! arguments through the default color scheme and write to the correct
//! stream (stdout vs stderr).

pub mod context;
pub mod io;
pub mod repl;
pub mod terminal;

pub use context::ShellContext;
pub use repl::{Shell, ShellError};
pub use terminal::TerminalGuard;

// Re-export the most commonly-used IO symbols so callers can do
// `use voe_shell::io::{ColorRole, ColorScheme};` in one line.
pub use io::{ColorRole, ColorScheme};
