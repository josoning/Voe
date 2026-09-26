use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;
use voe_fs::FsRepoManager;
use voe_repo_api::repository::RepoManager;

fn temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let uuid = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    p.push(format!(
        "voe_cfg_int_{}_{}_{}",
        name,
        uuid,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&p);
    p
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

fn lock_json(dir: &Path) -> Value {
    let lock_path = dir.join(".voe").join("voeconfig.lock");
    let raw = fs::read_to_string(&lock_path).expect("lock file should exist");
    serde_json::from_str(&raw).expect("lock should be valid JSON")
}

fn toml_path(dir: &Path) -> PathBuf {
    dir.join("voeconfig.toml")
}

#[test]
fn test_init_creates_toml_and_lock() {
    let dir = temp_dir("init_both");
    let mgr = FsRepoManager::new();
    mgr.init(dir.clone()).expect("init");

    assert!(
        toml_path(&dir).exists(),
        "voeconfig.toml must be created at repo root"
    );
    assert!(
        dir.join(".voe").join("voeconfig.lock").exists(),
        "voeconfig.lock must be created inside .voe/"
    );

    let toml_content = fs::read_to_string(toml_path(&dir)).unwrap();
    assert!(toml_content.contains("repositoryformatversion"));

    let lock = lock_json(&dir);
    assert!(lock.get("toml_hash").unwrap().as_str().unwrap().len() == 64);
    assert!(lock.get("device_id").unwrap().as_str().unwrap().len() > 0);
    assert!(lock.get("compiled_ignores").unwrap().is_array());

    cleanup(&dir);
}

#[test]
fn test_lock_device_id_is_uuid_format() {
    let dir = temp_dir("dev_uuid");
    let mgr = FsRepoManager::new();
    mgr.init(dir.clone()).expect("init");

    let lock = lock_json(&dir);
    let dev = lock["device_id"].as_str().unwrap();
    let parts: Vec<&str> = dev.split('-').collect();
    assert_eq!(parts.len(), 5, "UUID must have 5 hyphen-separated groups");
    assert_eq!(parts[0].len(), 8);
    assert_eq!(parts[1].len(), 4);
    assert_eq!(parts[2].len(), 4);
    assert_eq!(parts[3].len(), 4);
    assert_eq!(parts[4].len(), 12);

    cleanup(&dir);
}

#[test]
fn test_toml_hash_is_sha256_hex() {
    let dir = temp_dir("hash_hex");
    let mgr = FsRepoManager::new();
    mgr.init(dir.clone()).expect("init");

    let lock = lock_json(&dir);
    let hash = lock["toml_hash"].as_str().unwrap();
    assert_eq!(hash.len(), 64, "SHA-256 hex must be 64 chars");
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));

    cleanup(&dir);
}

#[test]
fn test_set_user_survives_reopen() {
    let dir = temp_dir("survive");
    let mgr = FsRepoManager::new();
    {
        let mut repo = mgr.init(dir.clone()).expect("init");
        repo.config_manager_mut()
            .set_user_name("Zoe".to_string())
            .expect("set name");
        repo.config_manager_mut()
            .set_user_email("zoe@x.com".to_string())
            .expect("set email");
    }

    let repo2 = mgr.open(dir.clone()).expect("reopen");
    assert_eq!(
        repo2.config_manager().get_user_name(),
        Some("Zoe".to_string())
    );
    assert_eq!(
        repo2.config_manager().get_user_email(),
        Some("zoe@x.com".to_string())
    );

    cleanup(&dir);
}

#[test]
fn test_device_id_identical_across_reopens() {
    let dir = temp_dir("dev_id_stable");
    let mgr = FsRepoManager::new();

    mgr.init(dir.clone()).expect("init");
    let dev_first = lock_json(&dir)["device_id"].as_str().unwrap().to_string();

    drop(mgr.open(dir.clone()).expect("open1"));
    drop(mgr.open(dir.clone()).expect("open2"));

    let dev_second = lock_json(&dir)["device_id"].as_str().unwrap().to_string();
    assert_eq!(
        dev_first, dev_second,
        "device_id must not change across reopen/open cycles"
    );

    cleanup(&dir);
}

#[test]
fn test_external_toml_modification_updates_lock() {
    let dir = temp_dir("ext_mod");
    let mgr = FsRepoManager::new();
    let mut repo = mgr.init(dir.clone()).expect("init");

    let hash_before = lock_json(&dir)["toml_hash"].as_str().unwrap().to_string();

    let new_toml = r#"
[user]
name = "External"
email = "ext@x.com"

[[ignore]]
pattern = "*.tmp"
recursive = false
"#;
    fs::write(toml_path(&dir), new_toml).unwrap();

    repo.config_manager().refresh().expect("refresh");

    let hash_after = lock_json(&dir)["toml_hash"].as_str().unwrap().to_string();
    assert_ne!(
        hash_before, hash_after,
        "hash must change after external toml modification"
    );

    assert_eq!(
        repo.config_manager().get_user_name(),
        Some("External".to_string())
    );
    assert_eq!(
        repo.config_manager().get_user_email(),
        Some("ext@x.com".to_string())
    );

    let lock = lock_json(&dir);
    let ignores = lock["compiled_ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1);
    assert_eq!(ignores[0]["pattern"].as_str().unwrap(), "*.tmp");

    cleanup(&dir);
}

#[test]
fn test_toml_rewrites_device_id() {
    let dir = temp_dir("toml_device_id");
    let mgr = FsRepoManager::new();
    let mut repo = mgr.init(dir.clone()).expect("init");

    let dev1 = lock_json(&dir)["device_id"].as_str().unwrap().to_string();

    let new_toml = r#"
[user]
name = "Changed"
"#;
    fs::write(toml_path(&dir), new_toml).unwrap();
    repo.config_manager().refresh().expect("refresh");

    let dev2 = lock_json(&dir)["device_id"].as_str().unwrap().to_string();
    assert_eq!(dev1, dev2, "rewriting toml must NOT regenerate device_id");

    cleanup(&dir);
}

#[test]
fn test_ignore_patterns_filter_correctly() {
    let dir = temp_dir("ignore_filter");
    let mgr = FsRepoManager::new();
    let mut repo = mgr.init(dir.clone()).expect("init");

    let toml_with_ignores = r#"
[[ignore]]
pattern = "*.log"
recursive = false

[[ignore]]
pattern = "target"
recursive = true
"#;
    fs::write(toml_path(&dir), toml_with_ignores).unwrap();
    repo.config_manager().refresh().expect("refresh");

    let cm = repo.config_manager();
    assert!(cm.is_ignored("app.log"));
    assert!(cm.is_ignored("deep/nested/error.log"));
    assert!(cm.is_ignored("target"));
    assert!(cm.is_ignored("target/debug/build"));
    assert!(cm.is_ignored("src/target"));
    assert!(!cm.is_ignored("main.rs"));
    assert!(!cm.is_ignored("logs/error.txt"));
    assert!(!cm.is_ignored("target_backup"));

    let lock = lock_json(&dir);
    let arr = lock["compiled_ignores"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["pattern"].as_str().unwrap(), "*.log");
    assert!(!arr[0]["recursive"].as_bool().unwrap());
    assert_eq!(arr[1]["pattern"].as_str().unwrap(), "target");
    assert!(arr[1]["recursive"].as_bool().unwrap());

    cleanup(&dir);
}

#[test]
fn test_lock_file_only_toml_hash_changed_triggers_recompile() {
    let dir = temp_dir("recompile");
    let mgr = FsRepoManager::new();
    let mut repo = mgr.init(dir.clone()).expect("init");

    fs::write(
        toml_path(&dir),
        r#"
[[ignore]]
pattern = "*.bak"
recursive = false
"#,
    )
    .unwrap();
    repo.config_manager().refresh().expect("refresh");

    let lock = lock_json(&dir);
    assert_eq!(
        lock["compiled_ignores"].as_array().unwrap().len(),
        1,
        "one ignore rule should be compiled"
    );
    assert_eq!(
        lock["compiled_ignores"][0]["pattern"].as_str().unwrap(),
        "*.bak"
    );

    cleanup(&dir);
}

#[test]
fn test_voeconfig_toml_appears_in_working_tree() {
    let dir = temp_dir("add_skips");
    let mgr = FsRepoManager::new();
    let _repo = mgr.init(dir.clone()).expect("init");

    let mut toml_file = fs::File::create(toml_path(&dir)).unwrap();
    writeln!(toml_file, "[user]\nname = \"Visible\"\n").unwrap();
    drop(toml_file);

    let mut f = fs::File::create(dir.join("main.rs")).unwrap();
    writeln!(f, "fn main() {{}}").unwrap();
    drop(f);

    let working = voe_fs::read_working_tree(&dir).expect("read tree");
    let paths: Vec<String> = working
        .keys()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    assert!(
        paths.iter().any(|p| p.ends_with("voeconfig.toml")),
        "voeconfig.toml at repo root IS visible to working tree (the voe add logic must hard-skip it)"
    );
    assert!(
        paths.iter().any(|p| p.ends_with("main.rs")),
        "main.rs must be visible"
    );
    assert!(
        !paths.iter().any(|p| p.contains(".voe")),
        ".voe/ files (including voeconfig.lock) must never appear in working tree — they are repo metadata"
    );

    cleanup(&dir);
}

#[test]
fn test_set_user_writes_toml_not_lock() {
    let dir = temp_dir("set_user_toml");
    let mgr = FsRepoManager::new();
    let mut repo = mgr.init(dir.clone()).expect("init");

    repo.config_manager_mut()
        .set_user_name("Writer".to_string())
        .expect("set");

    let raw = fs::read_to_string(toml_path(&dir)).unwrap();
    assert!(raw.contains("Writer"));
    assert!(raw.contains("name"));

    let lock_raw = fs::read_to_string(dir.join(".voe").join("voeconfig.lock")).unwrap();
    let lock_val: Value = serde_json::from_str(&lock_raw).unwrap();
    assert_eq!(
        lock_val["toml_hash"].as_str().unwrap().len(),
        64,
        "hash was updated after write"
    );

    cleanup(&dir);
}

#[test]
fn test_missing_lock_rebuilt_on_open() {
    let dir = temp_dir("missing_lock");
    let mgr = FsRepoManager::new();
    mgr.init(dir.clone()).expect("init");

    fs::remove_file(dir.join(".voe").join("voeconfig.lock")).unwrap();
    assert!(!dir.join(".voe").join("voeconfig.lock").exists());

    let repo = mgr
        .open(dir.clone())
        .expect("open should still work even without lock");
    let _ = repo.config_manager();

    repo.config_manager().refresh().expect("refresh");
    assert!(
        dir.join(".voe").join("voeconfig.lock").exists(),
        "lock must be auto-rebuilt after refresh"
    );

    let lock = lock_json(&dir);
    assert_eq!(lock["device_id"].as_str().unwrap().len(), 36);
    assert_eq!(lock["toml_hash"].as_str().unwrap().len(), 64);

    cleanup(&dir);
}

// ── Integration tests for empty-pattern filtering ──────────────────────────
// These drive the full pipeline: TOML bytes on disk → FsConfigManager.refresh()
// → deserialize_ignore → compile_ignores → lock file written → is_ignored().
// They verify that rules with empty `pattern` (which match nothing) are
// filtered out at the deserialization layer, so they never reach the
// compiled_ignores list or the is_ignored() matcher.

/// Bare `[ignore]` with no keys at all (pure section header). This is the
/// exact TOML shape that produced the user-reported parse crash before the
/// custom deserializer was added. It must now parse cleanly AND produce no
/// compiled ignore rules — an empty section is clearly not an intentional
/// ignore entry.
#[test]
fn test_bare_ignore_empty_section_produces_no_rules() {
    let dir = temp_dir("bare_ignore_empty");
    let mgr = FsRepoManager::new();
    let mut repo = mgr.init(dir.clone()).expect("init");

    fs::write(toml_path(&dir), "[ignore]\n").unwrap();
    repo.config_manager()
        .refresh()
        .expect("refresh must succeed");

    let lock = lock_json(&dir);
    let ignores = lock["compiled_ignores"].as_array().unwrap();
    assert!(
        ignores.is_empty(),
        "bare [ignore] section header (no pattern) must yield zero compiled rules"
    );

    // is_ignored must return false for any path when there are no rules.
    let cm = repo.config_manager();
    assert!(!cm.is_ignored("anything.rs"));
    assert!(!cm.is_ignored("deep/nested/path.txt"));
    assert!(!cm.is_ignored("target"));

    cleanup(&dir);
}

/// Bare `[ignore]` with `recursive = false` but NO `pattern` key. The Single
/// branch filter in deserialize_ignore collapses this to an empty Vec, since
/// a rule with no pattern matches nothing regardless of the recursive flag.
#[test]
fn test_bare_ignore_with_recursive_only_is_filtered() {
    let dir = temp_dir("bare_ignore_recursive_only");
    let mgr = FsRepoManager::new();
    let mut repo = mgr.init(dir.clone()).expect("init");

    fs::write(
        toml_path(&dir),
        r#"
[ignore]
recursive = false
"#,
    )
    .unwrap();
    repo.config_manager()
        .refresh()
        .expect("refresh must succeed");

    let lock = lock_json(&dir);
    let ignores = lock["compiled_ignores"].as_array().unwrap();
    assert!(
        ignores.is_empty(),
        "bare [ignore] with recursive but no pattern must be filtered out"
    );

    let cm = repo.config_manager();
    assert!(!cm.is_ignored("target"));
    assert!(!cm.is_ignored("build"));

    cleanup(&dir);
}

/// `[[ignore]]` array with a mix of valid rules and entries that have an
/// empty pattern string. Both branches of the deserializer filter empty
/// patterns, so only the two valid rules should end up in compiled_ignores.
#[test]
fn test_array_ignore_mixed_empty_and_valid_patterns() {
    let dir = temp_dir("mixed_patterns");
    let mgr = FsRepoManager::new();
    let mut repo = mgr.init(dir.clone()).expect("init");

    fs::write(
        toml_path(&dir),
        r#"
[[ignore]]
pattern = ""
recursive = true

[[ignore]]
pattern = "*.log"
recursive = false

[[ignore]]
pattern = ""

[[ignore]]
pattern = "target"
recursive = true
"#,
    )
    .unwrap();
    repo.config_manager()
        .refresh()
        .expect("refresh must succeed");

    let lock = lock_json(&dir);
    let ignores = lock["compiled_ignores"].as_array().unwrap();
    assert_eq!(
        ignores.len(),
        2,
        "only the two non-empty-pattern rules should survive filtering"
    );
    assert_eq!(ignores[0]["pattern"].as_str().unwrap(), "*.log");
    assert!(!ignores[0]["recursive"].as_bool().unwrap());
    assert_eq!(ignores[1]["pattern"].as_str().unwrap(), "target");
    assert!(ignores[1]["recursive"].as_bool().unwrap());

    // is_ignored should respect only the valid rules.
    let cm = repo.config_manager();
    assert!(cm.is_ignored("app.log"));
    assert!(cm.is_ignored("target/output"));
    assert!(!cm.is_ignored("main.rs"));
    assert!(!cm.is_ignored("empty_is_ignored_test"));

    cleanup(&dir);
}

/// `[[ignore]]` where every entry has an empty pattern. After filtering, the
/// compiled_ignores array must be empty and is_ignored must always return
/// false — a config full of no-op rules behaves as if there were no rules at
/// all.
#[test]
fn test_array_ignore_all_empty_patterns_yields_empty_list() {
    let dir = temp_dir("all_empty_patterns");
    let mgr = FsRepoManager::new();
    let mut repo = mgr.init(dir.clone()).expect("init");

    fs::write(
        toml_path(&dir),
        r#"
[[ignore]]
pattern = ""
recursive = true

[[ignore]]
pattern = ""
"#,
    )
    .unwrap();
    repo.config_manager()
        .refresh()
        .expect("refresh must succeed");

    let lock = lock_json(&dir);
    let ignores = lock["compiled_ignores"].as_array().unwrap();
    assert!(
        ignores.is_empty(),
        "all-empty-pattern [[ignore]] must produce zero compiled rules"
    );

    let cm = repo.config_manager();
    assert!(!cm.is_ignored("any.file"));
    assert!(!cm.is_ignored("nested/dir/structure"));
    assert!(!cm.is_ignored("target"));

    cleanup(&dir);
}

/// A regression guard: a normal `[[ignore]]` with a valid pattern must still
/// compile correctly after the empty-pattern filter was added. This ensures
/// the filter doesn't accidentally strip legitimate rules.
#[test]
fn test_array_ignore_valid_patterns_unaffected_by_filter() {
    let dir = temp_dir("valid_patterns_ok");
    let mgr = FsRepoManager::new();
    let mut repo = mgr.init(dir.clone()).expect("init");

    fs::write(
        toml_path(&dir),
        r#"
[[ignore]]
pattern = "node_modules"
recursive = true

[[ignore]]
pattern = "*.bak"
recursive = false
"#,
    )
    .unwrap();
    repo.config_manager()
        .refresh()
        .expect("refresh must succeed");

    let lock = lock_json(&dir);
    let ignores = lock["compiled_ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 2);
    assert_eq!(ignores[0]["pattern"].as_str().unwrap(), "node_modules");
    assert_eq!(ignores[1]["pattern"].as_str().unwrap(), "*.bak");

    let cm = repo.config_manager();
    assert!(cm.is_ignored("node_modules/foo.js"));
    assert!(cm.is_ignored("backup.bak"));
    assert!(!cm.is_ignored("src/main.rs"));

    cleanup(&dir);
}
