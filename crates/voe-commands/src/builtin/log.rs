use voe_mask::SimpleMaskResolver;
use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_repo_api::snapshot::SnapshotEngine;
use voe_types::error::Result;
use voe_types::object::ObjectId;

use crate::utils::resolve_repo;

const SHORT_OID_LEN: usize = 8;

struct CommitEntry {
    oid: String,
    full_oid: String,
    author_name: String,
    author_email: String,
    author_ts: u64,
    message: String,
    parents: Vec<String>,
}

fn format_unix_ts(secs: u64) -> String {
    let days_since_unix = secs / 86_400;
    let rem = secs % 86_400;
    let hours = rem / 3_600;
    let rem = rem % 3_600;
    let minutes = rem / 60;
    let seconds = rem % 60;

    let (year, month, day) = days_to_ymd(days_since_unix as i64);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn days_to_ymd(mut days: i64) -> (i64, u32, u32) {
    let months_non_leap = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let months_leap = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let mut year: i64 = 1970;

    loop {
        let year_days = if is_leap(year) { 366 } else { 365 };
        if days >= year_days {
            days -= year_days;
            year += 1;
        } else {
            break;
        }
    }

    let month_days = if is_leap(year) {
        months_leap
    } else {
        months_non_leap
    };
    for (i, &md) in month_days.iter().enumerate() {
        if days < md as i64 {
            return (year, (i as u32) + 1, (days as u32) + 1);
        }
        days -= md as i64;
    }

    (year, 12, 31)
}

fn collect_entries(
    repo: &dyn voe_repo_api::Repository,
    head: &ObjectId,
) -> Result<Vec<CommitEntry>> {
    let cs = repo.commit_store();
    let resolver = SimpleMaskResolver;
    let engine = SnapshotEngine::new(cs, repo.chunk_store(), &resolver);

    let pairs = engine.collect_commits(head)?;

    let entries = pairs
        .iter()
        .map(|(oid, c)| {
            let full_oid = oid.to_string();
            let short_oid: String = full_oid.chars().take(SHORT_OID_LEN).collect();
            CommitEntry {
                oid: short_oid,
                full_oid,
                author_name: c.author.name.clone(),
                author_email: c.author.email.clone(),
                author_ts: c.author.timestamp,
                message: c.message.clone(),
                parents: c.parents.iter().map(|p| p.to_string()).collect(),
            }
        })
        .collect();

    Ok(entries)
}

pub struct LogCommand;

impl Command for LogCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "log",
            description: "Show commit history",
            usage: "voe log [--max-count N] [--oneline]",
            examples: &["voe log", "voe log --max-count 5", "voe log --oneline"],
            aliases: &[],
        }
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        let (_root, repo) = resolve_repo(ctx)?;

        let max_count: Option<usize> = ctx
            .args
            .get("max-count")
            .and_then(|s| s.parse::<usize>().ok());

        let oneline = ctx.args.contains_key("oneline");

        let head = match repo.ref_store().get_head()? {
            Some(h) => h,
            None => {
                println!("No commits yet. Repository has no HEAD.");
                return Ok(());
            }
        };

        let entries = collect_entries(repo.as_ref(), &head)?;

        let entries = if let Some(limit) = max_count {
            entries.into_iter().take(limit).collect()
        } else {
            entries
        };

        if entries.is_empty() {
            println!("No commits.");
            return Ok(());
        }

        if oneline {
            for e in &entries {
                let subject = e.message.lines().next().unwrap_or("").trim();
                println!("{} {} {}", e.oid, e.author_name, subject);
            }
        } else {
            for e in &entries {
                let date_str = format_unix_ts(e.author_ts);
                let subject = e.message.lines().next().unwrap_or("").trim();
                println!("commit {}", e.full_oid);
                if e.parents.len() > 1 {
                    let short_parents: Vec<String> = e
                        .parents
                        .iter()
                        .map(|p| p.chars().take(SHORT_OID_LEN).collect())
                        .collect();
                    println!("Merge: {}", short_parents.join(" "));
                }
                println!("Author: {} <{}>", e.author_name, e.author_email);
                println!("Date:   {}", date_str);
                println!();
                println!("    {}", subject);
                println!();
            }
        }

        Ok(())
    }
}
