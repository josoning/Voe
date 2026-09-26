use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use regex::Regex;

use voe_repo_api::config::{CompiledIgnore, ConfigManager, ConfigSource, LockState, VoeConfig};
use voe_storage_api::StorageBackend;
use voe_types::error::{Result, VoeError};

use crate::backend::LocalFileBackend;
use crate::utils::{regex_like_pattern, sha256_hex};

pub struct TomlConfigSource {
    toml_path: PathBuf,
    backend: Arc<dyn StorageBackend>,
}

impl TomlConfigSource {
    pub fn new(toml_path: PathBuf) -> Self {
        Self::with_backend(toml_path, Arc::new(LocalFileBackend::new()))
    }

    pub fn with_backend(toml_path: PathBuf, backend: Arc<dyn StorageBackend>) -> Self {
        Self { toml_path, backend }
    }

    pub fn toml_path(&self) -> &PathBuf {
        &self.toml_path
    }
}

impl ConfigSource for TomlConfigSource {
    fn read_config(&self) -> Result<Option<VoeConfig>> {
        match self.backend.read_file_to_string(&self.toml_path) {
            Ok(content) => {
                let cfg = VoeConfig::from_toml_str(&content)?;
                Ok(Some(cfg))
            }
            Err(VoeError::FileNotFound { .. }) => Ok(None),
            Err(e) => Err(VoeError::Config(format!(
                "Failed to read {}: {}",
                self.toml_path.display(),
                e
            ))),
        }
    }

    fn write_config(&self, config: &VoeConfig) -> Result<()> {
        let content = config.to_toml_string()?;
        self.backend.ensure_parent_dir(&self.toml_path)?;
        self.backend
            .write_file(&self.toml_path, content.as_bytes())
            .map_err(|e| {
                VoeError::Config(format!(
                    "Failed to write {}: {}",
                    self.toml_path.display(),
                    e
                ))
            })
    }

    fn read_raw(&self) -> Result<Option<String>> {
        match self.backend.read_file_to_string(&self.toml_path) {
            Ok(c) => Ok(Some(c)),
            Err(VoeError::FileNotFound { .. }) => Ok(None),
            Err(e) => Err(VoeError::Config(format!("Failed to read toml: {}", e))),
        }
    }

    fn path(&self) -> &PathBuf {
        &self.toml_path
    }
}

pub struct FsConfigManager<S: ConfigSource> {
    source: S,
    lock_path: PathBuf,
    backend: Arc<dyn StorageBackend>,
    inner: Mutex<Inner>,
}

struct Inner {
    cached_config: VoeConfig,
    cached_lock: LockState,
}

impl<S: ConfigSource> FsConfigManager<S> {
    pub fn new(source: S, lock_path: PathBuf) -> Self {
        Self::with_backend(source, lock_path, Arc::new(LocalFileBackend::new()))
    }

    pub fn with_backend(source: S, lock_path: PathBuf, backend: Arc<dyn StorageBackend>) -> Self {
        Self {
            source,
            lock_path,
            backend,
            inner: Mutex::new(Inner {
                cached_config: VoeConfig::default(),
                cached_lock: LockState::default(),
            }),
        }
    }

    pub fn lock_path(&self) -> &PathBuf {
        &self.lock_path
    }

    fn load_lock(&self) -> Result<LockState> {
        match self.backend.read_file_to_string(&self.lock_path) {
            Ok(content) => serde_json::from_str(&content)
                .map_err(|e| VoeError::Config(format!("Failed to parse lock: {}", e))),
            Err(VoeError::FileNotFound { .. }) => Ok(LockState::default()),
            Err(e) => Err(VoeError::Config(format!(
                "Failed to read lock {}: {}",
                self.lock_path.display(),
                e
            ))),
        }
    }

    fn save_lock(&self, state: &LockState) -> Result<()> {
        self.backend.ensure_parent_dir(&self.lock_path)?;
        let json = serde_json::to_string_pretty(state)
            .map_err(|e| VoeError::Config(format!("Failed to serialize lock: {}", e)))?;
        self.backend
            .write_file(&self.lock_path, json.as_bytes())
            .map_err(|e| {
                VoeError::Config(format!(
                    "Failed to write lock {}: {}",
                    self.lock_path.display(),
                    e
                ))
            })
    }

    fn compile_ignores(config: &VoeConfig) -> Vec<CompiledIgnore> {
        config
            .ignore
            .iter()
            .map(|r| CompiledIgnore {
                pattern: r.pattern.clone(),
                recursive: r.recursive,
            })
            .collect()
    }

    fn ensure_device_id(lock: &mut LockState) {
        if lock.device_id.is_empty() {
            lock.device_id = uuid::Uuid::new_v4().to_string();
        }
    }

    pub(crate) fn refresh_internal(&self) -> Result<()> {
        let raw = self.source.read_raw()?;
        let raw = raw.unwrap_or_default();
        let new_hash = sha256_hex(&raw);

        let mut lock = self.load_lock().unwrap_or_default();
        Self::ensure_device_id(&mut lock);

        let config = if raw.is_empty() {
            VoeConfig::default()
        } else {
            VoeConfig::from_toml_str(&raw)?
        };

        if lock.toml_hash.is_empty() || lock.toml_hash != new_hash {
            lock.toml_hash = new_hash;
            lock.compiled_ignores = Self::compile_ignores(&config);
            self.save_lock(&lock)?;
        }

        let mut inner = self
            .inner
            .lock()
            .map_err(|_| VoeError::Config("Config manager mutex poisoned".to_string()))?;
        inner.cached_config = config;
        inner.cached_lock = lock;
        Ok(())
    }

    pub fn init_defaults(&self) -> Result<()> {
        let existing = self.source.read_config()?;
        if existing.is_none() {
            let mut cfg = VoeConfig::default();
            cfg.core.repositoryformatversion = 0;
            self.source.write_config(&cfg)?;
        }
        self.refresh_internal()
    }
}

impl<S: ConfigSource> ConfigManager for FsConfigManager<S> {
    fn refresh(&self) -> Result<()> {
        self.refresh_internal()
    }

    fn config(&self) -> VoeConfig {
        // Recover from mutex poisoning instead of panicking — a poisoned mutex
        // merely means some prior thread panicked while holding it; the cached
        // config is still valid.
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.cached_config.clone()
    }

    fn set_user_name(&mut self, name: String) -> Result<()> {
        {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| VoeError::Config("config mutex poisoned".to_string()))?;
            inner.cached_config.set_user_name(name.clone());
            self.source.write_config(&inner.cached_config)?;
        }
        self.refresh_internal()
    }

    fn set_user_email(&mut self, email: String) -> Result<()> {
        {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| VoeError::Config("config mutex poisoned".to_string()))?;
            inner.cached_config.set_user_email(email.clone());
            self.source.write_config(&inner.cached_config)?;
        }
        self.refresh_internal()
    }

    fn is_ignored(&self, rel_path: &str) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let segments: Vec<&str> = rel_path.split('/').collect();
        for ci in &inner.cached_lock.compiled_ignores {
            let esc = regex_like_pattern(&ci.pattern);
            let re = match Regex::new(&format!("^{}$", esc)) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if segments.iter().any(|s| re.is_match(s)) {
                return true;
            }
        }
        false
    }

    fn lock_state(&self) -> LockState {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.cached_lock.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;
    use voe_repo_api::config::{IgnoreRule, VoeConfig};

    fn tempdir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nonce = format!("voe_cfg_test_{}_{}", name, std::process::id());
        p.push(nonce);
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    #[test]
    fn test_toml_roundtrip() {
        let dir = tempdir("toml_roundtrip");
        let toml_path = dir.join("voeconfig.toml");
        let lock_path = dir.join("voeconfig.lock");

        let source = TomlConfigSource::new(toml_path.clone());
        let mut cfg = VoeConfig::default();
        cfg.set_user_name("Alice".to_string());
        cfg.set_user_email("alice@example.com".to_string());
        cfg.ignore.push(IgnoreRule {
            pattern: "*.log".to_string(),
            recursive: false,
        });
        source.write_config(&cfg).unwrap();

        let mgr = FsConfigManager::new(source, lock_path.clone());
        mgr.refresh().unwrap();

        let loaded = mgr.config();
        assert_eq!(loaded.user.name, Some("Alice".to_string()));
        assert_eq!(loaded.user.email, Some("alice@example.com".to_string()));
        assert_eq!(loaded.ignore.len(), 1);

        let lock = mgr.lock_state();
        assert!(!lock.device_id.is_empty(), "device_id should be generated");
        assert!(!lock.toml_hash.is_empty(), "toml_hash should be set");
        assert!(!lock.compiled_ignores.is_empty());

        let saved_lock = std::fs::read_to_string(&lock_path).unwrap();
        assert!(saved_lock.contains("device_id"));
        assert!(saved_lock.contains("toml_hash"));
        cleanup(&dir);
    }

    #[test]
    fn test_refresh_detects_toml_change() {
        let dir = tempdir("refresh_change");
        let toml_path = dir.join("voeconfig.toml");
        let lock_path = dir.join("voeconfig.lock");

        {
            let mut f = std::fs::File::create(&toml_path).unwrap();
            f.write_all(b"[user]\nname = \"Old\"\nemail = \"old@x.com\"\n")
                .unwrap();
        }

        let source = TomlConfigSource::new(toml_path.clone());
        let mgr = FsConfigManager::new(source, lock_path.clone());
        mgr.refresh().unwrap();
        assert_eq!(mgr.config().user.name, Some("Old".to_string()));

        let old_hash = mgr.lock_state().toml_hash.clone();

        {
            let mut f = std::fs::File::create(&toml_path).unwrap();
            f.write_all(b"[user]\nname = \"New\"\nemail = \"new@x.com\"\n")
                .unwrap();
        }

        mgr.refresh().unwrap();
        assert_eq!(mgr.config().user.name, Some("New".to_string()));
        let new_hash = mgr.lock_state().toml_hash.clone();
        assert_ne!(old_hash, new_hash, "hash should change after toml update");
        cleanup(&dir);
    }

    #[test]
    fn test_device_id_preserved_across_refresh() {
        let dir = tempdir("device_id");
        let toml_path = dir.join("voeconfig.toml");
        let lock_path = dir.join("voeconfig.lock");

        std::fs::write(&toml_path, "[user]\nname = \"A\"\n").unwrap();

        let source = TomlConfigSource::new(toml_path.clone());
        let mgr = FsConfigManager::new(source, lock_path.clone());
        mgr.refresh().unwrap();
        let dev1 = mgr.lock_state().device_id.clone();
        assert!(!dev1.is_empty());

        mgr.refresh().unwrap();
        let dev2 = mgr.lock_state().device_id.clone();
        assert_eq!(dev1, dev2, "device_id must be stable across refreshes");
        cleanup(&dir);
    }

    #[test]
    fn test_compile_ignore_recursive() {
        let dir = tempdir("compile_ignore");
        let toml_path = dir.join("voeconfig.toml");
        let lock_path = dir.join("voeconfig.lock");

        let mut cfg = VoeConfig::default();
        cfg.ignore.push(IgnoreRule {
            pattern: "target".to_string(),
            recursive: true,
        });
        cfg.ignore.push(IgnoreRule {
            pattern: "*.tmp".to_string(),
            recursive: false,
        });
        let source = TomlConfigSource::new(toml_path.clone());
        source.write_config(&cfg).unwrap();

        let mgr = FsConfigManager::new(source, lock_path.clone());
        mgr.refresh().unwrap();

        let lock = mgr.lock_state();
        assert_eq!(lock.compiled_ignores.len(), 2);
        assert_eq!(lock.compiled_ignores[0].pattern, "target");
        assert!(lock.compiled_ignores[0].recursive);
        assert_eq!(lock.compiled_ignores[1].pattern, "*.tmp");
        assert!(!lock.compiled_ignores[1].recursive);
        cleanup(&dir);
    }

    #[test]
    fn test_is_ignored_matches_compiled() {
        let dir = tempdir("is_ignored");
        let toml_path = dir.join("voeconfig.toml");
        let lock_path = dir.join("voeconfig.lock");

        let mut cfg = VoeConfig::default();
        cfg.ignore.push(IgnoreRule {
            pattern: "*.log".to_string(),
            recursive: false,
        });
        cfg.ignore.push(IgnoreRule {
            pattern: "node_modules".to_string(),
            recursive: true,
        });
        let source = TomlConfigSource::new(toml_path.clone());
        source.write_config(&cfg).unwrap();

        let mgr = FsConfigManager::new(source, lock_path.clone());
        mgr.refresh().unwrap();

        assert!(mgr.is_ignored("app.log"));
        assert!(mgr.is_ignored("logs/error.log"));
        assert!(mgr.is_ignored("node_modules/foo"));
        assert!(mgr.is_ignored("src/node_modules/foo"));
        assert!(!mgr.is_ignored("main.rs"));
        assert!(!mgr.is_ignored("logs/error.txt"));
        cleanup(&dir);
    }

    #[test]
    fn test_missing_lock_is_rebuilt() {
        let dir = tempdir("no_lock");
        let toml_path = dir.join("voeconfig.toml");
        let lock_path = dir.join("voeconfig.lock");

        std::fs::write(&toml_path, "[user]\nname = \"X\"\n").unwrap();
        assert!(!lock_path.exists());

        let source = TomlConfigSource::new(toml_path.clone());
        let mgr = FsConfigManager::new(source, lock_path.clone());
        mgr.refresh().unwrap();

        assert!(lock_path.exists(), "lock file should be auto-created");
        assert!(!mgr.lock_state().device_id.is_empty());
        assert_eq!(mgr.config().user.name, Some("X".to_string()));
        cleanup(&dir);
    }

    #[test]
    fn test_set_user_updates_toml() {
        let dir = tempdir("set_user");
        let toml_path = dir.join("voeconfig.toml");
        let lock_path = dir.join("voeconfig.lock");

        let cfg = VoeConfig::default();
        let source = TomlConfigSource::new(toml_path.clone());
        source.write_config(&cfg).unwrap();

        let mut mgr = FsConfigManager::new(source, lock_path.clone());
        mgr.refresh().unwrap();

        mgr.set_user_name("Bob".to_string()).unwrap();
        mgr.set_user_email("bob@x.com".to_string()).unwrap();

        let raw = std::fs::read_to_string(&toml_path).unwrap();
        assert!(raw.contains("Bob"));
        assert!(raw.contains("bob@x.com"));

        assert_eq!(mgr.config().user.name, Some("Bob".to_string()));
        assert_eq!(mgr.config().user.email, Some("bob@x.com".to_string()));
        cleanup(&dir);
    }

    #[test]
    fn test_set_user_name_email_api() {
        let dir = tempdir("get_user_api");
        let toml_path = dir.join("voeconfig.toml");
        let lock_path = dir.join("voeconfig.lock");

        let mut cfg = VoeConfig::default();
        cfg.set_user_name("Carol".to_string());
        cfg.set_user_email("carol@x.com".to_string());
        let source = TomlConfigSource::new(toml_path.clone());
        source.write_config(&cfg).unwrap();

        let mgr = FsConfigManager::new(source, lock_path.clone());
        mgr.refresh().unwrap();

        assert_eq!(mgr.get_user_name(), Some("Carol".to_string()));
        assert_eq!(mgr.get_user_email(), Some("carol@x.com".to_string()));
        cleanup(&dir);
    }
}
