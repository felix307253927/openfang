/*
 * @Author             : Felix
 * @Email              : 307253927@qq.com
 * @Date               : 2026-03-22 16:08:27
 * @LastEditors        : Felix
 * @LastEditTime       : 2026-03-28 10:26:05
 */
//! Model catalog — registry of known models with metadata, pricing, and auth detection.
//!
//! Provides a comprehensive catalog of 130+ builtin models across 28 providers,
//! with alias resolution, auth status detection, and pricing lookups.

use openfang_types::model_catalog::{
    AuthStatus, ModelCatalogEntry, ModelTier, ProviderInfo, AI21_BASE_URL, ANTHROPIC_BASE_URL,
    AZURE_OPENAI_BASE_URL, BEDROCK_BASE_URL, CEREBRAS_BASE_URL, CHUTES_BASE_URL,
    CODING_PLAN_BASE_URL, COHERE_BASE_URL, DEEPSEEK_BASE_URL, FIREWORKS_BASE_URL, GEMINI_BASE_URL,
    GITHUB_COPILOT_BASE_URL, GROQ_BASE_URL, HUGGINGFACE_BASE_URL, KIMI_CODING_BASE_URL,
    LEMONADE_BASE_URL, LMSTUDIO_BASE_URL, MINIMAX_BASE_URL, MISTRAL_BASE_URL, MODELSCOPE_BASE_URL,
    MOONSHOT_BASE_URL, NVIDIA_NIM_BASE_URL, OLLAMA_BASE_URL, OPENAI_BASE_URL, OPENROUTER_BASE_URL,
    PERPLEXITY_BASE_URL, QIANFAN_BASE_URL, QWEN_BASE_URL, REPLICATE_BASE_URL, SAMBANOVA_BASE_URL,
    TOGETHER_BASE_URL, UNIGPT_BASE_URL, VENICE_BASE_URL, VLLM_BASE_URL, VOLCENGINE_BASE_URL,
    VOLCENGINE_CODING_BASE_URL, XAI_BASE_URL, ZHIPU_BASE_URL, ZHIPU_CODING_BASE_URL,
};
use std::collections::HashMap;

/// The model catalog — registry of all known models and providers.
pub struct ModelCatalog {
    models: Vec<ModelCatalogEntry>,
    aliases: HashMap<String, String>,
    providers: Vec<ProviderInfo>,
}

impl ModelCatalog {
    /// Create a new catalog populated with builtin models and providers.
    pub fn new() -> Self {
        let models = builtin_models();
        let mut aliases = builtin_aliases();
        let mut providers = builtin_providers();

        // Auto-register aliases defined on model entries
        for model in &models {
            for alias in &model.aliases {
                let lower = alias.to_lowercase();
                aliases.entry(lower).or_insert_with(|| model.id.clone());
            }
        }

        // Set model counts on providers
        for provider in &mut providers {
            provider.model_count = models.iter().filter(|m| m.provider == provider.id).count();
        }

        Self {
            models,
            aliases,
            providers,
        }
    }

    /// Detect which providers have API keys configured.
    ///
    /// Checks `std::env::var()` for each provider's API key env var.
    /// Only checks presence — never reads or stores the actual secret.
    pub fn detect_auth(&mut self) {
        for provider in &mut self.providers {
            // Claude Code is special: no API key needed, but we probe for CLI
            // installation so the dashboard shows "Configured" vs "Not Installed".
            if provider.id == "claude-code" {
                provider.auth_status = if crate::drivers::claude_code::claude_code_available() {
                    AuthStatus::Configured
                } else {
                    AuthStatus::Missing
                };
                continue;
            }
            if provider.id == "qwen-code" {
                provider.auth_status = if crate::drivers::qwen_code::qwen_code_available() {
                    AuthStatus::Configured
                } else {
                    AuthStatus::Missing
                };
                continue;
            }

            if !provider.key_required {
                provider.auth_status = AuthStatus::NotRequired;
                continue;
            }

            // Primary: check the provider's declared env var
            let has_key = std::env::var(&provider.api_key_env).is_ok();

            // Secondary: provider-specific fallback auth
            let has_fallback = match provider.id.as_str() {
                "gemini" => std::env::var("GOOGLE_API_KEY").is_ok(),
                "codex" => {
                    std::env::var("OPENAI_API_KEY").is_ok() || read_codex_credential().is_some()
                }
                // claude-code is handled above (before key_required check)
                _ => false,
            };

            provider.auth_status = if has_key || has_fallback {
                AuthStatus::Configured
            } else {
                AuthStatus::Missing
            };
        }
    }

    /// List all models in the catalog.
    pub fn list_models(&self) -> &[ModelCatalogEntry] {
        &self.models
    }

    /// Find a model by its canonical ID, display name, or alias.
    pub fn find_model(&self, id_or_alias: &str) -> Option<&ModelCatalogEntry> {
        let lower = id_or_alias.to_lowercase();
        // Direct ID match first
        if let Some(entry) = self.models.iter().find(|m| m.id.to_lowercase() == lower) {
            return Some(entry);
        }
        // Display-name match for dashboard/UI payloads that send labels.
        if let Some(entry) = self
            .models
            .iter()
            .find(|m| m.display_name.to_lowercase() == lower)
        {
            return Some(entry);
        }
        // Alias resolution
        if let Some(canonical) = self.aliases.get(&lower) {
            return self.models.iter().find(|m| m.id == *canonical);
        }
        None
    }

    /// Resolve an alias to a canonical model ID, or None if not an alias.
    pub fn resolve_alias(&self, alias: &str) -> Option<&str> {
        self.aliases.get(&alias.to_lowercase()).map(|s| s.as_str())
    }

    /// List all providers.
    pub fn list_providers(&self) -> &[ProviderInfo] {
        &self.providers
    }

    /// Get a provider by ID.
    pub fn get_provider(&self, provider_id: &str) -> Option<&ProviderInfo> {
        self.providers.iter().find(|p| p.id == provider_id)
    }

    /// List models from a specific provider.
    pub fn models_by_provider(&self, provider: &str) -> Vec<&ModelCatalogEntry> {
        self.models
            .iter()
            .filter(|m| m.provider == provider)
            .collect()
    }

    /// Return the default model ID for a provider (first model in catalog order).
    pub fn default_model_for_provider(&self, provider: &str) -> Option<String> {
        // Check aliases first — e.g. "minimax" alias resolves to "MiniMax-M2.5"
        if let Some(model_id) = self.aliases.get(provider) {
            return Some(model_id.clone());
        }
        // Fall back to the first model registered for this provider
        self.models
            .iter()
            .find(|m| m.provider == provider)
            .map(|m| m.id.clone())
    }

    /// List models that are available (from configured providers only).
    pub fn available_models(&self) -> Vec<&ModelCatalogEntry> {
        let configured: Vec<&str> = self
            .providers
            .iter()
            .filter(|p| p.auth_status != AuthStatus::Missing)
            .map(|p| p.id.as_str())
            .collect();
        self.models
            .iter()
            .filter(|m| configured.contains(&m.provider.as_str()))
            .collect()
    }

    /// Get pricing for a model: (input_cost_per_million, output_cost_per_million).
    pub fn pricing(&self, model_id: &str) -> Option<(f64, f64)> {
        self.find_model(model_id)
            .map(|m| (m.input_cost_per_m, m.output_cost_per_m))
    }

    /// List all alias mappings.
    pub fn list_aliases(&self) -> &HashMap<String, String> {
        &self.aliases
    }

    /// Set a custom base URL for a provider, overriding the default.
    ///
    /// Returns `true` if the provider was found and updated.
    pub fn set_provider_url(&mut self, provider: &str, url: &str) -> bool {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
            p.base_url = url.to_string();
            true
        } else {
            // Custom provider — add a new entry so it appears in /api/providers
            let env_var = format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"));
            self.providers.push(ProviderInfo {
                id: provider.to_string(),
                display_name: provider.to_string(),
                api_key_env: env_var,
                base_url: url.to_string(),
                key_required: true,
                auth_status: AuthStatus::Missing,
                model_count: 0,
                is_local: provider == "vllm" || provider == "ollama",
            });
            // Re-detect auth for the newly added provider
            self.detect_auth();
            true
        }
    }

    /// Apply a batch of provider URL overrides from config.
    ///
    /// Each entry maps a provider ID to a custom base URL.
    /// Unknown providers are automatically added as custom OpenAI-compatible entries.
    /// Providers with explicit URL overrides are marked as configured since
    /// the user intentionally set them up (e.g. local proxies, custom endpoints).
    pub fn apply_url_overrides(&mut self, overrides: &HashMap<String, String>) {
        for (provider, url) in overrides {
            if self.set_provider_url(provider, url) {
                // Mark as configured so models from this provider show as available
                if let Some(p) = self.providers.iter_mut().find(|p| p.id == *provider) {
                    if p.auth_status == AuthStatus::Missing {
                        p.auth_status = AuthStatus::Configured;
                    }
                }
            }
        }
    }

    /// List models filtered by tier.
    pub fn models_by_tier(&self, tier: ModelTier) -> Vec<&ModelCatalogEntry> {
        self.models.iter().filter(|m| m.tier == tier).collect()
    }

    /// Merge dynamically discovered models from a local provider.
    ///
    /// Adds models not already in the catalog with `Local` tier and zero cost.
    /// Also updates the provider's `model_count`.
    pub fn merge_discovered_models(&mut self, provider: &str, model_ids: &[String]) {
        let existing_ids: std::collections::HashSet<String> = self
            .models
            .iter()
            .filter(|m| m.provider == provider)
            .map(|m| m.id.to_lowercase())
            .collect();

        let mut added = 0usize;
        for id in model_ids {
            if existing_ids.contains(&id.to_lowercase()) {
                continue;
            }
            // Generate a human-friendly display name
            let display = format!("{} ({})", id, provider);
            self.models.push(ModelCatalogEntry {
                id: id.clone(),
                display_name: display,
                provider: provider.to_string(),
                tier: ModelTier::Local,
                context_window: 32_768,
                max_output_tokens: 4_096,
                input_cost_per_m: 0.0,
                output_cost_per_m: 0.0,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
                aliases: Vec::new(),
            });
            added += 1;
        }

        // Update model count on the provider
        if added > 0 {
            if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
                p.model_count = self
                    .models
                    .iter()
                    .filter(|m| m.provider == provider)
                    .count();
            }
        }
    }

    /// Add a custom model at runtime.
    ///
    /// Returns `true` if the model was added, `false` if a model with the same
    /// ID **and** provider already exists (case-insensitive).
    pub fn add_custom_model(&mut self, entry: ModelCatalogEntry) -> bool {
        let lower_id = entry.id.to_lowercase();
        let lower_provider = entry.provider.to_lowercase();
        if self
            .models
            .iter()
            .any(|m| m.id.to_lowercase() == lower_id && m.provider.to_lowercase() == lower_provider)
        {
            return false;
        }
        let provider = entry.provider.clone();
        self.models.push(entry);

        // Update provider model count
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
            p.model_count = self
                .models
                .iter()
                .filter(|m| m.provider == provider)
                .count();
        }
        true
    }

    /// Remove a custom model by ID.
    ///
    /// Only removes models with `Custom` tier to prevent accidental deletion
    /// of builtin models. Returns `true` if removed.
    pub fn remove_custom_model(&mut self, model_id: &str) -> bool {
        let lower = model_id.to_lowercase();
        let before = self.models.len();
        self.models
            .retain(|m| !(m.id.to_lowercase() == lower && m.tier == ModelTier::Custom));
        self.models.len() < before
    }

    /// Load custom models from a JSON file.
    ///
    /// Merges them into the catalog. Skips models that already exist.
    pub fn load_custom_models(&mut self, path: &std::path::Path) {
        if !path.exists() {
            return;
        }
        let Ok(data) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(entries) = serde_json::from_str::<Vec<ModelCatalogEntry>>(&data) else {
            return;
        };
        for entry in entries {
            self.add_custom_model(entry);
        }
    }

    /// Save all custom-tier models to a JSON file.
    pub fn save_custom_models(&self, path: &std::path::Path) -> Result<(), String> {
        let custom: Vec<&ModelCatalogEntry> = self
            .models
            .iter()
            .filter(|m| m.tier == ModelTier::Custom)
            .collect();
        let json = serde_json::to_string_pretty(&custom)
            .map_err(|e| format!("Failed to serialize custom models: {e}"))?;
        std::fs::write(path, json)
            .map_err(|e| format!("Failed to write custom models file: {e}"))?;
        Ok(())
    }
}

impl Default for ModelCatalog {
    fn default() -> Self {
        Self::new()
    }
}

/// Read an OpenAI API key from the Codex CLI credential file.
///
/// Checks `$CODEX_HOME/auth.json` or `~/.codex/auth.json`.
/// Returns `Some(api_key)` if the file exists and contains a valid, non-expired token.
/// Only checks presence — the actual key value is used transiently, never stored.
pub fn read_codex_credential() -> Option<String> {
    let codex_home = std::env::var("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .ok()
        .or_else(|| {
            #[cfg(target_os = "windows")]
            {
                std::env::var("USERPROFILE")
                    .ok()
                    .map(|h| std::path::PathBuf::from(h).join(".codex"))
            }
            #[cfg(not(target_os = "windows"))]
            {
                std::env::var("HOME")
                    .ok()
                    .map(|h| std::path::PathBuf::from(h).join(".codex"))
            }
        })?;

    let auth_path = codex_home.join("auth.json");
    let content = std::fs::read_to_string(&auth_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Check expiry if present
    if let Some(expires_at) = parsed.get("expires_at").and_then(|v| v.as_i64()) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if now >= expires_at {
            return None; // Expired
        }
    }

    parsed
        .get("api_key")
        .or_else(|| parsed.get("token"))
        .or_else(|| parsed.get("tokens").and_then(|t| t.get("id_token")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Builtin data
// ---------------------------------------------------------------------------

fn builtin_providers() -> Vec<ProviderInfo> {
    vec![
        ProviderInfo {
            id: "anthropic".into(),
            display_name: "Anthropic".into(),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            base_url: ANTHROPIC_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "openai".into(),
            display_name: "OpenAI".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            base_url: OPENAI_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "gemini".into(),
            display_name: "Google Gemini".into(),
            api_key_env: "GEMINI_API_KEY".into(),
            base_url: GEMINI_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "deepseek".into(),
            display_name: "DeepSeek".into(),
            api_key_env: "DEEPSEEK_API_KEY".into(),
            base_url: DEEPSEEK_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "groq".into(),
            display_name: "Groq".into(),
            api_key_env: "GROQ_API_KEY".into(),
            base_url: GROQ_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "openrouter".into(),
            display_name: "OpenRouter".into(),
            api_key_env: "OPENROUTER_API_KEY".into(),
            base_url: OPENROUTER_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "mistral".into(),
            display_name: "Mistral AI".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            base_url: MISTRAL_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "together".into(),
            display_name: "Together AI".into(),
            api_key_env: "TOGETHER_API_KEY".into(),
            base_url: TOGETHER_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "fireworks".into(),
            display_name: "Fireworks AI".into(),
            api_key_env: "FIREWORKS_API_KEY".into(),
            base_url: FIREWORKS_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "ollama".into(),
            display_name: "Ollama".into(),
            api_key_env: "OLLAMA_API_KEY".into(),
            base_url: OLLAMA_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: true,
        },
        ProviderInfo {
            id: "vllm".into(),
            display_name: "vLLM".into(),
            api_key_env: "VLLM_API_KEY".into(),
            base_url: VLLM_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: true,
        },
        ProviderInfo {
            id: "lmstudio".into(),
            display_name: "LM Studio".into(),
            api_key_env: "LMSTUDIO_API_KEY".into(),
            base_url: LMSTUDIO_BASE_URL.into(),
            key_required: false,
            auth_status: AuthStatus::NotRequired,
            model_count: 0,
            is_local: true,
        },
        ProviderInfo {
            id: "lemonade".into(),
            display_name: "Lemonade".into(),
            api_key_env: "LEMONADE_API_KEY".into(),
            base_url: LEMONADE_BASE_URL.into(),
            key_required: false,
            auth_status: AuthStatus::NotRequired,
            model_count: 0,
            is_local: false,
        },
        // ── New providers (8) ──────────────────────────────────────
        ProviderInfo {
            id: "perplexity".into(),
            display_name: "Perplexity AI".into(),
            api_key_env: "PERPLEXITY_API_KEY".into(),
            base_url: PERPLEXITY_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "cohere".into(),
            display_name: "Cohere".into(),
            api_key_env: "COHERE_API_KEY".into(),
            base_url: COHERE_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "ai21".into(),
            display_name: "AI21 Labs".into(),
            api_key_env: "AI21_API_KEY".into(),
            base_url: AI21_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "cerebras".into(),
            display_name: "Cerebras".into(),
            api_key_env: "CEREBRAS_API_KEY".into(),
            base_url: CEREBRAS_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "sambanova".into(),
            display_name: "SambaNova".into(),
            api_key_env: "SAMBANOVA_API_KEY".into(),
            base_url: SAMBANOVA_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "huggingface".into(),
            display_name: "Hugging Face".into(),
            api_key_env: "HF_API_KEY".into(),
            base_url: HUGGINGFACE_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "xai".into(),
            display_name: "xAI".into(),
            api_key_env: "XAI_API_KEY".into(),
            base_url: XAI_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "replicate".into(),
            display_name: "Replicate".into(),
            api_key_env: "REPLICATE_API_TOKEN".into(),
            base_url: REPLICATE_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── GitHub Copilot ───────────────────────────────────────────
        ProviderInfo {
            id: "github-copilot".into(),
            display_name: "GitHub Copilot".into(),
            api_key_env: "GITHUB_TOKEN".into(),
            base_url: GITHUB_COPILOT_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── Chutes.ai ───────────────────────────────────────────────
        ProviderInfo {
            id: "chutes".into(),
            display_name: "Chutes.ai".into(),
            api_key_env: "CHUTES_API_KEY".into(),
            base_url: CHUTES_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── Venice.ai ────────────────────────────────────────────────
        ProviderInfo {
            id: "venice".into(),
            display_name: "Venice.ai".into(),
            api_key_env: "VENICE_API_KEY".into(),
            base_url: VENICE_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── NVIDIA NIM ────────────────────────────────────────────────
        ProviderInfo {
            id: "nvidia".into(),
            display_name: "NVIDIA NIM".into(),
            api_key_env: "NVIDIA_API_KEY".into(),
            base_url: NVIDIA_NIM_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── Chinese providers (5) ────────────────────────────────────
        ProviderInfo {
            id: "qwen".into(),
            display_name: "Qwen (Alibaba)".into(),
            api_key_env: "DASHSCOPE_API_KEY".into(),
            base_url: QWEN_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "minimax".into(),
            display_name: "MiniMax".into(),
            api_key_env: "MINIMAX_API_KEY".into(),
            base_url: MINIMAX_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "zhipu".into(),
            display_name: "Zhipu AI (GLM)".into(),
            api_key_env: "ZHIPU_API_KEY".into(),
            base_url: ZHIPU_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "zhipu_coding".into(),
            display_name: "Zhipu Coding (CodeGeeX)".into(),
            api_key_env: "ZHIPU_CODING_API_KEY".into(),
            base_url: ZHIPU_CODING_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "moonshot".into(),
            display_name: "Moonshot (Kimi)".into(),
            api_key_env: "MOONSHOT_API_KEY".into(),
            base_url: MOONSHOT_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "kimi_coding".into(),
            display_name: "KimiCode".into(),
            api_key_env: "KIMI_API_KEY".into(),
            base_url: KIMI_CODING_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "qianfan".into(),
            display_name: "百度千帆".into(),
            api_key_env: "QIANFAN_API_KEY".into(),
            base_url: QIANFAN_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "modelscope".into(),
            display_name: "ModelScope".into(),
            api_key_env: "MODELSCOPE_API_KEY".into(),
            base_url: MODELSCOPE_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── Volcano Engine (Doubao) ──────────────────────────────────
        ProviderInfo {
            id: "volcengine".into(),
            display_name: "字节(Doubao)".into(),
            api_key_env: "VOLCENGINE_API_KEY".into(),
            base_url: VOLCENGINE_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "volcengine_coding".into(),
            display_name: "字节Coding Plan".into(),
            api_key_env: "VOLCENGINE_CODING_API_KEY".into(),
            base_url: VOLCENGINE_CODING_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── AWS Bedrock ──────────────────────────────────────────────
        ProviderInfo {
            id: "bedrock".into(),
            display_name: "AWS Bedrock".into(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            base_url: BEDROCK_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── Azure OpenAI ───────────────────────────────────────────
        ProviderInfo {
            id: "azure".into(),
            display_name: "Azure OpenAI".into(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            base_url: AZURE_OPENAI_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── OpenAI Codex ────────────────────────────────────────────
        ProviderInfo {
            id: "codex".into(),
            display_name: "OpenAI Codex".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            base_url: OPENAI_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── Claude Code CLI ─────────────────────────────────────────
        ProviderInfo {
            id: "claude-code".into(),
            display_name: "Claude Code".into(),
            api_key_env: String::new(),
            base_url: String::new(),
            key_required: false,
            auth_status: AuthStatus::NotRequired,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "unigpt".into(),
            display_name: "UniGPT".into(),
            api_key_env: "UNIGPT_API_KEY".into(),
            base_url: UNIGPT_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
        // ── Qwen Code CLI ──────────────────────────────────────────
        ProviderInfo {
            id: "qwen-code".into(),
            display_name: "Qwen Code".into(),
            api_key_env: String::new(),
            base_url: String::new(),
            key_required: false,
            auth_status: AuthStatus::NotRequired,
            model_count: 0,
            is_local: false,
        },
        ProviderInfo {
            id: "coding_plan".into(),
            display_name: "千问CodingPlan".into(),
            api_key_env: "CODING_PLAN_API_KEY".into(),
            base_url: CODING_PLAN_BASE_URL.into(),
            key_required: true,
            auth_status: AuthStatus::Missing,
            model_count: 0,
            is_local: false,
        },
    ]
}

fn builtin_aliases() -> HashMap<String, String> {
    let pairs = [
        ("sonnet", "claude-sonnet-4-6"),
        ("claude-sonnet", "claude-sonnet-4-6"),
        ("opus", "claude-opus-4-6"),
        ("claude-opus", "claude-opus-4-6"),
        // DeepSeek aliases
        ("deepseek-v3", "deepseek-chat"),
        ("deepseek-r1", "deepseek-reasoner"),
        // Claude Code aliases
        ("claude-code", "claude-code/sonnet"),
        ("claude-code-opus", "claude-code/opus"),
        ("claude-code-sonnet", "claude-code/sonnet"),
        ("claude-code-haiku", "claude-code/haiku"),
        // Qwen Code aliases
        ("qwen-code", "qwen-code/qwen3-coder"),
        ("qwen-coder", "qwen-code/qwen3-coder"),
        ("qwen-coder-plus", "qwen-code/qwen-coder-plus"),
        ("qwq", "qwen-code/qwq-32b"),
    ];
    pairs
        .into_iter()
        .map(|(k, v)| (k.to_lowercase(), v.to_string()))
        .collect()
}

fn builtin_models() -> Vec<ModelCatalogEntry> {
    vec![
        // ══════════════════════════════════════════════════════════════
        // Anthropic (7)
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "claude-opus-4-6".into(),
            display_name: "Claude Opus 4.6".into(),
            provider: "anthropic".into(),
            tier: ModelTier::Frontier,
            context_window: 200_000,
            max_output_tokens: 128_000,
            input_cost_per_m: 5.0,
            output_cost_per_m: 25.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["opus".into(), "claude-opus".into()],
        },
        ModelCatalogEntry {
            id: "claude-sonnet-4-6".into(),
            display_name: "Claude Sonnet 4.6".into(),
            provider: "anthropic".into(),
            tier: ModelTier::Smart,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_cost_per_m: 3.0,
            output_cost_per_m: 15.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["sonnet".into(), "claude-sonnet".into()],
        },
        // ══════════════════════════════════════════════════════════════
        // OpenAI (16)
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "gpt-4o".into(),
            display_name: "GPT-4o".into(),
            provider: "openai".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 16_384,
            input_cost_per_m: 2.50,
            output_cost_per_m: 10.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["gpt4".into(), "gpt4o".into()],
        },
        ModelCatalogEntry {
            id: "gpt-4.1".into(),
            display_name: "GPT-4.1".into(),
            provider: "openai".into(),
            tier: ModelTier::Frontier,
            context_window: 1_047_576,
            max_output_tokens: 32_768,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "o3".into(),
            display_name: "o3".into(),
            provider: "openai".into(),
            tier: ModelTier::Frontier,
            context_window: 200_000,
            max_output_tokens: 100_000,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "o3-mini".into(),
            display_name: "o3-mini".into(),
            provider: "openai".into(),
            tier: ModelTier::Smart,
            context_window: 200_000,
            max_output_tokens: 100_000,
            input_cost_per_m: 1.10,
            output_cost_per_m: 4.40,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "o4-mini".into(),
            display_name: "o4-mini".into(),
            provider: "openai".into(),
            tier: ModelTier::Smart,
            context_window: 200_000,
            max_output_tokens: 100_000,
            input_cost_per_m: 1.10,
            output_cost_per_m: 4.40,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "gpt-5".into(),
            display_name: "GPT-5".into(),
            provider: "openai".into(),
            tier: ModelTier::Frontier,
            context_window: 400_000,
            max_output_tokens: 128_000,
            input_cost_per_m: 1.25,
            output_cost_per_m: 10.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "gpt-5-mini".into(),
            display_name: "GPT-5 Mini".into(),
            provider: "openai".into(),
            tier: ModelTier::Balanced,
            context_window: 400_000,
            max_output_tokens: 128_000,
            input_cost_per_m: 0.25,
            output_cost_per_m: 2.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["gpt5-mini".into()],
        },
        ModelCatalogEntry {
            id: "gpt-5.1".into(),
            display_name: "GPT-5.1".into(),
            provider: "openai".into(),
            tier: ModelTier::Frontier,
            context_window: 400_000,
            max_output_tokens: 128_000,
            input_cost_per_m: 1.25,
            output_cost_per_m: 10.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "gpt-5.2".into(),
            display_name: "GPT-5.2".into(),
            provider: "openai".into(),
            tier: ModelTier::Frontier,
            context_window: 400_000,
            max_output_tokens: 128_000,
            input_cost_per_m: 1.75,
            output_cost_per_m: 14.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["gpt5".into()],
        },
        // ══════════════════════════════════════════════════════════════
        // DeepSeek (4)
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "deepseek-chat".into(),
            display_name: "DeepSeek V3".into(),
            provider: "deepseek".into(),
            tier: ModelTier::Smart,
            context_window: 64_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.27,
            output_cost_per_m: 1.10,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["deepseek".into(), "deepseek-v3".into()],
        },
        ModelCatalogEntry {
            id: "deepseek-reasoner".into(),
            display_name: "DeepSeek R1".into(),
            provider: "deepseek".into(),
            tier: ModelTier::Frontier,
            context_window: 64_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.55,
            output_cost_per_m: 2.19,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["deepseek-r1".into()],
        },
        ModelCatalogEntry {
            id: "deepseek-coder".into(),
            display_name: "DeepSeek Coder V2".into(),
            provider: "deepseek".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.14,
            output_cost_per_m: 0.28,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        // ══════════════════════════════════════════════════════════════
        // Azure OpenAI (4)
        // These represent common Azure deployment names. Users deploy models
        // under their own deployment names, so these are illustrative defaults.
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "azure/gpt-4o".into(),
            display_name: "GPT-4o (Azure)".into(),
            provider: "azure".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 16_384,
            input_cost_per_m: 2.50,
            output_cost_per_m: 10.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "azure/gpt-4o-mini".into(),
            display_name: "GPT-4o Mini (Azure)".into(),
            provider: "azure".into(),
            tier: ModelTier::Fast,
            context_window: 128_000,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.15,
            output_cost_per_m: 0.60,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "azure/gpt-4.1".into(),
            display_name: "GPT-4.1 (Azure)".into(),
            provider: "azure".into(),
            tier: ModelTier::Frontier,
            context_window: 1_047_576,
            max_output_tokens: 32_768,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "azure/gpt-4.1-mini".into(),
            display_name: "GPT-4.1 Mini (Azure)".into(),
            provider: "azure".into(),
            tier: ModelTier::Fast,
            context_window: 1_047_576,
            max_output_tokens: 32_768,
            input_cost_per_m: 0.40,
            output_cost_per_m: 1.60,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        // ══════════════════════════════════════════════════════════════
        // NVIDIA NIM (5)
        // ══════════════════════════════════════════════════════════════
        // ══════════════════════════════════════════════════════════════
        // Qwen / Alibaba (6)
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "qwen-max".into(),
            display_name: "Qwen Max".into(),
            provider: "qwen".into(),
            tier: ModelTier::Frontier,
            context_window: 32_768,
            max_output_tokens: 8_192,
            input_cost_per_m: 4.00,
            output_cost_per_m: 12.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "qwen-plus".into(),
            display_name: "Qwen Plus".into(),
            provider: "qwen".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.80,
            output_cost_per_m: 2.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["qwen".into()],
        },
        ModelCatalogEntry {
            id: "qwen3-235b-a22b".into(),
            display_name: "Qwen3 235B".into(),
            provider: "qwen".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 8_192,
            input_cost_per_m: 4.00,
            output_cost_per_m: 12.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["qwen3".into()],
        },
        ModelCatalogEntry {
            id: "qwen-vl-max".into(),
            display_name: "Qwen VL Max".into(),
            provider: "qwen".into(),
            tier: ModelTier::Frontier,
            context_window: 32_768,
            max_output_tokens: 8_192,
            input_cost_per_m: 3.00,
            output_cost_per_m: 9.00,
            supports_tools: false,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        // ══════════════════════════════════════════════════════════════
        // MiniMax (6)
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "MiniMax-M2.5".into(),
            display_name: "MiniMax M2.5".into(),
            provider: "minimax".into(),
            tier: ModelTier::Frontier,
            context_window: 1_048_576,
            max_output_tokens: 16_384,
            input_cost_per_m: 1.10,
            output_cost_per_m: 4.40,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["minimax-m2.5".into()],
        },
        // ══════════════════════════════════════════════════════════════
        // Zhipu AI / GLM (6)
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "glm-5".into(),
            display_name: "GLM-5".into(),
            provider: "zhipu".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 1.00,
            output_cost_per_m: 3.20,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["glm-5".into()],
        },
        ModelCatalogEntry {
            id: "glm-4.7".into(),
            display_name: "GLM-4.7".into(),
            provider: "zhipu".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.60,
            output_cost_per_m: 2.20,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        // ══════════════════════════════════════════════════════════════
        // Moonshot / Kimi (5)
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "kimi-k2".into(),
            display_name: "Kimi K2".into(),
            provider: "moonshot".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "kimi-k2.5".into(),
            display_name: "Kimi K2.5".into(),
            provider: "moonshot".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["kimi-k2.5-0711".into()],
        },
        // ══════════════════════════════════════════════════════════════
        // Kimi for Code (1)
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "kimi-for-coding".into(),
            display_name: "Kimi For Coding".into(),
            provider: "kimi_coding".into(),
            tier: ModelTier::Frontier,
            context_window: 262_144,
            max_output_tokens: 32_768,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        // ══════════════════════════════════════════════════════════════
        // Baidu Qianfan / ERNIE (3)
        // ══════════════════════════════════════════════════════════════
        // ══════════════════════════════════════════════════════════════
        // Volcano Engine / Doubao (4)
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "doubao-seed-2.0-lite".into(),
            display_name: "doubao-seed-2.0-lite".into(),
            provider: "volcengine".into(),
            tier: ModelTier::Smart,
            context_window: 262_144,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.80,
            output_cost_per_m: 2.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "doubao-seed-2.0-code".into(),
            display_name: "doubao-seed-2.0-code".into(),
            provider: "volcengine_coding".into(),
            tier: ModelTier::Smart,
            context_window: 262_144,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.80,
            output_cost_per_m: 2.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "kimi-k2.5".into(),
            display_name: "kimi-k2.5".into(),
            provider: "volcengine_coding".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.50,
            output_cost_per_m: 1.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["doubao-code".into()],
        },
        // ══════════════════════════════════════════════════════════════
        // OpenAI Codex (2) — reuses OpenAI driver
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "codex/gpt-5.4".into(),
            display_name: "GPT-5.4 (Codex)".into(),
            provider: "codex".into(),
            tier: ModelTier::Frontier,
            context_window: 1_047_576,
            max_output_tokens: 32_768,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["codex".into(), "codex-5.4".into()],
        },
        ModelCatalogEntry {
            id: "codex/gpt-4.1".into(),
            display_name: "GPT-4.1 (Codex)".into(),
            provider: "codex".into(),
            tier: ModelTier::Frontier,
            context_window: 1_047_576,
            max_output_tokens: 32_768,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["codex-4.1".into()],
        },
        ModelCatalogEntry {
            id: "codex/o4-mini".into(),
            display_name: "o4-mini (Codex)".into(),
            provider: "codex".into(),
            tier: ModelTier::Smart,
            context_window: 200_000,
            max_output_tokens: 100_000,
            input_cost_per_m: 1.10,
            output_cost_per_m: 4.40,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["codex-o4".into()],
        },
        // ══════════════════════════════════════════════════════════════
        // Claude Code CLI (3) — subprocess-based
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "claude-code/opus".into(),
            display_name: "Claude Opus (CLI)".into(),
            provider: "claude-code".into(),
            tier: ModelTier::Frontier,
            context_window: 200_000,
            max_output_tokens: 128_000,
            input_cost_per_m: 5.0,
            output_cost_per_m: 25.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["claude-code-opus".into()],
        },
        ModelCatalogEntry {
            id: "claude-code/sonnet".into(),
            display_name: "Claude Sonnet (CLI)".into(),
            provider: "claude-code".into(),
            tier: ModelTier::Smart,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_cost_per_m: 3.0,
            output_cost_per_m: 15.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["claude-code".into(), "claude-code-sonnet".into()],
        },
        ModelCatalogEntry {
            id: "claude-code/haiku".into(),
            display_name: "Claude Haiku (CLI)".into(),
            provider: "claude-code".into(),
            tier: ModelTier::Fast,
            context_window: 200_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.25,
            output_cost_per_m: 1.25,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["claude-code-haiku".into()],
        },
        // ══════════════════════════════════════════════════════════════
        // Qwen Code CLI (3) — subprocess-based, free via Qwen OAuth
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "qwen-code/qwen-coder-plus".into(),
            display_name: "Qwen Coder Plus (CLI)".into(),
            provider: "qwen-code".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 65_536,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["qwen-coder-plus".into()],
        },
        ModelCatalogEntry {
            id: "qwen-code/qwen3-coder".into(),
            display_name: "Qwen3 Coder (CLI)".into(),
            provider: "qwen-code".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 65_536,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["qwen-code".into(), "qwen-coder".into()],
        },
        ModelCatalogEntry {
            id: "qwen-code/qwq-32b".into(),
            display_name: "QwQ 32B (CLI)".into(),
            provider: "qwen-code".into(),
            tier: ModelTier::Balanced,
            context_window: 131_072,
            max_output_tokens: 65_536,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["qwq".into()],
        },
        // ══════════════════════════════════════════════════════════════
        // coding_plan
        // ══════════════════════════════════════════════════════════════
        ModelCatalogEntry {
            id: "glm-5".into(),
            display_name: "GLM-5".into(),
            provider: "coding_plan".into(),
            tier: ModelTier::Fast,
            context_window: 248320,
            max_output_tokens: 248320,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["glm-5".into()],
        },
        ModelCatalogEntry {
            id: "qwen3.5-plus".into(),
            display_name: "Qwen3.5-Plus".into(),
            provider: "coding_plan".into(),
            tier: ModelTier::Fast,
            context_window: 248320,
            max_output_tokens: 248320,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["qwen3.5-plus".into()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_has_models() {
        let catalog = ModelCatalog::new();
        assert!(catalog.list_models().len() >= 30);
    }

    #[test]
    fn test_catalog_has_providers() {
        let catalog = ModelCatalog::new();
        assert_eq!(catalog.list_providers().len(), 41);
    }

    #[test]
    fn test_find_model_by_id() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("claude-sonnet-4-20250514").unwrap();
        assert_eq!(entry.display_name, "Claude Sonnet 4");
        assert_eq!(entry.provider, "anthropic");
        assert_eq!(entry.tier, ModelTier::Smart);
    }

    #[test]
    fn test_find_model_by_alias() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("sonnet").unwrap();
        assert_eq!(entry.id, "claude-sonnet-4-6");
    }

    #[test]
    fn test_find_model_case_insensitive() {
        let catalog = ModelCatalog::new();
        assert!(catalog.find_model("Claude-Sonnet-4-20250514").is_some());
        assert!(catalog.find_model("SONNET").is_some());
    }

    #[test]
    fn test_find_model_not_found() {
        let catalog = ModelCatalog::new();
        assert!(catalog.find_model("nonexistent-model").is_none());
    }

    #[test]
    fn test_resolve_alias() {
        let catalog = ModelCatalog::new();
        assert_eq!(catalog.resolve_alias("sonnet"), Some("claude-sonnet-4-6"));
        assert_eq!(
            catalog.resolve_alias("haiku"),
            Some("claude-haiku-4-5-20251001")
        );
        assert!(catalog.resolve_alias("nonexistent").is_none());
    }

    #[test]
    fn test_models_by_provider() {
        let catalog = ModelCatalog::new();
        let anthropic = catalog.models_by_provider("anthropic");
        assert_eq!(anthropic.len(), 7);
        assert!(anthropic.iter().all(|m| m.provider == "anthropic"));
    }

    #[test]
    fn test_models_by_tier() {
        let catalog = ModelCatalog::new();
        let frontier = catalog.models_by_tier(ModelTier::Frontier);
        assert!(frontier.len() >= 3); // At least opus, gpt-4.1, gemini-2.5-pro
        assert!(frontier.iter().all(|m| m.tier == ModelTier::Frontier));
    }

    #[test]
    fn test_pricing_lookup() {
        let catalog = ModelCatalog::new();
        let (input, output) = catalog.pricing("claude-sonnet-4-20250514").unwrap();
        assert!((input - 3.0).abs() < 0.001);
        assert!((output - 15.0).abs() < 0.001);
    }

    #[test]
    fn test_pricing_via_alias() {
        let catalog = ModelCatalog::new();
        let (input, output) = catalog.pricing("sonnet").unwrap();
        assert!((input - 3.0).abs() < 0.001);
        assert!((output - 15.0).abs() < 0.001);
    }

    #[test]
    fn test_pricing_not_found() {
        let catalog = ModelCatalog::new();
        assert!(catalog.pricing("nonexistent").is_none());
    }

    #[test]
    fn test_detect_auth_local_providers() {
        let mut catalog = ModelCatalog::new();
        catalog.detect_auth();
        // Local providers should be NotRequired
        let ollama = catalog.get_provider("ollama").unwrap();
        assert_eq!(ollama.auth_status, AuthStatus::NotRequired);
        let vllm = catalog.get_provider("vllm").unwrap();
        assert_eq!(vllm.auth_status, AuthStatus::NotRequired);
    }

    #[test]
    fn test_available_models_includes_local() {
        let mut catalog = ModelCatalog::new();
        catalog.detect_auth();
        let available = catalog.available_models();
        // Local providers (ollama, vllm, lmstudio) should always be available
        assert!(available.iter().any(|m| m.provider == "ollama"));
    }

    #[test]
    fn test_provider_model_counts() {
        let catalog = ModelCatalog::new();
        let anthropic = catalog.get_provider("anthropic").unwrap();
        assert_eq!(anthropic.model_count, 7);
        let groq = catalog.get_provider("groq").unwrap();
        assert_eq!(groq.model_count, 10);
    }

    #[test]
    fn test_list_aliases() {
        let catalog = ModelCatalog::new();
        let aliases = catalog.list_aliases();
        assert!(aliases.len() >= 20);
        assert_eq!(aliases.get("sonnet").unwrap(), "claude-sonnet-4-6");
        // New aliases
        assert_eq!(aliases.get("grok").unwrap(), "grok-4-0709");
        assert_eq!(aliases.get("jamba").unwrap(), "jamba-1.5-large");
    }

    #[test]
    fn test_find_grok_by_alias() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("grok").unwrap();
        assert_eq!(entry.id, "grok-4-0709");
        assert_eq!(entry.provider, "xai");
    }

    #[test]
    fn test_find_model_by_display_name() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("Grok 4").unwrap();
        assert_eq!(entry.id, "grok-4-0709");
        assert_eq!(entry.provider, "xai");
    }

    #[test]
    fn test_new_providers_in_catalog() {
        let catalog = ModelCatalog::new();
        assert!(catalog.get_provider("perplexity").is_some());
        assert!(catalog.get_provider("cohere").is_some());
        assert!(catalog.get_provider("ai21").is_some());
        assert!(catalog.get_provider("cerebras").is_some());
        assert!(catalog.get_provider("sambanova").is_some());
        assert!(catalog.get_provider("huggingface").is_some());
        assert!(catalog.get_provider("xai").is_some());
        assert!(catalog.get_provider("replicate").is_some());
    }

    #[test]
    fn test_xai_models() {
        let catalog = ModelCatalog::new();
        let xai = catalog.models_by_provider("xai");
        assert_eq!(xai.len(), 9);
        assert!(xai.iter().any(|m| m.id == "grok-4-0709"));
        assert!(xai.iter().any(|m| m.id == "grok-4-fast-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-4-fast-non-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-4-1-fast-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-4-1-fast-non-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-3"));
        assert!(xai.iter().any(|m| m.id == "grok-3-mini"));
        assert!(xai.iter().any(|m| m.id == "grok-2"));
        assert!(xai.iter().any(|m| m.id == "grok-2-mini"));
    }

    #[test]
    fn test_perplexity_models() {
        let catalog = ModelCatalog::new();
        let pp = catalog.models_by_provider("perplexity");
        assert_eq!(pp.len(), 4);
    }

    #[test]
    fn test_cohere_models() {
        let catalog = ModelCatalog::new();
        let co = catalog.models_by_provider("cohere");
        assert_eq!(co.len(), 4);
    }

    #[test]
    fn test_default_creates_valid_catalog() {
        let catalog = ModelCatalog::default();
        assert!(!catalog.list_models().is_empty());
        assert!(!catalog.list_providers().is_empty());
    }

    #[test]
    fn test_merge_adds_new_models() {
        let mut catalog = ModelCatalog::new();
        let before = catalog.models_by_provider("ollama").len();
        catalog.merge_discovered_models(
            "ollama",
            &["codestral:latest".to_string(), "qwen2:7b".to_string()],
        );
        let after = catalog.models_by_provider("ollama").len();
        assert_eq!(after, before + 2);
        // Verify the new models are Local tier with zero cost
        let qwen = catalog.find_model("qwen2:7b").unwrap();
        assert_eq!(qwen.tier, ModelTier::Local);
        assert!((qwen.input_cost_per_m).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_skips_existing() {
        let mut catalog = ModelCatalog::new();
        // "llama3.2" is already a builtin Ollama model
        let before = catalog.list_models().len();
        catalog.merge_discovered_models("ollama", &["llama3.2".to_string()]);
        let after = catalog.list_models().len();
        assert_eq!(after, before); // no new model added
    }

    #[test]
    fn test_merge_updates_model_count() {
        let mut catalog = ModelCatalog::new();
        let before_count = catalog.get_provider("ollama").unwrap().model_count;
        catalog.merge_discovered_models("ollama", &["new-model:latest".to_string()]);
        let after_count = catalog.get_provider("ollama").unwrap().model_count;
        assert_eq!(after_count, before_count + 1);
    }

    #[test]
    fn test_chinese_providers_in_catalog() {
        let catalog = ModelCatalog::new();
        assert!(catalog.get_provider("qwen").is_some());
        assert!(catalog.get_provider("minimax").is_some());
        assert!(catalog.get_provider("zhipu").is_some());
        assert!(catalog.get_provider("zhipu_coding").is_some());
        assert!(catalog.get_provider("moonshot").is_some());
        assert!(catalog.get_provider("qianfan").is_some());
        assert!(catalog.get_provider("bedrock").is_some());
    }

    #[test]
    fn test_chinese_model_aliases() {
        let catalog = ModelCatalog::new();
        assert!(catalog.find_model("kimi").is_some());
        assert!(catalog.find_model("glm").is_some());
        assert!(catalog.find_model("codegeex").is_some());
        assert!(catalog.find_model("ernie").is_some());
        assert!(catalog.find_model("minimax").is_some());
        // MiniMax M2.5 — by exact ID, alias, and case-insensitive
        let m25 = catalog.find_model("MiniMax-M2.5").unwrap();
        assert_eq!(m25.provider, "minimax");
        assert_eq!(m25.tier, ModelTier::Frontier);
        assert!(catalog.find_model("minimax-m2.5").is_some());
        // Default "minimax" alias now points to M2.5
        let default = catalog.find_model("minimax").unwrap();
        assert_eq!(default.id, "MiniMax-M2.5");
        // MiniMax M2.5 Highspeed — by exact ID and aliases
        let hs = catalog.find_model("MiniMax-M2.5-highspeed").unwrap();
        assert_eq!(hs.provider, "minimax");
        assert_eq!(hs.tier, ModelTier::Smart);
        assert!(hs.supports_vision);
        assert!(hs.supports_tools);
        assert!(catalog.find_model("minimax-m2.5-highspeed").is_some());
        assert!(catalog.find_model("minimax-highspeed").is_some());
        // abab7-chat
        let abab7 = catalog.find_model("abab7-chat").unwrap();
        assert_eq!(abab7.provider, "minimax");
        assert!(abab7.supports_vision);
    }

    #[test]
    fn test_bedrock_models() {
        let catalog = ModelCatalog::new();
        let bedrock = catalog.models_by_provider("bedrock");
        assert_eq!(bedrock.len(), 8);
    }

    #[test]
    fn test_set_provider_url() {
        let mut catalog = ModelCatalog::new();
        let old_url = catalog.get_provider("ollama").unwrap().base_url.clone();
        assert_eq!(old_url, OLLAMA_BASE_URL);

        let updated = catalog.set_provider_url("ollama", "http://192.168.1.100:11434/v1");
        assert!(updated);
        assert_eq!(
            catalog.get_provider("ollama").unwrap().base_url,
            "http://192.168.1.100:11434/v1"
        );
    }

    #[test]
    fn test_set_provider_url_unknown() {
        let mut catalog = ModelCatalog::new();
        let initial_count = catalog.list_providers().len();
        let updated = catalog.set_provider_url("my-custom-llm", "http://localhost:9999");
        // Unknown providers are now auto-registered as custom entries
        assert!(updated);
        assert_eq!(catalog.list_providers().len(), initial_count + 1);
        assert_eq!(
            catalog.get_provider("my-custom-llm").unwrap().base_url,
            "http://localhost:9999"
        );
    }

    #[test]
    fn test_apply_url_overrides() {
        let mut catalog = ModelCatalog::new();
        let mut overrides = HashMap::new();
        overrides.insert("ollama".to_string(), "http://10.0.0.5:11434/v1".to_string());
        overrides.insert("vllm".to_string(), "http://10.0.0.6:8000/v1".to_string());
        overrides.insert("nonexistent".to_string(), "http://nowhere".to_string());

        catalog.apply_url_overrides(&overrides);

        assert_eq!(
            catalog.get_provider("ollama").unwrap().base_url,
            "http://10.0.0.5:11434/v1"
        );
        assert_eq!(
            catalog.get_provider("vllm").unwrap().base_url,
            "http://10.0.0.6:8000/v1"
        );
        // lmstudio should be unchanged
        assert_eq!(
            catalog.get_provider("lmstudio").unwrap().base_url,
            LMSTUDIO_BASE_URL
        );
    }

    #[test]
    fn test_codex_provider() {
        let catalog = ModelCatalog::new();
        let codex = catalog.get_provider("codex").unwrap();
        assert_eq!(codex.display_name, "OpenAI Codex");
        assert_eq!(codex.api_key_env, "OPENAI_API_KEY");
        assert!(codex.key_required);
    }

    #[test]
    fn test_codex_models() {
        let catalog = ModelCatalog::new();
        let models = catalog.models_by_provider("codex");
        assert_eq!(models.len(), 3);
        assert!(models.iter().any(|m| m.id == "codex/gpt-5.4"));
        assert!(models.iter().any(|m| m.id == "codex/gpt-4.1"));
        assert!(models.iter().any(|m| m.id == "codex/o4-mini"));
    }

    #[test]
    fn test_codex_aliases() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("codex").unwrap();
        assert_eq!(entry.id, "codex/gpt-5.4");
    }

    #[test]
    fn test_claude_code_provider() {
        let catalog = ModelCatalog::new();
        let cc = catalog.get_provider("claude-code").unwrap();
        assert_eq!(cc.display_name, "Claude Code");
        assert!(!cc.key_required);
    }

    #[test]
    fn test_claude_code_models() {
        let catalog = ModelCatalog::new();
        let models = catalog.models_by_provider("claude-code");
        assert_eq!(models.len(), 3);
        assert!(models.iter().any(|m| m.id == "claude-code/opus"));
        assert!(models.iter().any(|m| m.id == "claude-code/sonnet"));
        assert!(models.iter().any(|m| m.id == "claude-code/haiku"));
    }

    #[test]
    fn test_claude_code_aliases() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("claude-code").unwrap();
        assert_eq!(entry.id, "claude-code/sonnet");
    }

    #[test]
    fn test_qwen_code_provider() {
        let catalog = ModelCatalog::new();
        let qc = catalog.get_provider("qwen-code").unwrap();
        assert_eq!(qc.display_name, "Qwen Code");
        assert!(!qc.key_required);
    }

    #[test]
    fn test_qwen_code_models() {
        let catalog = ModelCatalog::new();
        let models = catalog.models_by_provider("qwen-code");
        assert_eq!(models.len(), 3);
        assert!(models.iter().any(|m| m.id == "qwen-code/qwen3-coder"));
        assert!(models.iter().any(|m| m.id == "qwen-code/qwen-coder-plus"));
        assert!(models.iter().any(|m| m.id == "qwen-code/qwq-32b"));
    }

    #[test]
    fn test_qwen_code_aliases() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("qwen-code").unwrap();
        assert_eq!(entry.id, "qwen-code/qwen3-coder");
    }

    #[test]
    fn test_azure_provider_in_catalog() {
        let catalog = ModelCatalog::new();
        let azure = catalog.get_provider("azure").unwrap();
        assert_eq!(azure.display_name, "Azure OpenAI");
        assert_eq!(azure.api_key_env, "AZURE_OPENAI_API_KEY");
        assert!(azure.key_required);
        assert!(azure.base_url.is_empty()); // user must supply their own
    }

    #[test]
    fn test_azure_models() {
        let catalog = ModelCatalog::new();
        let models = catalog.models_by_provider("azure");
        assert_eq!(models.len(), 4);
        assert!(models.iter().any(|m| m.id == "azure/gpt-4o"));
        assert!(models.iter().any(|m| m.id == "azure/gpt-4o-mini"));
        assert!(models.iter().any(|m| m.id == "azure/gpt-4.1"));
        assert!(models.iter().any(|m| m.id == "azure/gpt-4.1-mini"));
    }

    #[test]
    fn test_azure_model_lookup() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("azure/gpt-4o").unwrap();
        assert_eq!(entry.provider, "azure");
        assert_eq!(entry.display_name, "GPT-4o (Azure)");
        assert_eq!(entry.tier, ModelTier::Smart);
        assert!(entry.supports_tools);
        assert!(entry.supports_vision);
    }
}
