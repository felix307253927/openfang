use crate::{
    routes::{self, AppState, PatchAgentConfigRequest},
    types::{SpawnRequest, SpawnResponse},
    uni_util::is_in_home_dir,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use openfang_runtime::tool_runner::builtin_tool_definitions;
use openfang_types::agent::{AgentId, AgentManifest, ModelConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct BuiltinToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuiltinToolsResponse {
    pub tools: Vec<BuiltinToolInfo>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerInfo {
    pub name: String,
    pub enabled: bool,
    pub transport: McpTransportInfo,
    pub timeout_secs: u64,
    pub tools_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum McpTransportInfo {
    Stdio { command: String, args: Vec<String> },
    Sse { url: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServersResponse {
    pub servers: Vec<McpServerInfo>,
    pub total: usize,
}

#[derive(Debug, Deserialize)]
pub struct SetMcpRequest {
    pub servers: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetWorkspaceRequest {
    /// New absolute path for the agent workspace.
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetMcpResponse {
    pub status: String,
    pub servers: Vec<String>,
}
#[derive(serde::Deserialize)]
pub struct AgentConfigRequest {
    pub mcp_servers: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    pub tool_allowlist: Option<Vec<String>>,
    pub tool_blocklist: Option<Vec<String>>,
    #[serde(flatten)]
    pub patch: Option<PatchAgentConfigRequest>,
}

/// GET /api/agents — List all agents.
pub async fn list_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Snapshot catalog once for enrichment
    let catalog = state.kernel.model_catalog.read().ok();
    let dm = &state.kernel.config.default_model;

    let entries = state.kernel.registry.list();
    let mut agents_to_kill = Vec::new();

    // Check each agent's workspace and collect agents to kill
    for entry in &entries {
        let should_kill = match &entry.manifest.workspace {
            None => {
                // No workspace configured - kill the agent
                tracing::warn!("Agent {} has no workspace configured", entry.id);
                true
            }
            Some(workspace_path) => {
                if !workspace_path.exists() {
                    tracing::warn!(
                        "Agent {} workspace not found: {}",
                        entry.id,
                        workspace_path.display()
                    );
                    true
                } else if workspace_path.is_dir() {
                    // Check if directory is empty
                    match std::fs::read_dir(workspace_path) {
                        Ok(mut dir_entries) => {
                            let is_empty = dir_entries.next().is_none();
                            if is_empty {
                                tracing::warn!(
                                    "Agent {} workspace is empty: {}",
                                    entry.id,
                                    workspace_path.display()
                                );
                            }
                            is_empty
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Agent {} workspace read error: {} - {}",
                                entry.id,
                                workspace_path.display(),
                                e
                            );
                            true
                        }
                    }
                } else {
                    false
                }
            }
        };

        if should_kill {
            agents_to_kill.push(entry.id);
        }
    }

    // Kill agents with invalid workspaces
    for agent_id in agents_to_kill {
        match state.kernel.kill_agent(agent_id) {
            Ok(()) => {
                tracing::info!("Killed agent {} due to invalid workspace", agent_id);
            }
            Err(e) => {
                tracing::warn!("Failed to kill agent {}: {}", agent_id, e);
            }
        }
    }

    // Re-fetch the list after cleanup
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .into_iter()
        .map(|e| {
            // Resolve "default" provider/model to actual kernel defaults
            let provider =
                if e.manifest.model.provider.is_empty() || e.manifest.model.provider == "default" {
                    dm.provider.as_str()
                } else {
                    e.manifest.model.provider.as_str()
                };
            let model = if e.manifest.model.model.is_empty() || e.manifest.model.model == "default"
            {
                dm.model.as_str()
            } else {
                e.manifest.model.model.as_str()
            };

            // Enrich from catalog
            let (tier, auth_status) = catalog
                .as_ref()
                .map(|cat| {
                    let tier = cat
                        .find_model(model)
                        .map(|m| format!("{:?}", m.tier).to_lowercase())
                        .unwrap_or_else(|| "unknown".to_string());
                    let auth = cat
                        .get_provider(provider)
                        .map(|p| format!("{:?}", p.auth_status).to_lowercase())
                        .unwrap_or_else(|| "unknown".to_string());
                    (tier, auth)
                })
                .unwrap_or(("unknown".to_string(), "unknown".to_string()));

            let ready = matches!(e.state, openfang_types::agent::AgentState::Running)
                && auth_status != "missing";

            serde_json::json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "mode": e.mode,
                "created_at": e.created_at.to_rfc3339(),
                "last_active": e.last_active.to_rfc3339(),
                "model_provider": provider,
                "model_name": model,
                "model_tier": tier,
                "auth_status": auth_status,
                "ready": ready,
                "profile": e.manifest.profile,
                "identity": {
                    "emoji": e.identity.emoji,
                    "avatar_url": e.identity.avatar_url,
                    "color": e.identity.color,
                },
            })
        })
        .collect();

    Json(agents)
}

/// DELETE /api/agents/:id — Kill an agent.
pub async fn kill_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found or already terminated"})),
            )
        }
    };

    match state.kernel.kill_agent(agent_id) {
        Ok(()) => {
            if let Err(e) = remove_agent_workspace(entry.manifest.workspace.as_ref()) {
                tracing::error!("Agent {} workspace removed: {}", agent_id, e);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "killed", "agent_id": id})),
            )
        }
        Err(e) => {
            tracing::warn!("kill_agent failed for {id}: {e}");
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found or already terminated"})),
            )
        }
    }
}

/// GET /api/agents/:id — Get a single agent's detailed info.
pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": entry.id.to_string(),
            "name": entry.name,
            "state": format!("{:?}", entry.state),
            "mode": entry.mode,
            "profile": entry.manifest.profile,
            "created_at": entry.created_at.to_rfc3339(),
            "session_id": entry.session_id.0.to_string(),
            "model": {
                "provider": entry.manifest.model.provider,
                "model": entry.manifest.model.model,
            },
            "capabilities": {
                "tools": entry.manifest.capabilities.tools,
                "network": entry.manifest.capabilities.network,
            },
            "description": entry.manifest.description,
            "tags": entry.manifest.tags,
            "identity": {
                "emoji": entry.identity.emoji,
                "avatar_url": entry.identity.avatar_url,
                "color": entry.identity.color,
            },
            "system_prompt": entry.manifest.model.system_prompt,
            "skills": entry.manifest.skills,
            "skills_mode": if entry.manifest.skills.is_empty() { "all" } else { "allowlist" },
            "mcp_servers": entry.manifest.mcp_servers,
            "mcp_servers_mode": if entry.manifest.mcp_servers.is_empty() { "all" } else { "allowlist" },
            "fallback_models": entry.manifest.fallback_models,
            "tool_allowlist": entry.manifest.tool_allowlist,
            "tool_blocklist": entry.manifest.tool_blocklist,
            "workspace": entry.manifest.workspace,
            "vibe": entry.identity.vibe,
            "archetype": entry.identity.archetype,
        })),
    )
}

/// POST /api/agents — Spawn a new agent.
pub async fn spawn_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SpawnRequest>,
) -> impl IntoResponse {
    // Resolve template name → manifest_toml if template is provided and manifest_toml is empty
    let manifest_toml = req.manifest_toml.clone();

    // SECURITY: Reject oversized manifests to prevent parser memory exhaustion.
    const MAX_MANIFEST_SIZE: usize = 1024 * 1024; // 1MB
    if manifest_toml.len() > MAX_MANIFEST_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Manifest too large (max 1MB)"})),
        );
    }

    // SECURITY: Verify Ed25519 signature when a signed manifest is provided
    if let Some(ref signed_json) = req.signed_manifest {
        match state.kernel.verify_signed_manifest(signed_json) {
            Ok(verified_toml) => {
                // Ensure the signed manifest matches the provided manifest_toml
                if verified_toml.trim() != manifest_toml.trim() {
                    tracing::warn!("Signed manifest content does not match manifest_toml");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(
                            serde_json::json!({"error": "Signed manifest content does not match manifest_toml"}),
                        ),
                    );
                }
            }
            Err(e) => {
                tracing::warn!("Manifest signature verification failed: {e}");
                state.kernel.audit_log.record(
                    "system",
                    openfang_runtime::audit::AuditAction::AuthAttempt,
                    "manifest signature verification failed",
                    format!("error: {e}"),
                );
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "Manifest signature verification failed"})),
                );
            }
        }
    }

    let mut manifest: AgentManifest = match toml::from_str(&manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Invalid manifest TOML: {e}");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid manifest format"})),
            );
        }
    };
    tracing::debug!("Manifest: {:?}", manifest.model);
    if manifest.model.provider.is_empty() || manifest.model.provider == "default" {
        let default_model = state.kernel.config.default_model.clone();
        let mut model = ModelConfig::default();
        model.provider = default_model.provider.clone();
        model.model = default_model.model.clone();
        if !default_model.api_key_env.is_empty() {
            model.api_key_env = Some(default_model.api_key_env.clone());
        }
        if !default_model.base_url.is_none() {
            model.base_url = default_model.base_url.clone();
        }
        manifest.model = model;
    }

    let name = manifest.name.clone();
    match state.kernel.spawn_agent(manifest) {
        Ok(id) => (
            StatusCode::CREATED,
            Json(serde_json::json!(SpawnResponse {
                agent_id: id.to_string(),
                name,
            })),
        ),
        Err(e) => {
            tracing::warn!("Spawn failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Agent spawn failed"})),
            )
        }
    }
}

pub async fn patch_agent_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut req): Json<AgentConfigRequest>,
) -> Response {
    let agent_not_found = || {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        )
            .into_response()
    };
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return agent_not_found();
        }
    };
    if let Some(ref mcp_servers) = req.mcp_servers {
        if state
            .kernel
            .registry
            .update_mcp_servers(agent_id, mcp_servers.clone())
            .is_err()
        {
            return agent_not_found();
        }
    }
    if let Some(ref skills) = req.skills {
        if state
            .kernel
            .registry
            .update_skills(agent_id, skills.clone())
            .is_err()
        {
            return agent_not_found();
        }
    }
    if req.tool_allowlist.is_some() || req.tool_blocklist.is_some() {
        if state
            .kernel
            .registry
            .update_tool_filters(
                agent_id,
                req.tool_allowlist.take(),
                req.tool_blocklist.take(),
            )
            .is_err()
        {
            return agent_not_found();
        }
    }

    if let Some(request) = req.patch.take() {
        return routes::patch_agent_config(State(state), Path(id.clone()), Json(request))
            .await
            .into_response();
    }

    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": "Invalid Patch Agent Config Request"})),
    )
        .into_response()
}

pub async fn get_agent_builtin_tools_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "                        agent ID"})),
            )
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };

    let all_builtins = builtin_tool_definitions();
    let tool_allowlist = &entry.manifest.tool_allowlist;
    let tool_blocklist = &entry.manifest.tool_blocklist;

    let tools: Vec<Value> = all_builtins
        .iter()
        .map(|t| {
            let allowed = tool_allowlist.is_empty() || tool_allowlist.contains(&t.name);
            let not_blocked = !tool_blocklist.contains(&t.name);
            let available = allowed && not_blocked;

            serde_json::json!({
                "available": available,
                "name": t.name.clone(),
                "description": t.description.clone(),
                "input_schema": t.input_schema.clone(),
            })
        })
        .collect();

    let total = tools.len();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "tools": tools,
            "total": total,
        })),
    )
}

pub async fn get_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };

    let config_servers: std::collections::HashSet<String> = state
        .kernel
        .config
        .mcp_servers
        .iter()
        .map(|s| s.name.clone())
        .collect();

    let mcp_tools = match state.kernel.mcp_tools.lock() {
        Ok(tools) => tools,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to lock MCP tools"})),
            )
        }
    };

    let mut server_tools_count: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for tool in mcp_tools.iter() {
        if let Some(server_name) = extract_mcp_server_name(&tool.name) {
            *server_tools_count
                .entry(server_name.to_string())
                .or_insert(0) += 1;
        }
    }

    let enabled_servers: std::collections::HashSet<String> =
        entry.manifest.mcp_servers.iter().cloned().collect();

    let is_all_mode = entry.manifest.mcp_servers.is_empty();

    let servers: Vec<McpServerInfo> = state
        .kernel
        .config
        .mcp_servers
        .iter()
        .map(|s| {
            let enabled = if is_all_mode {
                config_servers.contains(&s.name)
            } else {
                enabled_servers.contains(&s.name)
            };

            let transport = match &s.transport {
                openfang_types::config::McpTransportEntry::Stdio { command, args } => {
                    McpTransportInfo::Stdio {
                        command: command.clone(),
                        args: args.clone(),
                    }
                }
                openfang_types::config::McpTransportEntry::Sse { url } => {
                    McpTransportInfo::Sse { url: url.clone() }
                }
            };

            McpServerInfo {
                name: s.name.clone(),
                enabled,
                transport,
                timeout_secs: s.timeout_secs,
                tools_count: *server_tools_count.get(&s.name).unwrap_or(&0),
            }
        })
        .collect();

    let total = servers.len();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "servers": servers,
            "total": total,
        })),
    )
}

pub async fn set_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetMcpRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };

    match state
        .kernel
        .set_agent_mcp_servers(agent_id, req.servers.clone())
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "servers": req.servers,
            })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

fn extract_mcp_server_name(tool_name: &str) -> Option<&str> {
    if tool_name.starts_with("mcp_") {
        let parts: Vec<&str> = tool_name.splitn(2, '_').collect();
        if parts.len() >= 2 {
            return Some(parts[1]);
        }
    }
    None
}

/// PUT /api/agents/:id/workspace — Change an agent's workspace directory.
///
/// Moves all existing workspace files to the new path, then updates the
/// in-memory registry. The move first tries an atomic `rename` (same
/// filesystem); on cross-device failure it falls back to a recursive copy
/// followed by removal of the old directory.
pub async fn set_agent_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetWorkspaceRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };

    let new_path = std::path::PathBuf::from(&req.path);

    if !new_path.is_absolute() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Workspace path must be absolute"})),
        );
    }

    // Reject path traversal components
    if req.path.contains("..") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Path traversal not allowed"})),
        );
    }

    let old_path = entry.manifest.workspace.clone();

    // Release the registry lock before any blocking file I/O
    drop(entry);

    // No-op when paths are identical
    if old_path.as_deref() == Some(new_path.as_path()) {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "old_workspace": new_path.display().to_string(),
                "new_workspace": new_path.display().to_string(),
                "files_moved": 0,
            })),
        );
    }

    let files_moved: usize = if let Some(ref old) = old_path {
        if old.exists() {
            // Define specific items to move
            let items_to_move = vec![
                "memory",
                "sessions",
                "logs",
                "skills",
                "output",
                "data",
                "AGENT.json",
                "IDENTITY.md",
                "BOOTSTRAP.md",
                "AGENTS.md",
                "MEMORY.md",
                "TOOLS.md",
                "USER.md",
                "SOUL.md",
            ];

            // Check if old directory is under .openfang/workspace
            let path_str = old.to_string_lossy();
            let is_under_openfang_workspace = path_str.contains(".openfang/workspace")
                || path_str.contains(".openfang\\workspace");

            tracing::debug!(
                "Workspace move: old={}, is_under_openfang_workspace={}",
                old.display(),
                is_under_openfang_workspace
            );

            // Ensure target directory exists
            if let Err(e) = std::fs::create_dir_all(&new_path) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::json!({"error": format!("Failed to create target directory: {e}")}),
                    ),
                );
            }

            let mut total_files_moved = 0;

            // Move each specified item
            for item in items_to_move {
                let source = old.join(item);
                let target = new_path.join(item);

                // Skip if source doesn't exist
                if !source.exists() {
                    tracing::debug!("Skipping non-existent item: {}", source.display());
                    continue;
                }

                // Count files before moving
                if source.is_file() {
                    total_files_moved += 1;
                } else if source.is_dir() {
                    total_files_moved += count_files_recursive(&source);
                }

                // Try atomic rename first
                if let Err(_) = std::fs::rename(&source, &target) {
                    // If rename fails (cross-device or other reasons), use recursive move
                    if source.is_dir() {
                        if let Err(e) = move_dir_recursive(&source, &target) {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(
                                    serde_json::json!({"error": format!("Failed to move {}: {e}", item)}),
                                ),
                            );
                        }
                        // Remove the now-empty source directory
                        if let Err(e) = std::fs::remove_dir_all(&source) {
                            tracing::warn!(
                                "Failed to remove source directory {}: {e}",
                                source.display()
                            );
                        }
                    } else {
                        // For files, copy then remove
                        if let Err(e) = std::fs::copy(&source, &target) {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(
                                    serde_json::json!({"error": format!("Failed to copy file {}: {e}", item)}),
                                ),
                            );
                        }
                        if let Err(e) = std::fs::remove_file(&source) {
                            tracing::warn!(
                                "Failed to remove source file {}: {e}",
                                source.display()
                            );
                        }
                    }
                }
            }

            // If old directory is under .openfang/workspace, remove it after moving files
            if is_under_openfang_workspace {
                tracing::info!("Removing old workspace directory: {}", old.display());
                if let Err(e) = std::fs::remove_dir_all(old) {
                    tracing::warn!(
                        "Failed to remove old workspace directory {}: {e}",
                        old.display()
                    );
                } else {
                    tracing::info!(
                        "Successfully removed old workspace directory: {}",
                        old.display()
                    );
                }
            } else {
                tracing::info!(
                    "Keeping old workspace directory (not under .openfang/workspace): {}",
                    old.display()
                );
            }

            total_files_moved
        } else {
            // Old path recorded but missing on disk — just create the new dir
            if let Err(e) = std::fs::create_dir_all(&new_path) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to create workspace: {e}")})),
                );
            }
            0
        }
    } else {
        // No previous workspace — create the new directory
        if let Err(e) = std::fs::create_dir_all(&new_path) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to create workspace: {e}")})),
            );
        }
        0
    };

    if let Err(e) = state
        .kernel
        .registry
        .update_workspace(agent_id, Some(new_path.clone()))
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update registry: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "old_workspace": old_path.as_ref().map(|p| p.display().to_string()),
            "new_workspace": new_path.display().to_string(),
            "files_moved": files_moved,
        })),
    )
}

fn count_files_recursive(dir: &std::path::Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_file() {
                    count += 1;
                } else if ft.is_dir() {
                    count += count_files_recursive(&entry.path());
                }
            }
        }
    }
    count
}

fn move_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        // Skip entries that are on the path to dst (infinite-recursion guard).
        if dst.starts_with(&src_path) {
            continue;
        }
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            move_dir_recursive(&src_path, &dst_path)?;
            // Directory is now empty; remove it.
            std::fs::remove_dir(&src_path)?;
        } else {
            // Try atomic rename first; only copy+remove when cross-device.
            if std::fs::rename(&src_path, &dst_path).is_err() {
                std::fs::copy(&src_path, &dst_path)?;
                std::fs::remove_file(&src_path)?;
            }
        }
    }
    Ok(())
}

pub async fn get_agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    let available = state
        .kernel
        .skill_registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .list()
        .into_iter()
        .filter(|s| s.enabled)
        .map(|s| {
            serde_json::json!({
                "name": s.manifest.skill.name,
                "description": s.manifest.skill.description,
                "enabled": s.enabled,
            })
        })
        .collect::<Vec<_>>();
    let mode = if entry.manifest.skills.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.skills,
            "available": available,
            "mode": mode,
        })),
    )
}

/// POST /api/skills/install_local — Install a skill from a local zip upload.
///
/// Accepts raw zip bytes via the request body. The client must set:
/// - `Content-Type: application/zip` or `application/octet-stream`
/// - `X-Skill-Name` header (optional, used as fallback skill name)
///
/// The zip is extracted into `~/.openfang/skills/{name}/`, then the same
/// format detection + security pipeline as ClawHub install runs.
pub async fn install_local_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    use openfang_skills::openclaw_compat;
    use openfang_skills::verify::{SkillVerifier, WarningSeverity};
    use sha2::{Digest, Sha256};

    const MAX_SKILL_ZIP_SIZE: usize = 50 * 1024 * 1024; // 50 MB

    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Empty request body"})),
        );
    }

    if body.len() > MAX_SKILL_ZIP_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Zip file too large (max 50 MB)"})),
        );
    }

    // SHA256 of uploaded content
    let sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&body);
        hex::encode(hasher.finalize())
    };

    let skill_name_hint = headers
        .get("X-Skill-Name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("local-skill")
        .to_string();

    let skills_dir = state.kernel.config.home_dir.join("skills");

    // Detect content type: zip (PK magic) or SKILL.md (starts with ---)
    let content_str = String::from_utf8_lossy(&body);
    let is_skillmd = content_str.trim_start().starts_with("---");

    // Determine skill name from zip contents or header hint
    let slug = if !is_skillmd && body.len() >= 4 && body[0] == 0x50 && body[1] == 0x4b {
        // Peek into zip to find skill name from skill.toml or SKILL.md frontmatter
        extract_skill_name_from_zip(&body).unwrap_or_else(|| sanitize_name(&skill_name_hint))
    } else {
        sanitize_name(&skill_name_hint)
    };

    // For zip files, extract directly into skills_dir (zip already contains the root folder).
    // For non-zip, create skill_dir as before.
    let skill_dir = skills_dir.join(&slug);

    // Extract content
    if is_skillmd {
        // SKILL.md — create skill_dir and write into it
        if skill_dir.join("skill.toml").exists() {
            if let Err(e) = std::fs::remove_dir_all(&skill_dir) {
                tracing::warn!("Failed to remove old skill dir: {e}");
            }
        }
        if let Err(e) = std::fs::create_dir_all(&skill_dir) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error": format!("Failed to create skill directory: {e}")}),
                ),
            );
        }
        if let Err(e) = std::fs::write(skill_dir.join("SKILL.md"), &*body) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to write SKILL.md: {e}")})),
            );
        }
    } else if body.len() >= 4 && body[0] == 0x50 && body[1] == 0x4b {
        // Zip archive — extract directly into skills_dir root so the zip's
        // internal folder becomes the skill directory (avoids double nesting).
        let zip_root = detect_zip_root_folder(&body);

        // If the zip contains a root folder, clean up the old install
        if let Some(ref root_name) = zip_root {
            let existing = skills_dir.join(root_name);
            if existing.exists() {
                if let Err(e) = std::fs::remove_dir_all(&existing) {
                    tracing::warn!("Failed to remove old skill dir: {e}");
                }
            }
        }

        if let Err(e) = std::fs::create_dir_all(&skills_dir) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error": format!("Failed to create skills directory: {e}")}),
                ),
            );
        }

        let cursor = std::io::Cursor::new(&*body);
        match zip::ZipArchive::new(cursor) {
            Ok(mut archive) => {
                for i in 0..archive.len() {
                    let mut file = match archive.by_index(i) {
                        Ok(f) => f,
                        Err(e) => {
                            tracing::warn!(index = i, error = %e, "Skipping zip entry");
                            continue;
                        }
                    };
                    let Some(enclosed_name) = file.enclosed_name() else {
                        tracing::warn!("Skipping zip entry with unsafe path");
                        continue;
                    };
                    // Skip macOS resource fork metadata (__MACOSX/ and ._xxx files)
                    let path_str = enclosed_name.to_string_lossy();
                    if path_str.starts_with("__MACOSX") || path_str.contains("/__MACOSX") {
                        continue;
                    }
                    if enclosed_name
                        .file_name()
                        .map_or(false, |n| n.to_string_lossy().starts_with("._"))
                    {
                        continue;
                    }
                    // Extract into skills_dir directly (not skill_dir)
                    let out_path = skills_dir.join(enclosed_name);
                    if file.is_dir() {
                        if let Err(e) = std::fs::create_dir_all(&out_path) {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(
                                    serde_json::json!({"error": format!("Failed to create dir: {e}")}),
                                ),
                            );
                        }
                    } else {
                        if let Some(parent) = out_path.parent() {
                            if let Err(e) = std::fs::create_dir_all(parent) {
                                return (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    Json(
                                        serde_json::json!({"error": format!("Failed to create parent dir: {e}")}),
                                    ),
                                );
                            }
                        }
                        let mut out_file = match std::fs::File::create(&out_path) {
                            Ok(f) => f,
                            Err(e) => {
                                return (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    Json(
                                        serde_json::json!({"error": format!("Failed to create file {}: {e}", out_path.display())}),
                                    ),
                                );
                            }
                        };
                        if let Err(e) = std::io::copy(&mut file, &mut out_file) {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(
                                    serde_json::json!({"error": format!("Failed to write file {}: {e}", out_path.display())}),
                                ),
                            );
                        }
                    }
                }
                tracing::info!(slug = %slug, entries = archive.len(), "Extracted local skill zip into skills root");

                // Zip extracted successfully — reload and return immediately
                state.kernel.reload_skills();
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "installed",
                        "slug": slug,
                        "sha256": sha256,
                    })),
                );
            }
            Err(e) => {
                // Fallback: save raw zip into skill_dir
                tracing::warn!(slug = %slug, error = %e, "Failed to read zip, saving raw");
                if let Err(e2) = std::fs::create_dir_all(&skill_dir) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(
                            serde_json::json!({"error": format!("Failed to create skill dir: {e2}")}),
                        ),
                    );
                }
                if let Err(e2) = std::fs::write(skill_dir.join("skill.zip"), &*body) {
                    let _ = std::fs::remove_dir_all(&skill_dir);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": format!("Failed to save raw zip: {e2}")})),
                    );
                }
            }
        }
    } else {
        // Fallback: treat as package.json
        if let Err(e) = std::fs::create_dir_all(&skill_dir) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error": format!("Failed to create skill directory: {e}")}),
                ),
            );
        }
        if let Err(e) = std::fs::write(skill_dir.join("package.json"), &*body) {
            let _ = std::fs::remove_dir_all(&skill_dir);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to write package.json: {e}")})),
            );
        }
    }

    // Format detection + conversion + security scan (same pipeline as clawhub.rs install())
    let mut all_warnings = Vec::new();
    let mut tool_translations = Vec::new();
    let mut is_prompt_only = false;

    let manifest = if is_skillmd || openclaw_compat::detect_skillmd(&skill_dir) {
        match openclaw_compat::convert_skillmd(&skill_dir) {
            Ok(converted) => {
                tool_translations = converted.tool_translations;
                is_prompt_only = converted.manifest.runtime.runtime_type
                    == openfang_skills::SkillRuntime::PromptOnly;

                // Prompt injection scan
                let prompt_warnings = SkillVerifier::scan_prompt_content(&converted.prompt_context);
                if prompt_warnings
                    .iter()
                    .any(|w| w.severity == WarningSeverity::Critical)
                {
                    let critical_msgs: Vec<_> = prompt_warnings
                        .iter()
                        .filter(|w| w.severity == WarningSeverity::Critical)
                        .map(|w| w.message.clone())
                        .collect();
                    let _ = std::fs::remove_dir_all(&skill_dir);
                    return (
                        StatusCode::FORBIDDEN,
                        Json(serde_json::json!({
                            "error": format!("Skill blocked due to prompt injection: {}", critical_msgs.join("; ")),
                        })),
                    );
                }
                all_warnings.extend(prompt_warnings);

                // Write prompt context
                if let Err(e) =
                    openclaw_compat::write_prompt_context(&skill_dir, &converted.prompt_context)
                {
                    let _ = std::fs::remove_dir_all(&skill_dir);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(
                            serde_json::json!({"error": format!("Failed to write prompt_context: {e}")}),
                        ),
                    );
                }

                // Binary dependency check (same as clawhub)
                for bin in &converted.required_bins {
                    if which_check(bin).is_none() {
                        all_warnings.push(openfang_skills::verify::SkillWarning {
                            severity: WarningSeverity::Warning,
                            message: format!("Required binary not found: {bin}"),
                        });
                    }
                }

                converted.manifest
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&skill_dir);
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("Failed to convert SKILL.md: {e}")})),
                );
            }
        }
    } else if openclaw_compat::detect_openclaw_skill(&skill_dir) {
        match openclaw_compat::convert_openclaw_skill(&skill_dir) {
            Ok(m) => m,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&skill_dir);
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("Failed to convert skill: {e}")})),
                );
            }
        }
    } else {
        // let _ = std::fs::remove_dir_all(&skill_dir);
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Downloaded content is not a recognized skill format"
            })),
        );
    };

    // Manifest security scan
    let manifest_warnings = SkillVerifier::security_scan(&manifest);
    all_warnings.extend(manifest_warnings);

    // Write skill.toml (always, same as clawhub)
    // if let Err(e) = openclaw_compat::write_openfang_manifest(&skill_dir, &manifest) {
    //     let _ = std::fs::remove_dir_all(&skill_dir);
    //     return (
    //         StatusCode::INTERNAL_SERVER_ERROR,
    //         Json(serde_json::json!({"error": format!("Failed to write skill.toml: {e}")})),
    //     );
    // }

    // Hot-reload skills
    state.kernel.reload_skills();

    let warnings: Vec<serde_json::Value> = all_warnings
        .iter()
        .map(|w| {
            serde_json::json!({
                "severity": format!("{:?}", w.severity),
                "message": w.message,
            })
        })
        .collect();

    let translations: Vec<serde_json::Value> = tool_translations
        .iter()
        .map(|(from, to)| serde_json::json!({"from": from, "to": to}))
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "installed",
            "name": manifest.skill.name,
            "version": manifest.skill.version,
            "slug": slug,
            "sha256": sha256,
            "is_prompt_only": is_prompt_only,
            "warnings": warnings,
            "tool_translations": translations,
        })),
    )
}

/// Detect the root folder name inside a zip archive (first path component of the first entry).
fn detect_zip_root_folder(bytes: &[u8]) -> Option<String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).ok()?;
    for i in 0..archive.len() {
        let file = archive.by_index(i).ok()?;
        let path = file.enclosed_name()?;
        if let Some(first) = path.components().next() {
            let name = first.as_os_str().to_string_lossy().to_string();
            // Skip __MACOSX metadata folder
            if !name.is_empty() && name != "__MACOSX" {
                return Some(name);
            }
        }
    }
    None
}

/// Extract skill name from a zip archive by peeking at skill.toml or SKILL.md frontmatter.
fn extract_skill_name_from_zip(bytes: &[u8]) -> Option<String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).ok()?;

    // Try skill.toml first
    if let Ok(mut file) = archive.by_name("skill.toml") {
        let mut content = String::new();
        std::io::Read::read_to_string(&mut file, &mut content).ok()?;
        if let Ok(manifest) = toml::from_str::<openfang_skills::SkillManifest>(&content) {
            if !manifest.skill.name.is_empty() {
                return Some(sanitize_name(&manifest.skill.name));
            }
        }
    }

    // Try SKILL.md frontmatter
    if let Ok(mut file) = archive.by_name("SKILL.md") {
        let mut content = String::new();
        std::io::Read::read_to_string(&mut file, &mut content).ok()?;
        // Simple frontmatter name extraction: look for "name: xxx"
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("name:") {
                let name = trimmed.trim_start_matches("name:").trim().trim_matches('"');
                if !name.is_empty() {
                    return Some(sanitize_name(name));
                }
            }
            if trimmed == "---" && !content.starts_with("---") {
                break; // End of frontmatter
            }
        }
    }

    None
}

/// Sanitize a skill name to be filesystem-safe.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Check if a binary is available on PATH (same as clawhub.rs).
fn which_check(name: &str) -> Option<std::path::PathBuf> {
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("where").arg(name).output()
    } else {
        std::process::Command::new("which").arg(name).output()
    };

    match result {
        Ok(output) if output.status.success() => {
            let path_str = String::from_utf8_lossy(&output.stdout);
            let first_line = path_str.lines().next()?;
            Some(std::path::PathBuf::from(first_line.trim()))
        }
        _ => None,
    }
}

fn remove_agent_workspace<P: AsRef<std::path::Path>>(
    agent_workspace: Option<P>,
) -> Result<(), String> {
    if let Some(workspace) = agent_workspace {
        let workspace = workspace.as_ref();
        tracing::info!("Agent_workspace workspace: {}", workspace.display());
        if is_in_home_dir(workspace) {
            tracing::debug!("Removing workspace: {}", workspace.display());
            std::fs::remove_dir_all(&workspace)
                .map_err(|e| format!("Failed to remove workspace: {e}"))?;
        } else {
            [
                "data",
                "logs",
                "memory",
                "output",
                "sessions",
                "skills",
                "AGENT.json",
                "AGENTS.md",
                "BOOTSTRAP.md",
                "IDENTITY.md",
                "MEMORY.md",
                "SOUL.md",
                "TOOLS.md",
                "USER.md",
            ]
            .iter()
            .map(|f| workspace.join(f))
            .for_each(|f| {
                tracing::debug!("Removing file or directory: {}", f.display());
                if f.is_file() {
                    std::fs::remove_file(f)
                        .unwrap_or_else(|e| tracing::warn!("Failed to remove file: {e}"));
                } else if f.is_dir() {
                    std::fs::remove_dir_all(f)
                        .unwrap_or_else(|e| tracing::warn!("Failed to remove directory: {e}"));
                }
            });
        }
    }

    Ok(())
}
