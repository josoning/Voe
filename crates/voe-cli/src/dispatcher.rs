use std::collections::HashMap;

use voe_commands::builtin::register_builtin_commands;
use voe_commands::registry::CommandRegistry;
use voe_fs::FsRepoManager;
use voe_plugin::registry::PluginRegistry;
use voe_repo_api::command::CommandContext;
use voe_repo_api::repository::RepoManager;

use crate::app::{BranchmanagerAction, MaskmanagerAction, PluginAction, StagemanagerAction, VoeCommand};

pub struct Dispatcher {
    commands: CommandRegistry,
    #[allow(dead_code)]
    plugins: PluginRegistry,
    repo_manager: Box<dyn RepoManager>,
}

#[allow(dead_code)]
impl Dispatcher {
    pub fn new() -> Self {
        let mut commands = CommandRegistry::new();
        register_builtin_commands(&mut commands);
        Self {
            commands,
            plugins: PluginRegistry::new(),
            repo_manager: Box::new(FsRepoManager::new()),
        }
    }

    pub fn with_plugins(plugins: PluginRegistry) -> Self {
        let mut commands = CommandRegistry::new();
        register_builtin_commands(&mut commands);

        for (_, plugin) in plugins.iter() {
            for cmd in plugin.commands() {
                let _ = commands.register_boxed(cmd);
            }
        }

        Self {
            commands,
            plugins,
            repo_manager: Box::new(FsRepoManager::new()),
        }
    }

    pub fn command_registry(&self) -> &CommandRegistry {
        &self.commands
    }

    pub fn plugin_registry(&self) -> &PluginRegistry {
        &self.plugins
    }

    pub fn repo_manager(&self) -> &dyn RepoManager {
        self.repo_manager.as_ref()
    }

    pub fn dispatch(&self, cmd: &VoeCommand) -> anyhow::Result<()> {
        let (sub_name, args) = to_args(cmd);

        // Collect raw token strings from env for commands that
        // implement clap_command() for type-safe argument parsing.
        let raw_args: Vec<String> = std::env::args().skip(2).collect();

        let mut ctx = CommandContext {
            repo_manager: Some(self.repo_manager.as_ref()),
            args,
            flags: Vec::new(),
            raw_args,
        };

        self.commands
            .execute(sub_name, &mut ctx)
            .map_err(|e| anyhow::anyhow!("{}", e))
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a typed clap `VoeCommand` into the same flat `(name, HashMap)`
/// shape that the old manual `extract_args` produced.  This keeps the
/// `CommandContext.args` interface stable while the CLI layer gains
/// type safety.
fn to_args(cmd: &VoeCommand) -> (&'static str, HashMap<String, String>) {
    let mut args = HashMap::new();

    macro_rules! opt_insert {
        ($key:expr, $val:expr) => {
            if let Some(v) = $val {
                args.insert($key.to_string(), v.clone());
            }
        };
    }
    macro_rules! flag_insert {
        ($key:expr, $flag:expr) => {
            if $flag {
                args.insert($key.to_string(), "true".to_string());
            }
        };
    }

    let name = match cmd {
        VoeCommand::Init { path, name, email } => {
            opt_insert!("path", path);
            opt_insert!("name", name);
            opt_insert!("email", email);
            "init"
        }
        VoeCommand::Stagemanager { action } => {
            match action {
                None => {
                    args.insert("subcommand".to_string(), "list".to_string());
                }
                Some(StagemanagerAction::List) => {
                    args.insert("subcommand".to_string(), "list".to_string());
                }
                Some(StagemanagerAction::Add { n }) => {
                    args.insert("subcommand".to_string(), "add".to_string());
                    args.insert("n".to_string(), n.to_string());
                }
                Some(StagemanagerAction::Remove { n }) => {
                    args.insert("subcommand".to_string(), "remove".to_string());
                    args.insert("n".to_string(), n.to_string());
                }
                Some(StagemanagerAction::Reload) => {
                    args.insert("subcommand".to_string(), "reload".to_string());
                }
            }
            "stagemanager"
        }
        VoeCommand::Maskmanager { action } => {
            match action {
                None => {
                    args.insert("subcommand".to_string(), "list".to_string());
                }
                Some(MaskmanagerAction::List) => {
                    args.insert("subcommand".to_string(), "list".to_string());
                }
                Some(MaskmanagerAction::Split { n }) => {
                    args.insert("subcommand".to_string(), "split".to_string());
                    args.insert("n".to_string(), n.to_string());
                }
                Some(MaskmanagerAction::Merge { indices }) => {
                    args.insert("subcommand".to_string(), "merge".to_string());
                    args.insert(
                        "indices".to_string(),
                        indices
                            .iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join("\x1f"),
                    );
                }
                Some(MaskmanagerAction::Reload) => {
                    args.insert("subcommand".to_string(), "reload".to_string());
                }
            }
            "maskmanager"
        }
        VoeCommand::Branchmanager { action } => {
            match action {
                BranchmanagerAction::Switch { target, force } => {
                    args.insert("target".to_string(), target.clone());
                    flag_insert!("force", *force);
                    args.insert("subcommand".to_string(), "switch".to_string());
                }
                BranchmanagerAction::Create { name } => {
                    args.insert("name".to_string(), name.clone());
                    args.insert("subcommand".to_string(), "create".to_string());
                }
                BranchmanagerAction::Delete { name } => {
                    args.insert("name".to_string(), name.clone());
                    args.insert("subcommand".to_string(), "delete".to_string());
                }
                BranchmanagerAction::Rename { name, new } => {
                    args.insert("name".to_string(), name.clone());
                    args.insert("new-name".to_string(), new.clone());
                    args.insert("subcommand".to_string(), "rename".to_string());
                }
                BranchmanagerAction::Current => {
                    args.insert("subcommand".to_string(), "current".to_string());
                }
                BranchmanagerAction::AddLabel { name, label } => {
                    args.insert("name".to_string(), name.clone());
                    args.insert("label".to_string(), label.clone());
                    args.insert("subcommand".to_string(), "add-label".to_string());
                }
                BranchmanagerAction::RemoveLabel { name, label } => {
                    args.insert("name".to_string(), name.clone());
                    args.insert("label".to_string(), label.clone());
                    args.insert("subcommand".to_string(), "remove-label".to_string());
                }
                BranchmanagerAction::List => {
                    args.insert("subcommand".to_string(), "list".to_string());
                }
            }
            "branchmanager"
        }
        VoeCommand::Commit { message } => {
            opt_insert!("message", message);
            "commit"
        }
        VoeCommand::Status => "status",
        VoeCommand::Log { max_count, oneline } => {
            opt_insert!("max-count", max_count);
            flag_insert!("oneline", *oneline);
            "log"
        }
        VoeCommand::Reset {
            target,
            soft,
            mixed,
        } => {
            args.insert("target".to_string(), target.clone());
            flag_insert!("soft", *soft);
            flag_insert!("mixed", *mixed);
            "reset"
        }
        VoeCommand::Rollback { target, force } => {
            args.insert("target".to_string(), target.clone());
            flag_insert!("force", *force);
            "rollback"
        }
        VoeCommand::Merge {
            target,
            message,
            abort,
        } => {
            opt_insert!("target", target);
            opt_insert!("message", message);
            flag_insert!("abort", *abort);
            "merge"
        }
        VoeCommand::Plugin { action } => {
            match action {
                PluginAction::List => {
                    args.insert("subcommand".to_string(), "list".to_string());
                }
                PluginAction::Install { path } => {
                    args.insert("subcommand".to_string(), "install".to_string());
                    args.insert("path".to_string(), path.clone());
                }
            }
            "plugin"
        }
        VoeCommand::Shell => "shell",
    };

    (name, args)
}
