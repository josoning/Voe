use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use clap::{Arg, Command as ClapCommand};
use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_types::error::{Result as VoeResult, VoeError};

pub struct InitCommand;

fn try_git_config(key: &str) -> Option<String> {
    let out = StdCommand::new("git")
        .args(["config", "--get", key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn prompt(label: &str, default: Option<&str>) -> VoeResult<String> {
    let hint = match default {
        Some(d) => format!(" [{}]", d),
        None => String::new(),
    };
    print!("{}{}: ", label, hint);
    io::stdout()
        .flush()
        .map_err(|e| VoeError::Other(e.to_string()))?;

    let mut buf = String::new();
    io::stdin()
        .read_line(&mut buf)
        .map_err(|e| VoeError::Other(e.to_string()))?;

    let trimmed = buf.trim().to_string();
    if trimmed.is_empty() {
        Ok(default.map(|d| d.to_string()).unwrap_or_default())
    } else {
        Ok(trimmed)
    }
}

fn setup_user_config(
    ctx: &mut CommandContext,
    repo_path: &Path,
    name_flag: Option<String>,
    email_flag: Option<String>,
) -> VoeResult<bool> {
    let repo_manager = ctx
        .repo_manager
        .ok_or_else(|| VoeError::Other("RepoManager not available".to_string()))?;

    let mut repo = repo_manager.open(repo_path.to_path_buf())?;

    let git_name = try_git_config("user.name");
    let git_email = try_git_config("user.email");

    let name: Option<String>;
    let email: Option<String>;

    if name_flag.is_some() || email_flag.is_some() {
        name = name_flag.or_else(|| git_name.clone());
        email = email_flag.or_else(|| git_email.clone());
    } else if git_name.is_some() || git_email.is_some() {
        name = git_name;
        email = git_email;
    } else if io::stdin().is_terminal() {
        let default_name = Some("Voe User");
        let default_email = Some("voe@example.com");
        let input_name = prompt("Author name", default_name)?;
        let input_email = prompt("Author email", default_email)?;
        name = Some(if input_name.is_empty() {
            "Voe User".to_string()
        } else {
            input_name
        });
        email = Some(if input_email.is_empty() {
            "voe@example.com".to_string()
        } else {
            input_email
        });
    } else {
        name = None;
        email = None;
    }

    let mut configured = false;
    if let Some(n) = name {
        repo.config_manager_mut().set_user_name(n)?;
        configured = true;
    }
    if let Some(e) = email {
        repo.config_manager_mut().set_user_email(e)?;
        configured = true;
    }
    Ok(configured)
}

impl Command for InitCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "init",
            description: "Initialize a new Voe repository",
            usage: "voe init [PATH] [--name NAME] [--email EMAIL]",
            examples: &[
                "voe init",
                "voe init my-project",
                "voe init --name Alice --email alice@example.com",
            ],
            aliases: &[],
        }
    }

    fn clap_command(&self) -> Option<ClapCommand> {
        Some(
            ClapCommand::new("init")
                .about("Initialize a new Voe repository")
                .arg(
                    Arg::new("path")
                        .help("Directory to initialize (defaults to current directory)")
                        .index(1)
                        .required(false),
                )
                .arg(
                    Arg::new("name")
                        .long("name")
                        .help("Author name (skip interactive prompt)")
                        .required(false),
                )
                .arg(
                    Arg::new("email")
                        .long("email")
                        .help("Author email (skip interactive prompt)")
                        .required(false),
                ),
        )
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        let path = ctx
            .args
            .get("path")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let repo_manager = ctx
            .repo_manager
            .ok_or_else(|| VoeError::Other("RepoManager not available".to_string()))?;

        repo_manager.init(path.clone())?;

        println!("Initialized empty Voe repository in {}", path.display());

        let name_flag = ctx.args.get("name").cloned();
        let email_flag = ctx.args.get("email").cloned();

        match setup_user_config(ctx, &path, name_flag, email_flag) {
            Ok(true) => {
                let nm = ctx
                    .repo_manager
                    .unwrap()
                    .open(path.clone())?
                    .config_manager()
                    .get_user_name()
                    .unwrap_or_default();
                let em = ctx
                    .repo_manager
                    .unwrap()
                    .open(path.clone())?
                    .config_manager()
                    .get_user_email()
                    .unwrap_or_default();
                println!("Author identity configured: \"{} <{}>\"", nm, em);
            }
            Ok(false) => {
                println!("Hint: set author identity before committing:");
                println!("  voe config set user.name  \"Your Name\"");
                println!("  voe config set user.email \"you@example.com\"");
                if try_git_config("user.name").is_none() {
                    println!("  (or `git config --global user.name ...` — voe auto-detects)");
                }
            }
            Err(e) => {
                eprintln!("Warning: author config setup failed: {}", e);
            }
        }

        Ok(())
    }
}
