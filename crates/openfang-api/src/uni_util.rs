/*
 * @Author             : Felix
 * @Email              : 307253927@qq.com
 * @Date               : 2026-03-19 14:08:38
 * @LastEditors        : Felix
 * @LastEditTime       : 2026-03-19 16:54:10
 */

use openfang_skills::openclaw_compat;
use openfang_skills::registry::SkillRegistry;
use openfang_skills::verify::SkillVerifier;
use openfang_skills::SkillError;
use openfang_types::config::openfang_home_dir;
use std::path::Path;
use tracing::{info, warn};

/// Check if the path is in the home directory.
/// if the path is in the home directory, return true.
/// if the path is not exist, return false.
pub fn is_in_home_dir<P: AsRef<Path>>(path: P) -> bool {
    let canonical_home = match openfang_home_dir().canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };

    let canonical_path = match path.as_ref().canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };

    canonical_path.starts_with(canonical_home)
}

/// Reload skills from the skills directory.
/// If the skills directory does not exist, return 0.
/// If the skills directory exists, reload all skills from the directory.
pub fn reload_skills(skill_registry: &mut SkillRegistry) -> Result<usize, SkillError> {
    if !skill_registry.skills_dir.exists() {
        return Ok(0);
    }

    let mut count = 0;
    let entries = std::fs::read_dir(&skill_registry.skills_dir)?;

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

        match skill_registry.load_skill(&path) {
            Ok(_) => count += 1,
            Err(e) => {
                warn!("Failed to load skill at {}: {e}", path.display());
            }
        }
    }

    info!(
        "Loaded {count} skills from {}",
        skill_registry.skills_dir.display()
    );
    Ok(count)
}

#[test]
fn test_is_in_home_dir() {
    let home_dir = openfang_home_dir();
    println!("home_dir: {:?}", home_dir);
    assert!(is_in_home_dir(&home_dir), "Path1 is in home directory");
    assert!(!is_in_home_dir("~"), "Path2 is not in home directory");
}
