use clap::{Arg, ArgAction, Command as ClapCommand};
use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_repo_api::model::branch::BranchId;
use voe_repo_api::model::branch_store::CreateBranchOptions;
use voe_types::error::{Result, VoeError};
use voe_types::object::ObjectId;

use crate::utils::{resolve_oid, resolve_repo, snapshot_for};

pub struct BranchManagerCommand;

impl Command for BranchManagerCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "branchmanager",
            description: "Manage branches: switch, create, delete, rename, list",
            usage: "voe branchmanager <subcommand> [args...]",
            examples: &[
                "voe branchmanager switch main",
                "voe branchmanager switch feature@dev --force",
                "voe branchmanager create feature@dev",
                "voe branchmanager delete feature@dev",
                "voe branchmanager rename feature feature/new",
                "voe branchmanager current",
                "voe branchmanager list",
                "voe branchmanager add-label main mainline",
                "voe branchmanager remove-label feature fix",
            ],
            aliases: &["branchmgr"],
        }
    }

    fn clap_command(&self) -> Option<ClapCommand> {
        Some(
            ClapCommand::new("branchmanager")
                .about("Manage branches: switch, create, delete, rename, list")
                .subcommand(
                    ClapCommand::new("switch")
                        .about("Switch to a branch, alias, or commit and update the working tree")
                        .arg(
                            Arg::new("target")
                                .help("Branch, alias, or commit OID to switch to")
                                .index(1)
                                .required(true),
                        )
                        .arg(
                            Arg::new("force")
                                .short('f')
                                .long("force")
                                .help("Overwrite uncommitted changes in the working tree")
                                .action(ArgAction::SetTrue)
                                .required(false),
                        ),
                )
                .subcommand(
                    ClapCommand::new("create")
                        .about("Create a new branch at the current HEAD")
                        .arg(
                            Arg::new("name")
                                .help("Branch identifier (e.g. feature@dev)")
                                .index(1)
                                .required(true),
                        ),
                )
                .subcommand(
                    ClapCommand::new("delete")
                        .about("Delete a branch")
                        .arg(
                            Arg::new("name")
                                .help("Branch identifier or alias to delete")
                                .index(1)
                                .required(true),
                        ),
                )
                .subcommand(
                    ClapCommand::new("rename")
                        .about("Rename a branch")
                        .arg(
                            Arg::new("name")
                                .help("Current branch identifier")
                                .index(1)
                                .required(true),
                        )
                        .arg(
                            Arg::new("new-name")
                                .help("New name for the branch")
                                .index(2)
                                .required(true),
                        ),
                )
                .subcommand(
                    ClapCommand::new("current")
                        .about("Print the name of the currently checked-out branch"),
                )
                .subcommand(
                    ClapCommand::new("add-label")
                        .about("Add a label to a branch")
                        .arg(
                            Arg::new("name")
                                .help("Branch identifier")
                                .index(1)
                                .required(true),
                        )
                        .arg(
                            Arg::new("label")
                                .help("Label to add")
                                .index(2)
                                .required(true),
                        ),
                )
                .subcommand(
                    ClapCommand::new("remove-label")
                        .about("Remove a label from a branch")
                        .arg(
                            Arg::new("name")
                                .help("Branch identifier")
                                .index(1)
                                .required(true),
                        )
                        .arg(
                            Arg::new("label")
                                .help("Label to remove")
                                .index(2)
                                .required(true),
                        ),
                )
                .subcommand(
                    ClapCommand::new("list")
                        .alias("ls")
                        .about("List all branches"),
                ),
        )
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        let subcommand = ctx
            .args
            .get("subcommand")
            .cloned()
            .unwrap_or_else(|| "list".to_string());

        match subcommand.as_str() {
            "switch" => cmd_switch(ctx),
            "create" => cmd_create(ctx),
            "delete" => cmd_delete(ctx),
            "rename" => cmd_rename(ctx),
            "current" => cmd_current(ctx),
            "add-label" => cmd_add_label(ctx),
            "remove-label" => cmd_remove_label(ctx),
            "list" | "ls" => cmd_list(ctx),
            other => Err(VoeError::Other(format!(
                "Unknown subcommand '{}'. Available: switch, create, delete, rename, current, add-label, remove-label, list",
                other
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Switch
// ---------------------------------------------------------------------------

enum Resolution {
    Branch(voe_repo_api::model::branch::Branch),
    Detached(ObjectId),
}

fn resolve_as_branch(
    repo: &dyn voe_repo_api::Repository,
    target: &str,
) -> Result<Option<voe_repo_api::model::branch::Branch>> {
    let bs = repo.branch_store();
    let branch_id = match bs.resolve_reference(target) {
        Ok(id) => id,
        Err(VoeError::Other(_)) | Err(VoeError::Branch(_)) | Err(VoeError::BranchNotFound(_)) => {
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    match bs.get_branch(&branch_id)? {
        Some(branch) => Ok(Some(branch)),
        None => Ok(None),
    }
}

fn cmd_switch(ctx: &mut CommandContext) -> CommandResult {
    let (_root, repo) = resolve_repo(ctx)?;

    let target = ctx
        .args
        .get("target")
        .cloned()
        .ok_or_else(|| VoeError::Other("Usage: voe branchmanager switch <target>".to_string()))?;

    let force = ctx.args.contains_key("force");

    let resolution = match resolve_as_branch(repo.as_ref(), &target) {
        Ok(Some(branch)) => Resolution::Branch(branch),
        Ok(None) => Resolution::Detached(resolve_oid(repo.as_ref(), &target)?),
        Err(e) => return Err(e),
    };

    let (new_head_oid, mode_label) = match &resolution {
        Resolution::Branch(b) => (b.head.clone(), format!("branch {}", b.id.canonical())),
        Resolution::Detached(oid) => (oid.clone(), format!("commit {}", oid)),
    };

    let current_head = repo.ref_store().get_head()?;

    if Some(&new_head_oid) == current_head.as_ref() {
        match &resolution {
            Resolution::Branch(b) => repo.branch_store().switch_branch(&b.id)?,
            Resolution::Detached(_) => repo.branch_store().detach_head()?,
        }
        let suffix = match repo.branch_store().current_branch()? {
            Some(b) => format!(" ({})", b.id.canonical()),
            None => String::new(),
        };
        println!("Already at {}{}", mode_label, suffix);
        return Ok(());
    }

    if !force {
        if let Some(head_oid) = &current_head {
            let disk = repo.read_working_tree()?;
            let head_snap = snapshot_for(repo.as_ref(), head_oid)?;
            if disk != head_snap {
                return Err(VoeError::Other(
                    "Working tree has uncommitted changes.\n\
                     Commit or stash your changes first, or pass --force to discard them."
                        .to_string(),
                ));
            }
        }
    }

    let target_snap = snapshot_for(repo.as_ref(), &new_head_oid)?;
    repo.write_snapshot_to_disk(&target_snap)?;

    match &resolution {
        Resolution::Branch(b) => repo.branch_store().switch_branch(&b.id)?,
        Resolution::Detached(_) => {
            repo.ref_store().set_head(&new_head_oid)?;
            repo.branch_store().detach_head()?;
        }
    }

    println!("Switched to {}", mode_label);
    Ok(())
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

fn cmd_create(ctx: &mut CommandContext) -> CommandResult {
    let (_root, repo) = resolve_repo(ctx)?;
    let name = ctx
        .args
        .get("name")
        .cloned()
        .ok_or_else(|| VoeError::Other("Usage: voe branchmanager create <name>".to_string()))?;

    let bs = repo.branch_store();
    let refs = repo.ref_store();

    let head_oid = refs.get_head()?.ok_or_else(|| {
        VoeError::Other("No HEAD set — create a commit first before branching".to_string())
    })?;

    let branch_id = BranchId::parse(&name)?;

    if bs.get_branch(&branch_id)?.is_some() {
        return Err(VoeError::BranchAlreadyExists(branch_id.storage_key()));
    }

    let device_id = repo.config_manager().lock_state().device_id.clone();
    let device_id = if device_id.is_empty() {
        "unknown".to_string()
    } else {
        device_id
    };

    let created = bs.create_branch(
        &branch_id,
        head_oid.clone(),
        &device_id,
        &CreateBranchOptions::default(),
    )?;

    println!(
        "Created branch '{}' at {}",
        created.id.canonical(),
        &head_oid.to_string()[..8]
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

fn cmd_delete(ctx: &mut CommandContext) -> CommandResult {
    let (_root, repo) = resolve_repo(ctx)?;
    let name = ctx
        .args
        .get("name")
        .cloned()
        .ok_or_else(|| VoeError::Other("Usage: voe branchmanager delete <name>".to_string()))?;

    let bs = repo.branch_store();
    let branch_id = bs.resolve_reference(&name)?;

    if bs.get_branch(&branch_id)?.is_none() {
        return Err(VoeError::BranchNotFound(branch_id.storage_key()));
    }

    bs.delete_branch(&branch_id)?;
    println!("Deleted branch '{}'", branch_id.canonical());
    Ok(())
}

// ---------------------------------------------------------------------------
// Rename
// ---------------------------------------------------------------------------

fn cmd_rename(ctx: &mut CommandContext) -> CommandResult {
    let (_root, repo) = resolve_repo(ctx)?;
    let name = ctx
        .args
        .get("name")
        .cloned()
        .ok_or_else(|| VoeError::Other(
            "Usage: voe branchmanager rename <name> <new-name>".to_string(),
        ))?;
    let new_name = ctx
        .args
        .get("new-name")
        .cloned()
        .ok_or_else(|| VoeError::Other(
            "Usage: voe branchmanager rename <name> <new-name>".to_string(),
        ))?;

    let bs = repo.branch_store();
    let branch_id = bs.resolve_reference(&name)?;

    if bs.get_branch(&branch_id)?.is_none() {
        return Err(VoeError::BranchNotFound(branch_id.storage_key()));
    }

    let old_canonical = branch_id.canonical();
    let updated = bs.rename_branch(&branch_id, &new_name)?;
    println!(
        "Renamed branch '{}' → '{}'",
        old_canonical,
        updated.id.canonical()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Current
// ---------------------------------------------------------------------------

fn cmd_current(ctx: &mut CommandContext) -> CommandResult {
    let (_root, repo) = resolve_repo(ctx)?;
    match repo.branch_store().current_branch()? {
        Some(branch) => {
            println!("{}", branch.id.canonical());
            Ok(())
        }
        None => Err(VoeError::Other(
            "HEAD is detached — no current branch".to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

fn cmd_list(ctx: &mut CommandContext) -> CommandResult {
    let (_root, repo) = resolve_repo(ctx)?;
    let bs = repo.branch_store();

    let current = bs.current_branch()?;
    let current_key = current.as_ref().map(|b| b.id.storage_key());
    let branches = bs.list_branches()?;

    if branches.is_empty() {
        println!("No branches yet.");
        return Ok(());
    }

    for branch in branches {
        let marker = match current_key.as_deref() {
            Some(k) if k == branch.id.storage_key() => "*",
            _ => " ",
        };
        print!("{} {}", marker, branch.id.canonical());
        if !branch.aliases.is_empty() {
            let alias_strs: Vec<String> = branch.aliases.iter().map(|a| a.display()).collect();
            print!("  ({})", alias_strs.join(", "));
        }
        println!();
    }

    if current.is_none() {
        println!("(HEAD is detached)");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Add Label
// ---------------------------------------------------------------------------

fn cmd_add_label(ctx: &mut CommandContext) -> CommandResult {
    let (_root, repo) = resolve_repo(ctx)?;
    let name = ctx
        .args
        .get("name")
        .cloned()
        .ok_or_else(|| VoeError::Other(
            "Usage: voe branchmanager add-label <name> <label>".to_string(),
        ))?;
    let label = ctx
        .args
        .get("label")
        .cloned()
        .ok_or_else(|| VoeError::Other(
            "Usage: voe branchmanager add-label <name> <label>".to_string(),
        ))?;

    let bs = repo.branch_store();
    let branch_id = bs.resolve_reference(&name)?;

    if bs.get_branch(&branch_id)?.is_none() {
        return Err(VoeError::BranchNotFound(branch_id.storage_key()));
    }

    let old_canonical = branch_id.canonical();
    let updated = bs.add_label(&branch_id, &label)?;

    if old_canonical == updated.id.canonical() {
        println!(
            "Branch '{}' already carries label '@{}' — unchanged",
            old_canonical, label
        );
    } else {
        println!("Added label '@{}' → '{}'", label, updated.id.canonical());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Remove Label
// ---------------------------------------------------------------------------

fn cmd_remove_label(ctx: &mut CommandContext) -> CommandResult {
    let (_root, repo) = resolve_repo(ctx)?;
    let name = ctx
        .args
        .get("name")
        .cloned()
        .ok_or_else(|| VoeError::Other(
            "Usage: voe branchmanager remove-label <name> <label>".to_string(),
        ))?;
    let label = ctx
        .args
        .get("label")
        .cloned()
        .ok_or_else(|| VoeError::Other(
            "Usage: voe branchmanager remove-label <name> <label>".to_string(),
        ))?;

    let bs = repo.branch_store();
    let branch_id = bs.resolve_reference(&name)?;

    if bs.get_branch(&branch_id)?.is_none() {
        return Err(VoeError::BranchNotFound(branch_id.storage_key()));
    }

    let old_canonical = branch_id.canonical();
    let updated = bs.remove_label(&branch_id, &label)?;

    if old_canonical == updated.id.canonical() {
        println!(
            "Branch '{}' does not carry label '@{}' — unchanged",
            old_canonical, label
        );
    } else {
        println!(
            "Removed label '@{}' → '{}'",
            label, updated.id.canonical()
        );
    }
    Ok(())
}