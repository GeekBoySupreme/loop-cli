//! Configuration management for Loop.
//!
//! All config is stored at `~/.loop/config.toml`. The config module
//! handles loading, saving, and validating the configuration.

pub mod types;

pub use types::*;

use crate::error::LoopError;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "loop-cli";

/// Returns the root Loop config directory: `~/.loop/`
pub fn loop_home() -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".loop")
}

/// Returns the path to the config file: `~/.loop/config.toml`
pub fn config_path() -> PathBuf {
    loop_home().join("config.toml")
}

/// Returns the plugins directory: `~/.loop/plugins/`
pub fn plugins_dir() -> PathBuf {
    loop_home().join("plugins")
}

/// Returns the checkpoints directory: `~/.loop/checkpoints/`
pub fn checkpoints_dir() -> PathBuf {
    loop_home().join("checkpoints")
}

/// Returns the skills directory: `~/.loop/skills/`
pub fn skills_dir() -> PathBuf {
    loop_home().join("skills")
}

/// Returns the MCP tools cache directory: `~/.loop/mcp/`
pub fn mcp_dir() -> PathBuf {
    loop_home().join("mcp")
}

/// Ensure all required directories exist
pub fn ensure_dirs() -> anyhow::Result<()> {
    let dirs = [
        loop_home(),
        plugins_dir(),
        checkpoints_dir(),
        skills_dir(),
        mcp_dir(),
    ];
    for dir in &dirs {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

impl LoopConfig {
    /// Load config from `~/.loop/config.toml`
    pub fn load() -> Result<Self, LoopError> {
        let path = config_path();
        if !path.exists() {
            return Err(LoopError::Config(
                "No config file found. Run `loop init` first.".into(),
            ));
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| LoopError::Config(format!("Failed to read config: {}", e)))?;
        let mut config: LoopConfig = toml::from_str(&content)
            .map_err(|e| LoopError::Config(format!("Failed to parse config: {}", e)))?;
        let migrated = hydrate_api_keys(&mut config.models)?;
        if migrated {
            write_config_metadata(&config)?;
        }
        Ok(config)
    }

    /// Save config to `~/.loop/config.toml`
    pub fn save(&self) -> Result<(), LoopError> {
        ensure_dirs().map_err(|e| LoopError::Config(e.to_string()))?;
        store_api_keys(&self.models)?;
        write_config_metadata(self)
    }

    /// Get the provider config and model ID for the default model
    pub fn default_provider_info(&self) -> Result<(&str, ProviderKind), LoopError> {
        let model = &self.default_model;

        // Determine provider from model name prefixes or explicit mapping
        if self
            .models
            .openrouter
            .as_ref()
            .is_some_and(|config| config.model == *model)
        {
            Ok((model.as_str(), ProviderKind::OpenRouter))
        } else if model.starts_with("claude") {
            if self.models.anthropic.is_some() {
                Ok((model.as_str(), ProviderKind::Anthropic))
            } else {
                Err(LoopError::ApiKeyMissing("anthropic".into()))
            }
        } else if model.starts_with("gpt") || model.starts_with("o1") || model.starts_with("o3") {
            if self.models.openai.is_some() {
                Ok((model.as_str(), ProviderKind::OpenAI))
            } else {
                Err(LoopError::ApiKeyMissing("openai".into()))
            }
        } else if model.starts_with("gemini") {
            if self.models.gemini.is_some() {
                Ok((model.as_str(), ProviderKind::Gemini))
            } else {
                Err(LoopError::ApiKeyMissing("gemini".into()))
            }
        } else if model.starts_with("llama") || model.starts_with("mixtral") {
            if self.models.groq.is_some() {
                Ok((model.as_str(), ProviderKind::Groq))
            } else {
                Err(LoopError::ApiKeyMissing("groq".into()))
            }
        } else if model.starts_with("gemma") {
            if self.models.ollama.is_some() {
                Ok((model.as_str(), ProviderKind::Ollama))
            } else {
                Err(LoopError::ModelNotConfigured("ollama/gemma".into()))
            }
        } else {
            Err(LoopError::ModelNotConfigured(model.clone()))
        }
    }
}

fn credential(provider: &str) -> Result<keyring::Entry, LoopError> {
    keyring::Entry::new(KEYRING_SERVICE, provider).map_err(|error| {
        LoopError::Config(format!("Failed to open OS credential store: {}", error))
    })
}

fn store_key(provider: &str, api_key: &str) -> Result<(), LoopError> {
    if api_key.is_empty() {
        return Ok(());
    }
    credential(provider)?
        .set_password(api_key)
        .map_err(|error| {
            LoopError::Config(format!(
                "Failed to encrypt {} API key in OS credential store: {}",
                provider, error
            ))
        })
}

fn load_key(provider: &str) -> Result<String, LoopError> {
    match credential(provider)?.get_password() {
        Ok(api_key) => Ok(api_key),
        Err(keyring::Error::NoEntry) => Ok(String::new()),
        Err(error) => Err(LoopError::Config(format!(
            "Failed to read {} API key from OS credential store: {}",
            provider, error
        ))),
    }
}

fn store_api_keys(models: &ModelConfig) -> Result<(), LoopError> {
    if let Some(config) = &models.anthropic {
        store_key("anthropic", &config.api_key)?;
    }
    if let Some(config) = &models.openai {
        store_key("openai", &config.api_key)?;
    }
    if let Some(config) = &models.gemini {
        store_key("gemini", &config.api_key)?;
    }
    if let Some(config) = &models.groq {
        store_key("groq", &config.api_key)?;
    }
    if let Some(config) = &models.openrouter {
        store_key("openrouter", &config.api_key)?;
    }
    Ok(())
}

fn hydrate_key(provider: &str, api_key: &mut String) -> Result<bool, LoopError> {
    let migrated = !api_key.is_empty();
    if migrated {
        store_key(provider, api_key)?;
    }
    *api_key = load_key(provider)?;
    Ok(migrated)
}

fn hydrate_api_keys(models: &mut ModelConfig) -> Result<bool, LoopError> {
    let mut migrated = false;
    if let Some(config) = &mut models.anthropic {
        migrated |= hydrate_key("anthropic", &mut config.api_key)?;
    }
    if let Some(config) = &mut models.openai {
        migrated |= hydrate_key("openai", &mut config.api_key)?;
    }
    if let Some(config) = &mut models.gemini {
        migrated |= hydrate_key("gemini", &mut config.api_key)?;
    }
    if let Some(config) = &mut models.groq {
        migrated |= hydrate_key("groq", &mut config.api_key)?;
    }
    if let Some(config) = &mut models.openrouter {
        migrated |= hydrate_key("openrouter", &mut config.api_key)?;
    }
    Ok(migrated)
}

fn write_config_metadata(config: &LoopConfig) -> Result<(), LoopError> {
    let content = toml::to_string_pretty(config)
        .map_err(|error| LoopError::Config(format!("Failed to serialize config: {}", error)))?;
    let path = config_path();
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| LoopError::Config(format!("Failed to write config: {}", error)))?;
    file.write_all(content.as_bytes())
        .map_err(|error| LoopError::Config(format!("Failed to write config: {}", error)))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(
            |error| LoopError::Config(format!("Failed to secure config permissions: {}", error)),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_config_never_contains_api_keys() {
        let sentinel = "loop-secret-must-not-be-serialized".to_string();
        let models = ModelConfig {
            anthropic: Some(AnthropicConfig {
                api_key: sentinel.clone(),
                models: Vec::new(),
            }),
            openai: Some(OpenAIConfig {
                api_key: sentinel.clone(),
                models: Vec::new(),
            }),
            gemini: Some(GeminiConfig {
                api_key: sentinel.clone(),
                models: Vec::new(),
            }),
            groq: Some(GroqConfig {
                api_key: sentinel.clone(),
                models: Vec::new(),
            }),
            ollama: None,
            openrouter: Some(OpenRouterConfig {
                api_key: sentinel.clone(),
                model: "openai/gpt-4o".to_string(),
            }),
        };

        let serialized = toml::to_string(&models).unwrap();
        assert!(!serialized.contains(&sentinel));
        assert!(!serialized.contains("api_key"));
    }
}
