use std::collections::HashMap;

use crate::repository::RepoManager;
use clap::Command as ClapCommand;
use voe_types::error::{Result, VoeError};

#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub usage: &'static str,
    pub examples: &'static [&'static str],
    /// Alternative names that map to this command, e.g. `&["maskmgr"]` for
    /// "maskmanager" or `&["stagemgr"]` for "stagemanager".  The registry uses this
    /// list to build an alias → canonical-name index so shell and CLI
    /// dispatch can resolve shorthands transparently.
    pub aliases: &'static [&'static str],
}

pub struct CommandContext<'a> {
    pub repo_manager: Option<&'a dyn RepoManager>,
    pub args: HashMap<String, String>,
    pub flags: Vec<String>,
    /// Raw token strings from the command line (or shell input),
    /// excluding the command name itself.  Commands that implement
    /// [`Command::clap_command`] can parse these tokens directly
    /// for type-safe argument access.
    pub raw_args: Vec<String>,
}

impl<'a> CommandContext<'a> {
    pub fn new() -> Self {
        Self {
            repo_manager: None,
            args: HashMap::new(),
            flags: Vec::new(),
            raw_args: Vec::new(),
        }
    }

    /// Parse `self.raw_args` using the command's own clap definition
    /// and return the matched arguments.  Returns an error when the
    /// command does not implement [`Command::clap_command`].
    pub fn parse_raw_args(&self, cmd: &dyn Command) -> Result<clap::ArgMatches> {
        let clap_cmd = cmd
            .clap_command()
            .ok_or_else(|| VoeError::Other("Command does not provide clap definition".into()))?;
        // Prepend the command name so clap doesn't interpret the first
        // raw token as the binary/program name.
        let name = clap_cmd.get_name().to_string();
        let full_args = std::iter::once(name).chain(self.raw_args.iter().cloned());
        clap_cmd
            .try_get_matches_from(full_args)
            .map_err(|e| VoeError::Other(format!("Argument parse error: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoCommand;

    impl Command for EchoCommand {
        fn info(&self) -> CommandInfo {
            CommandInfo {
                name: "echo",
                description: "Echo a message",
                usage: "echo <message>",
                examples: &[],
                aliases: &[],
            }
        }
        fn execute(&self, _ctx: &mut CommandContext) -> CommandResult {
            Ok(())
        }
        fn clap_command(&self) -> Option<ClapCommand> {
            Some(
                ClapCommand::new("echo")
                    .about("Echo a message")
                    .arg(clap::Arg::new("message").index(1).required(true)),
            )
        }
    }

    struct NoClapCommand;

    impl Command for NoClapCommand {
        fn info(&self) -> CommandInfo {
            CommandInfo {
                name: "no-clap",
                description: "",
                usage: "",
                examples: &[],
                aliases: &[],
            }
        }
        fn execute(&self, _ctx: &mut CommandContext) -> CommandResult {
            Ok(())
        }
    }

    #[test]
    fn parse_raw_args_parses_correctly() {
        let cmd = EchoCommand;
        let mut ctx = CommandContext::new();
        ctx.raw_args = vec!["hello world".to_string()];
        let matches = ctx.parse_raw_args(&cmd).unwrap();
        assert_eq!(
            matches.get_one::<String>("message").map(|s| s.as_str()).unwrap(),
            "hello world"
        );
    }

    #[test]
    fn parse_raw_args_fails_when_missing_required_arg() {
        let cmd = EchoCommand;
        let ctx = CommandContext::new();
        let result = ctx.parse_raw_args(&cmd);
        assert!(result.is_err());
    }

    #[test]
    fn parse_raw_args_returns_error_when_no_clap_command() {
        let cmd = NoClapCommand;
        let ctx = CommandContext::new();
        let result = ctx.parse_raw_args(&cmd);
        assert!(result.is_err());
    }

    #[test]
    fn command_context_has_raw_args() {
        let mut ctx = CommandContext::new();
        assert!(ctx.raw_args.is_empty());
        ctx.raw_args.push("--flag".to_string());
        assert_eq!(ctx.raw_args.len(), 1);
    }
}

impl<'a> Default for CommandContext<'a> {
    fn default() -> Self {
        Self::new()
    }
}

pub type CommandResult = Result<()>;

pub trait Command: Send + Sync {
    fn info(&self) -> CommandInfo;
    fn execute(&self, ctx: &mut CommandContext) -> CommandResult;

    /// Optional clap [`ClapCommand`] describing this command's arguments.
    ///
    /// Override this method to provide a clap definition that the shell
    /// REPL can use for type-safe argument parsing, automatic
    /// validation, `--help` generation, and "did you mean" error
    /// suggestions.  When `None` is returned (the default) the shell
    /// falls back to a lightweight positional/token-based parser.
    ///
    /// The returned [`ClapCommand`] should be self-contained — the shell
    /// injects any subcommands dynamically based on the registry, so
    /// the author does not need to nest them manually.
    ///
    /// # Example
    /// ```ignore
    /// fn clap_command(&self) -> Option<ClapCommand> {
    ///     Some(ClapCommand::new("maskmanager")
    ///         .about("Edit masks")
    ///         .arg(Arg::new("n").required(true)))
    /// }
    /// ```
    fn clap_command(&self) -> Option<ClapCommand> {
        None
    }
}
