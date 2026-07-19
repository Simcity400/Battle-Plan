//! Exact engine Active-DLC display-name to authenticated VFS-layer join.
//!
//! `dlc_load.json` is deliberately absent from this API. The caller supplies
//! the descriptor roster authenticated by the launch/VFS binding and the two
//! byte-identical post-device engine-manager observations. Unknown, duplicate,
//! missing, or colliding names fail before H4CT COMMIT.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::modset::ModsetReport;
use crate::portable_content_manifest::{normalize_stable_layer_id, PortableLaunchBinding};

pub const EFFECTIVE_RUNTIME_DLC_SCHEMA: &str = "hoi4.effective_runtime_dlc.v1";
pub const REQUESTED_PROFILE_SELECTION_SCHEMA: &str = "hoi4.requested_profile_selection.v1";

/// Launcher-requested DLC evidence. This is intentionally separate from the
/// effective engine roster: `dlc_load.json` is not runtime activation truth.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RequestedProfileSelection {
    pub schema: &'static str,
    pub dlc_load_sha256: String,
    /// Normalized descriptor IDs, sorted by raw UTF-8 byte order.
    pub disabled_dlc_descriptor_ids: Vec<String>,
    /// Stable descriptor-folder IDs requested by the launcher profile.
    pub requested_enabled_dlc_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedDlcDescriptor {
    /// Exact case-sensitive descriptor `name` parsed as a Latin-1 byte string.
    pub display_name: String,
    /// Stable product/layer ID: the descriptor folder basename.
    pub layer_id: String,
    /// Portable descriptor path: `dlc/<layer_id>/<file>.dlc`.
    pub descriptor_id: String,
    /// Raw descriptor-file SHA-256 authenticated by the VFS roster gate.
    pub descriptor_file_sha256: String,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedDlcVfsRoster<'a> {
    pub launch: &'a PortableLaunchBinding,
    pub descriptors: &'a [AuthenticatedDlcDescriptor],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveRuntimeDlc {
    pub schema: &'static str,
    pub active_display_names_engine_order: Vec<String>,
    pub enabled_dlc_ids: BTreeSet<String>,
    pub display_name_to_layer_id: BTreeMap<String, String>,
}

/// Derive requested-profile evidence from the already-resolved report. No file
/// or process access occurs here; the hash comes from the existing identical
/// pair of `dlc_load.json` reads.
pub fn requested_profile_selection_from_modset(
    report: &ModsetReport,
) -> Result<RequestedProfileSelection, String> {
    if !report.resolved {
        return Err("requested profile selection requires a resolved modset".into());
    }
    let dlc_load_sha256 = report
        .dlc_load_sha256
        .clone()
        .filter(|value| valid_sha256(value))
        .ok_or("resolved modset omitted a valid byte-stable dlc_load.json SHA-256")?;

    let installed = normalized_descriptor_set(&report.installed_dlcs, "installed")?;
    let disabled = normalized_descriptor_set(&report.disabled_dlcs, "disabled")?;
    let enabled = normalized_descriptor_set(&report.enabled_dlcs, "enabled")?;
    if !disabled.is_subset(&installed) {
        return Err("requested disabled DLC descriptors are outside the installed census".into());
    }
    let expected_enabled = installed
        .difference(&disabled)
        .cloned()
        .collect::<BTreeSet<_>>();
    if enabled != expected_enabled {
        return Err(
            "requested enabled DLC descriptors do not equal installed minus disabled".into(),
        );
    }

    let mut requested_enabled_dlc_ids = BTreeSet::new();
    for descriptor_id in &enabled {
        let layer_id = descriptor_folder_id(descriptor_id)?;
        if !requested_enabled_dlc_ids.insert(layer_id.to_string()) {
            return Err(format!(
                "requested enabled DLC descriptors repeat stable layer ID {layer_id:?}"
            ));
        }
    }

    Ok(RequestedProfileSelection {
        schema: REQUESTED_PROFILE_SELECTION_SCHEMA,
        dlc_load_sha256,
        disabled_dlc_descriptor_ids: disabled.into_iter().collect(),
        requested_enabled_dlc_ids,
    })
}

/// Materialize the installed descriptor roster from the exact launch-bound
/// bytes already retained by `ModsetReport`. This function performs no file or
/// process access and is safe to call before the report is dropped.
pub fn authenticated_dlc_descriptors_from_modset(
    report: &ModsetReport,
) -> Result<Vec<AuthenticatedDlcDescriptor>, String> {
    if !report.resolved {
        return Err("authenticated DLC descriptors require a resolved modset".into());
    }
    let installed = report
        .installed_dlcs
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if installed.len() != report.installed_dlcs.len()
        || report
            .dlc_descriptor_evidence
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            != installed
    {
        return Err("installed DLC census and descriptor evidence sets disagree".into());
    }
    let mut rows = Vec::with_capacity(installed.len());
    let mut names = BTreeSet::<String>::new();
    let mut layers = BTreeSet::<String>::new();
    for descriptor_id in installed {
        let parts = descriptor_id.split('/').collect::<Vec<_>>();
        if parts.len() != 3 || parts[0] != "dlc" {
            return Err(format!(
                "installed DLC descriptor ID is malformed: {descriptor_id:?}"
            ));
        }
        let evidence = report
            .dlc_descriptor_evidence
            .get(&descriptor_id)
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                format!("installed DLC descriptor lacks launch-bound evidence: {descriptor_id:?}")
            })?;
        if !names.insert(evidence.display_name.clone()) {
            return Err(format!(
                "installed DLC descriptors collide on display name {:?}",
                evidence.display_name
            ));
        }
        if !layers.insert(parts[1].to_string()) {
            return Err(format!(
                "installed DLC descriptors repeat layer ID {:?}",
                parts[1]
            ));
        }
        rows.push(AuthenticatedDlcDescriptor {
            display_name: evidence.display_name.clone(),
            layer_id: parts[1].to_string(),
            descriptor_id,
            descriptor_file_sha256: evidence.descriptor_file_sha256.clone(),
        });
    }
    rows.sort_by(|left, right| left.layer_id.as_bytes().cmp(right.layer_id.as_bytes()));
    Ok(rows)
}

/// Join an engine-manager Active-DLC snapshot to the authenticated DLC VFS set.
///
/// The observed order is retained, while set equality is enforced against the
/// launch binding. The caller owns the double-stable memory-read gate; this
/// function accepts only one of the already-compared name sequences.
pub fn validate_effective_runtime_dlc_roster(
    roster: AuthenticatedDlcVfsRoster<'_>,
    observed_engine_order_names: &[String],
) -> Result<EffectiveRuntimeDlc, String> {
    if roster.launch.enabled_dlc_ids.len() != roster.launch.enabled_dlc_layers.len()
        || roster
            .launch
            .enabled_dlc_layers
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            != roster.launch.enabled_dlc_ids
    {
        return Err("authenticated launch DLC ID/layer sets disagree".into());
    }
    if roster.descriptors.len() != roster.launch.enabled_dlc_ids.len() {
        return Err("authenticated descriptor roster does not cover every DLC VFS layer".into());
    }

    let mut by_name = BTreeMap::<String, String>::new();
    let mut descriptor_ids = BTreeSet::<String>::new();
    let mut layer_ids = BTreeSet::<String>::new();
    for row in roster.descriptors {
        if row.display_name.is_empty()
            || row.layer_id.is_empty()
            || !valid_descriptor_id(&row.descriptor_id, &row.layer_id)
            || !valid_sha256(&row.descriptor_file_sha256)
        {
            return Err(format!(
                "authenticated DLC descriptor row is malformed for {:?}",
                row.layer_id
            ));
        }
        if by_name
            .insert(row.display_name.clone(), row.layer_id.clone())
            .is_some()
        {
            return Err(format!(
                "authenticated DLC descriptors collide on display name {:?}",
                row.display_name
            ));
        }
        if !layer_ids.insert(row.layer_id.clone()) {
            return Err(format!(
                "authenticated DLC descriptors repeat layer ID {:?}",
                row.layer_id
            ));
        }
        if !descriptor_ids.insert(row.descriptor_id.clone()) {
            return Err(format!(
                "authenticated DLC descriptors repeat descriptor ID {:?}",
                row.descriptor_id
            ));
        }
        let bound = roster
            .launch
            .enabled_dlc_layers
            .get(&row.layer_id)
            .ok_or_else(|| {
                format!(
                    "descriptor display name {:?} maps outside authenticated DLC VFS layers",
                    row.display_name
                )
            })?;
        if bound.layer_id != row.descriptor_id {
            return Err(format!(
                "descriptor ID {:?} disagrees with authenticated VFS layer {:?}",
                row.descriptor_id, bound.layer_id
            ));
        }
    }
    if layer_ids != roster.launch.enabled_dlc_ids {
        return Err("authenticated descriptor layer IDs differ from DLC VFS layer IDs".into());
    }

    let mut observed_ids = BTreeSet::<String>::new();
    let mut observed_names = BTreeSet::<String>::new();
    for name in observed_engine_order_names {
        if !observed_names.insert(name.clone()) {
            return Err(format!(
                "engine Active DLC roster repeats display name {name:?}"
            ));
        }
        let layer_id = by_name
            .get(name)
            .ok_or_else(|| format!("engine Active DLC display name has no descriptor: {name:?}"))?;
        if !observed_ids.insert(layer_id.clone()) {
            return Err(format!(
                "engine Active DLC names collide on stable layer ID {layer_id:?}"
            ));
        }
    }
    if observed_ids != roster.launch.enabled_dlc_ids {
        let missing = roster
            .launch
            .enabled_dlc_ids
            .difference(&observed_ids)
            .cloned()
            .collect::<Vec<_>>();
        let unexpected = observed_ids
            .difference(&roster.launch.enabled_dlc_ids)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "engine Active DLC set differs from authenticated DLC VFS layers: missing={missing:?} unexpected={unexpected:?}"
        ));
    }

    Ok(EffectiveRuntimeDlc {
        schema: EFFECTIVE_RUNTIME_DLC_SCHEMA,
        active_display_names_engine_order: observed_engine_order_names.to_vec(),
        enabled_dlc_ids: observed_ids,
        display_name_to_layer_id: by_name,
    })
}

fn valid_descriptor_id(value: &str, layer_id: &str) -> bool {
    let parts = value.split('/').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0] == "dlc"
        && parts[1] == layer_id
        && !parts[2].is_empty()
        && parts[2].ends_with(".dlc")
        && parts.iter().all(|part| !matches!(*part, "" | "." | ".."))
}

fn normalized_descriptor_set(values: &[String], label: &str) -> Result<BTreeSet<String>, String> {
    let mut result = BTreeSet::new();
    for value in values {
        let normalized = normalize_stable_layer_id(value)?;
        descriptor_folder_id(&normalized)?;
        if !result.insert(normalized) {
            return Err(format!("{label} DLC descriptor list contains a duplicate"));
        }
    }
    Ok(result)
}

fn descriptor_folder_id(descriptor_id: &str) -> Result<&str, String> {
    let parts = descriptor_id.split('/').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "dlc"
        || parts[1].is_empty()
        || parts[2].is_empty()
        || !parts[2].ends_with(".dlc")
    {
        return Err(format!(
            "DLC selection is not dlc/<id>/<descriptor>.dlc: {descriptor_id:?}"
        ));
    }
    Ok(parts[1])
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portable_content_manifest::{BoundDlcLayer, PortableLaunchBinding};
    use portable_content_manifest::LayerStorageKind;

    fn launch(ids: &[&str]) -> PortableLaunchBinding {
        let enabled_dlc_layers = ids
            .iter()
            .map(|id| {
                (
                    (*id).to_string(),
                    BoundDlcLayer {
                        layer_id: format!("dlc/{id}/{id}.dlc"),
                        descriptor_sha256: "1".repeat(64),
                        source_container_sha256: None,
                        storage_kind: LayerStorageKind::Directory,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        PortableLaunchBinding {
            executable_sha256: "2".repeat(64),
            ordered_mods: Vec::new(),
            enabled_dlc_ids: enabled_dlc_layers.keys().cloned().collect(),
            enabled_dlc_layers,
            integrated_dlc_layers: BTreeMap::new(),
            localisation_language: "english".into(),
            required_settings: BTreeMap::new(),
        }
    }

    fn descriptor(id: &str, name: &str) -> AuthenticatedDlcDescriptor {
        AuthenticatedDlcDescriptor {
            display_name: name.into(),
            layer_id: id.into(),
            descriptor_id: format!("dlc/{id}/{id}.dlc"),
            descriptor_file_sha256: "a".repeat(64),
        }
    }

    fn requested_report() -> ModsetReport {
        ModsetReport {
            resolved: true,
            errors: Vec::new(),
            exe_path: None,
            exe_size_bytes: None,
            exe_sha256: None,
            user_dir: None,
            user_dir_source: None,
            dlc_load_mtime_unix: Some(1),
            dlc_load_sha256: Some("b".repeat(64)),
            process_start_unix: Some(2),
            enabled_mods: Vec::new(),
            installed_dlcs: vec!["dlc/dlc_a/a.dlc".into(), "dlc/dlc_b/b.dlc".into()],
            disabled_dlcs: vec!["dlc/dlc_b/b.dlc".into()],
            enabled_dlcs: vec!["dlc/dlc_a/a.dlc".into()],
            dlc_display_names: BTreeMap::new(),
            dlc_descriptor_evidence: BTreeMap::new(),
            localisation_language: None,
            display_variant: None,
        }
    }

    #[test]
    fn requested_profile_is_separate_hash_bound_evidence() {
        let requested = requested_profile_selection_from_modset(&requested_report()).unwrap();
        assert_eq!(requested.schema, REQUESTED_PROFILE_SELECTION_SCHEMA);
        assert_eq!(requested.dlc_load_sha256, "b".repeat(64));
        assert_eq!(
            requested.disabled_dlc_descriptor_ids,
            vec!["dlc/dlc_b/b.dlc"]
        );
        assert_eq!(
            requested.requested_enabled_dlc_ids,
            BTreeSet::from(["dlc_a".into()])
        );

        let mut inconsistent = requested_report();
        inconsistent.enabled_dlcs.push("dlc/dlc_b/b.dlc".into());
        assert!(requested_profile_selection_from_modset(&inconsistent).is_err());
        inconsistent = requested_report();
        inconsistent.dlc_load_sha256 = None;
        assert!(requested_profile_selection_from_modset(&inconsistent).is_err());
    }

    #[test]
    fn exact_runtime_names_map_to_the_authenticated_vfs_set() {
        let launch = launch(&["dlc_a", "dlc_b"]);
        let rows = [descriptor("dlc_a", "A"), descriptor("dlc_b", "B")];
        let names = ["B".to_string(), "A".to_string()];
        let effective = validate_effective_runtime_dlc_roster(
            AuthenticatedDlcVfsRoster {
                launch: &launch,
                descriptors: &rows,
            },
            &names,
        )
        .unwrap();
        assert_eq!(effective.active_display_names_engine_order, names);
        assert_eq!(effective.enabled_dlc_ids, launch.enabled_dlc_ids);
    }

    #[test]
    fn unknown_duplicate_missing_and_name_collision_fail_closed() {
        let launch = launch(&["dlc_a", "dlc_b"]);
        let rows = [descriptor("dlc_a", "A"), descriptor("dlc_b", "B")];
        for names in [
            vec!["A".to_string(), "Unknown".to_string()],
            vec!["A".to_string(), "A".to_string()],
            vec!["A".to_string()],
        ] {
            assert!(validate_effective_runtime_dlc_roster(
                AuthenticatedDlcVfsRoster {
                    launch: &launch,
                    descriptors: &rows,
                },
                &names,
            )
            .is_err());
        }

        let colliding = [descriptor("dlc_a", "A"), descriptor("dlc_b", "A")];
        assert!(validate_effective_runtime_dlc_roster(
            AuthenticatedDlcVfsRoster {
                launch: &launch,
                descriptors: &colliding,
            },
            &["A".to_string(), "B".to_string()],
        )
        .is_err());
    }

    #[test]
    fn descriptorless_and_integrated_layers_cannot_enter_the_product_roster() {
        let mut launch = launch(&["dlc_product"]);
        launch.integrated_dlc_layers.insert(
            "integrated".into(),
            BoundDlcLayer {
                layer_id: "integrated_dlc/integrated".into(),
                descriptor_sha256: "3".repeat(64),
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
            },
        );
        let rows = [descriptor("dlc_product", "Product")];
        let effective = validate_effective_runtime_dlc_roster(
            AuthenticatedDlcVfsRoster {
                launch: &launch,
                descriptors: &rows,
            },
            &["Product".to_string()],
        )
        .unwrap();
        assert_eq!(
            effective.enabled_dlc_ids,
            BTreeSet::from(["dlc_product".into()])
        );
    }
}
