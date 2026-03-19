/*
* @Author             : Felix
* @Email              : 307253927@qq.com
* @Date               : 2026-03-19 17:02:07
 * @LastEditors        : Felix
 * @LastEditTime       : 2026-03-19 17:30:33
*/
use axum::{extract::State, response::IntoResponse, Json};
use openfang_skills::openclaw_compat;
use openfang_skills::verify::SkillVerifier;
use reqwest::StatusCode;
use std::sync::Arc;
use tracing::{info, warn};

use crate::routes::AppState;

/// Reload skills 会重新加载 skills 目录下的所有技能。
/// 会重新解析所有技能的 Skill.md 文件。
pub async fn reload_skills(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let skill_registry = match state.kernel.skill_registry.write() {
        Ok(registry) => registry,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Failed to reload skills"})),
            )
        }
    };

    if !skill_registry.skills_dir.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No skills directory found"})),
        );
    }

    let mut count = 0;
    let entries = match std::fs::read_dir(&skill_registry.skills_dir) {
        Ok(entries) => entries,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Failed to reload skills: {:?}", e)})),
            );
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Auto-detect SKILL.md and convert to skill.toml + prompt_context.md
        if openclaw_compat::detect_skillmd(&path) {
            match openclaw_compat::convert_skillmd(&path) {
                Ok(converted) => {
                    // SECURITY: Scan prompt content for injection attacks
                    // before accepting the skill. 341 malicious skills were
                    // found on ClawHub — block critical threats at load time.
                    let warnings = SkillVerifier::scan_prompt_content(&converted.prompt_context);
                    let has_critical = warnings.iter().any(|w| {
                        matches!(
                            w.severity,
                            openfang_skills::verify::WarningSeverity::Critical
                        )
                    });
                    if has_critical {
                        warn!(
                            skill = %converted.manifest.skill.name,
                            "BLOCKED: SKILL.md contains critical prompt injection patterns"
                        );
                        for w in &warnings {
                            warn!("  [{:?}] {}", w.severity, w.message);
                        }
                        continue;
                    }
                    if !warnings.is_empty() {
                        for w in &warnings {
                            warn!(
                                skill = %converted.manifest.skill.name,
                                "[{:?}] {}",
                                w.severity,
                                w.message
                            );
                        }
                    }

                    info!(
                        skill = %converted.manifest.skill.name,
                        "Auto-converting SKILL.md to OpenFang format"
                    );
                    if let Err(e) =
                        openclaw_compat::write_openfang_manifest(&path, &converted.manifest)
                    {
                        warn!("Failed to write skill.toml for {}: {e}", path.display());
                        continue;
                    }
                    if let Err(e) =
                        openclaw_compat::write_prompt_context(&path, &converted.prompt_context)
                    {
                        warn!(
                            "Failed to write prompt_context.md for {}: {e}",
                            path.display()
                        );
                    }
                    // Fall through to load the newly written skill.toml
                }
                Err(e) => {
                    warn!("Failed to convert SKILL.md at {}: {e}", path.display());
                    continue;
                }
            }
        } else {
            continue;
        }
        count += 1;
    }
    // Drop the write lock to allow other threads to access the registry
    drop(skill_registry);
    // Reload skills from the kernel
    state.kernel.reload_skills();
    (StatusCode::OK, Json(serde_json::json!({"count": count})))
}
