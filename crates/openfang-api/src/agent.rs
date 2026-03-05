use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use openfang_runtime::tool_runner::builtin_tool_definitions;
use openfang_types::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::routes::AppState;

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

#[derive(Debug, Clone, Serialize)]
pub struct SetMcpResponse {
    pub status: String,
    pub servers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillsResponse {
    pub skills: Vec<SkillInfo>,
    pub total: usize,
}

#[derive(Debug, Deserialize)]
pub struct SetSkillsRequest {
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetSkillsResponse {
    pub status: String,
    pub skills: Vec<String>,
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

    let tools: Vec<BuiltinToolInfo> = all_builtins
        .iter()
        .filter(|t| {
            let name = &t.name;
            let allowed = tool_allowlist.is_empty() || tool_allowlist.contains(name);
            let not_blocked = !tool_blocklist.contains(name);
            allowed && not_blocked
        })
        .map(|t| BuiltinToolInfo {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
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

    let skill_registry = match state.kernel.skill_registry.read() {
        Ok(reg) => reg,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to read skill registry: {}", e)})),
            )
        }
    };

    let enabled_skills: std::collections::HashSet<String> =
        entry.manifest.skills.iter().cloned().collect();

    let is_all_mode = entry.manifest.skills.is_empty();

    let all_skills = skill_registry.list();

    let skills: Vec<SkillInfo> = all_skills
        .iter()
        .map(|s| {
            let enabled = if is_all_mode {
                true
            } else {
                enabled_skills.contains(&s.manifest.skill.name)
            };

            SkillInfo {
                name: s.manifest.skill.name.clone(),
                description: s.manifest.skill.description.clone(),
                version: s.manifest.skill.version.clone(),
                author: Some(s.manifest.skill.author.clone()),
                tags: s.manifest.skill.tags.clone(),
                enabled,
            }
        })
        .collect();

    let total = skills.len();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "skills": skills,
            "total": total,
        })),
    )
}

pub async fn set_agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSkillsRequest>,
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

    match state.kernel.set_agent_skills(agent_id, req.skills.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "skills": req.skills,
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
