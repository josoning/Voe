pub mod branchmanager;
pub mod commit;
pub mod help;
pub mod init;
pub mod log;
pub mod maskmanager;
pub mod merge;
pub mod plugin;
pub mod reset;
pub mod rollback;
pub mod shared {
    pub mod mask_listing;
}
pub mod stagemanager;
pub mod status;

use crate::registry::CommandRegistry;

/// Registers all builtin commands into the given registry. Registration failures
/// (e.g. duplicate command names) are reported via `tracing::warn!` rather than panicking,
/// so the binary keeps running even if a plugin or builtin collides.
pub fn register_builtin_commands(registry: &mut CommandRegistry) {
    let results = [
        registry.register(init::InitCommand),
        registry.register(stagemanager::StagemanagerCommand),
        registry.register(maskmanager::MaskManagerCommand),
        registry.register(branchmanager::BranchManagerCommand),
        registry.register(commit::CommitCommand),
        registry.register(status::StatusCommand),
        registry.register(log::LogCommand),
        registry.register(reset::ResetCommand),
        registry.register(rollback::RollbackCommand),
        registry.register(merge::MergeCommand),
        registry.register(help::HelpCommand),
        registry.register(plugin::PluginCommand),
    ];
    for result in results {
        if let Err(e) = result {
            tracing::warn!("failed to register builtin command: {}", e);
        }
    }
}
