use compact_contract::{
    compact_status_for_global_interpretation, parse_compact_record_header,
    COMPACT_STATUS_FAILURE_MASK,
};
use compact_contract::campaign_identity::ValidatedCid1;
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode, StreamValidationError,
};
use serde_json::{json, Value};
use snapshot::sha256::Sha256;
use snapshot::{Container, ExpectedIdentity};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::Component;
use std::path::{Path, PathBuf};

const RECORD_VERSION: u16 = 1;
const RECORD_HEADER_LEN: usize = 32;
const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
/// Reserved instrumentation-probe domain: the recorder excludes its records
/// from ALL stream accounting; this independent scan must mirror that.
const RC_PROBE_DOMAIN_ID: u16 = 250;
const RECORD_TYPE_FRAME_END: u16 = 2;
const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
#[cfg(test)]
const DOMAIN_FRAME: u16 = 1000;

/// This legacy opt-in is deliberately narrow: its name permits only partial
/// data and source-unavailable evidence. Every other canonical failure remains
/// fatal, including worker panic, bridge failure, invalid control, and no clock.
#[cfg(test)]
const ALLOW_PARTIAL_OR_SOURCE_FAILURE_MASK: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA
    | compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;

#[cfg(test)]
fn rejected_global_failure_bits(status_flags: u32, allow_partial_or_source: bool) -> u32 {
    let allowed = if allow_partial_or_source {
        ALLOW_PARTIAL_OR_SOURCE_FAILURE_MASK
    } else {
        0
    };
    status_flags & COMPACT_STATUS_FAILURE_MASK & !allowed
}

fn rejected_domain_failure_bits(
    domain_id: u16,
    raw_status_flags: u32,
    _allow_partial_or_source: bool,
) -> u32 {
    compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
        domain_id,
        raw_status_flags,
    )
}

#[derive(Debug, Clone)]
pub struct GateOptions {
    pub run_dir: PathBuf,
    pub required_domains: Vec<String>,
    pub min_game_hours: Option<i64>,
    pub max_stream_bytes: u64,
    pub max_record_bytes: u64,
    pub max_bytes_per_frame: u64,
    pub require_replay_verify: bool,
    pub allow_bridge_roots: bool,
    pub allow_partial_or_source_failure: bool,
    pub strict_semantic_units: bool,
    pub write_report: Option<PathBuf>,
}

impl Default for GateOptions {
    fn default() -> Self {
        Self {
            run_dir: PathBuf::new(),
            required_domains: Vec::new(),
            min_game_hours: None,
            max_stream_bytes: 128 * 1024 * 1024,
            max_record_bytes: 8 * 1024 * 1024,
            max_bytes_per_frame: 2 * 1024 * 1024,
            require_replay_verify: true,
            allow_bridge_roots: false,
            allow_partial_or_source_failure: false,
            strict_semantic_units: true,
            write_report: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GateReport {
    pub passed: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub details: Value,
}

impl GateReport {
    pub fn as_json(&self) -> Value {
        json!({
            "gate": "compact_replay_verification_gates",
            "passed": self.passed,
            "errors": self.errors,
            "warnings": self.warnings,
            "details": self.details,
        })
    }
}

/// Refuse a report destination that resolves to any artifact used as gate
/// authority. The comparison canonicalizes existing paths and parent
/// directories, so aliases through `.`/`..` or directory symlinks cannot
/// overwrite a just-verified input after the verdict was computed.
pub fn validate_report_output_path(options: &GateOptions) -> Result<(), String> {
    let Some(report_path) = options.write_report.as_deref() else {
        return Ok(());
    };
    let manifest_path = options.run_dir.join("native_session_manifest.json");
    let (_, manifest) = read_json_document(&manifest_path).map_err(|error| {
        format!(
            "cannot validate --write-report isolation because manifest {} is unreadable: {error}",
            manifest_path.display()
        )
    })?;
    let protected = [
        ("manifest", manifest_path),
        (
            "raw stream",
            required_run_file(&options.run_dir, &manifest, "raw_stream")?,
        ),
        (
            "replay container",
            required_run_file(&options.run_dir, &manifest, "replay_file")?,
        ),
    ];
    let report_identity = canonical_path_for_comparison(report_path).map_err(|error| {
        format!(
            "cannot validate --write-report path {}: {error}",
            report_path.display()
        )
    })?;
    for (role, path) in protected {
        let protected_identity = canonical_path_for_comparison(&path).map_err(|error| {
            format!(
                "cannot validate --write-report isolation from {role} {}: {error}",
                path.display()
            )
        })?;
        if equivalent_paths(&report_identity, &protected_identity) {
            return Err(format!(
                "--write-report path {} aliases protected {role} input {}",
                report_path.display(),
                path.display()
            ));
        }
    }
    Ok(())
}

fn canonical_path_for_comparison(path: &Path) -> Result<PathBuf, String> {
    if let Ok(canonical) = fs::canonicalize(path) {
        return Ok(canonical);
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("{} does not name a file destination", path.display()))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        format!(
            "cannot canonicalize parent directory {}: {error}",
            parent.display()
        )
    })?;
    Ok(canonical_parent.join(file_name))
}

fn equivalent_paths(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

#[derive(Debug, Clone, Copy)]
struct DomainContract {
    id: u16,
    name: &'static str,
    requires_data: bool,
    required_units: &'static [&'static str],
    /// Data-unit counters that MUST be absent-or-zero for the gate to pass.
    /// A nonzero value means a fail-closed sub-slice health flag fired (e.g.
    /// the domain-46 ace roster was source-unavailable / partial / had its
    /// wing-assignment scan blocked on at least one frame). Present but zero
    /// is fine; nonzero fails unless a tolerated profile is in effect.
    zero_units: &'static [&'static str],
    /// Diagnostic "work performed" counters that make a zero-emission frame a
    /// HARD failure — if ANY of these is > 0 while the domain emitted no rows
    /// (`row_count_total == 0`), the reader did substantial work and then threw
    /// every row away, which is silent data loss masquerading as a tolerated
    /// PARTIAL. This fires EVEN under `allow_partial_or_source_failure` (a
    /// partial-tolerant profile still may not tolerate 39.5M rows in / 0 out).
    /// Motivating defect: d47 characters adapter processed millions of rows and
    /// emitted zero for a whole session behind a rowless-PARTIAL frame
    /// (2026-07-10; see extractor/documents/CONFIRMED_COMPLETE_DESIGN.md).
    work_implies_rows_units: &'static [&'static str],
    shared_recorder_known: bool,
}

const DOMAIN_CONTRACTS: &[DomainContract] = &[
    DomainContract {
        id: 10,
        name: "fuel_live",
        requires_data: true,
        required_units: &["fuel_country_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 11,
        name: "production_live",
        requires_data: true,
        required_units: &[
            "production_country_rows",
            "stockpile_rows",
            "military_line_rows",
            "naval_line_rows",
            "construction_queue_rows",
        ],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 12,
        name: "focus_live",
        requires_data: true,
        required_units: &["focus_country_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 13,
        name: "tech_live",
        requires_data: true,
        required_units: &["tech_country_rows", "active_research_slot_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 14,
        name: "politics_live",
        requires_data: true,
        required_units: &["politics_country_rows", "popularity_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 15,
        name: "politics_residuals",
        requires_data: true,
        required_units: &["politics_residual_country_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 16,
        name: "decisions_live",
        requires_data: true,
        required_units: &["decision_country_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 17,
        name: "diplomacy_live",
        requires_data: true,
        required_units: &["diplomacy_country_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 20,
        name: "divisions",
        requires_data: true,
        required_units: &["division_rows", "division_root_rows", "division_owner_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 21,
        name: "land_combats",
        requires_data: true,
        required_units: &["land_combat_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 22,
        name: "map_control",
        requires_data: true,
        required_units: &["map_control_state_rows", "map_control_province_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 23,
        name: "map_detail",
        requires_data: true,
        required_units: &["map_detail_state_rows", "map_detail_province_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 24,
        name: "weather_live",
        requires_data: true,
        required_units: &["weather_region_rows", "weather_observed_region_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 26,
        name: "division_equipment",
        requires_data: true,
        required_units: &["division_equipment_rows", "division_equipment_root_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 27,
        name: "army_orders",
        requires_data: true,
        required_units: &["army_order_rows", "army_order_root_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 28,
        name: "oob_command",
        requires_data: true,
        required_units: &["oob_command_rows", "oob_command_root_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 29,
        name: "ships",
        requires_data: true,
        required_units: &["ship_rows", "ship_observed_ship_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 30,
        name: "air_wings",
        requires_data: true,
        required_units: &["air_wing_rows", "air_wing_observed_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 31,
        name: "air_region_activity",
        requires_data: true,
        required_units: &[
            "air_region_activity_rows",
            "air_region_activity_observed_bucket_rows",
        ],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 32,
        name: "air_wing_loadouts",
        requires_data: true,
        required_units: &["air_wing_loadout_rows", "air_wing_loadout_equipment_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 33,
        name: "air_variant_defs",
        requires_data: true,
        required_units: &["air_variant_def_rows", "air_variant_def_module_token_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 34,
        name: "air_base_status",
        requires_data: true,
        required_units: &[
            "air_base_status_rows",
            "air_base_status_observed_air_base_rows",
        ],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 35,
        name: "air_variant_stats",
        requires_data: true,
        required_units: &[
            "air_variant_stats_rows",
            "air_variant_stats_observed_variant_rows",
        ],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 36,
        name: "air_variant_reserves",
        requires_data: true,
        required_units: &[
            "air_variant_reserve_rows",
            "air_variant_reserve_observed_stockpile_rows",
        ],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 37,
        name: "stockpile_variant_defs",
        requires_data: true,
        required_units: &[
            "stockpile_variant_def_rows",
            "stockpile_variant_def_module_token_rows",
        ],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 38,
        name: "air_superiority_inputs",
        requires_data: true,
        required_units: &[
            "air_superiority_input_rows",
            "air_superiority_input_observed_wing_rows",
        ],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 39,
        name: "air_variant_upgrades",
        requires_data: true,
        required_units: &[
            "air_variant_upgrade_rows",
            "air_variant_upgrade_observed_variant_rows",
        ],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 40,
        name: "stockpile_variant_upgrades",
        requires_data: true,
        required_units: &[
            "stockpile_variant_upgrade_rows",
            "stockpile_variant_upgrade_entry_rows",
        ],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 41,
        name: "projects_live",
        requires_data: true,
        required_units: &["project_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 42,
        name: "air_combat_losses_live",
        requires_data: true,
        required_units: &["air_strength_delta_rows", "air_strength_observed_wing_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 43,
        name: "market_trade_live",
        requires_data: true,
        required_units: &[],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 44,
        name: "supply_logistics_attrition_live",
        requires_data: true,
        required_units: &[
            "supply_province_logistics_rows",
            "supply_division_supply_rows",
            "supply_csupply_input_rows",
            "supply_csupply_efficiency_rows",
        ],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    // Domain 47 (`characters_leaders_cabinet_live`) folds as a FULL
    // REPLACEMENT, so a rowless frame deletes the world's characters. On
    // 2026-07-10 a single dead cached character root failed every frame for a
    // whole session while the adapter kept processing ~11k characters/frame —
    // 39.5M rows in, 0 out — behind a tolerated rowless PARTIAL. The
    // work_implies_rows_units below make that un-representable: if the adapter
    // reports ANY built rows but the domain emitted none, the gate fails even
    // under a partial-tolerant profile. requires_data holds (it emits a record
    // per hour); the invariant is about ROWS, not records.
    DomainContract {
        id: 47,
        name: "characters_leaders_cabinet_live",
        requires_data: true,
        required_units: &[],
        zero_units: &[],
        work_implies_rows_units: &[
            "domain47_adapter_character_rows",
            "domain47_adapter_advisor_roster_rows",
            "domain47_adapter_country_leader_role_rows",
            "domain47_adapter_unit_leader_rows",
            // G1-residue finding 5: previously-missing output families — an
            // active-ruler-only or trait-only session must not bypass the
            // invariant.
            "domain47_adapter_active_ruler_rows",
            "domain47_adapter_unit_leader_trait_rows",
        ],
        shared_recorder_known: true,
    },
    // Domain 46 (`air_variant_archetypes`) carries the wave-4 `ACE1`
    // ace-roster trailer. Its required units prove the ace slice actually
    // populated; its zero_units are the fail-closed per-frame health flags —
    // a nonzero count means the ace roster was source-unavailable, incomplete
    // (partial), or had its wing-assignment scan blocked on at least one
    // frame, which must fail the gate (Codex DEFECT: gate could stay green
    // while the ace slice silently died).
    DomainContract {
        id: 46,
        name: "air_variant_archetypes",
        requires_data: true,
        required_units: &[
            "air_variant_archetype_rows",
            "air_ace_roster_rows",
            "air_ace_roster_observed_ace_rows",
        ],
        zero_units: &[
            "air_ace_roster_source_unavailable_frames",
            "air_ace_roster_partial_frames",
            "air_ace_roster_assignment_blocked_frames",
        ],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 49,
        name: "naval_combat_lifecycle_live",
        requires_data: true,
        required_units: &["naval_combat_lifecycle_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 50,
        name: "naval_combat_members_live",
        requires_data: true,
        required_units: &["naval_combat_member_rows"],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    },
];

pub fn validate_run_dir(options: &GateOptions) -> GateReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut details = serde_json::Map::new();

    let manifest_path = options.run_dir.join("native_session_manifest.json");
    let (external_manifest_bytes, external_manifest) = match read_json_document(&manifest_path) {
        Ok(value) => value,
        Err(err) => {
            return GateReport {
                passed: false,
                errors: vec![format!(
                    "cannot read manifest {}: {err}",
                    manifest_path.display()
                )],
                warnings,
                details: json!({ "run_dir": options.run_dir }),
            };
        }
    };

    details.insert(
        "run_dir".to_string(),
        json!(options.run_dir.display().to_string()),
    );
    details.insert(
        "manifest".to_string(),
        json!(manifest_path.display().to_string()),
    );
    let weakenings = verification_weakenings(options);
    reject_verification_weakenings(&weakenings, &mut errors);
    details.insert(
        "verification_profile".to_string(),
        json!({
            "authoritative": options.require_replay_verify && weakenings.is_empty(),
            "replay_verify_required": options.require_replay_verify,
            "weakenings": weakenings,
        }),
    );
    if !options.require_replay_verify {
        errors.push(
            "--skip-replay-verify is nonauthoritative; a skipped container verification can never pass this gate"
                .to_string(),
        );
        warnings.push("verification weakened: replay container authority was skipped".to_string());
    }
    if options.allow_partial_or_source_failure {
        errors.push(
            "--allow-partial-or-source-failure is a non-certifying broad escape; canonical per-domain expected-partial normalization is always applied instead"
                .to_string(),
        );
    }

    let mut replay_authority = None;
    let manifest = if options.require_replay_verify {
        match load_verified_replay_authority(
            &external_manifest,
            &external_manifest_bytes,
            &options.run_dir,
        ) {
            Ok(authority) => {
                details.insert("replay_verify".to_string(), authority.details.clone());
                let manifest = authority.manifest.clone();
                replay_authority = Some(authority);
                manifest
            }
            Err(err) => {
                errors.push(err);
                external_manifest
            }
        }
    } else {
        external_manifest
    };

    require_str_eq(
        &manifest,
        "schema",
        "native_collector_session_v1",
        &mut errors,
    );
    require_str_eq(&manifest, "stream_format", "compact", &mut errors);
    require_bool_eq(&manifest, "extraction_perf_gate", true, &mut errors);
    require_array_empty(&manifest, "parse_errors", &mut errors);
    require_array_empty(&manifest, "ordering_errors", &mut errors);

    let scheduled_domains = string_array(manifest.get("scheduled_domains"));
    if scheduled_domains.is_empty() {
        errors.push(
            "scheduled_domains is empty; merge gates must schedule explicit domains".to_string(),
        );
    }
    let production_descriptors = compact_contract::production_profile_domains();
    let required_domains = options.required_domains.clone();
    let mut scheduled_descriptors = production_descriptors.clone();
    for domain in &required_domains {
        let Some(descriptor) = compact_contract::assigned_descriptor_by_name(domain) else {
            errors.push(format!("required compact domain {domain:?} is not assigned"));
            continue;
        };
        if !descriptor.schedulable
            || (!compact_contract::status_in_production_profile(descriptor.status)
                && !compact_contract::is_optional_fail_closed_companion(descriptor.id))
        {
            errors.push(format!(
                "required compact domain {domain:?} is neither production-complete nor an approved optional fail-closed companion"
            ));
            continue;
        }
        scheduled_descriptors.push(descriptor);
    }
    scheduled_descriptors.sort_by_key(|descriptor| descriptor.id);
    scheduled_descriptors.dedup_by_key(|descriptor| descriptor.id);
    let canonical_scheduled_names = scheduled_descriptors
        .iter()
        .map(|descriptor| descriptor.wire_name.to_string())
        .collect::<Vec<_>>();
    if scheduled_domains != canonical_scheduled_names {
        errors.push(format!(
            "scheduled_domains {:?} != canonical compact production-plus-required roster {:?}",
            scheduled_domains, canonical_scheduled_names
        ));
    }
    details.insert("scheduled_domains".to_string(), json!(scheduled_domains));
    details.insert("required_domains".to_string(), json!(required_domains));

    let scheduled_ids = scheduled_descriptors
        .iter()
        .map(|descriptor| descriptor.id)
        .collect::<Vec<_>>();
    let expected_contracts =
        compact_contract::emitted_domain_ids_for_scheduled_ids(&scheduled_ids)
        .into_iter()
        .filter_map(compact_contract::assigned_descriptor_by_id)
        .map(contract_for_descriptor)
        .collect::<Vec<_>>();
    for domain in &required_domains {
        if !scheduled_domains
            .iter()
            .any(|scheduled| scheduled == domain)
        {
            errors.push(format!(
                "required compact domain {} is not present in scheduled_domains",
                domain
            ));
        }
    }
    for contract in &expected_contracts {
        if !contract.shared_recorder_known {
            warnings.push(format!(
                "compact domain {} ({}) is lane-known but not yet in shared recorder_host decoder maps",
                contract.name, contract.id
            ));
        }
    }

    let min_game_hours = match certification_min_game_hours(options.min_game_hours) {
        Ok(value) => value,
        Err(error) => {
            errors.push(error);
            1
        }
    };
    details.insert("min_game_hours".to_string(), json!(min_game_hours));

    validate_hourly_manifest(&manifest, min_game_hours, &mut errors);
    validate_compact_manifest(
        &manifest,
        &expected_contracts,
        options,
        &mut errors,
        &mut warnings,
    );
    validate_bridge_and_root_debt(
        &manifest,
        &scheduled_domains,
        options,
        &mut errors,
        &mut warnings,
    );

    let stream_artifact = match required_run_file(&options.run_dir, &manifest, "raw_stream")
        .and_then(|path| read_stream_artifact(&path, &manifest, options.max_stream_bytes))
    {
        Ok(artifact) => {
            details.insert(
                "stream_file".to_string(),
                json!(artifact.path.display().to_string()),
            );
            details.insert("stream_storage".to_string(), artifact.as_json());
            Some(artifact)
        }
        Err(err) => {
            errors.push(err);
            None
        }
    };

    if let Some(artifact) = stream_artifact.as_ref() {
        validate_stream_file_against_manifest(
            &artifact.decoded,
            &manifest,
            &expected_contracts,
            min_game_hours,
            options,
            &mut errors,
        );
        if let Some(authority) = replay_authority.as_ref() {
            if authority.container_stream != artifact.decoded {
                errors.push(format!(
                    "replay container decoded collector stream differs from decoded run stream (container sha256 {}, run sha256 {})",
                    Sha256::hex(&authority.container_stream),
                    artifact.decoded_sha256,
                ));
            }
        }
    }

    GateReport {
        passed: errors.is_empty(),
        errors,
        warnings,
        details: Value::Object(details),
    }
}

fn certification_min_game_hours(value: Option<i64>) -> Result<i64, String> {
    match value {
        Some(value) if value > 0 => Ok(value),
        Some(value) => Err(format!(
            "--min-game-hours must be explicitly positive for certification, got {value}"
        )),
        None => Err(
            "--min-game-hours is required explicitly for certification; mutable manifest fallback is forbidden"
                .to_string(),
        ),
    }
}

fn read_json_document(path: &Path) -> Result<(Vec<u8>, Value), String> {
    let bytes = fs::read(path).map_err(|err| err.to_string())?;
    let value = snapshot::parse_json_strict(&bytes).map_err(|err| err.to_string())?;
    Ok((bytes, value))
}

fn validate_hourly_manifest(manifest: &Value, min_game_hours: i64, errors: &mut Vec<String>) {
    let hourly = manifest.get("hourly_requirement").unwrap_or(&Value::Null);
    if hourly.get("required").and_then(Value::as_bool) != Some(true) {
        errors.push("hourly_requirement.required is not true".to_string());
    }
    if hourly.get("met").and_then(Value::as_bool) != Some(true) {
        errors.push("hourly_requirement.met is not true".to_string());
    }
    if !string_array(hourly.get("problems")).is_empty() {
        errors.push(format!(
            "hourly_requirement.problems is not empty: {:?}",
            string_array(hourly.get("problems"))
        ));
    }
    match hourly.get("requested_game_hours").and_then(Value::as_i64) {
        Some(requested) if requested == min_game_hours => {}
        Some(requested) => errors.push(format!(
            "hourly_requirement.requested_game_hours {requested} != explicit --min-game-hours {min_game_hours}"
        )),
        None => errors.push(
            "hourly_requirement.requested_game_hours is missing or not an integer".to_string(),
        ),
    }
    let advanced = manifest
        .get("game_hours_advanced")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    if advanced < min_game_hours {
        errors.push(format!(
            "game_hours_advanced {advanced} is below required {min_game_hours}"
        ));
    }
    if manifest
        .get("skipped_game_hours")
        .and_then(Value::as_i64)
        .unwrap_or(-1)
        != 0
    {
        errors.push(format!(
            "skipped_game_hours is {:?}, expected 0",
            manifest.get("skipped_game_hours")
        ));
    }
    let unique_hours = manifest
        .get("unique_game_hour_count")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    if advanced >= 0 && unique_hours >= 0 {
        match advanced.checked_add(1) {
            Some(inclusive) if unique_hours == inclusive => {}
            Some(inclusive) => errors.push(format!(
                "unique_game_hour_count {unique_hours} != game_hours_advanced + 1 ({inclusive})"
            )),
            None => errors.push("game_hours_advanced cannot form an inclusive span".to_string()),
        }
    }
}

fn validate_compact_manifest(
    manifest: &Value,
    expected_contracts: &[DomainContract],
    options: &GateOptions,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let compact = manifest.get("compact_stream").unwrap_or(&Value::Null);
    if compact.get("record_magic").and_then(Value::as_str) != Some("H4CR") {
        errors.push("compact_stream.record_magic is not H4CR".to_string());
    }
    require_u64_eq(
        compact,
        "record_header_len",
        RECORD_HEADER_LEN as u64,
        errors,
    );
    require_u64_eq(compact, "record_version", RECORD_VERSION as u64, errors);
    require_bool_eq(compact, "decodable", true, errors);
    require_bool_eq(compact, "ordered", true, errors);
    require_bool_eq(compact, "worker_panic_seen", false, errors);
    require_bool_eq(compact, "bridge_unavailable_seen", false, errors);
    if !options.allow_partial_or_source_failure {
        require_bool_eq(compact, "source_unavailable_seen", false, errors);
        require_bool_eq(compact, "partial_data_seen", false, errors);
    } else {
        for key in ["source_unavailable_seen", "partial_data_seen"] {
            if compact.get(key).and_then(Value::as_bool) == Some(true) {
                warnings.push(format!("compact_stream.{key} is true"));
            }
        }
    }
    let coverage = compact.get("domain_coverage").unwrap_or(&Value::Null);
    require_bool_eq(coverage, "ok", true, errors);
    require_array_empty(coverage, "errors", errors);

    let frame_count = compact
        .get("frame_end_record_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if frame_count == 0 {
        errors.push("compact_stream.frame_end_record_count is 0".to_string());
    }
    let expected_count = expected_contracts.len() as u64;
    require_u64_eq(coverage, "expected_domain_count", expected_count, errors);
    require_u64_eq(
        compact,
        "domain_perf_record_count",
        frame_count.saturating_mul(expected_count),
        errors,
    );
    let expected_data_count = expected_contracts
        .iter()
        .filter(|contract| contract.requires_data)
        .count() as u64;
    require_u64_eq(
        compact,
        "domain_data_record_count",
        frame_count.saturating_mul(expected_data_count),
        errors,
    );

    let expected_domain_names = string_array(compact.get("expected_domains"));
    let expected_from_gate: BTreeSet<&str> = expected_contracts
        .iter()
        .map(|contract| contract.name)
        .collect();
    let expected_from_manifest: BTreeSet<&str> =
        expected_domain_names.iter().map(String::as_str).collect();
    if expected_from_manifest != expected_from_gate {
        errors.push(format!(
            "compact_stream.expected_domains {:?} != gate required {:?}",
            expected_from_manifest, expected_from_gate
        ));
    }
    let domains_seen = string_array(compact.get("domains_seen"));
    for contract in expected_contracts {
        if !domains_seen.iter().any(|domain| domain == contract.name) {
            errors.push(format!(
                "compact_stream.domains_seen is missing {}",
                contract.name
            ));
        }
    }
    if !domains_seen.iter().any(|domain| domain == "frame") {
        errors.push("compact_stream.domains_seen is missing frame".to_string());
    }

    if compact
        .get("total_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX)
        > options.max_stream_bytes
    {
        errors.push(format!(
            "compact_stream.total_bytes exceeds limit {}",
            options.max_stream_bytes
        ));
    }
    if compact
        .get("max_record_len")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX)
        > options.max_record_bytes
    {
        errors.push(format!(
            "compact_stream.max_record_len exceeds limit {}",
            options.max_record_bytes
        ));
    }
    if frame_count > 0 {
        let total = compact
            .get("total_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let bytes_per_frame = total / frame_count.max(1);
        if bytes_per_frame > options.max_bytes_per_frame {
            errors.push(format!(
                "compact stream average bytes per frame {bytes_per_frame} exceed limit {}",
                options.max_bytes_per_frame
            ));
        }
    }

    let per_domain = compact.get("per_domain").and_then(Value::as_object);
    let Some(per_domain) = per_domain else {
        errors.push("compact_stream.per_domain is missing or not an object".to_string());
        return;
    };
    for contract in expected_contracts {
        let Some(domain) = per_domain.get(contract.name) else {
            errors.push(format!(
                "compact_stream.per_domain.{} missing",
                contract.name
            ));
            continue;
        };
        validate_domain_manifest(domain, *contract, frame_count, options, errors);
    }
}

fn validate_domain_manifest(
    domain: &Value,
    contract: DomainContract,
    frame_count: u64,
    options: &GateOptions,
    errors: &mut Vec<String>,
) {
    require_u64_eq(domain, "domain_id", contract.id as u64, errors);
    require_u64_eq(domain, "record_count", frame_count, errors);
    if contract.requires_data {
        require_u64_eq(domain, "data_record_count", frame_count, errors);
    }
    let status_value = domain.get("status_flags_or");
    let Some(status) = status_value.and_then(Value::as_u64) else {
        errors.push(format!(
            "domain {} status_flags_or {:?} is missing or not an unsigned integer",
            contract.name, status_value
        ));
        return;
    };
    if status > u32::MAX as u64 {
        errors.push(format!(
            "domain {} status_flags_or {status} exceeds u32::MAX",
            contract.name
        ));
        return;
    }
    let rejected = rejected_domain_failure_bits(
        contract.id,
        status as u32,
        options.allow_partial_or_source_failure,
    );
    if rejected != 0 {
        errors.push(format!(
            "domain {} status_flags_or {status} contains rejected global failure bits {rejected:#x}",
            contract.name,
        ));
    }
    let read_count = domain
        .get("read_time_us")
        .and_then(|value| value.get("count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if read_count != frame_count {
        errors.push(format!(
            "domain {} read_time_us.count {read_count} != frame_end_record_count {frame_count}",
            contract.name
        ));
    }
    let units = domain
        .get("data_units_total")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if options.strict_semantic_units {
        for key in contract.required_units {
            if !units.contains_key(*key) {
                errors.push(format!(
                    "domain {} data_units_total is missing required counter {}",
                    contract.name, key
                ));
            }
        }
    }
    // Fail-closed sub-slice health flags: these counters must be absent-or-
    // zero. A nonzero value means a fail-closed condition fired on at least
    // one frame (e.g. the domain-46 ace roster was source-unavailable,
    // partial, or had its wing-assignment scan blocked). Checked unless the
    // profile explicitly tolerates partial/source failures.
    for key in contract.zero_units {
        let value = units.get(*key).and_then(Value::as_u64).unwrap_or(0);
        let named_tolerance = key.contains("source_unavailable") || key.contains("partial");
        if value != 0 && !(options.allow_partial_or_source_failure && named_tolerance) {
            errors.push(format!(
                "domain {} data_units_total counter {} is {value}, expected 0 (fail-closed sub-slice health flag fired)",
                contract.name, key
            ));
        }
    }
    // Work-implies-rows: a domain that reports substantial adapter work but
    // emits zero rows for the whole run threw every row away — silent data loss
    // hiding behind a tolerated rowless PARTIAL. This is fatal even under
    // `allow_partial_or_source_failure`: a partial-tolerant profile may accept
    // some rowless frames, but not a session where megarows went in and nothing
    // came out. (row_count_total is the emitted-row total; requires_data only
    // checks records exist, not rows.)
    if !contract.work_implies_rows_units.is_empty() {
        // G1-residue gate finding 5: the aggregate rule alone let ONE early
        // emitted row mask total loss on every later frame. The collector now
        // counts per-frame work-without-emission (adapter built output-family
        // rows, frame emitted zero data rows); any nonzero total is the same
        // silent-loss class and fails regardless of the aggregate.
        let work_without_emission_frames = units
            .get("domain47_work_without_emission_frames")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if contract.id == 47 && work_without_emission_frames != 0 {
            errors.push(format!(
                "domain {} discarded every built row on {work_without_emission_frames} frame(s)                  (adapter work without emission — silent per-frame data loss even though the                  whole-run row total is nonzero)",
                contract.name,
            ));
        }
        let emitted_rows = domain
            .get("row_count_total")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if emitted_rows == 0 {
            let worked: Vec<String> = contract
                .work_implies_rows_units
                .iter()
                .filter_map(|key| {
                    let value = units.get(*key).and_then(Value::as_u64).unwrap_or(0);
                    (value != 0).then(|| format!("{key}={value}"))
                })
                .collect();
            if !worked.is_empty() {
                errors.push(format!(
                    "domain {} performed adapter work ({}) but emitted row_count_total=0 — every built row was discarded (silent data loss behind a rowless frame)",
                    contract.name,
                    worked.join(", ")
                ));
            }
        }
    }
}

fn validate_bridge_and_root_debt(
    manifest: &Value,
    scheduled_domains: &[String],
    options: &GateOptions,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    if let Some(obj) = manifest.as_object() {
        for (key, value) in obj {
            if key.ends_with("_bridge_rows") {
                let rows = value.as_u64().unwrap_or(0);
                if rows > 0 {
                    let msg = format!("{key} is {rows}, expected 0 for DLL-only proof");
                    if options.allow_bridge_roots {
                        warnings.push(msg);
                    } else {
                        errors.push(msg);
                    }
                }
            }
        }
    }

    let root_checks = [
        (
            "division_root_source",
            "division_root_discovery_debt",
            [
                "divisions",
                "division_equipment",
                "army_orders",
                "oob_command",
            ]
            .as_slice(),
        ),
        (
            "ship_root_source",
            "ship_root_discovery_debt",
            ["ships"].as_slice(),
        ),
        (
            "projects_root_source",
            "projects_root_discovery_debt",
            ["projects_live"].as_slice(),
        ),
    ];
    for (source_key, debt_key, domains) in root_checks {
        let relevant = scheduled_domains
            .iter()
            .any(|scheduled| domains.iter().any(|domain| domain == scheduled));
        if !relevant {
            continue;
        }
        let source = manifest
            .get(source_key)
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        let debt = manifest
            .get(debt_key)
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        let source_ok = matches!(
            source,
            "dll_native" | "dll_project_cache_progress" | "dll_project_cache"
        );
        let debt_ok = debt.starts_with("none;");
        if !source_ok || !debt_ok {
            let msg = format!(
                "{source_key}={source:?} / {debt_key}={debt:?} is not merge-clean DLL-only root evidence"
            );
            if options.allow_bridge_roots {
                warnings.push(msg);
            } else {
                errors.push(msg);
            }
        }
    }

    if manifest
        .get("special_projects_scan")
        .and_then(Value::as_bool)
        == Some(true)
        && scheduled_domains
            .iter()
            .any(|domain| domain == "projects_live")
    {
        let msg =
            "special_projects_scan is true for projects_live; startup heap scan is not DLL-only completion";
        if options.allow_bridge_roots {
            warnings.push(msg.to_string());
        } else {
            errors.push(msg.to_string());
        }
    }
}

fn validate_stream_file_against_manifest(
    raw: &[u8],
    manifest: &Value,
    expected_contracts: &[DomainContract],
    min_game_hours: i64,
    options: &GateOptions,
    errors: &mut Vec<String>,
) {
    let compact = manifest.get("compact_stream").unwrap_or(&Value::Null);
    if compact
        .get("total_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX)
        != raw.len() as u64
    {
        errors.push(format!(
            "collector_stream.bin length {} != compact_stream.total_bytes {:?}",
            raw.len(),
            compact.get("total_bytes")
        ));
    }
    // The byte-identical embedded recorder manifest is replay authority at this
    // point.  Select the protocol before accepting any stream bytes: a Known
    // identity requires sequence-zero CID1, while every non-Known/legacy
    // manifest remains the sequence-one Legacy protocol.  Never sniff CID1.
    let identity_mode = stream_identity_mode_from_manifest(manifest);
    let summary = scan_compact_stream_with_identity_mode(raw, identity_mode);
    validate_scanned_hours(&summary, manifest, min_game_hours, errors);
    if !summary.parse_errors.is_empty() {
        errors.push(format!(
            "independent stream scan parse errors: {:?}",
            summary.parse_errors
        ));
    }
    if !summary.ordering_errors.is_empty() {
        errors.push(format!(
            "independent stream scan ordering errors: {:?}",
            summary.ordering_errors
        ));
    }
    if summary.skipped_hours != 0 {
        errors.push(format!(
            "independent stream scan found {} skipped game hours",
            summary.skipped_hours
        ));
    }
    let rejected = summary.per_domain_status_flags_or.iter().fold(
        0u32,
        |mask, (domain_id, raw_status)| {
            mask | compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
                *domain_id,
                *raw_status,
            )
        },
    );
    if rejected != 0 {
        errors.push(format!(
            "independent stream scan normalized status_flags_or {} contains rejected failure bits {rejected:#x}",
            summary.global_status_flags_or
        ));
    }
    let scanned_status = summary.global_status_flags_or | summary.frame_status_flags_or_raw;
    compare_u64(
        "status_flags_or",
        u64::from(scanned_status),
        compact,
        errors,
    );
    let scanned_failure_status =
        rejected | (summary.frame_status_flags_or_raw & COMPACT_STATUS_FAILURE_MASK);
    compare_u64(
        "failure_status_flags_or",
        u64::from(scanned_failure_status),
        compact,
        errors,
    );
    compare_u64("record_count", summary.record_count, compact, errors);
    compare_u64(
        "domain_perf_record_count",
        summary.domain_perf_record_count,
        compact,
        errors,
    );
    compare_u64(
        "domain_data_record_count",
        summary.domain_data_record_count,
        compact,
        errors,
    );
    compare_u64(
        "frame_end_record_count",
        summary.frame_end_record_count,
        compact,
        errors,
    );
    compare_u64(
        "total_payload_bytes",
        summary.total_payload_bytes,
        compact,
        errors,
    );
    compare_u64("max_payload_len", summary.max_payload_len, compact, errors);
    compare_u64("max_record_len", summary.max_record_len, compact, errors);
    validate_scanned_domain_authority(&summary, compact, expected_contracts, errors);
    validate_domain57_stream_authority(&summary, compact, errors);
    validate_campaign_identity_stream_authority(&summary, manifest, identity_mode, errors);

    if summary.framed_max_record_len > options.max_record_bytes {
        errors.push(format!(
            "independent stream scan framed max_record_len {} exceeds limit {}",
            summary.framed_max_record_len, options.max_record_bytes
        ));
    }
    let expected_ids = expected_contracts
        .iter()
        .map(|contract| contract.id)
        .collect::<Vec<_>>();
    let coverage_errors = compact_domain_coverage_errors(&summary, &expected_ids);
    if !coverage_errors.is_empty() {
        errors.push(format!(
            "independent stream scan coverage errors: {:?}",
            coverage_errors
        ));
    }
}

fn validate_scanned_domain_authority(
    summary: &HeaderScanSummary,
    compact: &Value,
    expected_contracts: &[DomainContract],
    errors: &mut Vec<String>,
) {
    let per_domain = compact.get("per_domain").and_then(Value::as_object);
    for contract in expected_contracts {
        let Some(manifest_domain) = per_domain.and_then(|domains| domains.get(contract.name))
        else {
            continue;
        };
        let perf = summary
            .per_domain_perf_authority
            .get(&contract.id)
            .copied()
            .unwrap_or_default();
        compare_manifest_authority_field(
            contract.name,
            "PERF",
            "row_count_total",
            perf.row_count,
            manifest_domain,
            errors,
        );
        compare_manifest_authority_field(
            contract.name,
            "PERF",
            "changed_rows_total",
            perf.changed_rows,
            manifest_domain,
            errors,
        );
        compare_manifest_authority_field(
            contract.name,
            "PERF",
            "unit_count_total",
            perf.unit_count,
            manifest_domain,
            errors,
        );
        compare_manifest_authority_field(
            contract.name,
            "PERF",
            "changed_unit_count_total",
            perf.changed_unit_count,
            manifest_domain,
            errors,
        );
        compare_manifest_authority_field(
            contract.name,
            "raw PERF/DATA",
            "status_flags_or",
            u64::from(
                summary
                    .per_domain_status_flags_or
                    .get(&contract.id)
                    .copied()
                    .unwrap_or(0),
            ),
            manifest_domain,
            errors,
        );
        if contract.requires_data {
            let data = summary
                .per_domain_data_authority
                .get(&contract.id)
                .cloned()
                .unwrap_or_default();
            compare_manifest_authority_field(
                contract.name,
                "DATA",
                "data_rows_total",
                data.row_count,
                manifest_domain,
                errors,
            );
            compare_manifest_authority_field(
                contract.name,
                "DATA",
                "data_changed_rows_total",
                data.changed_rows,
                manifest_domain,
                errors,
            );
            if let Some(data_status) = data.status_flags {
                if data_status != perf.status_flags {
                    errors.push(format!(
                        "domain {} DATA status authority {:#x} != PERF status authority {:#x}",
                        contract.name, data_status, perf.status_flags
                    ));
                }
            }
        }
        compare_manifest_units_exact(
            contract.name,
            summary.per_domain_combined_units.get(&contract.id),
            manifest_domain.get("data_units_total"),
            errors,
        );
    }
    for ((hour, domain_id), data) in &summary.per_hour_data_authority {
        let Some(perf) = summary.per_hour_perf_authority.get(&(*hour, *domain_id)) else {
            errors.push(format!(
                "game_hour_raw {hour}: domain {domain_id} DATA authority has no matching PERF authority"
            ));
            continue;
        };
        if let Some(data_status) = data.status_flags {
            if data_status != perf.status_flags {
                errors.push(format!(
                    "game_hour_raw {hour}: domain {domain_id} DATA status {:#x} != PERF status {:#x}",
                    data_status, perf.status_flags
                ));
            }
        }
        match *domain_id {
            47 => {
                if perf.row_count != data.row_count
                    || perf.changed_rows != data.changed_rows
                    || perf.unit_count != data.row_count
                    || perf.changed_unit_count != data.changed_rows
                {
                    errors.push(format!(
                        "game_hour_raw {hour}: d47 DATA rows/changed {}/{} do not match PERF row/changed/unit/changed-unit {}/{}/{}/{}",
                        data.row_count,
                        data.changed_rows,
                        perf.row_count,
                        perf.changed_rows,
                        perf.unit_count,
                        perf.changed_unit_count,
                    ));
                }
            }
            53 => {
                if perf.unit_count != data.row_count || perf.changed_unit_count != data.changed_rows
                {
                    errors.push(format!(
                        "game_hour_raw {hour}: d53 DATA rows/changed {}/{} do not match PERF unit/changed-unit {}/{} (PERF observed row/changed work {}/{})",
                        data.row_count,
                        data.changed_rows,
                        perf.unit_count,
                        perf.changed_unit_count,
                        perf.row_count,
                        perf.changed_rows,
                    ));
                }
            }
            69 => {
                // Shared authority intentionally omits zero-valued unit keys.
                // A successfully decoded empty TSG2 frame therefore carries an
                // empty unit map whose authoritative counts are all zero.
                let groups = data
                    .data_units
                    .get("tech_sharing_groups_groups")
                    .copied()
                    .unwrap_or(0);
                let members = data
                    .data_units
                    .get("tech_sharing_groups_members")
                    .copied()
                    .unwrap_or(0);
                let bonus_rows = data
                    .data_units
                    .get("tech_sharing_groups_dynamic_bonus_rows")
                    .copied()
                    .unwrap_or(0);
                if data.row_count != groups {
                    errors.push(format!(
                        "game_hour_raw {hour}: d69 DATA emitted rows {} != decoded group unit count {groups}",
                        data.row_count,
                    ));
                }
                if perf.row_count != groups || perf.changed_rows != groups {
                    errors.push(format!(
                        "game_hour_raw {hour}: d69 DATA group count {groups} does not match PERF row/changed {}/{}",
                        perf.row_count,
                        perf.changed_rows,
                    ));
                }
                if members != bonus_rows {
                    errors.push(format!(
                        "game_hour_raw {hour}: d69 DATA member count {members} != dynamic-bonus pair count {bonus_rows}",
                    ));
                }
                if perf.unit_count != members || perf.changed_unit_count != bonus_rows {
                    errors.push(format!(
                        "game_hour_raw {hour}: d69 DATA member/bonus-pair counts {members}/{bonus_rows} do not match PERF unit/changed-unit {}/{}",
                        perf.unit_count,
                        perf.changed_unit_count,
                    ));
                }
            }
            _ => {}
        }
    }
}

fn compare_manifest_authority_field(
    domain_name: &str,
    source: &str,
    field: &str,
    scanned: u64,
    manifest_domain: &Value,
    errors: &mut Vec<String>,
) {
    match manifest_domain.get(field).and_then(Value::as_u64) {
        Some(declared) if declared == scanned => {}
        Some(declared) => errors.push(format!(
            "domain {domain_name} independently scanned {source} {field} {scanned} != manifest {declared}"
        )),
        None => errors.push(format!(
            "domain {domain_name} manifest {field} is missing or not an unsigned integer"
        )),
    }
}

fn compare_manifest_units_exact(
    domain_name: &str,
    scanned: Option<&BTreeMap<String, u64>>,
    manifest: Option<&Value>,
    errors: &mut Vec<String>,
) {
    let Some(manifest) = manifest.and_then(Value::as_object) else {
        errors.push(format!(
            "domain {domain_name} manifest data_units_total is missing or not an object"
        ));
        return;
    };
    let empty = BTreeMap::new();
    let scanned = scanned.unwrap_or(&empty);
    for (name, value) in manifest {
        let Some(declared) = value.as_u64() else {
            errors.push(format!(
                "domain {domain_name} manifest data_units_total.{name} is not an unsigned integer"
            ));
            continue;
        };
        match scanned.get(name) {
            Some(observed) if *observed == declared => {}
            Some(observed) => errors.push(format!(
                "domain {domain_name} independently decoded data unit {name}={observed} != manifest {declared}"
            )),
            None => errors.push(format!(
                "domain {domain_name} manifest data unit {name}={declared} has no independently decoded authority"
            )),
        }
    }
    for (name, observed) in scanned {
        if !manifest.contains_key(name) {
            errors.push(format!(
                "domain {domain_name} independently decoded data unit {name}={observed} is missing from manifest"
            ));
        }
    }
}

fn validate_domain57_stream_authority(
    summary: &HeaderScanSummary,
    compact: &Value,
    errors: &mut Vec<String>,
) {
    const REQUIRED_ROOT_MASK: u32 = 0x7;
    let mut records_by_hour = BTreeMap::<i32, usize>::new();
    for record in &summary.domain57_frames {
        *records_by_hour.entry(record.header_hour_raw).or_default() += 1;
    }
    let exact_one_record_per_closed_frame = summary.frame_end_hour_counts.len()
        == summary.frame_end_record_count as usize
        && summary
            .frame_end_hour_counts
            .values()
            .all(|count| *count == 1)
        && records_by_hour.len() == summary.frame_end_hour_counts.len()
        && summary
            .frame_end_hour_counts
            .keys()
            .all(|hour| records_by_hour.get(hour) == Some(&1));
    if !exact_one_record_per_closed_frame {
        errors.push(
            "independent d57 authority does not have exactly one record per closed frame"
                .to_string(),
        );
    }
    if summary
        .domain57_frames
        .iter()
        .any(|record| record.root_evidence_mask != REQUIRED_ROOT_MASK)
    {
        errors.push("independent d57 authority is missing required root evidence mask 0x7".into());
    }
    if summary
        .domain57_frames
        .iter()
        .any(|record| record.speed_rows.len() != 1)
    {
        errors.push(
            "independent d57 authority does not have exactly one speed row per record".into(),
        );
    }
    if summary
        .domain57_frames
        .iter()
        .any(|record| record.pause_rows.len() != 1)
    {
        errors.push(
            "independent d57 authority does not have exactly one pause row per record".into(),
        );
    }
    if summary
        .domain57_frames
        .iter()
        .any(|record| record.payload_hour_raw != record.header_hour_raw)
    {
        errors.push("independent d57 authority has a payload/header hour mismatch".into());
    }
    if summary.domain57_frames.iter().any(|record| {
        record
            .speed_rows
            .iter()
            .chain(record.pause_rows.iter())
            .any(|row| row.event_hour_raw != record.header_hour_raw)
    }) {
        errors.push("independent d57 authority has an event/header hour mismatch".into());
    }

    let scanned = domain57_frame_evidence_json(summary);
    match compact.get("domain57_frame_evidence") {
        Some(declared) if declared == &scanned => {}
        Some(_) => errors.push(
            "compact_stream.domain57_frame_evidence does not exactly match independently decoded stream evidence"
                .to_string(),
        ),
        None => errors.push("compact_stream.domain57_frame_evidence is missing".to_string()),
    }
}

fn domain57_frame_evidence_json(summary: &HeaderScanSummary) -> Value {
    const REQUIRED_ROOT_MASK: u32 = 0x7;
    let mut speed_values = BTreeMap::<String, u64>::new();
    let mut pause_values = BTreeMap::<String, u64>::new();
    let mut root_masks = BTreeMap::<String, u64>::new();
    let mut speed_rows_per_record = BTreeMap::<String, u64>::new();
    let mut pause_rows_per_record = BTreeMap::<String, u64>::new();
    let mut records_per_closed_frame = BTreeMap::<String, u64>::new();
    let mut records_by_hour = BTreeMap::<i32, usize>::new();
    for record in &summary.domain57_frames {
        *records_by_hour.entry(record.header_hour_raw).or_default() += 1;
        *root_masks
            .entry(format!("0x{:08x}", record.root_evidence_mask))
            .or_default() += 1;
        *speed_rows_per_record
            .entry(record.speed_rows.len().to_string())
            .or_default() += 1;
        *pause_rows_per_record
            .entry(record.pause_rows.len().to_string())
            .or_default() += 1;
        for row in &record.speed_rows {
            *speed_values.entry(row.value_i32.to_string()).or_default() += 1;
        }
        for row in &record.pause_rows {
            *pause_values.entry(row.value_i32.to_string()).or_default() += 1;
        }
    }
    for hour in summary.frame_end_hour_counts.keys() {
        *records_per_closed_frame
            .entry(records_by_hour.get(hour).copied().unwrap_or(0).to_string())
            .or_default() += 1;
    }
    let closed_frame_count = summary.frame_end_record_count;
    let exact_one_record_per_closed_frame = summary.frame_end_hour_counts.len()
        == closed_frame_count as usize
        && summary
            .frame_end_hour_counts
            .values()
            .all(|count| *count == 1)
        && records_by_hour.len() == summary.frame_end_hour_counts.len()
        && summary
            .frame_end_hour_counts
            .keys()
            .all(|hour| records_by_hour.get(hour) == Some(&1));
    json!({
        "schema": "battle-plan.domain57_frame_evidence.v1",
        "required_root_mask": REQUIRED_ROOT_MASK,
        "record_count": summary.domain57_frames.len(),
        "closed_frame_count": closed_frame_count,
        "records_per_closed_frame_histogram": records_per_closed_frame,
        "root_presence_counts": {
            "player_country": summary.domain57_frames.iter().filter(|record| record.root_evidence_mask & 0x1 != 0).count(),
            "pause_clock": summary.domain57_frames.iter().filter(|record| record.root_evidence_mask & 0x2 != 0).count(),
            "game_speed": summary.domain57_frames.iter().filter(|record| record.root_evidence_mask & 0x4 != 0).count(),
        },
        "root_mask_histogram": root_masks,
        "speed": {
            "row_count": summary.domain57_frames.iter().map(|record| record.speed_rows.len()).sum::<usize>(),
            "rows_per_record_histogram": speed_rows_per_record,
            "value_histogram": speed_values,
        },
        "pause": {
            "row_count": summary.domain57_frames.iter().map(|record| record.pause_rows.len()).sum::<usize>(),
            "rows_per_record_histogram": pause_rows_per_record,
            "value_histogram": pause_values,
        },
        "payload_header_hour_mismatch_count": summary.domain57_frames.iter().filter(|record| record.payload_hour_raw != record.header_hour_raw).count(),
        "event_header_hour_mismatch_count": summary.domain57_frames.iter().map(|record| record.speed_rows.iter().chain(record.pause_rows.iter()).filter(|row| row.event_hour_raw != record.header_hour_raw).count()).sum::<usize>(),
        "exact_one_record_per_closed_frame": exact_one_record_per_closed_frame,
        "all_required_roots_per_record": summary.domain57_frames.iter().all(|record| record.root_evidence_mask == REQUIRED_ROOT_MASK),
        "exact_one_speed_row_per_record": summary.domain57_frames.iter().all(|record| record.speed_rows.len() == 1),
        "exact_one_pause_row_per_record": summary.domain57_frames.iter().all(|record| record.pause_rows.len() == 1),
        "records": summary.domain57_frames.iter().map(|record| json!({
            "sequence": record.sequence,
            "header_hour_raw": record.header_hour_raw,
            "payload_hour_raw": record.payload_hour_raw,
            "root_evidence_mask": record.root_evidence_mask,
            "speed": record.speed_rows.iter().map(|row| json!({"event_hour_raw": row.event_hour_raw, "value_i32": row.value_i32})).collect::<Vec<_>>(),
            "pause": record.pause_rows.iter().map(|row| json!({"event_hour_raw": row.event_hour_raw, "value_i32": row.value_i32})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn validate_campaign_identity_stream_authority(
    summary: &HeaderScanSummary,
    manifest: &Value,
    identity_mode: StreamIdentityMode,
    errors: &mut Vec<String>,
) {
    if let Err(error) = recorder_host::campaign_identity::validate_in_session_manifest(manifest) {
        errors.push(format!(
            "manifest campaign_identity grammar is invalid: {error}"
        ));
    }
    let declared = manifest.get("campaign_identity");
    match identity_mode {
        StreamIdentityMode::IdentityV1 => {
            let Some(cid1) = summary.validated_cid1.as_ref() else {
                errors.push(
                    "Known campaign_identity selected IdentityV1 but the complete stream scan retained no placement-authoritative CID1"
                        .to_string(),
                );
                return;
            };
            let scanned = recorder_host::campaign_identity::to_manifest_json(
                &recorder_host::campaign_identity::resolve_validated_cid1(cid1),
            );
            match declared {
                Some(value) if value == &scanned => {}
                Some(_) => errors.push(
                    "Known manifest campaign_identity does not exactly match independently admitted sequence-zero CID1 authority"
                        .to_string(),
                ),
                None => errors.push("manifest campaign_identity is missing".to_string()),
            }
        }
        StreamIdentityMode::Legacy => {
            // d68 is compatibility/witness authority only.  It may bind a
            // non-Known Legacy manifest, but can never mint Known identity.
            let scanned = recorder_host::campaign_identity::to_manifest_json(
                &recorder_host::campaign_identity::resolve(&summary.session_header_identity),
            );
            match declared {
                Some(value) if value == &scanned => {}
                Some(_) => errors.push(
                    "non-Known manifest campaign_identity does not exactly match independently decoded d68 stream authority"
                        .to_string(),
                ),
                None => errors.push("manifest campaign_identity is missing".to_string()),
            }
        }
    }
}

fn stream_identity_mode_from_manifest(manifest: &Value) -> StreamIdentityMode {
    if manifest
        .get("campaign_identity")
        .and_then(|identity| identity.get("state"))
        .and_then(Value::as_str)
        == Some("known")
    {
        StreamIdentityMode::IdentityV1
    } else {
        StreamIdentityMode::Legacy
    }
}

fn validate_scanned_hours(
    summary: &HeaderScanSummary,
    manifest: &Value,
    min_game_hours: i64,
    errors: &mut Vec<String>,
) {
    let scanned_unique = i64::try_from(summary.unique_hours.len()).unwrap_or(i64::MAX);
    let scanned_span = match (summary.unique_hours.first(), summary.unique_hours.last()) {
        (Some(first), Some(last)) => i64::from(*last) - i64::from(*first) + 1,
        _ => 0,
    };
    let manifest_unique = manifest
        .get("unique_game_hour_count")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let manifest_advanced = manifest
        .get("game_hours_advanced")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let manifest_span = manifest_advanced.checked_add(1).unwrap_or(i64::MAX);
    let required_span = min_game_hours.checked_add(1).unwrap_or(i64::MAX);

    if scanned_unique != manifest_unique {
        errors.push(format!(
            "independent stream unique hour count {scanned_unique} != manifest unique_game_hour_count {manifest_unique}"
        ));
    }
    if scanned_span != manifest_span {
        errors.push(format!(
            "independent stream inclusive hour span {scanned_span} != manifest game_hours_advanced + 1 ({manifest_span})"
        ));
    }
    if scanned_unique < required_span {
        errors.push(format!(
            "independent stream unique hour count {scanned_unique} is below required inclusive count {required_span}"
        ));
    }
    if scanned_span < required_span {
        errors.push(format!(
            "independent stream inclusive hour span {scanned_span} is below required {required_span}"
        ));
    }
}

#[derive(Debug)]
struct VerifiedReplayAuthority {
    manifest: Value,
    container_stream: Vec<u8>,
    details: Value,
}

#[derive(Debug)]
struct StaticArtifactIndexBinding {
    relative_path: String,
    sha256: String,
    bytes: u64,
}

#[derive(Debug)]
struct StaticBundleArtifactBinding {
    sha256: String,
    artifact_sha256: String,
}

#[derive(Debug)]
struct VerifiedStaticManifestAuthority {
    artifact_hashes: BTreeMap<String, String>,
    bundle_artifacts: BTreeMap<String, StaticBundleArtifactBinding>,
    bundle_id: String,
    product_modset_sha256: String,
    retained_manifest_sha256: String,
    retained_binding_count: usize,
    verifier_sha256: String,
}

fn required_static_artifact_relative_path(name: &str) -> Option<String> {
    let file_name = match name {
        compact_contract::STATIC_ARTIFACT_TEMPLATES_EQUIPMENT_CATALOG => {
            "domain59_static_catalog.json"
        }
        compact_contract::STATIC_ARTIFACT_LOCALISATION_JOIN => "localisation_catalog.json",
        compact_contract::STATIC_ARTIFACT_DECISIONS_STATIC_DEFINITIONS => {
            "decision_definitions_catalog.json"
        }
        compact_contract::STATIC_ARTIFACT_IDEA_TECH_LOC_XREF_STATIC => {
            "idea_tech_loc_xref_catalog.json"
        }
        compact_contract::STATIC_ARTIFACT_SHIP_DEFINITIONS_STATIC => {
            "ship_definitions_catalog.json"
        }
        compact_contract::STATIC_ARTIFACT_GFX_ICON_MANIFEST => "gfx_icon_manifest_catalog.json",
        compact_contract::STATIC_ARTIFACT_TECHNOLOGY_SHARING_STATIC => {
            "technology_sharing_static.json"
        }
        compact_contract::STATIC_ARTIFACT_FOCUS_DEFS_STATIC => "focus_defs_static.json",
        _ => return None,
    };
    Some(format!("static/{name}/{file_name}"))
}

fn required_static_bundle_file(name: &str) -> Option<&'static str> {
    Some(match name {
        compact_contract::STATIC_ARTIFACT_TEMPLATES_EQUIPMENT_CATALOG => {
            "artifacts/templates_equipment_catalog.json"
        }
        compact_contract::STATIC_ARTIFACT_LOCALISATION_JOIN => "artifacts/localisation_join.json",
        compact_contract::STATIC_ARTIFACT_DECISIONS_STATIC_DEFINITIONS => {
            "artifacts/decisions_static_definitions.json"
        }
        compact_contract::STATIC_ARTIFACT_IDEA_TECH_LOC_XREF_STATIC => {
            "artifacts/idea_tech_loc_xref_static.json"
        }
        compact_contract::STATIC_ARTIFACT_SHIP_DEFINITIONS_STATIC => {
            "artifacts/ship_definitions_static.json"
        }
        compact_contract::STATIC_ARTIFACT_GFX_ICON_MANIFEST => {
            "artifacts/gfx_icon_asset_manifest_static.json"
        }
        compact_contract::STATIC_ARTIFACT_TECHNOLOGY_SHARING_STATIC => {
            "artifacts/technology_sharing_static.json"
        }
        compact_contract::STATIC_ARTIFACT_FOCUS_DEFS_STATIC => {
            "artifacts/focus_defs_static.json"
        }
        _ => return None,
    })
}

fn required_static_gate_report_file(name: &str) -> Option<String> {
    compact_contract::REQUIRED_STATIC_ARTIFACTS
        .contains(&name)
        .then(|| format!("proofs/{name}.gate.json"))
}

fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical_json_value).collect()),
        Value::Object(object) => {
            let mut ordered = serde_json::Map::new();
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for key in keys {
                ordered.insert(key.clone(), canonical_json_value(&object[key]));
            }
            Value::Object(ordered)
        }
        other => other.clone(),
    }
}

fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&canonical_json_value(value))
        .map_err(|error| format!("cannot canonicalize JSON authority: {error}"))
}

fn canonical_json_sha256(value: &Value) -> Result<String, String> {
    canonical_json_bytes(value).map(|bytes| Sha256::hex(&bytes))
}

#[derive(Debug)]
struct RetainedFileEvidence {
    path: PathBuf,
    bytes: Vec<u8>,
    sha256: String,
}

fn verify_retained_file_evidence(
    value: Option<&Value>,
    label: &str,
    require_bytes: bool,
) -> Result<RetainedFileEvidence, String> {
    let evidence = value
        .and_then(Value::as_object)
        .ok_or_else(|| format!("manifest static_bundle {label} is missing or not an object"))?;
    let raw_path = evidence
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| format!("manifest static_bundle {label}.path is missing or empty"))?;
    if !Path::new(raw_path).is_absolute() {
        return Err(format!(
            "manifest static_bundle {label}.path must be the retained canonical absolute path, got {raw_path:?}"
        ));
    }
    let path = fs::canonicalize(raw_path).map_err(|error| {
        format!("cannot resolve retained static_bundle {label} file {raw_path:?}: {error}")
    })?;
    if !path.is_file() {
        return Err(format!(
            "retained static_bundle {label} path is not a file: {}",
            path.display()
        ));
    }
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "cannot read retained static_bundle {label} file {}: {error}",
            path.display()
        )
    })?;
    let observed_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if require_bytes {
        let claimed_bytes = evidence
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("manifest static_bundle {label}.bytes is invalid"))?;
        if claimed_bytes != observed_bytes {
            return Err(format!(
                "manifest static_bundle {label}.bytes {claimed_bytes} != retained file bytes {observed_bytes}"
            ));
        }
    }
    let claimed_sha256 = canonical_sha256(
        evidence.get("sha256"),
        &format!("manifest static_bundle {label}.sha256"),
    )?;
    let observed_sha256 = Sha256::hex(&bytes);
    if claimed_sha256 != observed_sha256 {
        return Err(format!(
            "manifest static_bundle {label}.sha256 {claimed_sha256} != retained file sha256 {observed_sha256}"
        ));
    }
    Ok(RetainedFileEvidence {
        path,
        bytes,
        sha256: observed_sha256,
    })
}

fn contained_bundle_file(
    bundle_dir: &Path,
    relative: &str,
    label: &str,
) -> Result<PathBuf, String> {
    let relative_path = validate_relative_path(relative, label)?;
    let path = fs::canonicalize(bundle_dir.join(relative_path)).map_err(|error| {
        format!("cannot resolve retained static bundle {label} {relative:?}: {error}")
    })?;
    if !path.starts_with(bundle_dir) || !path.is_file() {
        return Err(format!(
            "retained static bundle {label} is not a file contained by {}: {}",
            bundle_dir.display(),
            path.display()
        ));
    }
    Ok(path)
}

fn verify_retained_bundle_json_file(
    bundle_dir: &Path,
    evidence: Option<&Value>,
    label: &str,
    file_field: &str,
    expected_file: &str,
) -> Result<Value, String> {
    let evidence = evidence
        .and_then(Value::as_object)
        .ok_or_else(|| format!("retained static bundle {label} evidence is missing"))?;
    let file = evidence
        .get(file_field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("retained static bundle {label}.{file_field} is invalid"))?;
    if file != expected_file {
        return Err(format!(
            "retained static bundle {label}.{file_field} {file:?} != canonical {expected_file:?}"
        ));
    }
    let claimed_sha256 = canonical_sha256(
        evidence.get("sha256"),
        &format!("retained static bundle {label}.sha256"),
    )?;
    let path = contained_bundle_file(bundle_dir, file, label)?;
    let bytes = fs::read(&path)
        .map_err(|error| format!("cannot read retained static bundle {label}: {error}"))?;
    let observed_sha256 = Sha256::hex(&bytes);
    if observed_sha256 != claimed_sha256 {
        return Err(format!(
            "retained static bundle {label}.sha256 {claimed_sha256} != retained file sha256 {observed_sha256}"
        ));
    }
    snapshot::parse_json_strict(&bytes)
        .map_err(|error| format!("retained static bundle {label} is not strict JSON: {error}"))
}

fn canonical_sha256(value: Option<&Value>, label: &str) -> Result<String, String> {
    let sha256 = value
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} is missing or not a string"))?;
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{label} must be exactly 64 lowercase hexadecimal characters"
        ));
    }
    Ok(sha256.to_string())
}

fn required_static_manifest_hashes(manifest: &Value) -> Result<BTreeMap<String, String>, String> {
    let required = compact_contract::REQUIRED_STATIC_ARTIFACTS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let production_profile = manifest
        .get("production_profile")
        .and_then(Value::as_object)
        .ok_or_else(|| "manifest production_profile is missing or not an object".to_string())?;
    if production_profile.get("enabled").and_then(Value::as_bool) != Some(true) {
        return Err("manifest production_profile.enabled must be true".into());
    }
    let profile_rows = production_profile
        .get("required_static_artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "manifest production_profile.required_static_artifacts is missing or not an array"
                .to_string()
        })?;
    let mut profile_names = BTreeSet::new();
    for (index, row) in profile_rows.iter().enumerate() {
        let name = row.as_str().ok_or_else(|| {
            format!(
                "manifest production_profile.required_static_artifacts[{index}] is not a string"
            )
        })?;
        if !profile_names.insert(name) {
            return Err(format!(
                "manifest production_profile.required_static_artifacts duplicates role {name:?}"
            ));
        }
    }
    if profile_names != required {
        return Err(format!(
            "manifest production_profile.required_static_artifacts {:?} != canonical {:?}",
            profile_names, required
        ));
    }
    if !production_profile
        .get("missing_required_static_artifacts")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        return Err(
            "manifest production_profile.missing_required_static_artifacts must be an empty array"
                .into(),
        );
    }

    let rows = manifest
        .get("replay_packaging")
        .and_then(|value| value.get("static_artifacts"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "manifest replay_packaging.static_artifacts is missing or not an array".to_string()
        })?;
    if rows.len() != compact_contract::REQUIRED_STATIC_ARTIFACTS.len() {
        return Err(format!(
            "manifest replay_packaging.static_artifacts has {} rows, expected exactly {}",
            rows.len(),
            compact_contract::REQUIRED_STATIC_ARTIFACTS.len(),
        ));
    }
    let mut hashes = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                format!("manifest replay_packaging.static_artifacts[{index}].name is invalid")
            })?;
        if !required.contains(name) {
            return Err(format!(
                "manifest replay_packaging.static_artifacts contains unexpected role {name:?}"
            ));
        }
        let sha256 = canonical_sha256(
            row.get("sha256"),
            &format!("manifest static artifact {name:?} sha256"),
        )?;
        if hashes.insert(name.to_string(), sha256).is_some() {
            return Err(format!(
                "manifest replay_packaging.static_artifacts duplicates role {name:?}"
            ));
        }
    }
    for name in &required {
        if !hashes.contains_key(*name) {
            return Err(format!(
                "manifest replay_packaging.static_artifacts is missing required role {name:?}"
            ));
        }
    }

    let static_bundle = manifest
        .get("static_bundle")
        .and_then(Value::as_object)
        .ok_or_else(|| "manifest static_bundle is missing or not an object".to_string())?;
    if static_bundle.get("admission").and_then(Value::as_str) != Some("passed") {
        return Err("manifest static_bundle.admission must be passed".into());
    }
    canonical_sha256(
        static_bundle.get("bundle_id"),
        "manifest static_bundle.bundle_id",
    )?;
    let session_product_modset_sha256 = canonical_sha256(
        static_bundle.get("product_modset_sha256"),
        "manifest static_bundle.product_modset_sha256",
    )?;
    let portable_content = manifest
        .get("portable_content_manifest")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "production manifest portable_content_manifest is missing or not an object"
                .to_string()
        })?;
    if portable_content.get("schema").and_then(Value::as_str)
        != Some("hoi4.portable-content-manifest.v2")
    {
        return Err(
            "production manifest portable_content_manifest is not v2 product-modset-bound"
                .into(),
        );
    }
    let portable_product_modset_sha256 = canonical_sha256(
        portable_content.get("product_modset_sha256"),
        "manifest portable_content_manifest.product_modset_sha256",
    )?;
    if portable_product_modset_sha256 != session_product_modset_sha256 {
        return Err(
            "portable content manifest product-modset key differs from static-bundle admission"
                .into(),
        );
    }
    if static_bundle.get("artifact_count").and_then(Value::as_u64)
        != u64::try_from(required.len()).ok()
    {
        return Err(format!(
            "manifest static_bundle.artifact_count must equal {}",
            required.len()
        ));
    }
    let bundle_rows = static_bundle
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| "manifest static_bundle.artifacts is missing or not an array".to_string())?;
    if bundle_rows.len() != required.len() {
        return Err(format!(
            "manifest static_bundle.artifacts has {} rows, expected exactly {}",
            bundle_rows.len(),
            required.len()
        ));
    }
    let mut bundle_hashes = BTreeMap::new();
    for (index, row) in bundle_rows.iter().enumerate() {
        let role = row
            .get("role")
            .and_then(Value::as_str)
            .filter(|role| !role.is_empty())
            .ok_or_else(|| format!("manifest static_bundle.artifacts[{index}].role is invalid"))?;
        if !required.contains(role) {
            return Err(format!(
                "manifest static_bundle.artifacts contains unexpected role {role:?}"
            ));
        }
        let file = row
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("manifest static_bundle role {role:?} file is missing"))?;
        let expected_file = required_static_bundle_file(role)
            .expect("required static artifact has a canonical bundle file mapping");
        if file != expected_file {
            return Err(format!(
                "manifest static_bundle role {role:?} file {file:?} != canonical {expected_file:?}"
            ));
        }
        let sha256 = canonical_sha256(
            row.get("sha256"),
            &format!("manifest static_bundle role {role:?} sha256"),
        )?;
        canonical_sha256(
            row.get("artifact_sha256"),
            &format!("manifest static_bundle role {role:?} artifact_sha256"),
        )?;
        canonical_sha256(
            row.get("source_gate_report_sha256"),
            &format!("manifest static_bundle role {role:?} source_gate_report_sha256"),
        )?;
        if bundle_hashes.insert(role.to_string(), sha256).is_some() {
            return Err(format!(
                "manifest static_bundle.artifacts duplicates role {role:?}"
            ));
        }
    }
    if bundle_hashes != hashes {
        return Err(
            "manifest static_bundle artifact role/hash map does not exactly match replay_packaging.static_artifacts"
                .into(),
        );
    }
    let validate_file_evidence =
        |value: Option<&Value>, label: &str, require_bytes: bool| -> Result<(), String> {
            let evidence = value.and_then(Value::as_object).ok_or_else(|| {
                format!("manifest static_bundle {label} is missing or not an object")
            })?;
            if evidence
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .is_none()
            {
                return Err(format!(
                    "manifest static_bundle {label}.path is missing or empty"
                ));
            }
            if require_bytes {
                evidence
                    .get("bytes")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| format!("manifest static_bundle {label}.bytes is invalid"))?;
            }
            canonical_sha256(
                evidence.get("sha256"),
                &format!("manifest static_bundle {label}.sha256"),
            )?;
            Ok(())
        };
    validate_file_evidence(static_bundle.get("manifest"), "manifest", false)?;
    let bindings = static_bundle
        .get("bindings")
        .and_then(Value::as_object)
        .ok_or_else(|| "manifest static_bundle.bindings is missing or not an object".to_string())?;
    let required_bindings = [
        "executable",
        "mod_state",
        "dlc_state",
        "load_order",
        "source_inventory",
        "live_resolution_report",
    ];
    if bindings.len() != required_bindings.len() {
        return Err(format!(
            "manifest static_bundle.bindings has {} entries, expected exactly {}",
            bindings.len(),
            required_bindings.len()
        ));
    }
    for name in required_bindings {
        validate_file_evidence(bindings.get(name), &format!("bindings.{name}"), true)?;
    }
    validate_file_evidence(static_bundle.get("verifier"), "verifier", false)?;
    if static_bundle
        .get("token_gate_required")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("manifest static_bundle.token_gate_required must be true".into());
    }
    Ok(hashes)
}

fn verify_static_manifest_authority(
    manifest: &Value,
) -> Result<VerifiedStaticManifestAuthority, String> {
    let artifact_hashes = required_static_manifest_hashes(manifest)?;
    let static_bundle = manifest
        .get("static_bundle")
        .and_then(Value::as_object)
        .expect("static bundle shape checked above");
    let claimed_bundle_id = canonical_sha256(
        static_bundle.get("bundle_id"),
        "manifest static_bundle.bundle_id",
    )?;
    let claimed_product_modset_sha256 = canonical_sha256(
        static_bundle.get("product_modset_sha256"),
        "manifest static_bundle.product_modset_sha256",
    )?;

    let mut claimed_artifacts = BTreeMap::new();
    for row in static_bundle
        .get("artifacts")
        .expect("static bundle artifact roster checked above")
        .as_array()
        .expect("static bundle artifact roster checked above")
    {
        let role = row["role"]
            .as_str()
            .expect("static bundle artifact role checked above");
        claimed_artifacts.insert(role.to_string(), row);
    }

    let retained_manifest =
        verify_retained_file_evidence(static_bundle.get("manifest"), "manifest", false)?;
    if retained_manifest
        .path
        .file_name()
        .and_then(|name| name.to_str())
        != Some("static_bundle_manifest.json")
    {
        return Err(
            "retained static bundle manifest must be named static_bundle_manifest.json".into(),
        );
    }
    let bundle_dir = retained_manifest
        .path
        .parent()
        .ok_or_else(|| "retained static bundle manifest has no parent directory".to_string())?;
    let retained_json = snapshot::parse_json_strict(&retained_manifest.bytes)
        .map_err(|error| format!("retained static bundle manifest is not strict JSON: {error}"))?;
    if retained_json.get("schema").and_then(Value::as_str) != Some("extractor.static_bundle.v3") {
        return Err(
            "retained static bundle manifest schema is not extractor.static_bundle.v3".into(),
        );
    }
    let retained_object = retained_json
        .as_object()
        .ok_or_else(|| "retained static bundle manifest root is not an object".to_string())?;
    let retained_keys = retained_object
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_keys = [
        "schema",
        "artifacts",
        "bindings",
        "catalog_source_join",
        "token_join",
        "bundle_id",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if retained_keys != expected_keys {
        return Err(format!(
            "retained static bundle manifest fields {:?} != canonical {:?}",
            retained_keys, expected_keys
        ));
    }
    let retained_bundle_id = canonical_sha256(
        retained_json.get("bundle_id"),
        "retained static bundle manifest bundle_id",
    )?;
    let mut unhashed_manifest = retained_json.clone();
    unhashed_manifest
        .as_object_mut()
        .ok_or_else(|| "retained static bundle manifest root is not an object".to_string())?
        .remove("bundle_id");
    let recomputed_bundle_id = canonical_json_sha256(&unhashed_manifest)?;
    if retained_bundle_id != recomputed_bundle_id || claimed_bundle_id != retained_bundle_id {
        return Err(format!(
            "static bundle_id is not the canonical retained manifest identity: session={claimed_bundle_id}, retained={retained_bundle_id}, recomputed={recomputed_bundle_id}"
        ));
    }
    let source_join = verify_retained_bundle_json_file(
        bundle_dir,
        retained_json.get("catalog_source_join"),
        "catalog_source_join",
        "file",
        "proofs/catalog_source_join.json",
    )?;
    let source_join_evidence = retained_json["catalog_source_join"]
        .as_object()
        .expect("catalog source join evidence checked above");
    if source_join.get("passed").and_then(Value::as_bool) != Some(true)
        || source_join.get("schema").and_then(Value::as_str)
            != source_join_evidence.get("schema").and_then(Value::as_str)
    {
        return Err(
            "retained static bundle catalog_source_join did not pass or has a schema mismatch"
                .into(),
        );
    }
    let token_join = retained_json
        .get("token_join")
        .and_then(Value::as_object)
        .ok_or_else(|| "retained static bundle token_join evidence is missing".to_string())?;
    if token_join.get("status").and_then(Value::as_str) != Some("passed") {
        return Err("retained static bundle token_join.status must be passed".into());
    }
    let token_report = verify_retained_bundle_json_file(
        bundle_dir,
        retained_json.get("token_join"),
        "token_join",
        "report",
        "token_join_report.json",
    )?;
    if token_report.get("passed").and_then(Value::as_bool) != Some(true)
        || token_report.get("provenance") != token_join.get("provenance")
    {
        return Err(
            "retained static bundle token_join report did not pass or provenance mismatches manifest"
                .into(),
        );
    }

    let retained_rows = retained_json
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| "retained static bundle manifest artifacts are missing".to_string())?;
    if retained_rows.len() != compact_contract::REQUIRED_STATIC_ARTIFACTS.len() {
        return Err(format!(
            "retained static bundle manifest has {} artifacts, expected exactly {}",
            retained_rows.len(),
            compact_contract::REQUIRED_STATIC_ARTIFACTS.len()
        ));
    }
    let required = compact_contract::REQUIRED_STATIC_ARTIFACTS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut bundle_artifacts = BTreeMap::new();
    for (index, row) in retained_rows.iter().enumerate() {
        let role = row
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("retained static bundle artifact[{index}].role is invalid"))?;
        if !required.contains(role) {
            return Err(format!(
                "retained static bundle contains unexpected artifact role {role:?}"
            ));
        }
        let claimed = claimed_artifacts.get(role).ok_or_else(|| {
            format!("session static bundle omits retained artifact role {role:?}")
        })?;
        let file = row
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("retained static bundle role {role:?} file is invalid"))?;
        let expected_file = required_static_bundle_file(role)
            .expect("required static artifact has a canonical bundle file mapping");
        if file != expected_file || claimed.get("file").and_then(Value::as_str) != Some(file) {
            return Err(format!(
                "static bundle role {role:?} file is not exactly bound to canonical retained path {expected_file:?}"
            ));
        }
        let source_report = row
            .get("source_gate_report")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("retained static bundle role {role:?} source_gate_report is invalid")
            })?;
        let expected_source_report = required_static_gate_report_file(role)
            .expect("required static artifact has a canonical gate report mapping");
        if source_report != expected_source_report {
            return Err(format!(
                "retained static bundle role {role:?} source_gate_report {source_report:?} != canonical {expected_source_report:?}"
            ));
        }
        let retained_sha256 = canonical_sha256(
            row.get("sha256"),
            &format!("retained static bundle role {role:?} sha256"),
        )?;
        let artifact_sha256 = canonical_sha256(
            row.get("artifact_sha256"),
            &format!("retained static bundle role {role:?} artifact_sha256"),
        )?;
        let source_gate_report_sha256 = canonical_sha256(
            row.get("source_gate_report_sha256"),
            &format!("retained static bundle role {role:?} source_gate_report_sha256"),
        )?;
        for (field, retained_value) in [
            ("sha256", &retained_sha256),
            ("artifact_sha256", &artifact_sha256),
            ("source_gate_report_sha256", &source_gate_report_sha256),
        ] {
            if claimed.get(field).and_then(Value::as_str) != Some(retained_value.as_str()) {
                return Err(format!(
                    "session static bundle role {role:?} {field} does not match retained bundle manifest"
                ));
            }
        }
        let retained_bytes = row
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("retained static bundle role {role:?} bytes is invalid"))?;
        let artifact_path = contained_bundle_file(bundle_dir, file, &format!("artifact {role}"))?;
        let artifact_bytes = fs::read(&artifact_path).map_err(|error| {
            format!("cannot read retained static bundle artifact {role:?}: {error}")
        })?;
        if u64::try_from(artifact_bytes.len()).unwrap_or(u64::MAX) != retained_bytes
            || Sha256::hex(&artifact_bytes) != retained_sha256
        {
            return Err(format!(
                "retained static bundle artifact {role:?} bytes/hash do not match its manifest row"
            ));
        }
        let artifact_json = snapshot::parse_json_strict(&artifact_bytes).map_err(|error| {
            format!("retained static bundle artifact {role:?} is not strict JSON: {error}")
        })?;
        if row.get("schema").and_then(Value::as_str)
            != artifact_json.get("schema").and_then(Value::as_str)
        {
            return Err(format!(
                "retained static bundle artifact {role:?} schema does not match its manifest row"
            ));
        }
        let embedded_artifact_sha256 = canonical_sha256(
            artifact_json.get("artifact_sha256"),
            &format!("retained static bundle artifact {role:?} embedded artifact_sha256"),
        )?;
        let mut unhashed_artifact = artifact_json.clone();
        unhashed_artifact
            .as_object_mut()
            .ok_or_else(|| {
                format!("retained static bundle artifact {role:?} root is not an object")
            })?
            .remove("artifact_sha256");
        let recomputed_artifact_sha256 = canonical_json_sha256(&unhashed_artifact)?;
        if embedded_artifact_sha256 != recomputed_artifact_sha256
            || artifact_sha256 != embedded_artifact_sha256
        {
            return Err(format!(
                "static bundle artifact {role:?} self-hash is not bound to canonical artifact bytes"
            ));
        }

        let report_path = contained_bundle_file(
            bundle_dir,
            source_report,
            &format!("source gate report {role}"),
        )?;
        let report_bytes = fs::read(&report_path).map_err(|error| {
            format!("cannot read retained source gate report {role:?}: {error}")
        })?;
        let observed_report_sha256 = Sha256::hex(&report_bytes);
        if observed_report_sha256 != source_gate_report_sha256 {
            return Err(format!(
                "static bundle role {role:?} source_gate_report_sha256 is not bound to retained report bytes"
            ));
        }
        let report = snapshot::parse_json_strict(&report_bytes).map_err(|error| {
            format!("retained source gate report {role:?} is not strict JSON: {error}")
        })?;
        if report.get("passed").and_then(Value::as_bool) != Some(true)
            || report.get("artifact_sha256").and_then(Value::as_str)
                != Some(artifact_sha256.as_str())
        {
            return Err(format!(
                "retained source gate report {role:?} did not pass or names another artifact"
            ));
        }
        if bundle_artifacts
            .insert(
                role.to_string(),
                StaticBundleArtifactBinding {
                    sha256: retained_sha256,
                    artifact_sha256,
                },
            )
            .is_some()
        {
            return Err(format!(
                "retained static bundle duplicates artifact role {role:?}"
            ));
        }
    }
    if bundle_artifacts.len() != required.len() {
        return Err(
            "retained static bundle does not contain the exact required artifact roster".into(),
        );
    }

    let retained_bindings = retained_json
        .get("bindings")
        .and_then(Value::as_object)
        .ok_or_else(|| "retained static bundle bindings are missing".to_string())?;
    let product_modset = retained_bindings
        .get("product_modset")
        .and_then(Value::as_object)
        .ok_or_else(|| "retained static bundle product_modset binding is missing".to_string())?;
    let product_fields = [
        "schema",
        "executable_sha256",
        "mod_state_effective_content_sha256",
        "dlc_state_effective_content_sha256",
        "load_order_effective_content_sha256",
        "effective_vfs_namespace_sha256",
        "effective_vfs_rows_sha256",
        "product_modset_sha256",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if product_modset.keys().map(String::as_str).collect::<BTreeSet<_>>() != product_fields
        || product_modset.get("schema").and_then(Value::as_str)
            != Some("extractor.product_modset_key.v1")
    {
        return Err("retained static bundle product_modset binding is malformed".into());
    }
    let retained_product_modset_sha256 = canonical_sha256(
        product_modset.get("product_modset_sha256"),
        "retained static bundle product_modset_sha256",
    )?;
    let mut product_core = Value::Object(product_modset.clone());
    product_core
        .as_object_mut()
        .expect("product_modset object cloned above")
        .remove("product_modset_sha256");
    let recomputed_product_modset_sha256 = canonical_json_sha256(&product_core)?;
    if retained_product_modset_sha256 != recomputed_product_modset_sha256
        || claimed_product_modset_sha256 != retained_product_modset_sha256
    {
        return Err(format!(
            "product modset identity mismatch: session={claimed_product_modset_sha256}, retained={retained_product_modset_sha256}, recomputed={recomputed_product_modset_sha256}"
        ));
    }
    let session_bindings = static_bundle
        .get("bindings")
        .expect("session binding roster checked above")
        .as_object()
        .expect("session binding roster checked above");
    let executable = verify_retained_file_evidence(
        session_bindings.get("executable"),
        "bindings.executable",
        true,
    )?;
    let retained_executable = retained_bindings
        .get("executable")
        .and_then(Value::as_object)
        .ok_or_else(|| "retained static bundle executable binding is missing".to_string())?;
    if retained_executable.get("bytes").and_then(Value::as_u64)
        != u64::try_from(executable.bytes.len()).ok()
        || retained_executable.get("sha256").and_then(Value::as_str)
            != Some(executable.sha256.as_str())
        || retained_executable.get("name").and_then(Value::as_str)
            != executable.path.file_name().and_then(|name| name.to_str())
    {
        return Err(
            "retained static bundle executable binding does not match retained executable bytes"
                .into(),
        );
    }
    if product_modset.get("executable_sha256").and_then(Value::as_str)
        != Some(executable.sha256.as_str())
    {
        return Err("product modset executable identity differs from retained executable".into());
    }
    for name in ["mod_state", "dlc_state", "load_order"] {
        let retained_file = verify_retained_file_evidence(
            session_bindings.get(name),
            &format!("bindings.{name}"),
            true,
        )?;
        let document = snapshot::parse_json_strict(&retained_file.bytes).map_err(|error| {
            format!("retained static bundle binding {name:?} is not strict JSON: {error}")
        })?;
        let binding = retained_bindings
            .get(name)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("retained static bundle binding {name:?} is missing"))?;
        let canonical_document_sha256 = canonical_json_sha256(&document)?;
        if binding.get("role").and_then(Value::as_str) != Some(name)
            || binding.get("sha256").and_then(Value::as_str)
                != Some(canonical_document_sha256.as_str())
            || binding.get("bytes_sha256").and_then(Value::as_str)
                != Some(retained_file.sha256.as_str())
        {
            return Err(format!(
                "retained static bundle binding {name:?} does not match retained binding document bytes"
            ));
        }
        let product_field = match name {
            "mod_state" => "mod_state_effective_content_sha256",
            "dlc_state" => "dlc_state_effective_content_sha256",
            _ => "load_order_effective_content_sha256",
        };
        if canonical_sha256(
            binding.get("effective_content_sha256"),
            &format!("retained static bundle binding {name:?} effective_content_sha256"),
        )? != canonical_sha256(
            product_modset.get(product_field),
            &format!("product_modset.{product_field}"),
        )? {
            return Err(format!(
                "product modset {product_field} differs from retained {name} binding"
            ));
        }
    }
    let source_inventory = verify_retained_file_evidence(
        session_bindings.get("source_inventory"),
        "bindings.source_inventory",
        true,
    )?;
    let live_source = retained_bindings
        .get("live_source_inventory")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "retained static bundle live_source_inventory binding is missing".to_string()
        })?;
    let embedded_source_relative =
        live_source
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                "retained static bundle live_source_inventory.file is invalid".to_string()
            })?;
    if embedded_source_relative != "bindings/source_inventory.json" {
        return Err("retained static bundle live_source_inventory.file is not canonical".into());
    }
    let embedded_source_path = contained_bundle_file(
        bundle_dir,
        embedded_source_relative,
        "live source inventory",
    )?;
    let embedded_source_bytes = fs::read(&embedded_source_path)
        .map_err(|error| format!("cannot read embedded live source inventory: {error}"))?;
    if embedded_source_bytes != source_inventory.bytes
        || live_source.get("bytes").and_then(Value::as_u64)
            != u64::try_from(embedded_source_bytes.len()).ok()
        || live_source.get("file_sha256").and_then(Value::as_str)
            != Some(source_inventory.sha256.as_str())
    {
        return Err(
            "retained static bundle live source inventory is not byte-identical to retained binding evidence"
                .into(),
        );
    }
    for (binding_field, product_field) in [
        ("namespace_sha256", "effective_vfs_namespace_sha256"),
        ("rows_sha256", "effective_vfs_rows_sha256"),
    ] {
        if canonical_sha256(
            live_source.get(binding_field),
            &format!("retained live source {binding_field}"),
        )? != canonical_sha256(
            product_modset.get(product_field),
            &format!("product_modset.{product_field}"),
        )? {
            return Err(format!(
                "product modset {product_field} differs from retained live source binding"
            ));
        }
    }
    verify_retained_file_evidence(
        session_bindings.get("live_resolution_report"),
        "bindings.live_resolution_report",
        true,
    )?;
    let verifier = verify_retained_file_evidence(static_bundle.get("verifier"), "verifier", false)?;

    Ok(VerifiedStaticManifestAuthority {
        artifact_hashes,
        bundle_artifacts,
        bundle_id: recomputed_bundle_id,
        product_modset_sha256: recomputed_product_modset_sha256,
        retained_manifest_sha256: retained_manifest.sha256,
        retained_binding_count: session_bindings.len(),
        verifier_sha256: verifier.sha256,
    })
}

fn validate_required_static_artifacts(
    container: &Container,
    manifest: &Value,
    game_id: &str,
) -> Result<Value, String> {
    let manifest_authority = verify_static_manifest_authority(manifest)?;
    let expected_count = compact_contract::REQUIRED_STATIC_ARTIFACTS.len();
    let expected_index_path = format!("payload/{game_id}/index.json");
    let index_entries = container.entries_by_role("index");
    if index_entries.len() != 1 || index_entries[0].path != expected_index_path {
        return Err(format!(
            "replay container must have exactly one canonical index role at {expected_index_path:?}, found {:?}",
            index_entries.iter().map(|entry| entry.path.as_str()).collect::<Vec<_>>()
        ));
    }
    let index_bytes = container
        .read_entry_bytes(&expected_index_path)
        .map_err(|error| format!("cannot read authoritative replay index: {error}"))?;
    let index = snapshot::parse_json_strict(&index_bytes)
        .map_err(|error| format!("authoritative replay index is not strict JSON: {error}"))?;
    if index.get("game_id").and_then(Value::as_str) != Some(game_id) {
        return Err("authoritative replay index game_id does not match container identity".into());
    }
    let rows = index
        .get("static_artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "authoritative replay index.static_artifacts is missing or not an array".to_string()
        })?;
    if rows.len() != expected_count {
        return Err(format!(
            "authoritative replay index.static_artifacts has {} rows, expected exactly {expected_count}",
            rows.len()
        ));
    }
    let required = compact_contract::REQUIRED_STATIC_ARTIFACTS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut index_bindings = BTreeMap::<String, StaticArtifactIndexBinding>::new();
    let mut relative_paths = BTreeSet::new();
    for (row_index, row) in rows.iter().enumerate() {
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("replay index static_artifacts[{row_index}].name is invalid"))?;
        if !required.contains(name) {
            return Err(format!(
                "authoritative replay index contains unexpected static role {name:?}"
            ));
        }
        if row.get("present").and_then(Value::as_bool) != Some(true) {
            return Err(format!(
                "authoritative replay index static role {name:?} is not present=true"
            ));
        }
        if row.get("role").and_then(Value::as_str) != Some("static_artifact") {
            return Err(format!(
                "authoritative replay index static role {name:?} does not declare role static_artifact"
            ));
        }
        let relative_path = row
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("replay index static role {name:?} path is missing"))?;
        let expected_relative_path = required_static_artifact_relative_path(name)
            .expect("required static artifact has a canonical path mapping");
        if relative_path != expected_relative_path {
            return Err(format!(
                "authoritative replay index static role {name:?} path {relative_path:?} != canonical {expected_relative_path:?}"
            ));
        }
        if !relative_paths.insert(relative_path.to_string()) {
            return Err(format!(
                "authoritative replay index aliases multiple static roles to path {relative_path:?}"
            ));
        }
        let sha256 = canonical_sha256(
            row.get("sha256"),
            &format!("replay index static role {name:?} sha256"),
        )?;
        let bytes = row
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("replay index static role {name:?} bytes is invalid"))?;
        if index_bindings
            .insert(
                name.to_string(),
                StaticArtifactIndexBinding {
                    relative_path: relative_path.to_string(),
                    sha256,
                    bytes,
                },
            )
            .is_some()
        {
            return Err(format!(
                "authoritative replay index duplicates static role {name:?}"
            ));
        }
    }
    for name in &required {
        if !index_bindings.contains_key(*name) {
            return Err(format!(
                "authoritative replay index is missing required static role {name:?}"
            ));
        }
    }

    let static_entries = container.entries_by_role("static_artifact");
    if static_entries.len() != expected_count {
        return Err(format!(
            "replay container has {} static_artifact entries, expected exactly {expected_count}",
            static_entries.len()
        ));
    }
    let static_prefix = format!("payload/{game_id}/static/");
    for entry in container.entries() {
        if entry.path.starts_with(&static_prefix)
            && entry.role.as_deref() != Some("static_artifact")
        {
            return Err(format!(
                "replay container static path {:?} has non-static role {:?}",
                entry.path, entry.role
            ));
        }
    }
    let mut entries_by_path = BTreeMap::new();
    for entry in static_entries {
        let path = entry.path.clone();
        if entries_by_path.insert(path.clone(), entry).is_some() {
            return Err(format!(
                "replay container duplicates static_artifact path {path:?}"
            ));
        }
    }

    let mut details = Vec::with_capacity(expected_count);
    for name in compact_contract::REQUIRED_STATIC_ARTIFACTS {
        let binding = index_bindings
            .get(*name)
            .expect("required index binding checked above");
        let manifest_sha256 = manifest_authority
            .artifact_hashes
            .get(*name)
            .expect("required manifest binding checked above");
        if manifest_sha256 != &binding.sha256 {
            return Err(format!(
                "static role {name:?} manifest sha256 {manifest_sha256} != replay index sha256 {}",
                binding.sha256
            ));
        }
        let container_path = format!("payload/{game_id}/{}", binding.relative_path);
        let entry = entries_by_path.remove(&container_path).ok_or_else(|| {
            format!("static role {name:?} has no exact container entry at {container_path:?}")
        })?;
        if entry.sha256 != binding.sha256 || entry.bytes != binding.bytes {
            return Err(format!(
                "static role {name:?} container metadata sha256/bytes {}/{} != replay index {}/{}",
                entry.sha256, entry.bytes, binding.sha256, binding.bytes
            ));
        }
        let bytes = container
            .read_entry_bytes(&container_path)
            .map_err(|error| format!("cannot read static role {name:?}: {error}"))?;
        let observed_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let observed_sha256 = Sha256::hex(&bytes);
        if observed_bytes != binding.bytes || observed_sha256 != binding.sha256 {
            return Err(format!(
                "static role {name:?} bytes/hash {observed_bytes}/{observed_sha256} != bound {}/{}",
                binding.bytes, binding.sha256
            ));
        }
        let bundle_binding = manifest_authority
            .bundle_artifacts
            .get(*name)
            .expect("retained bundle artifact roster checked above");
        if observed_sha256 != bundle_binding.sha256 {
            return Err(format!(
                "static role {name:?} admitted bytes sha256 {observed_sha256} != retained bundle artifact sha256 {}",
                bundle_binding.sha256
            ));
        }
        let artifact_json = snapshot::parse_json_strict(&bytes).map_err(|error| {
            format!("admitted static role {name:?} is not strict JSON: {error}")
        })?;
        let embedded_artifact_sha256 = canonical_sha256(
            artifact_json.get("artifact_sha256"),
            &format!("admitted static role {name:?} artifact_sha256"),
        )?;
        let mut unhashed_artifact = artifact_json.clone();
        unhashed_artifact
            .as_object_mut()
            .ok_or_else(|| format!("admitted static role {name:?} root is not an object"))?
            .remove("artifact_sha256");
        let recomputed_artifact_sha256 = canonical_json_sha256(&unhashed_artifact)?;
        if embedded_artifact_sha256 != recomputed_artifact_sha256
            || embedded_artifact_sha256 != bundle_binding.artifact_sha256
        {
            return Err(format!(
                "static role {name:?} artifact_sha256 is not bound to canonical admitted artifact bytes"
            ));
        }
        details.push(json!({
            "name": name,
            "path": container_path,
            "bytes": observed_bytes,
            "sha256": observed_sha256,
        }));
    }
    if !entries_by_path.is_empty() {
        return Err(format!(
            "replay container has unexpected static_artifact entries: {:?}",
            entries_by_path.keys().collect::<Vec<_>>()
        ));
    }
    Ok(json!({
        "required_count": expected_count,
        "exact_required_roles": true,
        "bundle_id": manifest_authority.bundle_id,
        "product_modset_sha256": manifest_authority.product_modset_sha256,
        "retained_manifest_sha256": manifest_authority.retained_manifest_sha256,
        "retained_binding_count": manifest_authority.retained_binding_count,
        "verifier_sha256": manifest_authority.verifier_sha256,
        "artifacts": details,
    }))
}

fn load_verified_replay_authority(
    external_manifest: &Value,
    external_manifest_bytes: &[u8],
    run_dir: &Path,
) -> Result<VerifiedReplayAuthority, String> {
    let replay_path = required_run_file(run_dir, external_manifest, "replay_file")?;
    let expected = ExpectedIdentity {
        schema_version: Some("native_collector_session_v1".to_string()),
        ..ExpectedIdentity::default()
    };
    let container = Container::open(&replay_path, &expected).map_err(|err| {
        format!(
            "cannot open replay container {}: {err}",
            replay_path.display()
        )
    })?;
    container.verify_all().map_err(|err| {
        format!(
            "replay container verify_all failed for {}: {err}",
            replay_path.display()
        )
    })?;

    let replay_game_id = required_nonempty_str(external_manifest, "replay_game_id")?;
    if container.game_id() != replay_game_id {
        return Err(format!(
            "replay container game_id {:?} != recorder replay_game_id {:?}",
            container.game_id(),
            replay_game_id
        ));
    }
    let layout = replay_layout(external_manifest)?;

    let stream_entries = container.entries_by_role("collector_compact_stream");
    if stream_entries.len() != 1 {
        return Err(format!(
            "replay container has {} collector_compact_stream entries, expected 1",
            stream_entries.len()
        ));
    }
    let manifest_entries = container.entries_by_role("recorder_manifest");
    if manifest_entries.len() != 1 {
        return Err(format!(
            "replay container has {} recorder_manifest entries, expected 1",
            manifest_entries.len()
        ));
    }
    let expected_stream_path = format!("payload/{replay_game_id}/{}", layout.stream);
    let expected_manifest_path = format!("payload/{replay_game_id}/{}", layout.session_manifest);
    let stream_entry = &stream_entries[0];
    let manifest_entry = &manifest_entries[0];
    if stream_entry.path != expected_stream_path {
        return Err(format!(
            "collector_compact_stream path {:?} != replay_entries.stream authority {:?}",
            stream_entry.path, expected_stream_path
        ));
    }
    if manifest_entry.path != expected_manifest_path {
        return Err(format!(
            "recorder_manifest path {:?} != replay_entries.session_manifest authority {:?}",
            manifest_entry.path, expected_manifest_path
        ));
    }

    let container_stream = container
        .read_entry_bytes(&stream_entry.path)
        .map_err(|err| {
            format!(
                "cannot read collector_compact_stream {} from replay container: {err}",
                stream_entry.path
            )
        })?;
    let embedded_manifest_bytes =
        container
            .read_entry_bytes(&manifest_entry.path)
            .map_err(|err| {
                format!(
                    "cannot read recorder_manifest {} from replay container: {err}",
                    manifest_entry.path
                )
            })?;
    let external_sha256 = Sha256::hex(external_manifest_bytes);
    let embedded_sha256 = Sha256::hex(&embedded_manifest_bytes);
    if embedded_manifest_bytes != external_manifest_bytes {
        return Err(format!(
            "external native_session_manifest.json is not byte-identical to the authoritative embedded recorder_manifest (external bytes {}, sha256 {}; embedded bytes {}, sha256 {}); rejecting mutable-manifest rewrite",
            external_manifest_bytes.len(),
            external_sha256,
            embedded_manifest_bytes.len(),
            embedded_sha256,
        ));
    }
    let embedded_manifest = snapshot::parse_json_strict(&embedded_manifest_bytes)
        .map_err(|err| format!("embedded recorder_manifest is not strict JSON: {err}"))?;
    if required_nonempty_str(&embedded_manifest, "replay_game_id")? != container.game_id() {
        return Err(
            "embedded recorder_manifest replay_game_id changed after identity binding".into(),
        );
    }
    let static_artifacts =
        validate_required_static_artifacts(&container, &embedded_manifest, container.game_id())?;
    let container_content = container.content_manifest().ok_or_else(|| {
        "production replay container has no top-level portable content manifest".to_string()
    })?;
    let container_content_value = serde_json::to_value(container_content)
        .map_err(|error| format!("serialize admitted container content manifest failed: {error}"))?;
    if embedded_manifest.get("portable_content_manifest") != Some(&container_content_value) {
        return Err(
            "container top-level portable content manifest differs from embedded recorder authority"
                .into(),
        );
    }
    Ok(VerifiedReplayAuthority {
        manifest: embedded_manifest,
        container_stream,
        details: json!({
            "file": replay_path.display().to_string(),
            "game_id": container.game_id(),
            "collector_compact_stream": stream_entry.path,
            "collector_compact_stream_bytes": stream_entry.bytes,
            "collector_compact_stream_sha256": stream_entry.sha256,
            "recorder_manifest": manifest_entry.path,
            "recorder_manifest_bytes": embedded_manifest_bytes.len(),
            "recorder_manifest_sha256": embedded_sha256,
            "external_manifest_exact_match": true,
            "static_artifacts": static_artifacts,
            "signed": container.signed,
            "unsigned": container.unsigned,
        }),
    })
}

#[derive(Debug)]
struct ReplayLayout<'a> {
    stream: &'a str,
    session_manifest: &'a str,
}

fn replay_layout(manifest: &Value) -> Result<ReplayLayout<'_>, String> {
    let entries = manifest
        .get("replay_entries")
        .and_then(Value::as_object)
        .ok_or_else(|| "manifest replay_entries is missing or not an object".to_string())?;
    let string = |key: &str| {
        entries
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("manifest replay_entries.{key} is missing or empty"))
    };
    if string("stream_role")? != "collector_compact_stream" {
        return Err("manifest replay_entries.stream_role must be collector_compact_stream".into());
    }
    if string("session_manifest_role")? != "recorder_manifest" {
        return Err(
            "manifest replay_entries.session_manifest_role must be recorder_manifest".into(),
        );
    }
    let stream = string("stream")?;
    let session_manifest = string("session_manifest")?;
    validate_relative_path(stream, "replay_entries.stream")?;
    validate_relative_path(session_manifest, "replay_entries.session_manifest")?;
    Ok(ReplayLayout {
        stream,
        session_manifest,
    })
}

fn required_nonempty_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("manifest {key} is missing or empty"))
}

fn validate_relative_path(raw: &str, label: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if raw.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_)
                    | Component::RootDir
                    | Component::ParentDir
                    | Component::CurDir
            )
        })
    {
        return Err(format!(
            "manifest {label} must be a run-directory-contained relative path without . or .. components, got {raw:?}"
        ));
    }
    Ok(path.to_path_buf())
}

fn required_run_file(run_dir: &Path, manifest: &Value, key: &str) -> Result<PathBuf, String> {
    let relative = validate_relative_path(required_nonempty_str(manifest, key)?, key)?;
    let root = fs::canonicalize(run_dir).map_err(|err| {
        format!(
            "cannot canonicalize run directory {}: {err}",
            run_dir.display()
        )
    })?;
    let candidate = run_dir.join(relative);
    let canonical = fs::canonicalize(&candidate).map_err(|err| {
        format!(
            "cannot resolve manifest {key} {}: {err}",
            candidate.display()
        )
    })?;
    if !canonical.starts_with(&root) || !canonical.is_file() {
        return Err(format!(
            "manifest {key} does not resolve to a file contained by run directory {}: {}",
            root.display(),
            canonical.display()
        ));
    }
    Ok(canonical)
}

#[derive(Debug)]
struct StreamArtifact {
    path: PathBuf,
    compression: compact_stream::StreamCompression,
    stored: Vec<u8>,
    decoded: Vec<u8>,
    stored_sha256: String,
    decoded_sha256: String,
}

impl StreamArtifact {
    fn as_json(&self) -> Value {
        json!({
            "compression": self.compression.as_str(),
            "stored_bytes": self.stored.len(),
            "stored_sha256": self.stored_sha256,
            "decoded_bytes": self.decoded.len(),
            "decoded_sha256": self.decoded_sha256,
        })
    }
}

fn read_stream_artifact(
    path: &Path,
    manifest: &Value,
    max_decoded_bytes: u64,
) -> Result<StreamArtifact, String> {
    let storage = manifest
        .get("stream_storage")
        .and_then(Value::as_object)
        .ok_or_else(|| "manifest stream_storage is missing or not an object".to_string())?;
    let compression_name = storage
        .get("compression")
        .and_then(Value::as_str)
        .ok_or_else(|| "manifest stream_storage.compression is missing".to_string())?;
    let compression = match compression_name {
        "none" => compact_stream::StreamCompression::None,
        "zstd" => compact_stream::StreamCompression::Zstd,
        other => return Err(format!("unsupported manifest stream compression {other:?}")),
    };
    let detected = compact_stream::detect(path).map_err(|err| {
        format!(
            "detect compact stream storage {} failed: {err}",
            path.display()
        )
    })?;
    if detected != compression {
        return Err(format!(
            "manifest stream compression {} does not match stored file detection {}",
            compression.as_str(),
            detected.as_str()
        ));
    }
    let stored = fs::read(path).map_err(|err| {
        format!(
            "cannot read stored compact stream {}: {err}",
            path.display()
        )
    })?;
    let stored_sha256 = Sha256::hex(&stored);
    let expected_stored = storage
        .get("stored_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| "manifest stream_storage.stored_bytes is missing".to_string())?;
    if expected_stored != stored.len() as u64 {
        return Err(format!(
            "stored compact stream length {} != manifest stream_storage.stored_bytes {}",
            stored.len(),
            expected_stored
        ));
    }
    if let Some(expected_hash) = storage.get("stored_sha256").and_then(Value::as_str) {
        if expected_hash != stored_sha256 {
            return Err(format!(
                "stored compact stream sha256 {stored_sha256} != manifest stream_storage.stored_sha256 {expected_hash}"
            ));
        }
    }
    let mut reader = compact_stream::open_with_compression(path, compression).map_err(|err| {
        format!(
            "open decoded compact stream {} failed: {err}",
            path.display()
        )
    })?;
    let mut decoded = Vec::new();
    reader
        .by_ref()
        .take(max_decoded_bytes.saturating_add(1))
        .read_to_end(&mut decoded)
        .map_err(|err| format!("decode compact stream {} failed: {err}", path.display()))?;
    if decoded.len() as u64 > max_decoded_bytes {
        return Err(format!(
            "decoded collector stream bytes {} exceed limit {}",
            decoded.len(),
            max_decoded_bytes
        ));
    }
    let decoded_sha256 = Sha256::hex(&decoded);
    let expected_decoded = storage
        .get("decoded_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| "manifest stream_storage.decoded_bytes is missing".to_string())?;
    if expected_decoded != decoded.len() as u64 {
        return Err(format!(
            "decoded compact stream length {} != manifest stream_storage.decoded_bytes {}",
            decoded.len(),
            expected_decoded
        ));
    }
    let expected_hash = storage
        .get("decoded_sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "manifest stream_storage.decoded_sha256 is missing".to_string())?;
    if expected_hash != decoded_sha256 {
        return Err(format!(
            "decoded compact stream sha256 {decoded_sha256} != manifest stream_storage.decoded_sha256 {expected_hash}"
        ));
    }
    Ok(StreamArtifact {
        path: path.to_path_buf(),
        compression,
        stored,
        decoded,
        stored_sha256,
        decoded_sha256,
    })
}

fn verification_weakenings(options: &GateOptions) -> Vec<&'static str> {
    let mut values = Vec::new();
    if !options.require_replay_verify {
        values.push("replay_container_verification_skipped");
    }
    if options.allow_bridge_roots {
        values.push("bridge_root_debt_tolerated");
    }
    if options.allow_partial_or_source_failure {
        values.push("partial_or_source_failure_tolerated");
    }
    if !options.strict_semantic_units {
        values.push("semantic_unit_requirements_disabled");
    }
    values
}

fn reject_verification_weakenings(weakenings: &[&str], errors: &mut Vec<String>) {
    if !weakenings.is_empty() {
        errors.push(format!(
            "verification profile is non-certifying because it contains weakening(s): {}",
            weakenings.join(", ")
        ));
    }
}

#[derive(Debug, Default)]
struct HeaderScanSummary {
    record_count: u64,
    domain_perf_record_count: u64,
    domain_data_record_count: u64,
    frame_end_record_count: u64,
    total_payload_bytes: u64,
    max_payload_len: u64,
    max_record_len: u64,
    /// Framing-level maxima include reserved instrumentation probes even though
    /// recorder accounting intentionally excludes those records.
    framed_max_payload_len: u64,
    framed_max_record_len: u64,
    /// Global-status OR after applying each domain's lane-local mask.
    global_status_flags_or: u32,
    /// Raw status OR retained per domain so lane-local aliases never lose their
    /// identity before global interpretation.
    per_domain_status_flags_or: BTreeMap<u16, u32>,
    per_domain_perf_authority: BTreeMap<u16, PerfAuthority>,
    per_domain_data_authority: BTreeMap<u16, DataAuthority>,
    per_domain_combined_units: BTreeMap<u16, BTreeMap<String, u64>>,
    per_hour_perf_authority: BTreeMap<(i32, u16), PerfAuthority>,
    per_hour_data_authority: BTreeMap<(i32, u16), DataAuthority>,
    domain57_frames: Vec<recorder_host::domain_data_authority::Domain57FrameEvidence>,
    session_header_identity: recorder_host::campaign_identity::SessionHeaderIdentityObservation,
    /// Opaque placement authority retained only when the shared stateful
    /// validator admits the exact sequence-zero CID1 record.
    validated_cid1: Option<ValidatedCid1>,
    /// FRAME_END carries frame-scoped global status, not per-domain raw status.
    frame_status_flags_or_raw: u32,
    per_hour_frame_authority: BTreeMap<i32, FrameEndAuthority>,
    domains_seen: BTreeSet<u16>,
    frame_end_hour_counts: BTreeMap<i32, u64>,
    per_hour_perf_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    per_hour_data_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    unique_hours: Vec<i32>,
    skipped_hours: i32,
    parse_errors: Vec<String>,
    ordering_errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PerfAuthority {
    row_count: u64,
    changed_rows: u64,
    unit_count: u64,
    changed_unit_count: u64,
    status_flags: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DataAuthority {
    row_count: u64,
    changed_rows: u64,
    status_flags: Option<u32>,
    data_units: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameEndAuthority {
    emitted_domains: u32,
    scheduled_domains: u32,
    status_flags: u32,
}

#[cfg(test)]
fn scan_compact_stream(raw: &[u8]) -> HeaderScanSummary {
    scan_compact_stream_with_identity_mode(raw, StreamIdentityMode::Legacy)
}

fn scan_compact_stream_with_identity_mode(
    raw: &[u8],
    identity_mode: StreamIdentityMode,
) -> HeaderScanSummary {
    let mut summary = HeaderScanSummary::default();
    let mut stream_validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut open_frame_hour: Option<i32> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut final_record_is_clean_frame_end = false;
    let mut framing_complete = true;
    let mut session_header_identity =
        recorder_host::campaign_identity::SessionHeaderIdentityObservation::default();
    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                framing_complete = false;
                break;
            }
        };
        let sequence = header.sequence;
        let game_hour_raw = header.game_hour_raw;
        let domain_id = header.domain_id;
        let record_type = header.record_type;
        let payload_len = header.payload_len;
        let record_len = header.record_len();
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining bytes {remaining}"
            ));
            framing_complete = false;
            break;
        }
        let awaiting_identity = identity_mode == StreamIdentityMode::IdentityV1
            && summary.validated_cid1.is_none();
        if !awaiting_identity {
            if let Some(previous) = previous_sequence {
                match previous.checked_add(1) {
                    Some(expected) if sequence == expected => {}
                    Some(expected) => summary.ordering_errors.push(format!(
                        "offset {offset}: sequence discontinuity {previous} -> {sequence} \
                         (expected {expected}; a record was swallowed, duplicated, or reordered)"
                    )),
                    None => summary.ordering_errors.push(format!(
                        "offset {offset}: sequence {previous} exhausted; wrapping to {sequence} is forbidden"
                    )),
                }
            } else if sequence != 1 {
                summary.ordering_errors.push(format!(
                    "offset {offset}: first sequence is {sequence}, expected 1; stream prefix is missing"
                ));
            }
            previous_sequence = Some(sequence);
        }
        final_record_is_clean_frame_end = false;
        let payload = &raw[offset + RECORD_HEADER_LEN..offset + record_len];
        summary.framed_max_payload_len = summary.framed_max_payload_len.max(payload_len as u64);
        summary.framed_max_record_len = summary.framed_max_record_len.max(record_len as u64);
        let stateful_result = if stream_validator.is_terminal() {
            None
        } else {
            Some(stream_validator.validate_record(&header, payload))
        };
        match stateful_result {
            Some(Ok(AcceptedRecordKind::CampaignIdentity)) => {
                summary.validated_cid1 = stream_validator.identity().cloned();
                // CID1 participates in byte/framing caps but is not an ordinary
                // compact record and must never enter domain/hour accounting.
                offset = offset.saturating_add(record_len);
                continue;
            }
            Some(Ok(AcceptedRecordKind::Ordinary)) | None => {}
            Some(Err(error @ StreamValidationError::InvalidCampaignIdentity(_)))
            | Some(Err(error @ StreamValidationError::Terminal)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: stateful stream validation failed before dispatch: {error}"
                ));
                framing_complete = false;
                break;
            }
            // Once CID1 placement has been established, ordinary record
            // defects remain localizable.  Preserve the independent scanner's
            // complete-stream diagnostics/accounting behavior; the shared
            // validator is terminal and is deliberately not consulted again.
            Some(Err(_)) => {}
        }
        let envelope_ok = match compact_contract::validate_compact_record_semantics(
            domain_id,
            record_type,
            header.record_flags,
            payload,
        ) {
            Ok(()) => true,
            Err(reason) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: {reason}"));
                false
            }
        };
        // Reserved probe domain (250): pure instrumentation records (lane-wall
        // lw_*, read A/B, bt_* bound trips, so_* status origins). The recorder
        // routes these BEFORE all record/domain accounting, so this
        // independent scan must mirror that rule exactly — counting them here
        // made every probe-carrying stream disagree with its manifest
        // (record_count / domain_perf_record_count / domains_seen) and the
        // exact-count comparison could never pass (R2 gate finding 3,
        // pre-existing). Sequence ordering was already checked above.
        if record_type == RECORD_TYPE_DOMAIN_PERF && domain_id == RC_PROBE_DOMAIN_ID {
            offset = offset.saturating_add(record_len);
            continue;
        }
        summary.record_count += 1;
        summary.total_payload_bytes = summary
            .total_payload_bytes
            .saturating_add(payload_len as u64);
        summary.max_payload_len = summary.max_payload_len.max(payload_len as u64);
        summary.max_record_len = summary.max_record_len.max(record_len as u64);
        summary.domains_seen.insert(domain_id);
        match record_type {
            RECORD_TYPE_DOMAIN_PERF => {
                summary.domain_perf_record_count += 1;
                *summary
                    .per_hour_perf_domain_counts
                    .entry(game_hour_raw)
                    .or_default()
                    .entry(domain_id)
                    .or_default() += 1;
                if payload.len() >= 36 {
                    let raw_status = read_u32_le(payload, 32);
                    let unknown_status = domain_status_unknown_bits(domain_id, raw_status);
                    if unknown_status != 0 {
                        summary.parse_errors.push(format!(
                            "offset {offset}: DOMAIN_PERF domain {domain_id} status {raw_status:#x} contains unknown v1 bits {unknown_status:#x}"
                        ));
                    }
                    let row_count = read_u32_le(payload, 16) as u64;
                    let authority = PerfAuthority {
                        row_count,
                        changed_rows: read_u32_le(payload, 20) as u64,
                        unit_count: read_u32_le(payload, 24) as u64,
                        changed_unit_count: read_u32_le(payload, 28) as u64,
                        status_flags: raw_status,
                    };
                    summary
                        .per_hour_perf_authority
                        .entry((game_hour_raw, domain_id))
                        .or_insert(authority);
                    let total = summary
                        .per_domain_perf_authority
                        .entry(domain_id)
                        .or_default();
                    total.row_count = total.row_count.saturating_add(authority.row_count);
                    total.changed_rows = total.changed_rows.saturating_add(authority.changed_rows);
                    total.unit_count = total.unit_count.saturating_add(authority.unit_count);
                    total.changed_unit_count = total
                        .changed_unit_count
                        .saturating_add(authority.changed_unit_count);
                    total.status_flags |= authority.status_flags;
                    *summary
                        .per_domain_status_flags_or
                        .entry(domain_id)
                        .or_default() |= raw_status;
                    summary.global_status_flags_or |=
                        compact_status_for_global_interpretation(domain_id, raw_status);
                    if envelope_ok {
                        collect_perf_units(domain_id, header.record_flags, payload, &mut summary);
                    }
                }
                match open_frame_hour {
                    Some(open) if open != game_hour_raw => summary.parse_errors.push(format!(
                        "offset {offset}: DOMAIN_PERF hour {game_hour_raw} does not match open frame hour {open}"
                    )),
                    Some(_) => {}
                    None => {
                        if let Some(closed) = last_closed_hour {
                            match closed.checked_add(1) {
                                Some(expected) if game_hour_raw == expected => {}
                                Some(_) => summary.parse_errors.push(format!(
                                    "offset {offset}: DOMAIN_PERF hour {game_hour_raw} does not follow closed frame hour {closed}"
                                )),
                                None => summary.parse_errors.push(format!(
                                    "offset {offset}: closed frame hour {closed} exhausted; another frame is forbidden"
                                )),
                            }
                        }
                        match pending_perf_hour {
                            Some(pending) if pending != game_hour_raw => summary.parse_errors.push(format!(
                                "offset {offset}: DOMAIN_PERF hour {game_hour_raw} conflicts with pending frame hour {pending}"
                            )),
                            Some(_) => {}
                            None => pending_perf_hour = Some(game_hour_raw),
                        }
                    }
                }
            }
            RECORD_TYPE_DOMAIN_DATA => {
                summary.domain_data_record_count += 1;
                *summary
                    .per_hour_data_domain_counts
                    .entry(game_hour_raw)
                    .or_default()
                    .entry(domain_id)
                    .or_default() += 1;
                let data_valid = match recorder_host::stream_fold::validate_domain_data_payload(
                    domain_id,
                    payload,
                    header.record_flags,
                ) {
                    Ok(()) => true,
                    Err(reason) => {
                        summary.parse_errors.push(format!(
                            "offset {offset}: domain {domain_id} DATA payload failed canonical validation: {reason}"
                        ));
                        false
                    }
                };
                if data_valid && envelope_ok {
                    let result =
                        recorder_host::domain_data_authority::summarize_domain_data_payload(
                            domain_id,
                            payload,
                            recorder_host::domain_data_authority::DomainDataContext {
                                stream_offset: offset,
                                record_flags: header.record_flags,
                                sequence,
                                header_hour_raw: game_hour_raw,
                                session_header_identity: (domain_id == 68)
                                    .then_some(&mut session_header_identity),
                            },
                        );
                    match result {
                        Ok(decoded) => {
                            if let Some(status_flags) = decoded.status_flags {
                                let unknown_status =
                                    domain_status_unknown_bits(domain_id, status_flags);
                                if unknown_status != 0 {
                                    summary.parse_errors.push(format!(
                                        "offset {offset}: DOMAIN_DATA domain {domain_id} status {status_flags:#x} contains unknown v1 bits {unknown_status:#x}"
                                    ));
                                }
                            }
                            if let Some(frame) = decoded.auxiliary.domain57_frame.clone() {
                                summary.domain57_frames.push(frame);
                            }
                            if let Some(identity) =
                                decoded.auxiliary.domain68_session_header_identity.as_ref()
                            {
                                let auxiliary = recorder_host::campaign_identity::to_manifest_json(
                                    &recorder_host::campaign_identity::resolve(identity),
                                );
                                let accumulated =
                                    recorder_host::campaign_identity::to_manifest_json(
                                        &recorder_host::campaign_identity::resolve(
                                            &session_header_identity,
                                        ),
                                    );
                                if auxiliary != accumulated {
                                    summary.parse_errors.push(format!(
                                        "offset {offset}: DOMAIN_DATA d68 auxiliary identity does not match accumulated session-header identity"
                                    ));
                                }
                            }
                            let authority = DataAuthority {
                                row_count: decoded.emitted_rows,
                                changed_rows: decoded.changed_rows,
                                status_flags: decoded.status_flags,
                                data_units: decoded.data_units,
                            };
                            merge_data_authority(domain_id, &authority, &mut summary);
                            summary
                                .per_hour_data_authority
                                .entry((game_hour_raw, domain_id))
                                .or_insert(authority);
                        }
                        Err(error) => summary.parse_errors.extend(error.errors),
                    }
                }
                match open_frame_hour {
                    Some(open) if open != game_hour_raw => summary.parse_errors.push(format!(
                        "offset {offset}: DOMAIN_DATA hour {game_hour_raw} encountered before FRAME_END closed open frame hour {open}"
                    )),
                    Some(_) => {}
                    None => {
                        if let Some(closed) = last_closed_hour {
                            match closed.checked_add(1) {
                                Some(expected) if game_hour_raw == expected => {}
                                Some(_) => summary.parse_errors.push(format!(
                                    "offset {offset}: DOMAIN_DATA hour {game_hour_raw} does not follow closed frame hour {closed}"
                                )),
                                None => summary.parse_errors.push(format!(
                                    "offset {offset}: closed frame hour {closed} exhausted; another frame is forbidden"
                                )),
                            }
                        }
                        if let Some(pending) = pending_perf_hour {
                            if pending != game_hour_raw {
                                summary.parse_errors.push(format!(
                                    "offset {offset}: DOMAIN_DATA hour {game_hour_raw} conflicts with pending PERF hour {pending}"
                                ));
                            }
                        }
                        open_frame_hour = Some(game_hour_raw);
                    }
                }
            }
            RECORD_TYPE_FRAME_END => {
                summary.frame_end_record_count += 1;
                *summary
                    .frame_end_hour_counts
                    .entry(game_hour_raw)
                    .or_default() += 1;
                let mut clean = envelope_ok;
                if payload.len() == 32 {
                    let authority = FrameEndAuthority {
                        emitted_domains: read_u32_le(payload, 16),
                        scheduled_domains: read_u32_le(payload, 20),
                        status_flags: read_u32_le(payload, 24),
                    };
                    summary.frame_status_flags_or_raw |= authority.status_flags;
                    summary
                        .per_hour_frame_authority
                        .entry(game_hour_raw)
                        .or_insert(authority);
                    let unknown_status =
                        compact_contract::compact_status_unknown_flags_v1(authority.status_flags);
                    if unknown_status != 0 {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END status {:#x} contains unknown bits {unknown_status:#x}",
                            authority.status_flags
                        ));
                    }
                    let fatal_status = authority.status_flags & COMPACT_STATUS_FAILURE_MASK;
                    if fatal_status != 0 {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END status {:#x} contains fatal bits {fatal_status:#x}",
                            authority.status_flags
                        ));
                    }
                    let reserved = read_u32_le(payload, 28);
                    if reserved != 0 {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END reserved word is nonzero ({reserved:#x})"
                        ));
                    }
                    let observed_perf_records = summary
                        .per_hour_perf_domain_counts
                        .get(&game_hour_raw)
                        .map(|domains| domains.values().copied().sum::<u64>())
                        .unwrap_or(0);
                    if u64::from(authority.emitted_domains) != observed_perf_records {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END emitted_domains {} != independently observed DOMAIN_PERF count {observed_perf_records}",
                            authority.emitted_domains
                        ));
                    }
                    if authority.scheduled_domains != authority.emitted_domains {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END scheduled_domains {} != emitted_domains {}",
                            authority.scheduled_domains, authority.emitted_domains
                        ));
                    }
                }
                match open_frame_hour {
                    Some(open) if open == game_hour_raw => {}
                    Some(open) => {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END hour {game_hour_raw} does not close open frame hour {open}"
                        ));
                    }
                    None => {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END hour {game_hour_raw} has no open DATA frame"
                        ));
                    }
                }
                if let Some(closed) = last_closed_hour {
                    match closed.checked_add(1) {
                        Some(expected) if game_hour_raw == expected => {}
                        Some(_) => {
                            clean = false;
                            summary.parse_errors.push(format!(
                                "offset {offset}: closed frame hour {game_hour_raw} does not consecutively follow {closed}"
                            ));
                        }
                        None => {
                            clean = false;
                            summary.parse_errors.push(format!(
                                "offset {offset}: closed frame hour {closed} exhausted; another closure is forbidden"
                            ));
                        }
                    }
                }
                if clean {
                    open_frame_hour = None;
                    pending_perf_hour = None;
                    last_closed_hour = Some(game_hour_raw);
                    final_record_is_clean_frame_end = true;
                }
            }
            other => summary
                .parse_errors
                .push(format!("offset {offset}: unknown record type {other}")),
        }
        offset = offset.saturating_add(record_len);
    }
    if let Some(open) = open_frame_hour {
        summary
            .parse_errors
            .push(format!("stream ended with open DATA frame hour {open}"));
    }
    if framing_complete && !raw.is_empty() && !final_record_is_clean_frame_end {
        summary
            .parse_errors
            .push("stream is not closed by a final canonical FRAME_END record".to_string());
    }
    if identity_mode == StreamIdentityMode::IdentityV1 && summary.validated_cid1.is_none() {
        summary
            .parse_errors
            .push("IdentityV1 stream ended without an accepted CID1 header".to_string());
    }
    summary.unique_hours = summary.frame_end_hour_counts.keys().copied().collect();
    summary.skipped_hours = skipped_hours(&summary.unique_hours);
    summary.session_header_identity = session_header_identity;
    summary
}

fn merge_data_authority(
    domain_id: u16,
    authority: &DataAuthority,
    summary: &mut HeaderScanSummary,
) {
    let total = summary
        .per_domain_data_authority
        .entry(domain_id)
        .or_default();
    total.row_count = total.row_count.saturating_add(authority.row_count);
    total.changed_rows = total.changed_rows.saturating_add(authority.changed_rows);
    if let Some(status) = authority.status_flags {
        *total.status_flags.get_or_insert(0) |= status;
        *summary
            .per_domain_status_flags_or
            .entry(domain_id)
            .or_default() |= status;
        summary.global_status_flags_or |=
            compact_status_for_global_interpretation(domain_id, status);
    }
    for (name, value) in &authority.data_units {
        let total_value = total.data_units.entry(name.clone()).or_default();
        *total_value = total_value.saturating_add(*value);
        let combined = summary
            .per_domain_combined_units
            .entry(domain_id)
            .or_default()
            .entry(name.clone())
            .or_default();
        *combined = combined.saturating_add(*value);
    }
}

fn collect_perf_units(
    domain_id: u16,
    record_flags: u32,
    payload: &[u8],
    summary: &mut HeaderScanSummary,
) {
    if record_flags & compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER == 0 {
        return;
    }
    // validate_compact_record_semantics already proved the complete PDU1 shape.
    let unit_count = u16::from_le_bytes([payload[46], payload[47]]) as usize;
    let mut cursor = 48usize;
    for _ in 0..unit_count {
        let name_len = u16::from_le_bytes([payload[cursor], payload[cursor + 1]]) as usize;
        let value = u64::from_le_bytes(
            payload[cursor + 4..cursor + 12]
                .try_into()
                .expect("validated PDU1 unit value"),
        );
        cursor += 12;
        let name = std::str::from_utf8(&payload[cursor..cursor + name_len])
            .expect("validated PDU1 unit name");
        cursor += name_len;
        let aggregate = summary
            .per_domain_combined_units
            .entry(domain_id)
            .or_default()
            .entry(name.to_string())
            .or_default();
        if name == "d69_failed_slot_mask"
            || name.starts_with("d69_failed_slot_mask_")
            || name == "d69_stale_slot_mask"
        {
            *aggregate |= value;
        } else if name.ends_with("_sample") {
            *aggregate = value;
        } else {
            *aggregate = aggregate.saturating_add(value);
        }
    }
}

fn compact_domain_coverage_errors(
    summary: &HeaderScanSummary,
    expected_domains: &[u16],
) -> Vec<String> {
    let mut errors = Vec::new();
    let frame_hours: BTreeSet<i32> = summary.frame_end_hour_counts.keys().copied().collect();
    let mut all_record_hours = frame_hours.clone();
    all_record_hours.extend(summary.per_hour_perf_domain_counts.keys().copied());
    all_record_hours.extend(summary.per_hour_data_domain_counts.keys().copied());

    for hour in &all_record_hours {
        let frame_count = summary
            .frame_end_hour_counts
            .get(hour)
            .copied()
            .unwrap_or(0);
        if frame_count != 1 {
            errors.push(format!(
                "game_hour_raw {hour}: frame_end_count {frame_count} != 1"
            ));
        }
    }
    for hour in &summary.unique_hours {
        if let Some(frame) = summary.per_hour_frame_authority.get(hour) {
            let expected_count = expected_domains.len() as u64;
            if u64::from(frame.emitted_domains) != expected_count {
                errors.push(format!(
                    "game_hour_raw {hour}: FRAME_END emitted_domains {} != expected production domain count {expected_count}",
                    frame.emitted_domains
                ));
            }
            if u64::from(frame.scheduled_domains) != expected_count {
                errors.push(format!(
                    "game_hour_raw {hour}: FRAME_END scheduled_domains {} != expected production domain count {expected_count}",
                    frame.scheduled_domains
                ));
            }
        }
        for expected in expected_domains {
            let Some(descriptor) = compact_contract::assigned_descriptor_by_id(*expected) else {
                errors.push(format!("unknown compact domain id {expected}"));
                continue;
            };
            let contract = contract_for_descriptor(descriptor);
            let perf_count = summary
                .per_hour_perf_domain_counts
                .get(hour)
                .and_then(|domains| domains.get(expected))
                .copied()
                .unwrap_or(0);
            if perf_count != 1 {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} perf_record_count {perf_count} != 1",
                    contract.name
                ));
            }
            if contract.requires_data {
                let data_count = summary
                    .per_hour_data_domain_counts
                    .get(hour)
                    .and_then(|domains| domains.get(expected))
                    .copied()
                    .unwrap_or(0);
                if data_count != 1 {
                    errors.push(format!(
                        "game_hour_raw {hour}: compact domain {} data_record_count {data_count} != 1",
                        contract.name
                    ));
                }
            }
        }
    }
    for (hour, domain_counts) in &summary.per_hour_perf_domain_counts {
        if frame_hours.contains(hour) {
            continue;
        }
        for expected in expected_domains {
            let perf_count = domain_counts.get(expected).copied().unwrap_or(0);
            if perf_count != 0 {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} perf_record_count {perf_count} on non-frame hour",
                    domain_name(*expected)
                ));
            }
        }
    }
    for (hour, domain_counts) in &summary.per_hour_data_domain_counts {
        if frame_hours.contains(hour) {
            continue;
        }
        for expected in expected_domains {
            let Some(descriptor) = compact_contract::assigned_descriptor_by_id(*expected) else {
                continue;
            };
            let contract = contract_for_descriptor(descriptor);
            if !contract.requires_data {
                continue;
            }
            let data_count = domain_counts.get(expected).copied().unwrap_or(0);
            if data_count != 0 {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} data_record_count {data_count} on non-frame hour",
                    contract.name
                ));
            }
        }
    }
    errors
}

fn skipped_hours(hours: &[i32]) -> i32 {
    let mut skipped = 0i32;
    for pair in hours.windows(2) {
        let delta = pair[1].saturating_sub(pair[0]);
        if delta > 1 {
            skipped = skipped.saturating_add(delta - 1);
        }
    }
    skipped
}

fn require_str_eq(value: &Value, key: &str, expected: &str, errors: &mut Vec<String>) {
    if value.get(key).and_then(Value::as_str) != Some(expected) {
        errors.push(format!(
            "{key} is {:?}, expected {expected:?}",
            value.get(key)
        ));
    }
}

fn require_bool_eq(value: &Value, key: &str, expected: bool, errors: &mut Vec<String>) {
    if value.get(key).and_then(Value::as_bool) != Some(expected) {
        errors.push(format!(
            "{key} is {:?}, expected {expected}",
            value.get(key)
        ));
    }
}

fn require_u64_eq(value: &Value, key: &str, expected: u64, errors: &mut Vec<String>) {
    if value.get(key).and_then(Value::as_u64) != Some(expected) {
        errors.push(format!(
            "{key} is {:?}, expected {expected}",
            value.get(key)
        ));
    }
}

fn require_array_empty(value: &Value, key: &str, errors: &mut Vec<String>) {
    match value.get(key).and_then(Value::as_array) {
        Some(items) if items.is_empty() => {}
        other => errors.push(format!("{key} is {other:?}, expected empty array")),
    }
}

fn compare_u64(key: &str, scanned: u64, manifest: &Value, errors: &mut Vec<String>) {
    let expected = manifest.get(key).and_then(Value::as_u64);
    if expected != Some(scanned) {
        errors.push(format!(
            "independent stream scan {key} {scanned} != manifest {expected:?}"
        ));
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
fn contract_by_name(name: &str) -> Option<DomainContract> {
    let canonical = match name {
        "state_ownership" | "state_control" | "province_control" => "map_control",
        "state_detail" | "buildings_live" => "map_detail",
        other => other,
    };
    DOMAIN_CONTRACTS
        .iter()
        .copied()
        .find(|contract| contract.name == canonical)
}

fn contract_by_id(id: u16) -> Option<DomainContract> {
    DOMAIN_CONTRACTS
        .iter()
        .copied()
        .find(|contract| contract.id == id)
}

fn contract_for_descriptor(
    descriptor: compact_contract::CompactDomainDescriptor,
) -> DomainContract {
    contract_by_id(descriptor.id).unwrap_or(DomainContract {
        id: descriptor.id,
        name: descriptor.wire_name,
        requires_data: descriptor.requires_data_record,
        required_units: &[],
        zero_units: &[],
        work_implies_rows_units: &[],
        shared_recorder_known: true,
    })
}

fn domain_name(id: u16) -> &'static str {
    contract_by_id(id)
        .map(|contract| contract.name)
        .unwrap_or("unknown")
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Unassigned-bit view of a record's status word. The domain's registered
/// lane-local badges (`compact_contract::lane_local_status_mask` — d49, d51,
/// d53, d60) are that domain's own vocabulary and are stripped BEFORE the
/// v1 unassigned-bit check; checking the raw word rejected every production
/// d53 record carrying its badge bits 16..=22 (batch-3a: 379 of 514 hours at
/// status `0x530200`).
fn domain_status_unknown_bits(domain_id: u16, status_flags: u32) -> u32 {
    compact_contract::compact_status_unknown_flags_v1(
        compact_contract::compact_status_for_global_interpretation(domain_id, status_flags),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Regression: the d53 production status word (`PARTIAL_DATA` plus badge
    /// bits 16/17/20/22 — 379 of 514 batch-3a hours) must not be reported as
    /// carrying unassigned v1 bits, while the same word on a domain with no
    /// registered lane-local vocabulary still is.
    #[test]
    fn lane_local_badges_are_not_unknown_bits_for_their_own_domain() {
        let d53_word = 0x0053_0200;
        assert_eq!(super::domain_status_unknown_bits(53, d53_word), 0);
        let d60_word = (1 << 16) | (1 << 17);
        assert_eq!(super::domain_status_unknown_bits(60, d60_word), 0);
        // Domain-scoped, never a global amnesty.
        assert_eq!(super::domain_status_unknown_bits(52, d53_word), 0x0053_0000);
        assert_eq!(super::domain_status_unknown_bits(61, d60_word), d60_word);
        // A genuinely unassigned bit still surfaces for d53/d60.
        assert_eq!(super::domain_status_unknown_bits(53, 1 << 30), 1 << 30);
        assert_eq!(super::domain_status_unknown_bits(60, 1 << 23), 1 << 23);
    }

    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "compact_gate_{label}_{}_{}",
                std::process::id(),
                nonce
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn valid_domain57_summary() -> HeaderScanSummary {
        HeaderScanSummary {
            frame_end_record_count: 1,
            frame_end_hour_counts: BTreeMap::from([(100, 1)]),
            domain57_frames: vec![
                recorder_host::domain_data_authority::Domain57FrameEvidence {
                    sequence: 2,
                    header_hour_raw: 100,
                    payload_hour_raw: 100,
                    root_evidence_mask: 0x7,
                    speed_rows: vec![
                        recorder_host::domain_data_authority::Domain57ScalarEvidence {
                            event_hour_raw: 100,
                            value_i32: 5,
                        },
                    ],
                    pause_rows: vec![
                        recorder_host::domain_data_authority::Domain57ScalarEvidence {
                            event_hour_raw: 100,
                            value_i32: 0,
                        },
                    ],
                },
            ],
            ..HeaderScanSummary::default()
        }
    }

    fn valid_domain69_per_hour_summary() -> HeaderScanSummary {
        let data_units = BTreeMap::from([
            ("tech_sharing_groups_groups".to_string(), 2),
            ("tech_sharing_groups_members".to_string(), 3),
            ("tech_sharing_groups_dynamic_bonus_rows".to_string(), 3),
        ]);
        HeaderScanSummary {
            per_hour_perf_authority: BTreeMap::from([(
                (100, 69),
                PerfAuthority {
                    row_count: 2,
                    changed_rows: 2,
                    unit_count: 3,
                    changed_unit_count: 3,
                    status_flags: compact_contract::COMPACT_STATUS_DELTA_DATA,
                },
            )]),
            per_hour_data_authority: BTreeMap::from([(
                (100, 69),
                DataAuthority {
                    row_count: 2,
                    changed_rows: 0,
                    status_flags: Some(compact_contract::COMPACT_STATUS_DELTA_DATA),
                    data_units,
                },
            )]),
            ..HeaderScanSummary::default()
        }
    }

    #[test]
    fn header_scan_accepts_two_complete_fuel_hours() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut raw,
            2,
            100,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        append_record(
            &mut raw,
            4,
            101,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut raw,
            5,
            101,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut raw,
            6,
            101,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(summary.ordering_errors.is_empty());
        assert_eq!(summary.record_count, 6);
        assert_eq!(summary.frame_end_record_count, 2);
        assert_eq!(summary.skipped_hours, 0);
        assert!(compact_domain_coverage_errors(&summary, &[10]).is_empty());
    }

    #[test]
    fn probe_domain_records_are_excluded_from_all_stream_accounting() {
        // R2 gate finding 3 (pre-existing): the recorder manifest excludes
        // reserved probe-domain (250) records from record/domain accounting,
        // but this independent scan counted them — every probe-carrying
        // stream (lane-wall lw_*, bt_*, so_* units ride domain 250) then
        // disagreed with its manifest and the exact-count comparison could
        // never pass. The probe record must be invisible to every counter
        // while sequence ordering still covers it.
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut raw,
            2,
            100,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        // The lane-wall probe record (carries bt_*/so_* diagnostic units in
        // production; the trailer content is irrelevant to accounting).
        append_record(
            &mut raw,
            3,
            100,
            RC_PROBE_DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut raw,
            4,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(summary.ordering_errors.is_empty());
        assert_eq!(
            summary.record_count, 3,
            "probe record must not join record_count"
        );
        assert_eq!(
            summary.domain_perf_record_count, 1,
            "probe record must not join domain_perf_record_count"
        );
        assert!(
            !summary.domains_seen.contains(&RC_PROBE_DOMAIN_ID),
            "probe domain must not join domains_seen"
        );
        assert!(compact_domain_coverage_errors(&summary, &[10]).is_empty());

        // A probe record with a broken sequence still trips ordering errors.
        let mut bad = Vec::new();
        append_record(
            &mut bad,
            5,
            100,
            RC_PROBE_DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut bad,
            5,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let summary = scan_compact_stream(&bad);
        assert!(!summary.ordering_errors.is_empty());
    }

    fn d46_domain_manifest(units: Value) -> Value {
        json!({
            "domain_id": 46,
            "record_count": 1,
            "data_record_count": 1,
            "status_flags_or": 0,
            "read_time_us": {"count": 1},
            "data_units_total": units,
        })
    }

    #[test]
    fn d46_zero_unit_health_flag_fails_the_gate() {
        let contract = contract_by_id(46).expect("domain 46 contract exists");
        assert!(!contract.zero_units.is_empty());
        let domain = d46_domain_manifest(json!({
            "air_variant_archetype_rows": 203,
            "air_ace_roster_rows": 258,
            "air_ace_roster_observed_ace_rows": 258,
            // A partial frame fired: must fail the gate.
            "air_ace_roster_partial_frames": 1,
        }));
        let mut errors = Vec::new();
        validate_domain_manifest(&domain, contract, 1, &GateOptions::default(), &mut errors);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("air_ace_roster_partial_frames")
                    && error.contains("expected 0")),
            "{errors:?}"
        );
    }

    #[test]
    fn d46_clean_health_flags_pass_the_gate() {
        let contract = contract_by_id(46).expect("domain 46 contract exists");
        let domain = d46_domain_manifest(json!({
            "air_variant_archetype_rows": 203,
            "air_ace_roster_rows": 258,
            "air_ace_roster_observed_ace_rows": 258,
        }));
        let mut errors = Vec::new();
        validate_domain_manifest(&domain, contract, 1, &GateOptions::default(), &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    fn d47_domain_manifest(row_count_total: u64, units: Value) -> Value {
        json!({
            "domain_id": 47,
            "record_count": 1,
            "data_record_count": 1,
            "row_count_total": row_count_total,
            // SOURCE|PARTIAL — the tolerated status the defect hid behind.
            "status_flags_or": (1u64 << 8) | (1u64 << 9),
            "read_time_us": {"count": 1},
            "data_units_total": units,
        })
    }

    #[test]
    fn d47_adapter_work_with_zero_emission_is_fatal_even_when_partial_tolerated() {
        let contract = contract_by_id(47).expect("domain 47 contract exists");
        assert!(!contract.work_implies_rows_units.is_empty());
        // Reproduce the live defect: millions of adapter rows, zero emitted.
        let domain = d47_domain_manifest(
            0,
            json!({
                "domain47_adapter_character_rows": 39_553_062u64,
                "domain47_adapter_advisor_roster_rows": 29_790_670u64,
                "domain47_frame_parse_errors": 1560,
            }),
        );
        // Even a partial-TOLERANT profile must reject this.
        let options = GateOptions {
            allow_partial_or_source_failure: true,
            ..GateOptions::default()
        };
        let mut errors = Vec::new();
        validate_domain_manifest(&domain, contract, 1, &options, &mut errors);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("emitted row_count_total=0")
                    && e.contains("domain47_adapter_character_rows")),
            "{errors:?}"
        );
    }

    #[test]
    fn d47_healthy_emission_passes() {
        let contract = contract_by_id(47).expect("domain 47 contract exists");
        // Rows emitted -> the work-implies-rows invariant is satisfied.
        let domain = d47_domain_manifest(
            2_360_421,
            json!({
                "domain47_adapter_character_rows": 39_553_062u64,
            }),
        );
        let options = GateOptions {
            // Even with partial tolerance OFF, only the status check would fire;
            // isolate the work-implies-rows invariant by tolerating partial.
            allow_partial_or_source_failure: true,
            ..GateOptions::default()
        };
        let mut errors = Vec::new();
        validate_domain_manifest(&domain, contract, 1, &options, &mut errors);
        assert!(
            !errors
                .iter()
                .any(|e| e.contains("emitted row_count_total=0")),
            "{errors:?}"
        );
    }

    #[test]
    fn d47_no_work_and_no_rows_does_not_trip_work_implies_rows() {
        // A legitimately empty frame (no adapter work, no rows) must NOT trip
        // the work-implies-rows invariant — only work-without-emission does.
        let contract = contract_by_id(47).expect("domain 47 contract exists");
        let domain = d47_domain_manifest(0, json!({}));
        let options = GateOptions {
            allow_partial_or_source_failure: true,
            ..GateOptions::default()
        };
        let mut errors = Vec::new();
        validate_domain_manifest(&domain, contract, 1, &options, &mut errors);
        assert!(
            !errors
                .iter()
                .any(|e| e.contains("emitted row_count_total=0")),
            "{errors:?}"
        );
    }

    #[test]
    fn d46_missing_required_unit_fails_the_gate() {
        let contract = contract_by_id(46).expect("domain 46 contract exists");
        let domain = d46_domain_manifest(json!({
            "air_variant_archetype_rows": 203,
            // air_ace_roster_rows missing.
        }));
        let mut errors = Vec::new();
        validate_domain_manifest(&domain, contract, 1, &GateOptions::default(), &mut errors);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("missing required counter air_ace_roster_rows")),
            "{errors:?}"
        );
    }

    #[test]
    fn header_scan_rejects_missing_data_record() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut raw,
            2,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        let errors = compact_domain_coverage_errors(&summary, &[10]);
        assert!(errors
            .iter()
            .any(|error| error.contains("fuel_live data_record_count 0 != 1")));
    }

    #[test]
    fn header_scan_rejects_every_short_header_without_panicking() {
        for len in 0..RECORD_HEADER_LEN {
            let summary = scan_compact_stream(&vec![0xA5; len]);
            if len == 0 {
                assert!(summary.parse_errors.is_empty());
            } else {
                assert_eq!(summary.parse_errors.len(), 1, "short length {len}");
                assert!(
                    summary.parse_errors[0]
                        .contains(&format!("truncated record header: {len} byte(s)")),
                    "short length {len}: {:?}",
                    summary.parse_errors
                );
            }
        }
    }

    #[test]
    fn header_scan_rejects_sequence_gap() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        assert_eq!(summary.ordering_errors.len(), 1);
        assert!(
            summary.ordering_errors[0].contains("sequence discontinuity 1 -> 3")
                && summary.ordering_errors[0].contains("expected 2"),
            "{:?}",
            summary.ordering_errors
        );
    }

    #[test]
    fn header_scan_rejects_sequence_repeat_and_regression() {
        for next in [1, 0] {
            let mut raw = Vec::new();
            append_record(
                &mut raw,
                1,
                100,
                10,
                RECORD_TYPE_DOMAIN_PERF,
                &perf_payload(0),
            );
            append_record(
                &mut raw,
                next,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_payload(0),
            );

            let summary = scan_compact_stream(&raw);
            assert_eq!(summary.ordering_errors.len(), 1, "next={next}");
            assert!(
                summary.ordering_errors[0].contains(&format!("sequence discontinuity 1 -> {next}"))
                    && summary.ordering_errors[0].contains("expected 2"),
                "next={next}: {:?}",
                summary.ordering_errors
            );
        }
    }

    #[test]
    fn header_scan_rejects_missing_sequence_prefix() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            2,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        assert!(
            summary
                .ordering_errors
                .iter()
                .any(|error| error.contains("first sequence is 2, expected 1")),
            "{:?}",
            summary.ordering_errors
        );
    }

    #[test]
    fn header_scan_rejects_wrapping_sequence_continuity() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            u64::MAX,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut raw,
            0,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        assert!(
            summary
                .ordering_errors
                .iter()
                .any(|error| error.contains("exhausted") && error.contains("forbidden")),
            "{:?}",
            summary.ordering_errors
        );
    }

    #[test]
    fn header_scan_rejects_reserved_or_misplaced_record_flags() {
        let mut raw = Vec::new();
        append_record_with_flags(
            &mut raw,
            1,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            1,
            &frame_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        assert!(
            summary
                .parse_errors
                .iter()
                .any(|error| error.contains("record flags") && error.contains("rejected bits")),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn header_scan_accepts_only_canonical_record_flags() {
        let mut raw = Vec::new();
        append_record_with_flags(
            &mut raw,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_payload_with_empty_trailer(0),
        );
        append_record_with_flags(
            &mut raw,
            2,
            100,
            45,
            RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &frd_v2_empty_payload(),
        );
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn header_scan_requires_exact_frame_end_payload_size() {
        for len in [28, 29, 31, 33] {
            let mut raw = Vec::new();
            append_record(
                &mut raw,
                1,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &vec![0; len],
            );

            let summary = scan_compact_stream(&raw);
            assert!(
                summary.parse_errors.iter().any(|error| {
                    error.contains(&format!("FRAME_END payload length {len}"))
                        && error.contains("not 32")
                }),
                "len={len}: {:?}",
                summary.parse_errors
            );
        }
    }

    #[test]
    fn header_scan_rejects_fatal_and_unknown_frame_status() {
        for (status, expected) in [
            (
                compact_contract::COMPACT_STATUS_CONTROL_INVALID,
                "fatal bits",
            ),
            (1u32 << 31, "unknown bits"),
        ] {
            let raw = one_domain_stream_with_frame_payload(&frame_payload(status));
            let summary = scan_compact_stream(&raw);
            assert!(
                summary.parse_errors.iter().any(|error| {
                    error.contains("FRAME_END status") && error.contains(expected)
                }),
                "status={status:#x}: {:?}",
                summary.parse_errors
            );
        }
    }

    #[test]
    fn header_scan_rejects_unknown_v1_perf_and_data_status_bits() {
        let unknown = 1u32 << 31;
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(unknown),
        );
        append_record(
            &mut raw,
            2,
            100,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let summary = scan_compact_stream(&raw);
        assert!(
            summary.parse_errors.iter().any(|error| {
                error.contains("DOMAIN_PERF domain 10") && error.contains("unknown v1 bits")
            }),
            "{:?}",
            summary.parse_errors
        );

        let mut data = fuel_empty_payload();
        data[16..20].copy_from_slice(&unknown.to_le_bytes());
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(&mut raw, 2, 100, 10, RECORD_TYPE_DOMAIN_DATA, &data);
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let summary = scan_compact_stream(&raw);
        assert!(
            summary.parse_errors.iter().any(|error| {
                error.contains("DOMAIN_DATA domain 10") && error.contains("unknown v1 bits")
            }),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn domain57_stream_authority_binds_exact_evidence_and_all_frame_invariants() {
        let summary = valid_domain57_summary();
        let compact = json!({
            "domain57_frame_evidence": domain57_frame_evidence_json(&summary),
        });
        let mut errors = Vec::new();
        validate_domain57_stream_authority(&summary, &compact, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");

        let mut missing_record = valid_domain57_summary();
        missing_record.domain57_frames.clear();
        let mut missing_record_errors = Vec::new();
        validate_domain57_stream_authority(&missing_record, &compact, &mut missing_record_errors);
        assert!(missing_record_errors
            .iter()
            .any(|error| error.contains("exactly one record per closed frame")));

        let mut bad_root = valid_domain57_summary();
        bad_root.domain57_frames[0].root_evidence_mask = 0x3;
        let mut bad_root_errors = Vec::new();
        validate_domain57_stream_authority(&bad_root, &compact, &mut bad_root_errors);
        assert!(bad_root_errors
            .iter()
            .any(|error| error.contains("required root evidence mask 0x7")));

        let mut missing_speed = valid_domain57_summary();
        missing_speed.domain57_frames[0].speed_rows.clear();
        let mut missing_speed_errors = Vec::new();
        validate_domain57_stream_authority(&missing_speed, &compact, &mut missing_speed_errors);
        assert!(missing_speed_errors
            .iter()
            .any(|error| error.contains("exactly one speed row")));

        let mut missing_pause = valid_domain57_summary();
        missing_pause.domain57_frames[0].pause_rows.clear();
        let mut missing_pause_errors = Vec::new();
        validate_domain57_stream_authority(&missing_pause, &compact, &mut missing_pause_errors);
        assert!(missing_pause_errors
            .iter()
            .any(|error| error.contains("exactly one pause row")));

        let mut bad_payload_hour = valid_domain57_summary();
        bad_payload_hour.domain57_frames[0].payload_hour_raw = 101;
        let mut bad_payload_hour_errors = Vec::new();
        validate_domain57_stream_authority(
            &bad_payload_hour,
            &compact,
            &mut bad_payload_hour_errors,
        );
        assert!(bad_payload_hour_errors
            .iter()
            .any(|error| error.contains("payload/header hour mismatch")));

        let mut bad_event_hour = valid_domain57_summary();
        bad_event_hour.domain57_frames[0].speed_rows[0].event_hour_raw = 101;
        let mut bad_event_hour_errors = Vec::new();
        validate_domain57_stream_authority(&bad_event_hour, &compact, &mut bad_event_hour_errors);
        assert!(bad_event_hour_errors
            .iter()
            .any(|error| error.contains("event/header hour mismatch")));
    }

    #[test]
    fn campaign_identity_is_exactly_bound_to_retained_d68_observation() {
        let mut observation =
            recorder_host::campaign_identity::SessionHeaderIdentityObservation::default();
        observation.session_header_records_seen = 1;
        observation.session_header_rows = 1;
        observation.witnesses = recorder_host::campaign_identity::CampaignIdentityWitnesses {
            start_date_raw: Some(1_000),
            session: Some(7),
            ..Default::default()
        };
        let summary = HeaderScanSummary {
            session_header_identity: observation,
            ..HeaderScanSummary::default()
        };
        let expected = recorder_host::campaign_identity::to_manifest_json(
            &recorder_host::campaign_identity::resolve(&summary.session_header_identity),
        );
        let manifest = json!({ "campaign_identity": expected });
        let mut errors = Vec::new();
        validate_campaign_identity_stream_authority(
            &summary,
            &manifest,
            StreamIdentityMode::Legacy,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let mut forged = manifest;
        forged["campaign_identity"]["witnesses"]["session"] = json!(8);
        let mut forged_errors = Vec::new();
        validate_campaign_identity_stream_authority(
            &summary,
            &forged,
            StreamIdentityMode::Legacy,
            &mut forged_errors,
        );
        assert!(forged_errors.iter().any(|error| {
            error.contains("does not exactly match independently decoded d68 stream authority")
        }));
    }

    #[test]
    fn identity_v1_scan_skips_cid1_accounting_and_exactly_joins_known_manifest() {
        use compact_contract::campaign_identity::{
            encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses,
            COMPACT_DOMAIN_STREAM_HEADER, COMPACT_RECORD_TYPE_STREAM_HEADER,
        };

        let hour = compact_contract::calendar::DATE_EPOCH_CONST + (1936 * 365 * 24);
        let observation = Cid1Observation {
            game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_string(),
            game_unique_seed: 1_026_046_527,
            game_hour_raw: hour,
        };
        let fields = Cid1Fields::from_stable_observations(
            observation.clone(),
            observation,
            "a729-production".to_string(),
            Cid1Witnesses::default(),
        )
        .unwrap();
        let cid1_payload = encode_cid1(&fields).unwrap();
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            0,
            hour,
            COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            &cid1_payload,
        );
        append_record(
            &mut raw,
            1,
            hour,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut raw,
            2,
            hour,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut raw,
            3,
            hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let summary =
            scan_compact_stream_with_identity_mode(&raw, StreamIdentityMode::IdentityV1);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert!(summary.ordering_errors.is_empty(), "{:?}", summary.ordering_errors);
        assert_eq!(summary.record_count, 3, "CID1 is not an ordinary record");
        assert!(!summary.domains_seen.contains(&COMPACT_DOMAIN_STREAM_HEADER));
        let cid1 = summary.validated_cid1.as_ref().expect("retained CID1");
        let expected = recorder_host::campaign_identity::to_manifest_json(
            &recorder_host::campaign_identity::resolve_validated_cid1(cid1),
        );
        let manifest = json!({ "campaign_identity": expected });
        assert_eq!(
            stream_identity_mode_from_manifest(&manifest),
            StreamIdentityMode::IdentityV1
        );
        let mut errors = Vec::new();
        validate_campaign_identity_stream_authority(
            &summary,
            &manifest,
            StreamIdentityMode::IdentityV1,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let mut forged = manifest;
        forged["campaign_identity"]["game_unique_seed"] = json!("7");
        let mut forged_errors = Vec::new();
        validate_campaign_identity_stream_authority(
            &summary,
            &forged,
            StreamIdentityMode::IdentityV1,
            &mut forged_errors,
        );
        assert!(!forged_errors.is_empty());
    }

    #[test]
    fn known_manifest_never_falls_back_to_legacy_without_cid1() {
        let raw = one_domain_stream_with_frame_payload(&frame_payload(0));
        let summary =
            scan_compact_stream_with_identity_mode(&raw, StreamIdentityMode::IdentityV1);
        assert!(summary.validated_cid1.is_none());
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("invalid CID1 stream header")));
    }

    #[test]
    fn header_scan_rejects_nonzero_frame_reserved_word() {
        let payload = frame_payload_with_fields(1, 1, 0, 1);
        let summary = scan_compact_stream(&one_domain_stream_with_frame_payload(&payload));
        assert!(
            summary
                .parse_errors
                .iter()
                .any(|error| error.contains("FRAME_END reserved word is nonzero")),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn header_scan_rejects_frame_domain_count_mismatches_independently() {
        let emitted_mismatch = frame_payload_with_fields(0, 0, 0, 0);
        let summary = scan_compact_stream(&one_domain_stream_with_frame_payload(&emitted_mismatch));
        assert!(
            summary.parse_errors.iter().any(|error| {
                error.contains("emitted_domains 0")
                    && error.contains("independently observed DOMAIN_PERF count 1")
            }),
            "{:?}",
            summary.parse_errors
        );

        let scheduled_mismatch = frame_payload_with_fields(1, 2, 0, 0);
        let summary =
            scan_compact_stream(&one_domain_stream_with_frame_payload(&scheduled_mismatch));
        assert!(
            summary.parse_errors.iter().any(|error| {
                error.contains("scheduled_domains 2") && error.contains("emitted_domains 1")
            }),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn header_scan_accepts_informational_frame_status_and_exact_counts() {
        let payload = frame_payload_with_fields(
            1,
            1,
            compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE,
            0,
        );
        let summary = scan_compact_stream(&one_domain_stream_with_frame_payload(&payload));
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn header_scan_requires_exact_perf_payload_size() {
        for len in [36, 39, 41] {
            let mut raw = Vec::new();
            append_record(&mut raw, 1, 100, 10, RECORD_TYPE_DOMAIN_PERF, &vec![0; len]);

            let summary = scan_compact_stream(&raw);
            assert!(
                summary.parse_errors.iter().any(|error| {
                    error.contains("DOMAIN_PERF")
                        && (error.contains(&format!("length {len}"))
                            || error.contains(&format!(
                                "{} unannounced trailer",
                                len.saturating_sub(40)
                            )))
                }),
                "len={len}: {:?}",
                summary.parse_errors
            );
        }
    }

    #[test]
    fn header_scan_requires_perf_trailer_flag_and_shape_to_agree() {
        for (record_flags, payload) in [
            (
                compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
                perf_payload(0),
            ),
            (0, perf_payload_with_empty_trailer(0)),
        ] {
            let mut raw = Vec::new();
            append_record_with_flags(
                &mut raw,
                1,
                100,
                10,
                RECORD_TYPE_DOMAIN_PERF,
                record_flags,
                &payload,
            );
            let summary = scan_compact_stream(&raw);
            assert!(
                summary.parse_errors.iter().any(|error| {
                    error.contains("diagnostic trailer header is truncated")
                        || error.contains("unannounced trailer")
                }),
                "flags={record_flags:#x}: {:?}",
                summary.parse_errors
            );
        }
    }

    #[test]
    fn header_scan_rejects_interleaved_data_hours() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut raw,
            2,
            101,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        assert!(
            summary
                .parse_errors
                .iter()
                .any(|error| error.contains("before FRAME_END closed open frame hour 100")),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn header_scan_fails_closed_on_malformed_or_unknown_data_payloads() {
        for (domain_id, payload) in [(10, vec![0; 23]), (999, vec![])] {
            let mut raw = Vec::new();
            append_record(
                &mut raw,
                1,
                100,
                domain_id,
                RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            append_record(
                &mut raw,
                2,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_payload(0),
            );

            let summary = scan_compact_stream(&raw);
            assert!(
                summary.parse_errors.iter().any(|error| {
                    error.contains(&format!("domain {domain_id} DATA payload"))
                        && error.contains("canonical validation")
                }),
                "domain={domain_id}: {:?}",
                summary.parse_errors
            );
        }
    }

    #[test]
    fn header_scan_rejects_mismatched_and_trailing_records_after_close() {
        let mut mismatched = Vec::new();
        append_record(
            &mut mismatched,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut mismatched,
            2,
            101,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let summary = scan_compact_stream(&mismatched);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("does not close open frame hour 100")));

        let mut trailing = Vec::new();
        append_record(
            &mut trailing,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut trailing,
            2,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        append_record(
            &mut trailing,
            3,
            101,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        let summary = scan_compact_stream(&trailing);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| { error.contains("not closed by a final canonical FRAME_END") }));

        let mut trailing_probe = Vec::new();
        append_record(
            &mut trailing_probe,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut trailing_probe,
            2,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        append_record(
            &mut trailing_probe,
            3,
            100,
            RC_PROBE_DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        let summary = scan_compact_stream(&trailing_probe);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| { error.contains("not closed by a final canonical FRAME_END") }));
    }

    #[test]
    fn scanned_hours_must_match_manifest_and_minimum() {
        let summary = HeaderScanSummary {
            unique_hours: vec![100, 101],
            ..HeaderScanSummary::default()
        };
        let honest = json!({
            "unique_game_hour_count": 2,
            "game_hours_advanced": 1,
        });
        let mut errors = Vec::new();
        validate_scanned_hours(&summary, &honest, 1, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");

        let dishonest = json!({
            "unique_game_hour_count": 3,
            "game_hours_advanced": 2,
        });
        validate_scanned_hours(&summary, &dishonest, 1, &mut errors);
        assert!(errors
            .iter()
            .any(|error| error.contains("unique hour count 2")));
        assert!(errors
            .iter()
            .any(|error| error.contains("inclusive hour span 2")));

        let one_hour = HeaderScanSummary {
            unique_hours: vec![100],
            ..HeaderScanSummary::default()
        };
        let manifest = json!({
            "unique_game_hour_count": 1,
            "game_hours_advanced": 0,
        });
        let mut minimum_errors = Vec::new();
        validate_scanned_hours(&one_hour, &manifest, 1, &mut minimum_errors);
        assert_eq!(
            minimum_errors
                .iter()
                .filter(|error| error.contains("below required"))
                .count(),
            2,
            "{minimum_errors:?}"
        );
    }

    #[test]
    fn requested_duration_is_bound_to_cli_and_uses_inclusive_closed_frames() {
        let honest = json!({
            "hourly_requirement": {
                "required": true,
                "met": true,
                "problems": [],
                "requested_game_hours": 3,
            },
            "game_hours_advanced": 3,
            "unique_game_hour_count": 4,
            "skipped_game_hours": 0,
        });
        let mut errors = Vec::new();
        validate_hourly_manifest(&honest, 3, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");

        let mut mismatched_request = honest.clone();
        mismatched_request["hourly_requirement"]["requested_game_hours"] = json!(4);
        validate_hourly_manifest(&mismatched_request, 3, &mut errors);
        assert!(errors.iter().any(|error| {
            error.contains("requested_game_hours 4") && error.contains("--min-game-hours 3")
        }));

        let mut missing_closed_frame = honest;
        missing_closed_frame["unique_game_hour_count"] = json!(3);
        validate_hourly_manifest(&missing_closed_frame, 3, &mut errors);
        assert!(errors.iter().any(|error| {
            error.contains("unique_game_hour_count 3")
                && error.contains("game_hours_advanced + 1 (4)")
        }));
    }

    #[test]
    fn every_canonical_failure_bit_fails_manifest_and_stream_gates() {
        let canonical_bits = [
            compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PROFILE_MISMATCH,
            compact_contract::COMPACT_STATUS_NO_CLOCK,
            compact_contract::COMPACT_STATUS_NO_SCALAR_SOURCE,
            compact_contract::COMPACT_STATUS_WORKER_PANIC,
            compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            compact_contract::COMPACT_STATUS_CONTROL_INVALID,
        ];
        assert_eq!(
            canonical_bits.into_iter().fold(0, |mask, bit| mask | bit),
            COMPACT_STATUS_FAILURE_MASK,
            "test table must enumerate every canonical failure bit"
        );

        let contract = contract_by_id(10).expect("domain 10 contract exists");
        for bit in canonical_bits {
            let manifest = json!({
                "domain_id": 10,
                "record_count": 1,
                "data_record_count": 1,
                "row_count_total": 1,
                "status_flags_or": bit,
                "read_time_us": {"count": 1},
                "data_units_total": {"fuel_country_rows": 1},
            });
            let mut errors = Vec::new();
            validate_domain_manifest(&manifest, contract, 1, &GateOptions::default(), &mut errors);
            assert!(
                errors.iter().any(|error| error.contains("failure bits")),
                "manifest gate accepted canonical failure bit {bit:#x}: {errors:?}"
            );

            let mut raw = Vec::new();
            append_record(
                &mut raw,
                1,
                100,
                10,
                RECORD_TYPE_DOMAIN_PERF,
                &perf_payload(bit),
            );
            append_record(
                &mut raw,
                2,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_payload(0),
            );
            let summary = scan_compact_stream(&raw);
            assert_ne!(
                summary.global_status_flags_or & COMPACT_STATUS_FAILURE_MASK,
                0,
                "stream gate missed canonical failure bit {bit:#x}"
            );
        }
    }

    fn test_contract(domain_id: u16, name: &'static str) -> DomainContract {
        DomainContract {
            id: domain_id,
            name,
            requires_data: true,
            required_units: &[],
            zero_units: &[],
            work_implies_rows_units: &[],
            shared_recorder_known: true,
        }
    }

    fn test_domain_manifest(domain_id: u16, status: Value) -> Value {
        json!({
            "domain_id": domain_id,
            "record_count": 1,
            "data_record_count": 1,
            "row_count_total": 1,
            "status_flags_or": status,
            "read_time_us": {"count": 1},
            "data_units_total": {},
        })
    }

    #[test]
    fn d51_lane_local_bit12_is_not_global_control_invalid() {
        let bit12 = compact_contract::DOMAIN51_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED;
        assert_eq!(bit12, compact_contract::COMPACT_STATUS_CONTROL_INVALID);
        let contract = test_contract(51, "scripted_state_flags_variables_live");
        let manifest = test_domain_manifest(51, json!(bit12));
        let mut errors = Vec::new();
        validate_domain_manifest(&manifest, contract, 1, &GateOptions::default(), &mut errors);
        assert!(
            errors.is_empty(),
            "lane-local bit was treated globally: {errors:?}"
        );

        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            51,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(bit12),
        );
        append_record(
            &mut raw,
            2,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let summary = scan_compact_stream(&raw);
        assert_eq!(summary.per_domain_status_flags_or.get(&51), Some(&bit12));
        assert_eq!(summary.global_status_flags_or, 0);
        assert_eq!(summary.frame_status_flags_or_raw, 0);
    }

    #[test]
    fn partial_source_override_never_allows_no_clock_or_control_invalid() {
        let options = GateOptions {
            allow_partial_or_source_failure: true,
            ..GateOptions::default()
        };
        let contract = test_contract(10, "fuel_live");
        for bit in [
            compact_contract::COMPACT_STATUS_NO_CLOCK,
            compact_contract::COMPACT_STATUS_CONTROL_INVALID,
        ] {
            let manifest = test_domain_manifest(10, json!(bit));
            let mut errors = Vec::new();
            validate_domain_manifest(&manifest, contract, 1, &options, &mut errors);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("rejected global failure bits")),
                "override admitted non-permitted canonical failure {bit:#x}: {errors:?}"
            );
            assert_ne!(rejected_global_failure_bits(bit, true), 0);

            let mut raw = Vec::new();
            append_record(
                &mut raw,
                1,
                100,
                10,
                RECORD_TYPE_DOMAIN_PERF,
                &perf_payload(bit),
            );
            append_record(
                &mut raw,
                2,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_payload(0),
            );
            let summary = scan_compact_stream(&raw);
            assert_ne!(
                rejected_global_failure_bits(summary.global_status_flags_or, true),
                0,
                "stream override admitted non-permitted canonical failure {bit:#x}"
            );
        }
    }

    #[test]
    fn broad_partial_source_override_does_not_certify_unregistered_domain_failures() {
        let options = GateOptions {
            allow_partial_or_source_failure: true,
            ..GateOptions::default()
        };
        let contract = test_contract(10, "fuel_live");
        let permitted = compact_contract::COMPACT_STATUS_PARTIAL_DATA
            | compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
        let manifest = test_domain_manifest(10, json!(permitted));
        let mut errors = Vec::new();
        validate_domain_manifest(&manifest, contract, 1, &options, &mut errors);
        assert!(errors
            .iter()
            .any(|error| error.contains("rejected global failure bits")));
        assert_eq!(rejected_global_failure_bits(permitted, true), 0);
    }

    #[test]
    fn manifest_status_above_u32_is_rejected_without_truncation() {
        let contract = test_contract(10, "fuel_live");
        let oversized = u32::MAX as u64 + 1;
        let manifest = test_domain_manifest(10, json!(oversized));
        let mut errors = Vec::new();
        validate_domain_manifest(&manifest, contract, 1, &GateOptions::default(), &mut errors);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("exceeds u32::MAX")),
            "oversized status was truncated to zero: {errors:?}"
        );
    }

    #[test]
    fn run_paths_reject_traversal_absolute_and_dot_components() {
        for raw in [
            "../escape.bin",
            "nested/../escape.bin",
            "./stream.bin",
            "C:\\escape.bin",
        ] {
            assert!(
                validate_relative_path(raw, "raw_stream").is_err(),
                "{raw:?}"
            );
        }
        assert_eq!(
            validate_relative_path("nested/collector_stream.bin", "raw_stream").unwrap(),
            PathBuf::from("nested/collector_stream.bin")
        );
    }

    #[test]
    fn report_output_cannot_alias_any_verified_input() {
        let dir = TmpDir::new("report_alias");
        let manifest_path = dir.0.join("native_session_manifest.json");
        let stream_path = dir.0.join("collector_stream.bin");
        let replay_path = dir.0.join("native_session.battleplan");
        fs::write(
            &manifest_path,
            serde_json::to_vec(&json!({
                "raw_stream": "collector_stream.bin",
                "replay_file": "native_session.battleplan",
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(&stream_path, b"stream").unwrap();
        fs::write(&replay_path, b"replay").unwrap();

        for protected in [&manifest_path, &stream_path, &replay_path] {
            let options = GateOptions {
                run_dir: dir.0.clone(),
                write_report: Some(protected.clone()),
                ..GateOptions::default()
            };
            let error = validate_report_output_path(&options).unwrap_err();
            assert!(error.contains("aliases protected"), "{error}");
        }

        let alias_parent = dir.0.join("alias_parent");
        fs::create_dir(&alias_parent).unwrap();
        let lexical_alias = alias_parent.join("..").join("collector_stream.bin");
        let options = GateOptions {
            run_dir: dir.0.clone(),
            write_report: Some(lexical_alias),
            ..GateOptions::default()
        };
        let error = validate_report_output_path(&options).unwrap_err();
        assert!(error.contains("raw stream"), "{error}");

        let options = GateOptions {
            run_dir: dir.0.clone(),
            write_report: Some(dir.0.join("gate_report.json")),
            ..GateOptions::default()
        };
        assert!(validate_report_output_path(&options).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn report_output_rejects_directory_symlink_equivalent_when_supported() {
        let outer = TmpDir::new("report_symlink_alias");
        let run_dir = outer.0.join("run");
        fs::create_dir(&run_dir).unwrap();
        fs::write(
            run_dir.join("native_session_manifest.json"),
            serde_json::to_vec(&json!({
                "raw_stream": "collector_stream.bin",
                "replay_file": "native_session.battleplan",
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(run_dir.join("collector_stream.bin"), b"stream").unwrap();
        fs::write(run_dir.join("native_session.battleplan"), b"replay").unwrap();
        let alias = outer.0.join("run_alias");
        if std::os::windows::fs::symlink_dir(&run_dir, &alias).is_err() {
            return;
        }
        let options = GateOptions {
            run_dir,
            write_report: Some(alias.join("collector_stream.bin")),
            ..GateOptions::default()
        };
        let error = validate_report_output_path(&options).unwrap_err();
        assert!(error.contains("raw stream"), "{error}");
        fs::remove_dir(&alias).unwrap();
    }

    #[test]
    fn raw_and_zstd_storage_are_decoded_before_hash_and_length_checks() {
        let dir = TmpDir::new("storage");
        let decoded = b"H4CR decoded authority".repeat(32);
        for compression in [
            compact_stream::StreamCompression::None,
            compact_stream::StreamCompression::Zstd,
        ] {
            let path = dir.0.join(format!("{}.bin", compression.as_str()));
            let mut writer = compact_stream::StreamWriter::create(&path, compression).unwrap();
            writer.write_all(&decoded).unwrap();
            writer.finish().unwrap();
            let stored = fs::read(&path).unwrap();
            let manifest = json!({
                "stream_storage": {
                    "compression": compression.as_str(),
                    "stored_bytes": stored.len(),
                    "stored_sha256": Sha256::hex(&stored),
                    "decoded_bytes": decoded.len(),
                    "decoded_sha256": Sha256::hex(&decoded),
                }
            });
            let artifact = read_stream_artifact(&path, &manifest, 1 << 20).unwrap();
            assert_eq!(artifact.stored, stored);
            assert_eq!(artifact.decoded, decoded);
        }
    }

    #[test]
    fn embedded_manifest_is_authoritative_and_external_rewrite_is_rejected() {
        let dir = TmpDir::new("manifest_rewrite");
        let manifest = replay_test_manifest("g", &dir.0);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let authority = load_verified_replay_authority(&manifest, &manifest_bytes, &dir.0).unwrap();
        assert_eq!(authority.manifest, manifest);
        assert_eq!(
            authority.details["static_artifacts"]["required_count"],
            json!(compact_contract::REQUIRED_STATIC_ARTIFACTS.len())
        );
        assert_eq!(
            authority.details["static_artifacts"]["exact_required_roles"],
            json!(true)
        );

        let mut rewritten = manifest.clone();
        rewritten["duration_ms"] = json!(999_999);
        let rewritten_bytes = serde_json::to_vec_pretty(&rewritten).unwrap();
        let error = load_verified_replay_authority(&rewritten, &rewritten_bytes, &dir.0)
            .expect_err("mutable disk rewrite must fail");
        assert!(error.contains("not byte-identical"), "{error}");
    }

    #[test]
    fn static_manifest_rosters_and_hashes_are_exactly_bound() {
        let error = static_authority_error(
            "static_profile_missing",
            |manifest| {
                manifest["production_profile"]["required_static_artifacts"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            },
            |_, _| {},
        );
        assert!(error.contains("required_static_artifacts"), "{error}");

        let error = static_authority_error(
            "static_packaging_missing",
            |manifest| {
                manifest["replay_packaging"]["static_artifacts"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            },
            |_, _| {},
        );
        assert!(error.contains("expected exactly"), "{error}");

        let error = static_authority_error(
            "static_packaging_extra",
            |manifest| {
                manifest["replay_packaging"]["static_artifacts"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"name": "forged_extra", "sha256": "0".repeat(64)}));
            },
            |_, _| {},
        );
        assert!(error.contains("expected exactly"), "{error}");

        let error = static_authority_error(
            "static_bundle_duplicate",
            |manifest| {
                let bundle = manifest["static_bundle"]["artifacts"]
                    .as_array_mut()
                    .unwrap();
                bundle[6] = bundle[0].clone();
            },
            |_, _| {},
        );
        assert!(error.contains("duplicates role"), "{error}");

        let error = static_authority_error(
            "static_bundle_hash_mismatch",
            |manifest| {
                manifest["static_bundle"]["artifacts"][0]["sha256"] = json!("0".repeat(64));
            },
            |_, _| {},
        );
        assert!(error.contains("does not exactly match"), "{error}");

        let error = static_authority_error(
            "static_bundle_bad_id",
            |manifest| {
                manifest["static_bundle"]["bundle_id"] = json!("not-a-digest");
            },
            |_, _| {},
        );
        assert!(error.contains("bundle_id"), "{error}");

        let error = static_authority_error(
            "static_bundle_arbitrary_valid_id",
            |manifest| {
                manifest["static_bundle"]["bundle_id"] = json!("a".repeat(64));
            },
            |_, _| {},
        );
        assert!(
            error.contains("canonical retained manifest identity"),
            "{error}"
        );

        let error = static_authority_error(
            "static_bundle_arbitrary_product_modset",
            |manifest| {
                manifest["static_bundle"]["product_modset_sha256"] =
                    json!("9".repeat(64));
            },
            |_, _| {},
        );
        assert!(error.contains("differs from static-bundle"), "{error}");

        let error = static_authority_error(
            "static_bundle_bad_file",
            |manifest| {
                manifest["static_bundle"]["artifacts"][0]["file"] = json!("artifacts/wrong.json");
            },
            |_, _| {},
        );
        assert!(
            error.contains("file") && error.contains("canonical"),
            "{error}"
        );

        let error = static_authority_error(
            "static_bundle_bad_artifact_self_hash",
            |manifest| {
                manifest["static_bundle"]["artifacts"][0]["artifact_sha256"] = json!("bad");
            },
            |_, _| {},
        );
        assert!(error.contains("artifact_sha256"), "{error}");

        let error = static_authority_error(
            "static_bundle_arbitrary_valid_artifact_self_hash",
            |manifest| {
                manifest["static_bundle"]["artifacts"][0]["artifact_sha256"] =
                    json!("b".repeat(64));
            },
            |_, _| {},
        );
        assert!(
            error.contains("does not match retained bundle manifest"),
            "{error}"
        );

        let error = static_authority_error(
            "static_bundle_bad_report_hash",
            |manifest| {
                manifest["static_bundle"]["artifacts"][0]["source_gate_report_sha256"] =
                    json!("bad");
            },
            |_, _| {},
        );
        assert!(error.contains("source_gate_report_sha256"), "{error}");

        let error = static_authority_error(
            "static_bundle_arbitrary_valid_report_hash",
            |manifest| {
                manifest["static_bundle"]["artifacts"][0]["source_gate_report_sha256"] =
                    json!("c".repeat(64));
            },
            |_, _| {},
        );
        assert!(
            error.contains("does not match retained bundle manifest"),
            "{error}"
        );

        let error = static_authority_error(
            "static_bundle_missing_binding",
            |manifest| {
                manifest["static_bundle"]["bindings"]
                    .as_object_mut()
                    .unwrap()
                    .remove("source_inventory");
            },
            |_, _| {},
        );
        assert!(error.contains("bindings"), "{error}");

        let error = static_authority_error(
            "static_bundle_bad_verifier_hash",
            |manifest| {
                manifest["static_bundle"]["verifier"]["sha256"] = json!("bad");
            },
            |_, _| {},
        );
        assert!(error.contains("verifier.sha256"), "{error}");

        let error = static_authority_error(
            "static_bundle_arbitrary_valid_binding_hash",
            |manifest| {
                manifest["static_bundle"]["bindings"]["mod_state"]["sha256"] =
                    json!("d".repeat(64));
            },
            |_, _| {},
        );
        assert!(error.contains("retained file sha256"), "{error}");

        let error = static_authority_error(
            "static_bundle_arbitrary_valid_verifier_hash",
            |manifest| {
                manifest["static_bundle"]["verifier"]["sha256"] = json!("e".repeat(64));
            },
            |_, _| {},
        );
        assert!(error.contains("retained file sha256"), "{error}");

        let error = static_authority_error(
            "static_bundle_arbitrary_valid_manifest_hash",
            |manifest| {
                manifest["static_bundle"]["manifest"]["sha256"] = json!("f".repeat(64));
            },
            |_, _| {},
        );
        assert!(error.contains("retained file sha256"), "{error}");

        let error = static_authority_error(
            "static_bundle_no_token_gate",
            |manifest| {
                manifest["static_bundle"]["token_gate_required"] = json!(false);
            },
            |_, _| {},
        );
        assert!(error.contains("token_gate_required"), "{error}");
    }

    #[test]
    fn production_manifest_refuses_v1_or_mismatched_product_modset_content() {
        let error = static_authority_error(
            "portable_content_v1",
            |manifest| {
                manifest["portable_content_manifest"]["schema"] =
                    json!("hoi4.portable-content-manifest.v1");
            },
            |_, _| {},
        );
        assert!(error.contains("not v2 product-modset-bound"), "{error}");

        let error = static_authority_error(
            "portable_content_foreign_product",
            |manifest| {
                manifest["portable_content_manifest"]["product_modset_sha256"] =
                    json!("f".repeat(64));
            },
            |_, _| {},
        );
        assert!(error.contains("differs from static-bundle"), "{error}");
    }

    #[test]
    fn retained_static_evidence_bytes_cannot_drift_after_admission() {
        let error = retained_static_evidence_drift_error("retained_manifest_drift", |manifest| {
            let path = PathBuf::from(
                manifest["static_bundle"]["manifest"]["path"]
                    .as_str()
                    .unwrap(),
            );
            fs::write(path, b"{\"forged\":true}").unwrap();
        });
        assert!(error.contains("manifest.sha256"), "{error}");

        let error = retained_static_evidence_drift_error("retained_artifact_drift", |manifest| {
            let bundle_manifest = PathBuf::from(
                manifest["static_bundle"]["manifest"]["path"]
                    .as_str()
                    .unwrap(),
            );
            let role = manifest["static_bundle"]["artifacts"][0]["role"]
                .as_str()
                .unwrap();
            let file = required_static_bundle_file(role).unwrap();
            fs::write(
                bundle_manifest.parent().unwrap().join(file),
                b"{\"forged\":true}",
            )
            .unwrap();
        });
        assert!(
            error.contains("artifact") && error.contains("bytes/hash"),
            "{error}"
        );

        let error = retained_static_evidence_drift_error("retained_report_drift", |manifest| {
            let bundle_manifest = PathBuf::from(
                manifest["static_bundle"]["manifest"]["path"]
                    .as_str()
                    .unwrap(),
            );
            let role = manifest["static_bundle"]["artifacts"][0]["role"]
                .as_str()
                .unwrap();
            let report = required_static_gate_report_file(role).unwrap();
            fs::write(
                bundle_manifest.parent().unwrap().join(report),
                b"{\"passed\":true}",
            )
            .unwrap();
        });
        assert!(
            error.contains("source_gate_report_sha256") && error.contains("report bytes"),
            "{error}"
        );

        let error = retained_static_evidence_drift_error("retained_binding_drift", |manifest| {
            let path = PathBuf::from(
                manifest["static_bundle"]["bindings"]["mod_state"]["path"]
                    .as_str()
                    .unwrap(),
            );
            fs::write(path, b"{\"forged\":true}").unwrap();
        });
        assert!(error.contains("bindings.mod_state"), "{error}");

        let error = retained_static_evidence_drift_error("retained_verifier_drift", |manifest| {
            let path = PathBuf::from(
                manifest["static_bundle"]["verifier"]["path"]
                    .as_str()
                    .unwrap(),
            );
            fs::write(path, b"# forged verifier\n").unwrap();
        });
        assert!(error.contains("verifier.sha256"), "{error}");
    }

    #[test]
    fn internally_shaped_bundle_id_must_still_equal_canonical_manifest_identity() {
        let dir = TmpDir::new("internally_shaped_bundle_id");
        let mut manifest = replay_test_manifest("g", &dir.0);
        let retained_path = PathBuf::from(
            manifest["static_bundle"]["manifest"]["path"]
                .as_str()
                .unwrap(),
        );
        let retained_bytes = fs::read(&retained_path).unwrap();
        let mut retained = snapshot::parse_json_strict(&retained_bytes).unwrap();
        let arbitrary = "a".repeat(64);
        retained["bundle_id"] = json!(arbitrary);
        let forged_bytes = serde_json::to_vec_pretty(&retained).unwrap();
        fs::write(&retained_path, &forged_bytes).unwrap();
        manifest["static_bundle"]["bundle_id"] = json!(arbitrary);
        manifest["static_bundle"]["manifest"]["sha256"] = json!(Sha256::hex(&forged_bytes));
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let error = load_verified_replay_authority(&manifest, &manifest_bytes, &dir.0)
            .expect_err("internally shaped non-canonical bundle identity must fail");
        assert!(
            error.contains("canonical retained manifest identity"),
            "{error}"
        );
    }

    #[test]
    fn static_index_and_container_entries_reject_missing_extra_alias_and_metadata_forgery() {
        let error = static_authority_error(
            "static_index_missing",
            |_| {},
            |rows, _| {
                rows.pop();
            },
        );
        assert!(error.contains("index.static_artifacts"), "{error}");

        let error = static_authority_error(
            "static_index_duplicate",
            |_| {},
            |rows, _| {
                rows[6] = rows[0].clone();
            },
        );
        assert!(
            error.contains("aliases multiple static roles")
                || error.contains("duplicates static role"),
            "{error}"
        );

        let error = static_authority_error(
            "static_index_extra",
            |_| {},
            |rows, _| {
                rows.push(json!({
                    "name": "forged_extra",
                    "present": true,
                    "role": "static_artifact",
                    "path": "static/forged_extra/extra.json",
                    "sha256": "0".repeat(64),
                    "bytes": 1,
                }));
            },
        );
        assert!(error.contains("expected exactly"), "{error}");

        let error = static_authority_error(
            "static_index_alias",
            |_| {},
            |rows, _| {
                rows[6]["path"] = rows[0]["path"].clone();
            },
        );
        assert!(
            error.contains("path") && error.contains("canonical"),
            "{error}"
        );

        let error = static_authority_error(
            "static_index_hash",
            |_| {},
            |rows, _| {
                rows[0]["sha256"] = json!("0".repeat(64));
            },
        );
        assert!(error.contains("manifest sha256"), "{error}");

        let error = static_authority_error(
            "static_index_bytes",
            |_| {},
            |rows, _| {
                let bytes = rows[0]["bytes"].as_u64().unwrap();
                rows[0]["bytes"] = json!(bytes + 1);
            },
        );
        assert!(error.contains("container metadata sha256/bytes"), "{error}");

        let error = static_authority_error(
            "static_entry_missing",
            |_| {},
            |_, entries| {
                entries.pop();
            },
        );
        assert!(error.contains("static_artifact entries"), "{error}");

        let error = static_authority_error(
            "static_entry_extra",
            |_| {},
            |_, entries| {
                entries.push(snapshot::PackEntry {
                    path: "payload/g/static/forged_extra/extra.json".to_string(),
                    bytes: b"{}".to_vec(),
                    role: "static_artifact",
                    hour_index: None,
                });
            },
        );
        assert!(error.contains("static_artifact entries"), "{error}");

        let error = static_authority_error(
            "static_entry_wrong_role",
            |_| {},
            |_, entries| {
                entries[0].role = "other";
            },
        );
        assert!(
            error.contains("static_artifact entries") || error.contains("non-static role"),
            "{error}"
        );
    }

    #[test]
    fn replay_identity_and_required_roles_fail_closed() {
        let dir = TmpDir::new("identity_roles");
        let manifest = replay_test_manifest("expected", &dir.0);
        let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "foreign", &bytes, true);
        let error = load_verified_replay_authority(&manifest, &bytes, &dir.0).unwrap_err();
        assert!(error.contains("game_id"), "{error}");

        write_replay_fixture(&dir.0, "expected", &bytes, false);
        let error = load_verified_replay_authority(&manifest, &bytes, &dir.0).unwrap_err();
        assert!(error.contains("recorder_manifest entries"), "{error}");
    }

    #[test]
    fn replay_entries_role_declarations_are_required() {
        let dir = TmpDir::new("replay_entry_roles");
        let mut manifest = replay_test_manifest("g", &dir.0);
        manifest["replay_entries"]
            .as_object_mut()
            .unwrap()
            .remove("stream_role");
        assert!(replay_layout(&manifest).is_err());
    }

    #[test]
    fn skip_and_other_weakenings_are_machine_visible() {
        let options = GateOptions {
            require_replay_verify: false,
            allow_bridge_roots: true,
            allow_partial_or_source_failure: true,
            strict_semantic_units: false,
            ..GateOptions::default()
        };
        let weakenings = verification_weakenings(&options);
        assert_eq!(weakenings.len(), 4);
        let mut errors = Vec::new();
        reject_verification_weakenings(&weakenings, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bridge_root_debt_tolerated"));
        assert!(errors[0].contains("semantic_unit_requirements_disabled"));
        assert!(!errors.is_empty(), "a weakened profile must not certify");

        let defaults = GateOptions::default();
        let weakenings = verification_weakenings(&defaults);
        let mut errors = Vec::new();
        reject_verification_weakenings(&weakenings, &mut errors);
        assert!(weakenings.is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn oversized_probe_is_included_in_framing_caps_but_not_manifest_counters() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            RC_PROBE_DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        let summary = scan_compact_stream(&raw);
        assert_eq!(summary.record_count, 0);
        assert_eq!(summary.max_record_len, 0);
        assert_eq!(summary.framed_max_payload_len, 40);
        assert_eq!(summary.framed_max_record_len, 72);
    }

    #[test]
    fn pdu1_units_and_shared_d47_d53_authority_are_independently_decoded() {
        let mut summary = HeaderScanSummary::default();
        let payload = perf_payload_with_unit("adapter_rows", 7);
        collect_perf_units(
            47,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &payload,
            &mut summary,
        );
        assert_eq!(summary.per_domain_combined_units[&47]["adapter_rows"], 7);
        let d47 = characters_leaders_cabinet_live_lane::encode_payload(0, 0x1234, 0, &[]);
        let d53_status = diplomacy_intel_operatives_live::compact::STATUS_OPERATIONS_DETAIL_RESISTED
            | diplomacy_intel_operatives_live::compact::STATUS_INTEL_MAP_RESISTED;
        let d53 = diplomacy_intel_operatives_live::compact::encode_payload(
            &diplomacy_intel_operatives_live::compact::DiplomacyIntelSnapshot {
                status_flags: d53_status,
                ..Default::default()
            },
        );
        let d47 = recorder_host::domain_data_authority::summarize_domain_data_payload(
            47,
            &d47,
            recorder_host::domain_data_authority::DomainDataContext::default(),
        )
        .unwrap();
        assert_eq!(d47.status_flags, Some(0x1234));
        let d53 = recorder_host::domain_data_authority::summarize_domain_data_payload(
            53,
            &d53,
            recorder_host::domain_data_authority::DomainDataContext::default(),
        )
        .unwrap();
        assert_eq!(d53.status_flags, Some(d53_status));
    }

    #[test]
    fn d69_failed_slot_mask_chunks_or_across_perf_diagnostics() {
        let mut summary = HeaderScanSummary::default();
        for name in [
            "d69_failed_slot_mask_0_63",
            "d69_failed_slot_mask_64_127",
            "d69_failed_slot_mask_128_191",
            "d69_failed_slot_mask_192_255",
        ] {
            let payload = perf_payload_with_unit(name, 1);
            collect_perf_units(
                69,
                compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
                &payload,
                &mut summary,
            );
            collect_perf_units(
                69,
                compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
                &payload,
                &mut summary,
            );
            assert_eq!(summary.per_domain_combined_units[&69][name], 1);
        }

        let reason = perf_payload_with_unit("d69_fail_vtable_mismatch", 1);
        collect_perf_units(
            69,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &reason,
            &mut summary,
        );
        collect_perf_units(
            69,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &reason,
            &mut summary,
        );
        assert_eq!(
            summary.per_domain_combined_units[&69]["d69_fail_vtable_mismatch"],
            2
        );
    }

    #[test]
    fn d69_scanner_accepts_tsg2_and_rejects_legacy_or_malformed_payloads() {
        use compact_contract::tech_sharing_groups::{
            encode_payload, encode_payload_with_bonuses, TechSharingGroupBonusRow,
            TechSharingGroupRow, TechSharingMemberBonus,
        };

        let valid = encode_payload_with_bonuses(
            &[
                TechSharingGroupBonusRow {
                    id_token: 11,
                    members: vec![
                        TechSharingMemberBonus {
                            member_tag_db_id: 101,
                            dynamic_bonus_raw_fixed5: 5_000,
                        },
                        TechSharingMemberBonus {
                            member_tag_db_id: 102,
                            dynamic_bonus_raw_fixed5: -2_500,
                        },
                    ],
                },
                TechSharingGroupBonusRow {
                    id_token: 12,
                    members: vec![TechSharingMemberBonus {
                        member_tag_db_id: 103,
                        dynamic_bonus_raw_fixed5: 0,
                    }],
                },
            ],
            0,
            compact_contract::COMPACT_STATUS_DELTA_DATA,
        )
        .unwrap();
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            69,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_counts(2, 2, 3, 3, compact_contract::COMPACT_STATUS_DELTA_DATA),
        );
        append_record(&mut raw, 2, 100, 69, RECORD_TYPE_DOMAIN_DATA, &valid);
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let summary = scan_compact_stream(&raw);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let mut authority_errors = Vec::new();
        validate_scanned_domain_authority(&summary, &json!({}), &[], &mut authority_errors);
        assert!(authority_errors.is_empty(), "{authority_errors:?}");

        let empty = encode_payload_with_bonuses(&[], 0, 0).unwrap();
        let mut empty_raw = Vec::new();
        append_record(
            &mut empty_raw,
            1,
            100,
            69,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_counts(0, 0, 0, 0, 0),
        );
        append_record(&mut empty_raw, 2, 100, 69, RECORD_TYPE_DOMAIN_DATA, &empty);
        append_record(
            &mut empty_raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let empty_summary = scan_compact_stream(&empty_raw);
        assert!(
            empty_summary.parse_errors.is_empty(),
            "{:?}",
            empty_summary.parse_errors
        );
        let mut empty_errors = Vec::new();
        validate_scanned_domain_authority(&empty_summary, &json!({}), &[], &mut empty_errors);
        assert!(empty_errors.is_empty(), "{empty_errors:?}");

        let legacy = encode_payload(
            &[TechSharingGroupRow {
                id_token: 11,
                member_tag_db_ids: vec![101],
            }],
            0,
            0,
        );
        let mut malformed = valid;
        malformed[20..24].copy_from_slice(&3u32.to_le_bytes());
        for (label, payload) in [("legacy", legacy), ("malformed", malformed)] {
            let mut raw = Vec::new();
            append_record(
                &mut raw,
                1,
                100,
                69,
                RECORD_TYPE_DOMAIN_PERF,
                &perf_payload_with_counts(1, 1, 1, 1, 0),
            );
            append_record(&mut raw, 2, 100, 69, RECORD_TYPE_DOMAIN_DATA, &payload);
            append_record(
                &mut raw,
                3,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_payload(0),
            );
            let summary = scan_compact_stream(&raw);
            assert!(
                summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains("TSG2") || error.contains("tech-sharing bonus")),
                "{label}: {:?}",
                summary.parse_errors
            );
            assert!(!summary.per_hour_data_authority.contains_key(&(100, 69)));
        }
    }

    #[test]
    fn all_manifest_counter_families_and_exact_unit_map_are_independently_bound() {
        let contract = test_contract(10, "fuel_live");
        let mut summary = HeaderScanSummary::default();
        summary.per_domain_perf_authority.insert(
            10,
            PerfAuthority {
                row_count: 11,
                changed_rows: 12,
                unit_count: 13,
                changed_unit_count: 14,
                status_flags: 0,
            },
        );
        summary.per_domain_data_authority.insert(
            10,
            DataAuthority {
                row_count: 21,
                changed_rows: 22,
                status_flags: Some(0),
                data_units: BTreeMap::from([("fuel_country_rows".to_string(), 23)]),
            },
        );
        summary
            .per_domain_combined_units
            .insert(10, BTreeMap::from([("fuel_country_rows".to_string(), 23)]));
        let manifest_domain = json!({
            "row_count_total": 11,
            "changed_rows_total": 12,
            "unit_count_total": 13,
            "changed_unit_count_total": 14,
            "data_rows_total": 21,
            "data_changed_rows_total": 22,
            "status_flags_or": 0,
            "data_units_total": {"fuel_country_rows": 23},
        });
        let compact = json!({"per_domain": {"fuel_live": manifest_domain}});
        let mut errors = Vec::new();
        validate_scanned_domain_authority(&summary, &compact, &[contract], &mut errors);
        assert!(errors.is_empty(), "{errors:?}");

        for field in [
            "row_count_total",
            "changed_rows_total",
            "unit_count_total",
            "changed_unit_count_total",
            "data_rows_total",
            "data_changed_rows_total",
        ] {
            let mut forged = compact.clone();
            forged["per_domain"]["fuel_live"][field] = json!(999);
            let mut errors = Vec::new();
            validate_scanned_domain_authority(&summary, &forged, &[contract], &mut errors);
            assert!(
                errors.iter().any(|error| error.contains(field)),
                "field={field}: {errors:?}"
            );
        }

        let mut forged_status = compact.clone();
        forged_status["per_domain"]["fuel_live"]["status_flags_or"] = json!(1);
        let mut errors = Vec::new();
        validate_scanned_domain_authority(&summary, &forged_status, &[contract], &mut errors);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("raw PERF/DATA status_flags_or")),
            "{errors:?}"
        );

        let mut forged = compact.clone();
        forged["per_domain"]["fuel_live"]["data_units_total"]["forged_extra"] = json!(1);
        let mut errors = Vec::new();
        validate_scanned_domain_authority(&summary, &forged, &[contract], &mut errors);
        assert!(
            errors.iter().any(|error| error.contains("forged_extra")
                && error.contains("no independently decoded authority")),
            "{errors:?}"
        );

        let mut combined_status_summary = summary;
        combined_status_summary
            .per_domain_status_flags_or
            .insert(10, compact_contract::COMPACT_STATUS_PARTIAL_DATA);
        let mut combined_status_manifest = compact;
        combined_status_manifest["per_domain"]["fuel_live"]["status_flags_or"] =
            json!(compact_contract::COMPACT_STATUS_PARTIAL_DATA);
        let mut combined_status_errors = Vec::new();
        validate_scanned_domain_authority(
            &combined_status_summary,
            &combined_status_manifest,
            &[contract],
            &mut combined_status_errors,
        );
        assert!(
            combined_status_errors.is_empty(),
            "{combined_status_errors:?}"
        );
    }

    #[test]
    fn d47_and_d53_keep_their_distinct_per_hour_counter_semantics() {
        let mut summary = HeaderScanSummary::default();
        summary.per_hour_data_authority.insert(
            (100, 47),
            DataAuthority {
                row_count: 3,
                changed_rows: 3,
                status_flags: Some(0),
                data_units: BTreeMap::new(),
            },
        );
        summary.per_hour_perf_authority.insert(
            (100, 47),
            PerfAuthority {
                row_count: 3,
                changed_rows: 3,
                unit_count: 3,
                changed_unit_count: 3,
                status_flags: 0,
            },
        );
        summary.per_hour_data_authority.insert(
            (100, 53),
            DataAuthority {
                row_count: 4,
                changed_rows: 2,
                status_flags: Some(0),
                data_units: BTreeMap::new(),
            },
        );
        summary.per_hour_perf_authority.insert(
            (100, 53),
            PerfAuthority {
                row_count: 99,
                changed_rows: 88,
                unit_count: 4,
                changed_unit_count: 2,
                status_flags: 0,
            },
        );
        let mut errors = Vec::new();
        validate_scanned_domain_authority(&summary, &json!({}), &[], &mut errors);
        assert!(errors.is_empty(), "{errors:?}");

        summary
            .per_hour_perf_authority
            .get_mut(&(100, 47))
            .unwrap()
            .changed_rows = 2;
        summary
            .per_hour_perf_authority
            .get_mut(&(100, 53))
            .unwrap()
            .unit_count = 3;
        validate_scanned_domain_authority(&summary, &json!({}), &[], &mut errors);
        assert!(errors.iter().any(|error| error.contains("d47 DATA")));
        assert!(errors.iter().any(|error| error.contains("d53 DATA")));
    }

    #[test]
    fn d69_per_hour_authority_exactly_binds_groups_members_bonuses_and_status() {
        let summary = valid_domain69_per_hour_summary();
        let mut errors = Vec::new();
        validate_scanned_domain_authority(&summary, &json!({}), &[], &mut errors);
        assert!(errors.is_empty(), "{errors:?}");

        for (field, expected) in [
            ("row_count", "group count"),
            ("changed_rows", "group count"),
            ("unit_count", "member/bonus-pair counts"),
            ("changed_unit_count", "member/bonus-pair counts"),
        ] {
            let mut summary = valid_domain69_per_hour_summary();
            let perf = summary.per_hour_perf_authority.get_mut(&(100, 69)).unwrap();
            match field {
                "row_count" => perf.row_count = 1,
                "changed_rows" => perf.changed_rows = 1,
                "unit_count" => perf.unit_count = 2,
                "changed_unit_count" => perf.changed_unit_count = 2,
                _ => unreachable!(),
            }
            let mut errors = Vec::new();
            validate_scanned_domain_authority(&summary, &json!({}), &[], &mut errors);
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "field={field}: {errors:?}"
            );
        }

        let mut mismatched_pairs = valid_domain69_per_hour_summary();
        mismatched_pairs
            .per_hour_data_authority
            .get_mut(&(100, 69))
            .unwrap()
            .data_units
            .insert("tech_sharing_groups_dynamic_bonus_rows".to_string(), 2);
        let mut errors = Vec::new();
        validate_scanned_domain_authority(&mismatched_pairs, &json!({}), &[], &mut errors);
        assert!(errors
            .iter()
            .any(|error| error.contains("member count 3 != dynamic-bonus pair count 2")));

        let mut missing_units = valid_domain69_per_hour_summary();
        missing_units
            .per_hour_data_authority
            .get_mut(&(100, 69))
            .unwrap()
            .data_units
            .remove("tech_sharing_groups_members");
        let mut errors = Vec::new();
        validate_scanned_domain_authority(&missing_units, &json!({}), &[], &mut errors);
        assert!(errors.iter().any(|error| {
            error.contains("member count 0 != dynamic-bonus pair count 3")
                || error.contains("member/bonus-pair counts 0/3")
        }));

        let mut mismatched_status = valid_domain69_per_hour_summary();
        mismatched_status
            .per_hour_data_authority
            .get_mut(&(100, 69))
            .unwrap()
            .status_flags = Some(0);
        let mut errors = Vec::new();
        validate_scanned_domain_authority(&mismatched_status, &json!({}), &[], &mut errors);
        assert!(errors
            .iter()
            .any(|error| error.contains("domain 69 DATA status")));
    }

    #[test]
    fn production_roster_comes_from_compact_contract_registry() {
        let roster = compact_contract::production_profile_domains();
        assert!(!roster.is_empty());
        for descriptor in roster {
            let contract = contract_for_descriptor(descriptor);
            assert_eq!(contract.id, descriptor.id);
            assert_eq!(contract.name, descriptor.wire_name);
            assert_eq!(contract.requires_data, descriptor.requires_data_record);
        }
    }

    #[test]
    fn every_production_domain_routes_to_a_canonical_structural_decoder() {
        for descriptor in compact_contract::production_profile_domains() {
            let result = recorder_host::stream_fold::validate_domain_data_payload(
                descriptor.id,
                &[],
                0,
            );
            assert!(
                !matches!(result, Err(ref error) if error.contains("no exact structural decoder")),
                "domain {} ({}): {result:?}",
                descriptor.id,
                descriptor.wire_name
            );
        }
    }

    #[test]
    fn every_production_contract_routes_to_shared_data_authority() {
        for descriptor in compact_contract::production_profile_domains() {
            let result = recorder_host::domain_data_authority::summarize_domain_data_payload(
                descriptor.id,
                &[],
                recorder_host::domain_data_authority::DomainDataContext::default(),
            );
            assert!(
                !matches!(result, Err(ref error) if error.errors.iter().any(|message| message.contains("unsupported domain data payload"))),
                "domain {} ({}): {result:?}",
                descriptor.id,
                descriptor.wire_name
            );
        }
    }

    #[test]
    fn contract_knows_domain_42_shared_decoder() {
        let contract = contract_by_name("air_combat_losses_live").unwrap();
        assert_eq!(contract.id, 42);
        assert!(contract.requires_data);
        assert!(contract.shared_recorder_known);
    }

    fn append_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) {
        append_record_with_flags(
            out,
            sequence,
            game_hour_raw,
            domain_id,
            record_type,
            0,
            payload,
        );
    }

    fn replay_test_content_manifest(
        product_modset_sha256: &str,
    ) -> portable_content_manifest::ContentManifest {
        let mut manifest = portable_content_manifest::ContentManifest {
            schema: portable_content_manifest::SCHEMA.into(),
            product_modset_sha256: product_modset_sha256.into(),
            source_game: portable_content_manifest::SourceGameIdentity {
                version: "fixture-1.19.2".into(),
                build: "fixture-a729".into(),
                executable_sha256: "a".repeat(64),
                build_profile: "compact-gate-fixture".into(),
            },
            source_profile: portable_content_manifest::SourceProfileIdentity {
                profile_id: "fixture-profile".into(),
                playset_id: "fixture-playset".into(),
                selection_sha256: "0".repeat(64),
            },
            effective_vfs_manifest_sha256: "b".repeat(64),
            ordered_layers: Vec::new(),
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            required_content: Vec::new(),
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 =
            manifest.computed_selection_sha256().unwrap();
        manifest.with_computed_digest().unwrap()
    }

    fn replay_test_manifest(game_id: &str, evidence_root: &Path) -> Value {
        let static_artifacts = compact_contract::REQUIRED_STATIC_ARTIFACTS
            .iter()
            .map(|name| {
                let bytes = replay_test_static_artifact_bytes(name);
                json!({
                    "name": name,
                    "sha256": Sha256::hex(&bytes),
                })
            })
            .collect::<Vec<_>>();
        let static_bundle = replay_test_static_bundle(evidence_root);
        let product_modset_sha256 = static_bundle["product_modset_sha256"].clone();
        let portable_content_manifest = replay_test_content_manifest(
            product_modset_sha256
                .as_str()
                .expect("fixture product-modset key"),
        );
        json!({
            "schema": "native_collector_session_v1",
            "replay_game_id": game_id,
            "replay_file": "native_session.battleplan",
            "raw_stream": "collector_stream.bin",
            "replay_packaging": {
                "static_artifacts": static_artifacts,
            },
            "production_profile": {
                "enabled": true,
                "required_static_artifacts": compact_contract::REQUIRED_STATIC_ARTIFACTS,
                "missing_required_static_artifacts": [],
            },
            "static_bundle": static_bundle,
            "portable_content_manifest": portable_content_manifest,
            "replay_entries": {
                "stream": "collector_raw/collector_stream.bin",
                "stream_role": "collector_compact_stream",
                "session_manifest": "recorder/native_session_manifest.json",
                "session_manifest_role": "recorder_manifest"
            }
        })
    }

    fn replay_test_static_artifact_value(name: &str) -> Value {
        let mut value = json!({
            "schema": format!("fixture.{name}.static_artifact.v1"),
            "fixture": "compact_gate_static_authority",
            "name": name,
        });
        let self_hash = canonical_json_sha256(&value).unwrap();
        value["artifact_sha256"] = json!(self_hash);
        value
    }

    fn replay_test_static_artifact_bytes(name: &str) -> Vec<u8> {
        serde_json::to_vec(&replay_test_static_artifact_value(name)).unwrap()
    }

    fn replay_test_static_bundle(evidence_root: &Path) -> Value {
        let bundle_dir = evidence_root.join("retained_static_bundle");
        let artifacts_dir = bundle_dir.join("artifacts");
        let proofs_dir = bundle_dir.join("proofs");
        let embedded_bindings_dir = bundle_dir.join("bindings");
        let external_dir = evidence_root.join("retained_static_inputs");
        fs::create_dir_all(&artifacts_dir).unwrap();
        fs::create_dir_all(&proofs_dir).unwrap();
        fs::create_dir_all(&embedded_bindings_dir).unwrap();
        fs::create_dir_all(&external_dir).unwrap();

        let mut retained_artifacts = compact_contract::REQUIRED_STATIC_ARTIFACTS
            .iter()
            .map(|role| {
                let artifact = replay_test_static_artifact_value(role);
                let artifact_bytes = serde_json::to_vec(&artifact).unwrap();
                let artifact_file = required_static_bundle_file(role).unwrap();
                fs::write(bundle_dir.join(artifact_file), &artifact_bytes).unwrap();
                let artifact_sha256 = artifact["artifact_sha256"].as_str().unwrap();
                let report = json!({
                    "schema": format!("fixture.{role}.static_gate_report.v1"),
                    "passed": true,
                    "artifact_sha256": artifact_sha256,
                });
                let report_bytes = serde_json::to_vec_pretty(&report).unwrap();
                let report_file = required_static_gate_report_file(role).unwrap();
                fs::write(bundle_dir.join(&report_file), &report_bytes).unwrap();
                json!({
                    "role": role,
                    "file": artifact_file,
                    "bytes": artifact_bytes.len(),
                    "sha256": Sha256::hex(&artifact_bytes),
                    "artifact_sha256": artifact_sha256,
                    "schema": artifact["schema"],
                    "source_gate_report": report_file,
                    "source_gate_report_sha256": Sha256::hex(&report_bytes),
                })
            })
            .collect::<Vec<_>>();
        retained_artifacts.sort_by(|left, right| {
            left["role"]
                .as_str()
                .unwrap()
                .cmp(right["role"].as_str().unwrap())
        });

        let executable_bytes = b"fixture-hoi4-executable".to_vec();
        let executable_path = external_dir.join("hoi4.exe");
        fs::write(&executable_path, &executable_bytes).unwrap();
        let mut retained_state_bindings = serde_json::Map::new();
        let mut session_binding_evidence = serde_json::Map::new();
        session_binding_evidence.insert(
            "executable".to_string(),
            json!({
                "path": fs::canonicalize(&executable_path).unwrap().display().to_string(),
                "bytes": executable_bytes.len(),
                "sha256": Sha256::hex(&executable_bytes),
            }),
        );
        for name in ["mod_state", "dlc_state", "load_order"] {
            let document = json!({
                "schema": format!("fixture.{name}.v1"),
                "identity": {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
                "entries": [],
            });
            let bytes = serde_json::to_vec_pretty(&document).unwrap();
            let path = external_dir.join(format!("{name}.json"));
            fs::write(&path, &bytes).unwrap();
            let bytes_sha256 = Sha256::hex(&bytes);
            retained_state_bindings.insert(
                name.to_string(),
                json!({
                    "role": name,
                    "sha256": canonical_json_sha256(&document).unwrap(),
                    "bytes_sha256": bytes_sha256,
                    "effective_content_sha256": match name {
                        "mod_state" => "1111111111111111111111111111111111111111111111111111111111111111",
                        "dlc_state" => "2222222222222222222222222222222222222222222222222222222222222222",
                        _ => "3333333333333333333333333333333333333333333333333333333333333333",
                    },
                }),
            );
            session_binding_evidence.insert(
                name.to_string(),
                json!({
                    "path": fs::canonicalize(&path).unwrap().display().to_string(),
                    "bytes": bytes.len(),
                    "sha256": bytes_sha256,
                }),
            );
        }
        let source_inventory = json!({
            "schema": "fixture.source_inventory.v1",
            "identity": {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
            "rows": [],
        });
        let source_inventory_bytes = serde_json::to_vec_pretty(&source_inventory).unwrap();
        let source_inventory_path = external_dir.join("source_inventory.json");
        fs::write(&source_inventory_path, &source_inventory_bytes).unwrap();
        fs::write(
            embedded_bindings_dir.join("source_inventory.json"),
            &source_inventory_bytes,
        )
        .unwrap();
        let source_inventory_sha256 = Sha256::hex(&source_inventory_bytes);
        session_binding_evidence.insert(
            "source_inventory".to_string(),
            json!({
                "path": fs::canonicalize(&source_inventory_path).unwrap().display().to_string(),
                "bytes": source_inventory_bytes.len(),
                "sha256": source_inventory_sha256,
            }),
        );
        let live_resolution = serde_json::to_vec_pretty(&json!({
            "schema": "fixture.live_resolution.v1",
            "passed": true,
        }))
        .unwrap();
        let live_resolution_path = external_dir.join("live_resolution_report.json");
        fs::write(&live_resolution_path, &live_resolution).unwrap();
        session_binding_evidence.insert(
            "live_resolution_report".to_string(),
            json!({
                "path": fs::canonicalize(&live_resolution_path).unwrap().display().to_string(),
                "bytes": live_resolution.len(),
                "sha256": Sha256::hex(&live_resolution),
            }),
        );

        let catalog_source_join = serde_json::to_vec_pretty(&json!({
            "schema": "fixture.catalog_source_join.v1",
            "passed": true,
        }))
        .unwrap();
        fs::write(
            proofs_dir.join("catalog_source_join.json"),
            &catalog_source_join,
        )
        .unwrap();
        let token_join = serde_json::to_vec_pretty(&json!({
            "schema": "fixture.token_join.v1",
            "passed": true,
            "provenance": {"fixture": true},
        }))
        .unwrap();
        fs::write(bundle_dir.join("token_join_report.json"), &token_join).unwrap();

        let mut retained_bindings = serde_json::Map::new();
        retained_bindings.insert(
            "executable".to_string(),
            json!({
                "name": "hoi4.exe",
                "bytes": executable_bytes.len(),
                "sha256": Sha256::hex(&executable_bytes),
            }),
        );
        retained_bindings.extend(retained_state_bindings);
        let mut product_modset = json!({
            "schema": "extractor.product_modset_key.v1",
            "executable_sha256": Sha256::hex(&executable_bytes),
            "mod_state_effective_content_sha256": "1111111111111111111111111111111111111111111111111111111111111111",
            "dlc_state_effective_content_sha256": "2222222222222222222222222222222222222222222222222222222222222222",
            "load_order_effective_content_sha256": "3333333333333333333333333333333333333333333333333333333333333333",
            "effective_vfs_namespace_sha256": "4444444444444444444444444444444444444444444444444444444444444444",
            "effective_vfs_rows_sha256": "5555555555555555555555555555555555555555555555555555555555555555",
        });
        let product_modset_sha256 = canonical_json_sha256(&product_modset).unwrap();
        product_modset["product_modset_sha256"] = json!(product_modset_sha256);
        retained_bindings.insert("product_modset".to_string(), product_modset);
        retained_bindings.insert(
            "catalog_source_occurrences".to_string(),
            json!({
                "rows": retained_artifacts.len(),
                "sha256": canonical_json_sha256(&Value::Array(retained_artifacts.clone())).unwrap(),
            }),
        );
        retained_bindings.insert(
            "live_source_inventory".to_string(),
            json!({
                "file": "bindings/source_inventory.json",
                "bytes": source_inventory_bytes.len(),
                "file_sha256": source_inventory_sha256,
                "namespace_sha256": "4444444444444444444444444444444444444444444444444444444444444444",
                "rows_sha256": "5555555555555555555555555555555555555555555555555555555555555555",
            }),
        );
        let mut retained_manifest = json!({
            "schema": "extractor.static_bundle.v3",
            "artifacts": retained_artifacts,
            "bindings": retained_bindings,
            "catalog_source_join": {
                "file": "proofs/catalog_source_join.json",
                "schema": "fixture.catalog_source_join.v1",
                "sha256": Sha256::hex(&catalog_source_join),
            },
            "token_join": {
                "status": "passed",
                "report": "token_join_report.json",
                "sha256": Sha256::hex(&token_join),
                "provenance": {"fixture": true},
            },
        });
        let bundle_id = canonical_json_sha256(&retained_manifest).unwrap();
        retained_manifest["bundle_id"] = json!(bundle_id);
        let retained_manifest_bytes = serde_json::to_vec_pretty(&retained_manifest).unwrap();
        let retained_manifest_path = bundle_dir.join("static_bundle_manifest.json");
        fs::write(&retained_manifest_path, &retained_manifest_bytes).unwrap();

        let verifier_bytes = b"# fixture static bundle verifier\n".to_vec();
        let verifier_path = external_dir.join("static_bundle.py");
        fs::write(&verifier_path, &verifier_bytes).unwrap();
        let session_artifacts = retained_manifest["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                json!({
                    "role": row["role"],
                    "file": row["file"],
                    "sha256": row["sha256"],
                    "artifact_sha256": row["artifact_sha256"],
                    "source_gate_report_sha256": row["source_gate_report_sha256"],
                })
            })
            .collect::<Vec<_>>();
        json!({
            "admission": "passed",
            "bundle_id": bundle_id,
            "product_modset_sha256": product_modset_sha256,
            "artifact_count": compact_contract::REQUIRED_STATIC_ARTIFACTS.len(),
            "artifacts": session_artifacts,
            "manifest": {
                "path": fs::canonicalize(&retained_manifest_path).unwrap().display().to_string(),
                "sha256": Sha256::hex(&retained_manifest_bytes),
            },
            "bindings": session_binding_evidence,
            "verifier": {
                "path": fs::canonicalize(&verifier_path).unwrap().display().to_string(),
                "sha256": Sha256::hex(&verifier_bytes),
            },
            "token_gate_required": true,
        })
    }

    fn replay_test_static_artifacts(game_id: &str) -> (Vec<Value>, Vec<snapshot::PackEntry>) {
        let mut descriptors = Vec::new();
        let mut entries = Vec::new();
        for name in compact_contract::REQUIRED_STATIC_ARTIFACTS {
            let bytes = replay_test_static_artifact_bytes(name);
            let relative_path = required_static_artifact_relative_path(name).unwrap();
            descriptors.push(json!({
                "name": name,
                "present": true,
                "role": "static_artifact",
                "path": relative_path.clone(),
                "sha256": Sha256::hex(&bytes),
                "bytes": bytes.len(),
                "provenance": {"fixture": true},
            }));
            entries.push(snapshot::PackEntry {
                path: format!("payload/{game_id}/{relative_path}"),
                bytes,
                role: "static_artifact",
                hour_index: None,
            });
        }
        (descriptors, entries)
    }

    fn write_replay_fixture(
        dir: &Path,
        game_id: &str,
        manifest_bytes: &[u8],
        include_manifest_role: bool,
    ) {
        write_replay_fixture_with_static_mutation(
            dir,
            game_id,
            manifest_bytes,
            include_manifest_role,
            |_, _| {},
        );
    }

    fn write_replay_fixture_with_static_mutation<F>(
        dir: &Path,
        game_id: &str,
        manifest_bytes: &[u8],
        include_manifest_role: bool,
        mutate: F,
    ) where
        F: FnOnce(&mut Vec<Value>, &mut Vec<snapshot::PackEntry>),
    {
        let (mut static_artifacts, mut static_entries) = replay_test_static_artifacts(game_id);
        mutate(&mut static_artifacts, &mut static_entries);
        let index = serde_json::to_vec(&json!({
            "game_id": game_id,
            "schema_version": "1",
            "snapshots": [],
            "static_artifacts": static_artifacts,
        }))
        .unwrap();
        let mut entries = vec![
            snapshot::PackEntry {
                path: format!("payload/{game_id}/index.json"),
                bytes: index,
                role: "index",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game_id}/collector_raw/collector_stream.bin"),
                bytes: b"stream".to_vec(),
                role: "collector_compact_stream",
                hour_index: None,
            },
        ];
        entries.push(snapshot::PackEntry {
            path: format!("payload/{game_id}/recorder/native_session_manifest.json"),
            bytes: manifest_bytes.to_vec(),
            role: if include_manifest_role {
                "recorder_manifest"
            } else {
                "other"
            },
            hour_index: None,
        });
        entries.append(&mut static_entries);
        let session_manifest: Value = serde_json::from_slice(manifest_bytes).unwrap();
        let product_modset_sha256 = session_manifest
            .pointer("/portable_content_manifest/product_modset_sha256")
            .and_then(Value::as_str)
            .expect("fixture recorder manifest product-modset key");
        let packed = snapshot::pack(
            game_id,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some("native_collector_session_v1".to_string()),
                content_manifest: Some(replay_test_content_manifest(product_modset_sha256)),
                ..snapshot::PackIdentity::default()
            },
            false,
        )
        .unwrap();
        fs::write(dir.join("native_session.battleplan"), packed).unwrap();
    }

    fn static_authority_error<M, F>(label: &str, mutate_manifest: M, mutate: F) -> String
    where
        M: FnOnce(&mut Value),
        F: FnOnce(&mut Vec<Value>, &mut Vec<snapshot::PackEntry>),
    {
        let dir = TmpDir::new(label);
        let mut manifest = replay_test_manifest("g", &dir.0);
        mutate_manifest(&mut manifest);
        let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture_with_static_mutation(&dir.0, "g", &bytes, true, mutate);
        load_verified_replay_authority(&manifest, &bytes, &dir.0)
            .expect_err("forged static authority must fail closed")
    }

    fn retained_static_evidence_drift_error<F>(label: &str, mutate: F) -> String
    where
        F: FnOnce(&Value),
    {
        let dir = TmpDir::new(label);
        let manifest = replay_test_manifest("g", &dir.0);
        mutate(&manifest);
        let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &bytes, true);
        load_verified_replay_authority(&manifest, &bytes, &dir.0)
            .expect_err("retained static evidence drift must fail closed")
    }

    fn append_record_with_flags(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) {
        out.extend_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&record_flags.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn perf_payload_with_counts(
        row_count: u32,
        changed_rows: u32,
        unit_count: u32,
        changed_unit_count: u32,
        status_flags: u32,
    ) -> Vec<u8> {
        let mut payload = vec![0; 40];
        payload[16..20].copy_from_slice(&row_count.to_le_bytes());
        payload[20..24].copy_from_slice(&changed_rows.to_le_bytes());
        payload[24..28].copy_from_slice(&unit_count.to_le_bytes());
        payload[28..32].copy_from_slice(&changed_unit_count.to_le_bytes());
        payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn perf_payload(status_flags: u32) -> Vec<u8> {
        perf_payload_with_counts(0, 0, 0, 0, status_flags)
    }

    fn perf_payload_with_empty_trailer(status_flags: u32) -> Vec<u8> {
        let mut payload = perf_payload(status_flags);
        payload.extend_from_slice(b"PDU1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    fn perf_payload_with_unit(name: &str, value: u64) -> Vec<u8> {
        let mut payload = perf_payload(0);
        payload.extend_from_slice(b"PDU1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&value.to_le_bytes());
        payload.extend_from_slice(name.as_bytes());
        payload
    }

    fn fuel_empty_payload() -> Vec<u8> {
        let mut payload = vec![0; 24];
        payload[20..24].copy_from_slice(&32768u32.to_le_bytes());
        payload
    }

    fn one_domain_stream_with_frame_payload(frame_payload: &[u8]) -> Vec<u8> {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut raw,
            2,
            100,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            frame_payload,
        );
        raw
    }

    fn frame_payload(status_flags: u32) -> Vec<u8> {
        frame_payload_with_fields(1, 1, status_flags, 0)
    }

    fn frame_payload_with_fields(
        emitted_domains: u32,
        scheduled_domains: u32,
        status_flags: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = vec![0; 32];
        payload[16..20].copy_from_slice(&emitted_domains.to_le_bytes());
        payload[20..24].copy_from_slice(&scheduled_domains.to_le_bytes());
        payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
        payload[28..32].copy_from_slice(&reserved.to_le_bytes());
        payload
    }

    fn frd_v2_empty_payload() -> Vec<u8> {
        use recorder_host::domain_data_authority as authority;

        let mut payload = vec![0; 56];
        payload[0..4].copy_from_slice(b"FRD2");
        payload[4..6].copy_from_slice(&2u16.to_le_bytes());
        payload[6..8].copy_from_slice(&56u16.to_le_bytes());
        payload[8..12].copy_from_slice(&authority::COMPACT_FRD_STATUS_KNOWN_MASK.to_le_bytes());
        payload[48..52].copy_from_slice(&3u32.to_le_bytes());
        for (kind, flags, source, evidence, detail) in [
            (
                authority::COMPACT_FRD_BADGE_KIND_FOCUS_PAUSE_FAIL_CLOSED,
                authority::COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED
                    | authority::COMPACT_FRD_BADGE_FLAG_CURRENT_BUILD_PROBED,
                12u32,
                authority::COMPACT_FRD_BADGE_EVIDENCE_NO_PAUSED_TRUE_GROUND_TRUTH,
                authority::COMPACT_FRD_BADGE_DETAIL_FOCUS_PAUSE_FAIL_CLOSED,
            ),
            (
                authority::COMPACT_FRD_BADGE_KIND_DECISION_TARGET_FIELDS_FAIL_CLOSED,
                authority::COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED
                    | authority::COMPACT_FRD_BADGE_FLAG_CURRENT_BUILD_PROBED,
                16u32,
                authority::COMPACT_FRD_BADGE_EVIDENCE_TIMED_DEFINITION_LAYOUT_REJECTED,
                authority::COMPACT_FRD_BADGE_DETAIL_DECISION_TARGET_FIELDS_FAIL_CLOSED,
            ),
            (
                authority::COMPACT_FRD_BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE,
                authority::COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED
                    | authority::COMPACT_FRD_BADGE_FLAG_DERIVED_DETERMINISTIC,
                1000u32,
                authority::COMPACT_FRD_BADGE_EVIDENCE_COMPACT_FRAME_DELTA_DERIVED,
                authority::COMPACT_FRD_BADGE_DETAIL_DIRECT_EVENT_HOOKS_UNAVAILABLE,
            ),
        ] {
            payload.extend_from_slice(&kind.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&source.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&evidence.to_le_bytes());
            payload.extend_from_slice(&(detail.len() as u16).to_le_bytes());
            payload.extend_from_slice(detail.as_bytes());
        }
        payload
    }
}
