use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use construction_buildings_state_damage_live::{DOMAIN_ID, DOMAIN_NAME};
use serde_json::json;

const ASSIGNED_DOMAIN_ID: u16 = 48;

fn main() -> Result<(), String> {
    let out_dir = env::args().nth(1).ok_or_else(|| {
        "usage: construction_buildings_state_damage_domain_allocation_gate <out-dir>".to_string()
    })?;
    let out_dir = PathBuf::from(out_dir);
    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;

    let registry_path = PathBuf::from("replay_native/domain_registry.json");
    let registry_doc_path = PathBuf::from("replay_native/DOMAIN_REGISTRY.md");
    let collector_entrypoint_path = PathBuf::from("replay_native/collector/src/lib.rs");
    let recorder_entrypoint_path = PathBuf::from("replay_native/recorder_host/src/main.rs");
    let collector_registry_path = PathBuf::from("replay_native/collector/src/domain_registry.rs");
    let recorder_registry_path =
        PathBuf::from("replay_native/recorder_host/src/compact_domain_registry.rs");

    let registry_text = fs::read_to_string(&registry_path)
        .map_err(|err| format!("failed to read {}: {err}", registry_path.display()))?;
    let registry: serde_json::Value = serde_json::from_str(&registry_text)
        .map_err(|err| format!("failed to parse {}: {err}", registry_path.display()))?;
    let registry_doc = fs::read_to_string(&registry_doc_path)
        .map_err(|err| format!("failed to read {}: {err}", registry_doc_path.display()))?;
    let collector_entrypoint = fs::read_to_string(&collector_entrypoint_path).map_err(|err| {
        format!(
            "failed to read {}: {err}",
            collector_entrypoint_path.display()
        )
    })?;
    let recorder_entrypoint = fs::read_to_string(&recorder_entrypoint_path).map_err(|err| {
        format!(
            "failed to read {}: {err}",
            recorder_entrypoint_path.display()
        )
    })?;
    let collector_registry = fs::read_to_string(&collector_registry_path).map_err(|err| {
        format!(
            "failed to read {}: {err}",
            collector_registry_path.display()
        )
    })?;
    let recorder_registry = fs::read_to_string(&recorder_registry_path)
        .map_err(|err| format!("failed to read {}: {err}", recorder_registry_path.display()))?;

    let mut errors = Vec::new();
    if DOMAIN_ID != ASSIGNED_DOMAIN_ID {
        errors.push(format!(
            "lane DOMAIN_ID {DOMAIN_ID} != assigned domain id {ASSIGNED_DOMAIN_ID}"
        ));
    }

    let registry_domains = registry_domains(&registry);
    require_registry_domain_present(
        &registry_domains,
        46,
        "air_variant_archetypes_live",
        &mut errors,
    );
    require_registry_domain_status_in(
        &registry_domains,
        47,
        "characters_leaders_cabinet_live",
        &[
            "reserved",
            "lane_ready_shared_registration_blocked_remaining_matrix",
            "lane_ready_shared_adapter_registration_blocked_remaining_matrix",
        ],
        &mut errors,
    );
    require_registry_domain_status_in(
        &registry_domains,
        48,
        DOMAIN_NAME,
        &[
            "reserved",
            "lane_ready_target_state_oracle_blocked",
            "target_state_same_frame_observer_ready_external_save_ui_action_blocked",
        ],
        &mut errors,
    );

    if !registry_doc
        .contains("| 48 | `construction_buildings_state_damage_live` | lane-ready; target-state oracle fixture blocked |")
        && !registry_doc.contains("| 48 | `construction_buildings_state_damage_live` | target-state observer ready; external same-frame save/UI action blocked |")
        && !registry_doc.contains("| 48 | `construction_buildings_state_damage_live` | reserved |")
    {
        errors.push(
            "DOMAIN_REGISTRY.md does not document domain 48 as reserved or target-state blocked"
                .to_string(),
        );
    }

    let collector_registry_has_descriptor =
        registry_helper_has_descriptor(&collector_registry, 48, DOMAIN_NAME);
    let recorder_registry_has_descriptor =
        registry_helper_has_descriptor(&recorder_registry, 48, DOMAIN_NAME);
    if !collector_registry_has_descriptor {
        errors.push("collector registry helper lacks domain-48 descriptor".to_string());
    }
    if !recorder_registry_has_descriptor {
        errors.push("recorder registry helper lacks domain-48 descriptor".to_string());
    }

    let shared_collector_entrypoint_hooked = collector_entrypoint
        .contains("collect_domain48_compact")
        || collector_entrypoint.contains("COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE");
    let shared_recorder_entrypoint_hooked = recorder_entrypoint.contains("decode_domain48_compact")
        || recorder_entrypoint.contains("COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE")
        || recorder_entrypoint.contains("48 => decode_compact_construction");
    if shared_collector_entrypoint_hooked {
        errors.push(
            "shared collector entrypoint already contains a domain-48 implementation hook"
                .to_string(),
        );
    }
    if shared_recorder_entrypoint_hooked {
        errors.push(
            "shared recorder entrypoint already contains a domain-48 implementation hook"
                .to_string(),
        );
    }

    let proof = json!({
        "schema": "construction_buildings_state_damage_live_domain_allocation_gate_v2",
        "domain": {
            "name": DOMAIN_NAME,
            "assigned_id": DOMAIN_ID,
        },
        "gate_ok": errors.is_empty(),
        "registry_files": {
            "json": path_slash(&registry_path),
            "markdown": path_slash(&registry_doc_path),
            "collector_registry_helper": path_slash(&collector_registry_path),
            "recorder_registry_helper": path_slash(&recorder_registry_path),
        },
        "shared_entrypoints": {
            "collector": path_slash(&collector_entrypoint_path),
            "recorder": path_slash(&recorder_entrypoint_path),
            "collector_domain48_implementation_hooked": shared_collector_entrypoint_hooked,
            "recorder_domain48_implementation_hooked": shared_recorder_entrypoint_hooked,
        },
        "registry_evidence": {
            "domains": registry_domains,
            "collector_registry_has_domain48_descriptor": collector_registry_has_descriptor,
            "recorder_registry_has_domain48_descriptor": recorder_registry_has_descriptor,
        },
        "rejected_or_reserved_ids": [
            {
                "domain_id": 46,
                "status": "rejected_reserved_or_integrated",
                "owner": "air_variant_archetypes_live",
                "evidence": "domain_registry.json assigns id 46 to air_variant_archetypes_live"
            },
            {
                "domain_id": 47,
                "status": "rejected_reserved_or_lane_ready",
                "owner": "characters_leaders_cabinet_live",
                "evidence": "domain_registry.json status is reserved or lane-ready for characters_leaders_cabinet_live"
            }
        ],
        "assigned_id_check": {
            "domain_id": ASSIGNED_DOMAIN_ID,
            "registry_assigned_to_lane": registry_has_domain_status_in(
                &registry_domains,
                48,
                DOMAIN_NAME,
                &[
                    "reserved",
                    "lane_ready_target_state_oracle_blocked",
                    "target_state_same_frame_observer_ready_external_save_ui_action_blocked",
                ],
            ),
            "shared_entrypoint_hooks_absent": !shared_collector_entrypoint_hooked && !shared_recorder_entrypoint_hooked,
            "meaning": "registry descriptor is present; production collector/recorder hooks remain slot-gated"
        },
        "errors": errors,
    });

    let proof_path = out_dir.join("domain_allocation_gate.json");
    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", proof_path.display()))?;
    if proof["gate_ok"] != serde_json::Value::Bool(true) {
        return Err(format!(
            "domain allocation gate failed; wrote {}",
            proof_path.display()
        ));
    }
    Ok(())
}

fn registry_domains(registry: &serde_json::Value) -> Vec<serde_json::Value> {
    registry
        .get("domains")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|domain| {
            Some(json!({
                "id": domain.get("id")?.as_u64()?,
                "name": domain.get("name")?.as_str()?,
                "status": domain.get("status")?.as_str()?,
            }))
        })
        .collect()
}

fn require_registry_domain_status_in(
    domains: &[serde_json::Value],
    domain_id: u16,
    name: &str,
    statuses: &[&str],
    errors: &mut Vec<String>,
) {
    if !registry_has_domain_status_in(domains, domain_id, name, statuses) {
        errors.push(format!(
            "domain_registry.json missing id {domain_id} name {name} with status in {:?}",
            statuses
        ));
    }
}

fn require_registry_domain_present(
    domains: &[serde_json::Value],
    domain_id: u16,
    name: &str,
    errors: &mut Vec<String>,
) {
    if !domains.iter().any(|domain| {
        domain.get("id").and_then(serde_json::Value::as_u64) == Some(domain_id.into())
            && domain.get("name").and_then(serde_json::Value::as_str) == Some(name)
    }) {
        errors.push(format!(
            "domain_registry.json missing id {domain_id} name {name}"
        ));
    }
}

fn registry_has_domain_status_in(
    domains: &[serde_json::Value],
    domain_id: u16,
    name: &str,
    statuses: &[&str],
) -> bool {
    domains.iter().any(|domain| {
        domain.get("id").and_then(serde_json::Value::as_u64) == Some(domain_id.into())
            && domain.get("name").and_then(serde_json::Value::as_str) == Some(name)
            && domain
                .get("status")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|status| statuses.contains(&status))
    })
}

fn registry_helper_has_descriptor(source: &str, domain_id: u16, name: &str) -> bool {
    let id_literal = domain_id.to_string();
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        if line.trim() != id_literal && !line.trim().starts_with(&(id_literal.clone() + ",")) {
            continue;
        }
        let nearby: String = lines.by_ref().take(8).collect::<Vec<_>>().join("\n");
        if nearby.contains(name) {
            return true;
        }
    }
    source.contains(&format!("{domain_id},")) && source.contains(name)
}

fn path_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
