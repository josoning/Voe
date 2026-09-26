use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};

use crate::registry::CommandRegistry;

pub struct HelpCommand;

impl HelpCommand {
    pub fn show_all(registry: &CommandRegistry) {
        println!("Voe - Version Control System");
        println!();
        println!("Available commands:");
        for info in registry.infos() {
            println!("  {:<10} {}", info.name, info.description);
        }
    }

    pub fn show_command(registry: &CommandRegistry, name: &str) {
        if let Some(cmd) = registry.get(name) {
            let info = cmd.info();
            println!("{} - {}", info.name, info.description);
            println!();
            println!("Usage: {}", info.usage);
            if !info.examples.is_empty() {
                println!();
                println!("Examples:");
                for ex in info.examples {
                    println!("  {}", ex);
                }
            }
        } else {
            eprintln!("No such command: {}", name);
        }
    }
}

impl Command for HelpCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "help",
            description: "Show help for commands",
            usage: "voe help [COMMAND]",
            examples: &["voe help", "voe help init"],
            aliases: &[],
        }
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        if let Some(cmd_name) = ctx.args.get("command") {
            println!("Help for '{}' - use command registry for details", cmd_name);
        } else {
            println!("Voe - Version Control System");
            println!("Type 'voe help <command>' for more information on a specific command.");
        }
        Ok(())
    }
}
