/*
 * @Author             : Felix
 * @Email              : 307253927@qq.com
 * @Date               : 2026-03-09 14:47:41
 * @LastEditors        : Felix
 * @LastEditTime       : 2026-03-20 14:43:25
 */

use crate::routes::AppState;
use crate::uni_util::{check_axum_response_to_result, UniError, UniResult};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use openfang_types::model_catalog::ModelCatalogEntry;
use openfang_types::model_catalog::ModelTier;
use serde_json::Value;
use std::sync::Arc;

pub async fn update_models(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> UniResult {
    // Get the models array from the request body
    let models_array = match body.as_array() {
        Some(arr) => arr,
        None => {
            return UniError::InvalidParameter(
                "Request body must be an array of model objects".to_string(),
            )
            .into();
        }
    };

    // Get write access to the model catalog
    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    // Get current unigpt models from catalog
    let current_unigpt_models: std::collections::HashMap<String, String> = catalog
        .list_models()
        .iter()
        .filter(|m| m.provider == "unigpt" && m.tier == ModelTier::Custom)
        .map(|m| (m.id.clone(), m.display_name.clone()))
        .collect();

    // Track models from request
    let mut request_model_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut added_count = 0;
    let mut updated_count = 0;

    // Step 1: Process models from request
    for model_data in models_array {
        // Extract model parameters
        let id = model_data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if id.is_empty() {
            continue; // Skip models without id
        }

        let name = model_data
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&id)
            .to_string();

        let context_window = model_data
            .get("context_window")
            .and_then(|v| v.as_u64())
            .unwrap_or(128_000);

        let max_output_tokens = model_data
            .get("max_output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(8_192);

        // Add to request model IDs set
        request_model_ids.insert(id.clone());

        // Check if model exists in catalog
        if let Some(existing_name) = current_unigpt_models.get(&id) {
            // Model exists, check if name is the same
            if *existing_name == name {
                // Name is the same, do nothing
                continue;
            } else {
                // Name is different, need to update
                // First remove the existing model
                catalog.remove_custom_model(&id);
                // Then add the updated model
                let entry = ModelCatalogEntry {
                    id: id.clone(),
                    display_name: name,
                    provider: "unigpt".to_string(),
                    tier: ModelTier::Custom,
                    context_window,
                    max_output_tokens,
                    input_cost_per_m: 0.0,
                    output_cost_per_m: 0.0,
                    supports_tools: true,
                    supports_vision: false,
                    supports_streaming: true,
                    aliases: vec![],
                };
                if catalog.add_custom_model(entry) {
                    updated_count += 1;
                }
            }
        } else {
            // Model doesn't exist, add it
            let entry = ModelCatalogEntry {
                id: id.clone(),
                display_name: name,
                provider: "unigpt".to_string(),
                tier: ModelTier::Custom,
                context_window,
                max_output_tokens,
                input_cost_per_m: 0.0,
                output_cost_per_m: 0.0,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
                aliases: vec![],
            };
            if catalog.add_custom_model(entry) {
                added_count += 1;
            }
        }
    }

    // Step 2: Remove models that are in catalog but not in request
    let mut removed_count = 0;
    for (model_id, _) in current_unigpt_models {
        if !request_model_ids.contains(&model_id) && catalog.remove_custom_model(&model_id) {
            removed_count += 1;
        }
    }

    // Persist changes to disk
    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
    }

    // Return success response
    UniResult::Ok(serde_json::json!({
        "status": "success",
        "removed_count": removed_count,
        "added_count": added_count,
        "updated_count": updated_count
    }))
}
pub async fn patch_provider(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> UniResult {
    let provider = match body.get("provider").and_then(|v| v.as_str()) {
        Some(provider) => provider.to_string(),
        None => return UniError::InvalidParameter("provider is required".to_string()).into(),
    };
    if let Some(key) = body.get("key").and_then(|v| v.as_str()) {
        if !key.is_empty() {
            let res = crate::routes::set_provider_key(
                State(state.clone()),
                Path(provider.to_string()),
                Json(body.clone()),
            )
            .await
            .into_response();
            if let UniResult::Err(e) = check_axum_response_to_result(res).await {
                return e.into();
            }
        }
    }
    if let Some(url) = body.get("base_url").and_then(|v| v.as_str()) {
        if !url.is_empty() {
            let res = crate::routes::set_provider_url(
                State(state.clone()),
                Path(provider.to_string()),
                Json(body.clone()),
            )
            .await
            .into_response();
            if let UniResult::Err(e) = check_axum_response_to_result(res).await {
                return e.into();
            }
        }
    }
    UniResult::Ok(serde_json::json!({"status": "success"}))
}

pub async fn set_api_key(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> UniResult {
    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("unigpt")
        .to_string();
    let api_key = body
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if api_key.is_empty() {
        return UniError::InvalidParameter("apiKey is required".to_string()).into();
    }
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let api_key_env = catalog
        .get_provider(&provider)
        .map(|p| p.api_key_env.clone())
        .unwrap_or_else(|| {
            // Custom provider — derive env var: MY_PROVIDER → MY_PROVIDER_API_KEY
            format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"))
        });
    println!("api_key_env: {:?}", api_key_env);
    if provider == "unigpt" {
        std::env::set_var("UNIGPT_API_KEY", &api_key);
    }
    // Store in vault (best-effort — no-op if vault not initialized)
    state.kernel.store_credential(&api_key_env, &api_key);

    // Write to secrets.env file (dual-write for backward compat / vault corruption recovery)
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = crate::routes::write_secret_env(&secrets_path, &api_key_env, &api_key) {
        return UniError::InternalError(format!("Failed to write secrets.env: {e}")).into();
    }
    UniResult::Ok(serde_json::json!({
        "status": "success",
        "message": format!("{} apiKey set successfully", provider)
    }))
}

pub async fn get_default_model(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let default_model = state.kernel.config.default_model.clone();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "success",
            "default_model": {
                "name": default_model.model,
                "provider": default_model.provider,
            }
        })),
    )
}
