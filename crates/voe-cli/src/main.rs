mod app;
mod dispatcher;

use std::sync::Arc;

use clap::Parser;

use app::{Cli, VoeCommand};
use dispatcher::Dispatcher;
use voe_commands::builtin::register_builtin_commands;
use voe_commands::registry::CommandRegistry;
use voe_fs::FsRepoManager;
use voe_shell::repl::Shell;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();

    if cli.verbose > 0 {
        tracing::info!("Verbose mode enabled (level {})", cli.verbose);
    }

    // The shell command takes priority — it bypasses the dispatcher and
    // starts its own full-screen REPL loop.  Plugins are loaded before
    // we enter the shell so their commands appear in the registry.
    if matches!(cli.subcommand, VoeCommand::Shell) {
        run_shell();
        return;
    }

    let dispatcher = Dispatcher::new();

    if let Err(e) = dispatcher.dispatch(&cli.subcommand) {
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}

/// Construct and run the interactive shell.
///
/// This is intentionally a free function so the block above stays
/// linear.  Failure to enter the shell (e.g. stdin is closed) prints
/// an error and exits with code 1 — we do not silently fall through
/// to the non-interactive path because the user explicitly asked for
/// the shell.
fn run_shell() {
    let mut registry = CommandRegistry::new();
    register_builtin_commands(&mut registry);
    let registry = Arc::new(registry);
    let manager: Box<dyn voe_repo_api::repository::RepoManager> = Box::new(FsRepoManager::new());

    let shell = Shell::new(registry, Some(manager))
        .with_prompt("voe")
        .with_banner("Voe Shell — interactive mode.  Type 'help' for commands, 'exit' to quit.");

    if let Err(e) = shell.run() {
        eprintln!("Shell error: {:#}", e);
        std::process::exit(1);
    }
}
