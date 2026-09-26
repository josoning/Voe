use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

use voe_types::error::{Result, VoeError};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserSection {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoreSection {
    #[serde(default)]
    pub repositoryformatversion: u32,
    #[serde(default)]
    pub filemode: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IgnoreRule {
    pub pattern: String,
    pub recursive: bool,
}

impl Default for IgnoreRule {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            recursive: true,
        }
    }
}

/// Deserialize helper that accepts both `[[ignore]]` (array-of-tables) and a bare
/// `[ignore]` section (single table). TOML users often write just `[ignore]` as a
/// section header, which serde interprets as a map rather than a sequence — this
/// helper normalises both forms into `Vec<IgnoreRule>`.
fn deserialize_ignore<'de, D>(deserializer: D) -> std::result::Result<Vec<IgnoreRule>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IgnoreVariant {
        Array(Vec<IgnoreRule>),
        Single(IgnoreRule),
    }

    match IgnoreVariant::deserialize(deserializer)? {
        // Filter out entries with empty `pattern` in both branches — a rule with
        // no pattern matches nothing, so it's almost certainly an accidental
        // omission (placeholder line, empty section header, etc.) rather than an
        // intentional configuration. Keeping the same filter logic for Array and
        // Single guarantees symmetric behaviour regardless of which TOML form
        // the user picks.
        IgnoreVariant::Array(arr) => {
            Ok(arr.into_iter().filter(|r| !r.pattern.is_empty()).collect())
        }
        IgnoreVariant::Single(rule) => {
            if rule.pattern.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![rule])
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoeConfig {
    #[serde(default)]
    pub user: UserSection,
    #[serde(default)]
    pub core: CoreSection,
    #[serde(default, deserialize_with = "deserialize_ignore")]
    pub ignore: Vec<IgnoreRule>,
}

impl VoeConfig {
    pub fn user_name(&self) -> Option<&str> {
        self.user.name.as_deref()
    }

    pub fn user_email(&self) -> Option<&str> {
        self.user.email.as_deref()
    }

    pub fn set_user_name(&mut self, name: impl Into<String>) {
        self.user.name = Some(name.into());
    }

    pub fn set_user_email(&mut self, email: impl Into<String>) {
        self.user.email = Some(email.into());
    }

    pub fn to_toml_string(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| VoeError::Config(e.to_string()))
    }

    pub fn from_toml_str(input: &str) -> Result<Self> {
        toml::from_str(input).map_err(|e| VoeError::Config(e.to_string()))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LockState {
    #[serde(default)]
    pub toml_hash: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub compiled_ignores: Vec<CompiledIgnore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledIgnore {
    pub pattern: String,
    #[serde(default)]
    pub recursive: bool,
}

impl Default for CompiledIgnore {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            recursive: true,
        }
    }
}

pub trait ConfigSource: Send + Sync {
    fn read_config(&self) -> Result<Option<VoeConfig>>;
    fn write_config(&self, config: &VoeConfig) -> Result<()>;
    fn read_raw(&self) -> Result<Option<String>>;
    fn path(&self) -> &PathBuf;
}

pub trait ConfigManager: Send + Sync {
    fn refresh(&self) -> Result<()>;

    fn config(&self) -> VoeConfig;

    fn get_user_name(&self) -> Option<String> {
        self.config().user.name.clone()
    }

    fn get_user_email(&self) -> Option<String> {
        self.config().user.email.clone()
    }

    fn set_user_name(&mut self, name: String) -> Result<()>;
    fn set_user_email(&mut self, email: String) -> Result<()>;

    fn is_ignored(&self, rel_path: &str) -> bool;

    fn lock_state(&self) -> LockState;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that a TOML file containing only a bare `[ignore]` section header
    /// (no `pattern = ...` key) parses cleanly — this is the exact scenario that
    /// was previously producing "invalid type: map, expected a sequence".
    #[test]
    fn test_deserialize_bare_ignore_section() {
        let toml_str = "[ignore]\n";
        let config: VoeConfig = toml::from_str(toml_str).expect("bare [ignore] must parse");
        assert!(
            config.ignore.is_empty(),
            "empty [ignore] should yield zero rules"
        );
    }

    /// A bare `[ignore]` with an actual `pattern` value should produce one rule.
    #[test]
    fn test_deserialize_bare_ignore_with_pattern() {
        let toml_str = r#"
[ignore]
pattern = "target"
"#;
        let config: VoeConfig =
            toml::from_str(toml_str).expect("bare [ignore] with pattern must parse");
        assert_eq!(config.ignore.len(), 1);
        assert_eq!(config.ignore[0].pattern, "target");
        assert!(
            config.ignore[0].recursive,
            "recursive should default to true"
        );
    }

    /// The canonical `[[ignore]]` array-of-tables form must still work as before.
    #[test]
    fn test_deserialize_array_of_tables_ignore() {
        let toml_str = r#"
[[ignore]]
pattern = "*.log"
recursive = true

[[ignore]]
pattern = "target"
recursive = false
"#;
        let config: VoeConfig = toml::from_str(toml_str).expect("[[ignore]] must parse");
        assert_eq!(config.ignore.len(), 2);
        assert_eq!(config.ignore[0].pattern, "*.log");
        assert!(config.ignore[0].recursive);
        assert_eq!(config.ignore[1].pattern, "target");
        assert!(!config.ignore[1].recursive);
    }

    /// A completely empty TOML document should also produce a valid (default) config.
    #[test]
    fn test_deserialize_empty_config() {
        let config: VoeConfig = toml::from_str("").expect("empty TOML must parse");
        assert!(config.ignore.is_empty());
        assert!(config.user.name.is_none());
        assert!(config.core.filemode.is_none());
    }

    /// A full config mixing `[user]`, `[core]`, and bare `[ignore]` must parse.
    #[test]
    fn test_deserialize_full_config_with_bare_ignore() {
        let toml_str = r#"
[user]
name = "Alice"
email = "alice@example.com"

[core]
filemode = true

[ignore]
pattern = "build"
"#;
        let config: VoeConfig = toml::from_str(toml_str).expect("full config must parse");
        assert_eq!(config.user.name.as_deref(), Some("Alice"));
        assert_eq!(config.user.email.as_deref(), Some("alice@example.com"));
        assert_eq!(config.core.filemode, Some(true));
        assert_eq!(config.ignore.len(), 1);
        assert_eq!(config.ignore[0].pattern, "build");
    }

    // ── Tests via the production API (VoeConfig::from_toml_str) ────────────
    // These exercises the real error-mapping path that the CLI hits, wrapping
    // TOML errors as VoeError::Config rather than leaking raw toml::de::Error.

    /// The production entry point VoeConfig::from_toml_str must accept a bare
    /// `[ignore]` section and return a clean Ok — this is the exact code path
    /// that produced the user-reported panic.
    #[test]
    fn test_from_toml_str_bare_ignore_via_api() {
        let result = VoeConfig::from_toml_str("[ignore]\n");
        assert!(
            result.is_ok(),
            "bare [ignore] via from_toml_str should succeed"
        );
        let config = result.unwrap();
        assert!(config.ignore.is_empty());
    }

    /// `from_toml_str` must also accept the canonical `[[ignore]]` form.
    #[test]
    fn test_from_toml_str_array_ignore_via_api() {
        let toml_str = r#"
[[ignore]]
pattern = "*.log"
recursive = true
"#;
        let config = VoeConfig::from_toml_str(toml_str).expect("[[ignore]] must parse");
        assert_eq!(config.ignore.len(), 1);
        assert_eq!(config.ignore[0].pattern, "*.log");
    }

    /// A malformed TOML input must surface as `Err(VoeError::Config)` — never as
    /// a panic or a raw toml::de::Error.
    #[test]
    fn test_from_toml_str_invalid_toml_wraps_as_config_error() {
        let result = VoeConfig::from_toml_str("this is not valid toml [[[");
        assert!(result.is_err());
        let err_str = format!("{}", result.unwrap_err());
        assert!(
            err_str.contains("TOML") || err_str.contains("toml"),
            "error should mention TOML parsing, got: {}",
            err_str
        );
    }

    // ── Edge case: bare [ignore] with recursive only (pattern missing) ─────
    // The Single-branch filter removes rules whose `pattern` is empty, so a
    // bare section that sets `recursive = false` but omits `pattern` should
    // still collapse to an empty ignore list.

    #[test]
    fn test_bare_ignore_with_recursive_only_filtered_out() {
        let toml_str = r#"
[ignore]
recursive = false
"#;
        let config = VoeConfig::from_toml_str(toml_str).expect("must parse");
        assert!(
            config.ignore.is_empty(),
            "bare [ignore] with no pattern should be filtered out even \
                 when recursive is set"
        );
    }

    // ── Edge case: [[ignore]] entry with empty pattern ─────────────────────
    // Empty-pattern rules are filtered out in both the Array and Single branches
    // — a rule that matches nothing is almost certainly an accidental omission,
    // so the deserializer silently drops it rather than keeping a no-op rule.

    #[test]
    fn test_array_ignore_with_empty_pattern_filtered_out() {
        let toml_str = r#"
[[ignore]]
pattern = ""
recursive = true
"#;
        let config = VoeConfig::from_toml_str(toml_str).expect("must parse");
        assert!(
            config.ignore.is_empty(),
            "[[ignore]] with empty pattern should be filtered out, \
                 since a rule with no pattern matches nothing"
        );
    }

    // ── Config with no [ignore] section at all ──────────────────────────────
    // Verifies that `#[serde(default)]` on the VoeConfig::ignore field works
    // correctly — users who never write [ignore] should still get a valid
    // config with an empty ignore Vec.

    #[test]
    fn test_config_without_ignore_section_defaults_to_empty_vec() {
        let toml_str = r#"
[user]
name = "Bob"

[core]
repositoryformatversion = 1
"#;
        let config = VoeConfig::from_toml_str(toml_str).expect("must parse");
        assert!(
            config.ignore.is_empty(),
            "missing [ignore] section should yield an empty Vec"
        );
        assert_eq!(config.core.repositoryformatversion, 1);
        assert_eq!(config.user.name.as_deref(), Some("Bob"));
    }

    // ── IgnoreRule::Default sanity checks ────────────────────────────────────

    #[test]
    fn test_ignore_rule_default_values() {
        let rule = IgnoreRule::default();
        assert!(rule.pattern.is_empty());
        assert!(rule.recursive, "recursive must default to true");
    }

    // ── Round-trip: VoeConfig → TOML string → VoeConfig ────────────────────
    // Confirms that the serialized form of a config with ignore rules can be
    // parsed back into an equivalent VoeConfig. This protects against cases
    // where the serializer emits a form the deserializer cannot handle.

    #[test]
    fn test_ignore_rules_toml_roundtrip() {
        let original = VoeConfig {
            user: UserSection {
                name: Some("Carol".to_string()),
                email: Some("carol@example.com".to_string()),
            },
            core: CoreSection {
                repositoryformatversion: 1,
                filemode: Some(true),
            },
            ignore: vec![
                IgnoreRule {
                    pattern: "*.log".to_string(),
                    recursive: true,
                },
                IgnoreRule {
                    pattern: "target".to_string(),
                    recursive: false,
                },
            ],
        };

        let toml_str = original.to_toml_string().expect("serialize");
        let restored = VoeConfig::from_toml_str(&toml_str).expect("deserialize");

        assert_eq!(original.user.name, restored.user.name);
        assert_eq!(original.user.email, restored.user.email);
        assert_eq!(original.core.filemode, restored.core.filemode);
        assert_eq!(original.ignore.len(), restored.ignore.len());
        for (orig, rest) in original.ignore.iter().zip(restored.ignore.iter()) {
            assert_eq!(orig.pattern, rest.pattern);
            assert_eq!(orig.recursive, rest.recursive);
        }
    }

    // ── Multiple [[ignore]] entries + mixed other sections ──────────────────

    #[test]
    fn test_multiple_ignore_entries_mixed_with_other_sections() {
        let toml_str = r#"
[user]
name = "Dave"

[[ignore]]
pattern = "node_modules"
recursive = true

[[ignore]]
pattern = "*.tmp"
recursive = false

[core]
filemode = false

[[ignore]]
pattern = ".cache"
recursive = true
"#;
        let config = VoeConfig::from_toml_str(toml_str).expect("must parse");
        assert_eq!(config.user.name.as_deref(), Some("Dave"));
        assert_eq!(config.core.filemode, Some(false));
        assert_eq!(config.ignore.len(), 3);
        assert_eq!(config.ignore[0].pattern, "node_modules");
        assert!(config.ignore[0].recursive);
        assert_eq!(config.ignore[1].pattern, "*.tmp");
        assert!(!config.ignore[1].recursive);
        assert_eq!(config.ignore[2].pattern, ".cache");
        assert!(config.ignore[2].recursive);
    }
}
