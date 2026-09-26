use std::collections::HashMap;

use tracing;
use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_types::error::{Result, VoeError};

pub struct CommandRegistry {
    commands: HashMap<String, Box<dyn Command>>,
    /// Alias → canonical-name index.  Populated from each registered
    /// command's [`CommandInfo::aliases`] list so callers can resolve
    /// shorthand names like `"mm"` into the canonical `"maskmanager"`.
    aliases: HashMap<String, String>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    pub fn register<C: Command + 'static>(&mut self, command: C) -> Result<()> {
        self.register_boxed(Box::new(command))
    }

    pub fn register_boxed(&mut self, command: Box<dyn Command>) -> Result<()> {
        let info = command.info();
        let name = info.name.to_string();
        if self.commands.contains_key(&name) {
            return Err(VoeError::Command(format!(
                "Command '{}' is already registered",
                name
            )));
        }
        for alias in info.aliases {
            let alias_str = (*alias).to_string();
            if self.aliases.contains_key(&alias_str) || self.commands.contains_key(&alias_str) {
                tracing::warn!(
                    "alias '{}' for '{}' conflicts with an existing name — skipping",
                    alias_str, name
                );
                continue;
            }
            self.aliases.insert(alias_str, name.clone());
        }
        self.commands.insert(name, command);
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) -> Result<()> {
        self.commands.remove(name);
        self.aliases.retain(|_, canon| canon != name);
        Ok(())
    }

    /// Look up a command by canonical name or alias.  Returns the
    /// canonical `&dyn Command` if found.
    pub fn get(&self, name: &str) -> Option<&dyn Command> {
        let name_lc = name.to_ascii_lowercase();
        if let Some(cmd) = self.commands.get(name_lc.as_str()) {
            return Some(cmd.as_ref());
        }
        if let Some(canon) = self.aliases.get(name_lc.as_str()) {
            return self.commands.get(canon.as_str()).map(|c| c.as_ref());
        }
        None
    }

    /// Returns `true` when `name` matches either a canonical command
    /// name or a known alias.
    pub fn contains(&self, name: &str) -> bool {
        let name_lc = name.to_ascii_lowercase();
        self.commands.contains_key(name_lc.as_str()) || self.aliases.contains_key(name_lc.as_str())
    }

    /// All canonical command names, sorted.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.commands
            .keys()
            .map(|s| s.as_str())
            .chain(self.aliases.keys().map(|s| s.as_str()))
            .collect();
        names.sort();
        names
    }

    /// Canonical names plus all registered aliases, sorted.  Useful for
    /// shell completion that wants every known callable token.
    pub fn names_with_aliases(&self) -> Vec<&str> {
        let mut all: Vec<&str> = self.commands.keys().map(|k| k.as_str()).collect();
        all.extend(self.aliases.keys().map(|k| k.as_str()));
        all.sort();
        all
    }

    pub fn infos(&self) -> Vec<CommandInfo> {
        let mut infos: Vec<CommandInfo> = self.commands.values().map(|c| c.info()).collect();
        infos.sort_by(|a, b| a.name.cmp(b.name));
        infos
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    /// Resolve `name` (canonical or alias) to a command and execute it.
    /// Returns `UnknownCommand` if neither matches.
    pub fn execute(&self, name: &str, ctx: &mut CommandContext) -> CommandResult {
        let name_lc = name.to_ascii_lowercase();
        if let Some(cmd) = self.commands.get(name_lc.as_str()) {
            return cmd.execute(ctx);
        }
        if let Some(canon) = self.aliases.get(name_lc.as_str()) {
            if let Some(cmd) = self.commands.get(canon.as_str()) {
                return cmd.execute(ctx);
            }
        }
        Err(VoeError::Command(format!("Unknown command: {}", name)))
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCommand;

    impl Command for TestCommand {
        fn info(&self) -> CommandInfo {
            CommandInfo {
                name: "test",
                description: "A test command",
                usage: "test",
                examples: &[],
                aliases: &["t", "ts"],
            }
        }

        fn execute(&self, _ctx: &mut CommandContext) -> CommandResult {
            Ok(())
        }
    }

    #[test]
    fn register_new_command() {
        let mut reg = CommandRegistry::new();
        assert!(reg.register(TestCommand).is_ok());
        assert!(reg.contains("test"));
    }

    #[test]
    fn register_duplicate_command_fails() {
        let mut reg = CommandRegistry::new();
        reg.register(TestCommand).unwrap();
        let result = reg.register(TestCommand);
        assert!(result.is_err());
    }

    #[test]
    fn alias_resolves_to_canonical_name() {
        let mut reg = CommandRegistry::new();
        reg.register(TestCommand).unwrap();
        let cmd = reg.get("t");
        assert!(cmd.is_some());
    }

    #[test]
    fn alias_conflict_with_existing_name_is_skipped() {
        let mut reg = CommandRegistry::new();

        struct ConflictCommand;
        impl Command for ConflictCommand {
            fn info(&self) -> CommandInfo {
                CommandInfo {
                    name: "conflict",
                    description: "",
                    usage: "",
                    examples: &[],
                    aliases: &["test"],
                }
            }
            fn execute(&self, _ctx: &mut CommandContext) -> CommandResult {
                Ok(())
            }
        }

        reg.register(TestCommand).unwrap();
        // The alias "test" should be skipped (not crash), and command
        // registration should still succeed.
        assert!(reg.register(ConflictCommand).is_ok());
        // "conflict" should resolve to the conflict command
        assert!(reg.get("conflict").is_some());
    }

    #[test]
    fn names_returns_all_commands_and_aliases() {
        let mut reg = CommandRegistry::new();
        reg.register(TestCommand).unwrap();
        let names = reg.names();
        assert!(names.iter().any(|n| *n == "test"), "expected 'test' in names");
        assert!(names.iter().any(|n| *n == "t"), "expected 't' in names");
        assert!(names.iter().any(|n| *n == "ts"), "expected 'ts' in names");
    }

    #[test]
    fn unregister_removes_command() {
        let mut reg = CommandRegistry::new();
        reg.register(TestCommand).unwrap();
        assert!(reg.unregister("test").is_ok());
        assert!(!reg.contains("test"));
    }

    #[test]
    fn get_unknown_command_returns_none() {
        let reg = CommandRegistry::new();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn execute_runs_registered_command() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct TrackerCommand {
            executed: &'static AtomicBool,
        }

        impl Command for TrackerCommand {
            fn info(&self) -> CommandInfo {
                CommandInfo {
                    name: "tracker",
                    description: "",
                    usage: "",
                    examples: &[],
                    aliases: &[],
                }
            }
            fn execute(&self, _ctx: &mut CommandContext) -> CommandResult {
                self.executed.store(true, Ordering::SeqCst);
                Ok(())
            }
        }

        static EXECUTED: AtomicBool = AtomicBool::new(false);
        let mut reg = CommandRegistry::new();
        reg.register(TrackerCommand {
            executed: &EXECUTED,
        })
        .unwrap();

        let mut ctx = CommandContext::new();
        reg.execute("tracker", &mut ctx).unwrap();
        assert!(EXECUTED.load(Ordering::SeqCst));
    }
}
