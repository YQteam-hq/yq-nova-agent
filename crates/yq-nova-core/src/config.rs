use std::{fs, path::PathBuf, time::Duration};

use figment::{
    Figment, Profile,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

use crate::{
    embedding::{BaichuanConfig, JinaConfig, QwenConfig, ZhipuConfig},
    error::{NovaError, NovaResult},
};

pub(crate) mod duration_seconds {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, ser: S) -> Result<S::Ok, S::Error> {
        d.as_secs().serialize(ser)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(de)?;
        Ok(Duration::from_secs(secs))
    }
}

pub const DEFAULT_CONFIG_FILENAME: &str = "yq-nova.toml";

fn join_keys<T>(map: &std::collections::BTreeMap<String, T>) -> String {
    map.keys().cloned().collect::<Vec<_>>().join(", ")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub embedding: EmbeddingConfig,
    pub forgetting: ForgettingConfig,
    pub graph: GraphConfig,
    pub jobs: JobsConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,

    pub concurrency: usize,

    #[serde(with = "duration_seconds")]
    pub request_timeout: Duration,

    pub max_request_body_bytes: usize,

    pub auth_token: String,

    pub namespace_keys: std::collections::BTreeMap<String, String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7999".into(),
            concurrency: 32,
            request_timeout: Duration::from_secs(30),
            max_request_body_bytes: 10 * 1024 * 1024,
            auth_token: String::new(),
            namespace_keys: std::collections::BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub db_path: PathBuf,

    pub wal_mode: bool,

    pub page_size: i64,

    pub cache_size_kb: i64,

    pub busy_timeout_ms: u32,

    pub pool_max_connections: u32,

    pub pool_min_connections: u32,

    pub mmap_size_kb: i64,

    pub soft_heap_limit_kb: i64,

    pub wal_autocheckpoint_kb: i64,

    pub journal_size_limit_kb: i64,

    pub synchronous: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("./yq-nova.db"),
            wal_mode: true,
            page_size: 4096,
            cache_size_kb: 131_072,
            busy_timeout_ms: 5_000,
            pool_max_connections: 16,
            pool_min_connections: 2,
            mmap_size_kb: 262_144,
            soft_heap_limit_kb: 524_288,
            wal_autocheckpoint_kb: 1_024,
            journal_size_limit_kb: 131_072,
            synchronous: "normal".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    pub default_provider: String,

    pub openai_compatible: std::collections::BTreeMap<String, OpenAiCompatConfig>,

    pub fastembed_local: std::collections::BTreeMap<String, FastEmbedConfig>,

    pub zhipu: std::collections::BTreeMap<String, ZhipuConfig>,

    pub qwen: std::collections::BTreeMap<String, QwenConfig>,

    pub baichuan: std::collections::BTreeMap<String, BaichuanConfig>,

    pub jina: std::collections::BTreeMap<String, JinaConfig>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        let mut oai = std::collections::BTreeMap::new();
        oai.insert("default".into(), OpenAiCompatConfig::default());
        Self {
            default_provider: "default".into(),
            openai_compatible: oai,
            fastembed_local: std::collections::BTreeMap::new(),
            zhipu: std::collections::BTreeMap::new(),
            qwen: std::collections::BTreeMap::new(),
            baichuan: std::collections::BTreeMap::new(),
            jina: std::collections::BTreeMap::new(),
        }
    }
}

impl EmbeddingConfig {
    /// Checks that `default_provider` names a provider that can actually be
    /// built: the special `mock` provider (case-insensitive) or a key present
    /// in one of the provider maps.
    ///
    /// Deliberately separate from [`Config::validate`]: commands that never
    /// build an embedding provider (`config-show`, `check`, `init-db`) must
    /// keep working on an otherwise-broken config so they stay useful as
    /// diagnostics. Call this wherever a provider is about to be built.
    pub fn validate_provider_references(&self) -> NovaResult<()> {
        let key = self.default_provider.trim();
        if key.is_empty() {
            return Err(NovaError::config_msg("embedding.default_provider must be set"));
        }
        if key.eq_ignore_ascii_case("mock") {
            return Ok(());
        }
        let known = self.openai_compatible.contains_key(key)
            || self.fastembed_local.contains_key(key)
            || self.zhipu.contains_key(key)
            || self.qwen.contains_key(key)
            || self.baichuan.contains_key(key)
            || self.jina.contains_key(key);
        if !known {
            return Err(NovaError::config_msg(format!(
                "embedding.default_provider='{key}' not found. Available keys: openai_compatible \
                 [{}], zhipu [{}], qwen [{}], baichuan [{}], jina [{}], fastembed_local [{}], \
                 special 'mock'",
                join_keys(&self.openai_compatible),
                join_keys(&self.zhipu),
                join_keys(&self.qwen),
                join_keys(&self.baichuan),
                join_keys(&self.jina),
                join_keys(&self.fastembed_local),
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiCompatConfig {
    pub base_url: String,

    pub api_key: String,

    pub model: String,

    pub dimensions: usize,

    pub batch_size: usize,

    #[serde(with = "duration_seconds")]
    pub timeout: Duration,

    pub max_retries: u32,
}

impl Default for OpenAiCompatConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".into(),
            api_key: String::new(),
            model: "text-embedding-3-small".into(),
            dimensions: 1536,
            batch_size: 16,
            timeout: Duration::from_secs(15),
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FastEmbedConfig {
    pub model_name: String,

    pub dimensions: usize,

    pub cache_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ForgettingConfig {
    pub enabled: bool,

    #[serde(with = "duration_seconds")]
    pub stale_after: Duration,

    pub stale_importance_threshold: f32,

    pub action: String,

    #[serde(with = "duration_seconds")]
    pub check_interval: Duration,
}

impl Default for ForgettingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            stale_after: Duration::from_secs(90 * 86_400),
            stale_importance_threshold: 0.3,
            action: "archive".into(),
            check_interval: Duration::from_secs(600),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GraphConfig {
    pub auto_extract: bool,

    pub extract_llm: String,

    pub extract_prompt_file: Option<PathBuf>,

    pub openai_compatible_chat: std::collections::BTreeMap<String, OpenAiChatConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiChatConfig {
    pub base_url: String,

    pub api_key: String,

    pub model: String,

    #[serde(with = "duration_seconds")]
    pub timeout: Duration,
}

impl Default for OpenAiChatConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".into(),
            api_key: String::new(),
            model: "gpt-4o-mini".into(),
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JobsConfig {
    #[serde(with = "duration_seconds")]
    pub ttl_interval: Duration,
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            ttl_interval: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,

    pub json_format: bool,

    pub file: Option<PathBuf>,

    pub ansi: bool,

    pub otel_enabled: bool,

    pub otel_endpoint: String,

    pub otel_service_name: String,

    pub otel_sample_rate: f32,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            json_format: false,
            file: None,
            ansi: true,
            otel_enabled: false,
            otel_endpoint: "http://localhost:4318".into(),
            otel_service_name: "yq-nova".into(),
            otel_sample_rate: 1.0,
        }
    }
}

impl Config {
    pub fn load() -> NovaResult<Self> {
        let toml_path = std::env::var("YQ_NOVA_CONFIG").map(PathBuf::from).ok().or_else(|| {
            let local = PathBuf::from(DEFAULT_CONFIG_FILENAME);
            if local.exists() { Some(local) } else { None }
        });

        let mut figment = Figment::from(Serialized::defaults(Config::default()))
            .select(Profile::from_env_or("YQ_NOVA_PROFILE", "default"))
            .merge(Env::prefixed("YQ_NOVA_").split("__"));

        if let Some(path) = &toml_path {
            if path.exists() {
                figment = figment.merge(Toml::file(path));
            }
        }

        let cfg: Config = figment.extract()?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> NovaResult<()> {
        if self.server.bind.is_empty() {
            return Err(NovaError::config_msg("server.bind must not be empty"));
        }
        if self.storage.db_path.as_os_str().is_empty() {
            return Err(NovaError::config_msg("storage.db_path must not be empty"));
        }

        let path_str = self.storage.db_path.to_string_lossy();
        if path_str.contains("..") {
            return Err(NovaError::config_msg(
                "storage.db_path must not contain '..' components; use an absolute path",
            ));
        }
        if self.embedding.default_provider.is_empty() {
            return Err(NovaError::config_msg("embedding.default_provider must be set"));
        }

        for (name, fe) in &self.embedding.fastembed_local {
            if fe.model_name.trim().is_empty() {
                return Err(NovaError::config_msg(format!(
                    "embedding.fastembed_local['{name}'].model_name must not be empty"
                )));
            }
        }
        for (name, z) in &self.embedding.zhipu {
            if z.model.trim().is_empty() {
                return Err(NovaError::config_msg(format!(
                    "embedding.zhipu['{name}'].model must not be empty"
                )));
            }
        }
        for (name, q) in &self.embedding.qwen {
            if q.model.trim().is_empty() {
                return Err(NovaError::config_msg(format!(
                    "embedding.qwen['{name}'].model must not be empty"
                )));
            }
        }
        for (name, b) in &self.embedding.baichuan {
            if b.model.trim().is_empty() {
                return Err(NovaError::config_msg(format!(
                    "embedding.baichuan['{name}'].model must not be empty"
                )));
            }
        }
        for (name, j) in &self.embedding.jina {
            if j.model.trim().is_empty() {
                return Err(NovaError::config_msg(format!(
                    "embedding.jina['{name}'].model must not be empty"
                )));
            }
        }
        if self.forgetting.stale_importance_threshold < 0.0
            || self.forgetting.stale_importance_threshold > 1.0
        {
            return Err(NovaError::config_msg(
                "forgetting.stale_importance_threshold must be in [0, 1]",
            ));
        }
        if self.forgetting.action != "archive" && self.forgetting.action != "delete" {
            return Err(NovaError::config_msg("forgetting.action must be 'archive' or 'delete'"));
        }
        if self.server.max_request_body_bytes < 1024 {
            return Err(NovaError::config_msg(
                "server.max_request_body_bytes must be at least 1024",
            ));
        }
        let sync = self.storage.synchronous.to_ascii_lowercase();
        if !matches!(sync.as_str(), "full" | "normal" | "off" | "0" | "1" | "2" | "3") {
            return Err(NovaError::config_msg(
                "storage.synchronous must be one of full / normal / off (or numeric 0-3)",
            ));
        }
        if self.storage.wal_autocheckpoint_kb < 0 {
            return Err(NovaError::config_msg("storage.wal_autocheckpoint_kb must be >= 0"));
        }
        if self.storage.journal_size_limit_kb < 0 {
            return Err(NovaError::config_msg("storage.journal_size_limit_kb must be >= 0"));
        }
        if self.storage.mmap_size_kb < 0 {
            return Err(NovaError::config_msg("storage.mmap_size_kb must be >= 0"));
        }
        if self.storage.soft_heap_limit_kb < 0 {
            return Err(NovaError::config_msg("storage.soft_heap_limit_kb must be >= 0"));
        }
        if self.storage.pool_max_connections == 0 {
            return Err(NovaError::config_msg("storage.pool_max_connections must be >= 1"));
        }
        if self.storage.pool_min_connections > self.storage.pool_max_connections {
            return Err(NovaError::config_msg(format!(
                "storage.pool_min_connections ({}) must be <= storage.pool_max_connections ({})",
                self.storage.pool_min_connections, self.storage.pool_max_connections
            )));
        }
        // 0 keeps the SQLite/library default; any other value must be a legal
        // page size (power of two between 512 and 65536). A non-conforming
        // value is silently ignored by `PRAGMA page_size`, so the user would
        // believe a size is in effect that never was.
        let page_size = self.storage.page_size;
        if page_size != 0
            && (!(512..=65_536).contains(&page_size) || page_size & (page_size - 1) != 0)
        {
            return Err(NovaError::config_msg(
                "storage.page_size must be a power of two between 512 and 65536 (or 0 to keep the \
                 default)",
            ));
        }
        if !(0.0..=1.0).contains(&self.logging.otel_sample_rate) {
            return Err(NovaError::config_msg("logging.otel_sample_rate must be in [0, 1]"));
        }
        Ok(())
    }

    /// Convenience delegate to
    /// [`EmbeddingConfig::validate_provider_references`].
    pub fn validate_provider_references(&self) -> NovaResult<()> {
        self.embedding.validate_provider_references()
    }

    pub fn save_to(&self, path: &std::path::Path) -> NovaResult<()> {
        let s = toml::to_string_pretty(self)
            .map_err(|e| NovaError::config_msg(format!("serialise config: {e}")))?;
        fs::write(path, s)?;
        Ok(())
    }

    pub fn save_to_temp<W: std::io::Write>(&self, mut w: W) -> NovaResult<()> {
        let s = toml::to_string_pretty(self)
            .map_err(|e| NovaError::config_msg(format!("serialise config: {e}")))?;
        w.write_all(s.as_bytes())
            .map_err(|e| NovaError::config_msg(format!("write config to buffer: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate() {
        Config::default().validate().expect("defaults must validate");
    }

    #[test]
    fn db_path_traversal_rejected() {
        let mut cfg = Config::default();
        cfg.storage.db_path = PathBuf::from("/tmp/../../etc/passwd");
        let err = cfg.validate().expect_err("should fail");
        assert_eq!(err.code(), crate::error::ErrorCode::Config);
    }

    #[test]
    fn wrong_forgetting_action_rejected() {
        let mut cfg = Config::default();
        cfg.forgetting.action = "shred".into();
        cfg.validate().expect_err("should fail");
    }

    #[test]
    fn importance_threshold_out_of_range() {
        let mut cfg = Config::default();
        cfg.forgetting.stale_importance_threshold = 1.5;
        cfg.validate().expect_err("should fail");
    }

    #[test]
    fn pool_min_above_max_rejected() {
        let mut cfg = Config::default();
        cfg.storage.pool_min_connections = 8;
        cfg.storage.pool_max_connections = 4;
        let err = cfg.validate().expect_err("should fail");
        assert_eq!(err.code(), crate::error::ErrorCode::Config);
        assert!(err.to_string().contains("pool_min_connections"), "{err}");
    }

    #[test]
    fn pool_max_zero_rejected() {
        let mut cfg = Config::default();
        cfg.storage.pool_max_connections = 0;
        cfg.validate().expect_err("should fail");
    }

    #[test]
    fn page_size_must_be_power_of_two_in_range() {
        for good in [0, 512, 4096, 65_536] {
            let mut cfg = Config::default();
            cfg.storage.page_size = good;
            cfg.validate().unwrap_or_else(|e| panic!("page_size {good} should validate: {e}"));
        }
        for bad in [1, 1000, 3000, 65_537, -4096] {
            let mut cfg = Config::default();
            cfg.storage.page_size = bad;
            let err = cfg.validate().expect_err("should fail");
            assert!(err.to_string().contains("page_size"), "{err}");
        }
    }

    #[test]
    fn otel_sample_rate_range_checked() {
        for good in [0.0, 0.25, 1.0] {
            let mut cfg = Config::default();
            cfg.logging.otel_sample_rate = good;
            cfg.validate().unwrap_or_else(|e| panic!("rate {good} should validate: {e}"));
        }
        for bad in [-0.1, 1.5, f32::NAN] {
            let mut cfg = Config::default();
            cfg.logging.otel_sample_rate = bad;
            cfg.validate().expect_err("should fail");
        }
    }

    #[test]
    fn default_provider_must_resolve() {
        let cfg = Config::default();
        cfg.validate_provider_references()
            .expect("default config points at the 'default' openai_compatible key");

        let mut cfg = Config::default();
        cfg.embedding.default_provider = "mock".into();
        cfg.validate_provider_references().expect("mock is always available");

        let mut cfg = Config::default();
        cfg.embedding.default_provider = "MOCK".into();
        cfg.validate_provider_references().expect("mock match is case-insensitive");

        let mut cfg = Config::default();
        cfg.embedding.default_provider = "does-not-exist".into();
        let err = cfg.validate_provider_references().expect_err("should fail");
        assert!(err.to_string().contains("does-not-exist"), "{err}");
        assert!(err.to_string().contains("fastembed_local"), "{err}");
    }
}
