use compact_contract::campaign_identity::ValidatedCid1;
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode, StreamValidationError,
};
use compact_contract::{
    compact_status_for_global_interpretation, parse_compact_record_header,
    COMPACT_STATUS_FAILURE_MASK,
};
use serde_json::{json, Value};
use snapshot::sha256::Sha256;
use snapshot::{Container, ExpectedIdentity};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::Component;
use std::path::{Path, PathBuf};

const RECORDER_MANIFEST_SCHEMA: &str = recorder_host::SESSION_MANIFEST_SCHEMA;

const REPLAY_CONTAINER_SCHEMA_VERSION: &str = snapshot::CONTAINER_SCHEMA_VERSION;

const RECORD_VERSION: u16 = 1;
const RECORD_HEADER_LEN: usize = 32;
const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
const RC_PROBE_DOMAIN_ID: u16 = 250;
const RECORD_TYPE_FRAME_END: u16 = 2;
const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
const RECORD_TYPE_DOMAIN_GAP: u16 = compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP;
#[cfg(test)]
const DOMAIN_FRAME: u16 = 1000;

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
    pub gap_acceptance: GapAcceptanceProfile,
    pub write_report: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapAcceptanceProfile {
    TypedGapsAllowed,
    ZeroLossRequired,
}

impl GapAcceptanceProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TypedGapsAllowed => "typed_gaps_allowed",
            Self::ZeroLossRequired => "require_zero_loss",
        }
    }
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
            gap_acceptance: GapAcceptanceProfile::TypedGapsAllowed,
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
    zero_units: &'static [&'static str],
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
            "domain47_adapter_active_ruler_rows",
            "domain47_adapter_unit_leader_trait_rows",
        ],
        shared_recorder_known: true,
    },
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
            "gap_acceptance": options.gap_acceptance.as_str(),
            "typed_h4dg_is_format_valid": true,
            "zero_loss_claimed": options.gap_acceptance == GapAcceptanceProfile::ZeroLossRequired,
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
            options.max_stream_bytes,
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

    require_str_eq(&manifest, "schema", RECORDER_MANIFEST_SCHEMA, &mut errors);
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
            errors.push(format!(
                "required compact domain {domain:?} is not assigned"
            ));
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
    let expected_contracts = compact_contract::emitted_domain_ids_for_scheduled_ids(&scheduled_ids)
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

    validate_hourly_manifest(
        &manifest,
        min_game_hours,
        options.gap_acceptance,
        &mut errors,
    );
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
            bind_run_stream_to_certification(
                &authority.stream_certification,
                artifact,
                &mut errors,
            );
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

fn validate_hourly_manifest(
    manifest: &Value,
    min_game_hours: i64,
    gap_acceptance: GapAcceptanceProfile,
    errors: &mut Vec<String>,
) {
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
    let skipped_hours = manifest
        .get("skipped_game_hours")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    if skipped_hours < 0 {
        errors.push(format!(
            "skipped_game_hours is {:?}, expected a nonnegative integer",
            manifest.get("skipped_game_hours")
        ));
    } else if gap_acceptance == GapAcceptanceProfile::ZeroLossRequired && skipped_hours != 0 {
        errors.push(format!(
            "skipped_game_hours is {skipped_hours}, but the explicit zero-loss profile requires 0"
        ));
    }
    let unique_hours = manifest
        .get("unique_game_hour_count")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    if advanced >= 0 && unique_hours >= 0 {
        match advanced.checked_add(1) {
            Some(inclusive)
                if unique_hours
                    .checked_add(skipped_hours)
                    .is_some_and(|observed_span| observed_span == inclusive) => {}
            Some(inclusive) => errors.push(format!(
                "unique_game_hour_count {unique_hours} + skipped_game_hours {skipped_hours} != game_hours_advanced + 1 ({inclusive})"
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
    let perf_count = compact
        .get("domain_perf_record_count")
        .and_then(Value::as_u64);
    let gap_count = compact
        .get("domain_gap_record_count")
        .and_then(Value::as_u64);
    match perf_count.zip(gap_count) {
        Some((perf, gaps))
            if perf
                .checked_add(gaps)
                .is_some_and(|terminals| {
                    terminals == frame_count.saturating_mul(expected_count)
                }) => {}
        Some((perf, gaps)) => errors.push(format!(
            "compact_stream DOMAIN_PERF {perf} + H4DG {gaps} does not equal exact scheduled terminal cardinality {}",
            frame_count.saturating_mul(expected_count)
        )),
        None => errors.push(
            "compact_stream.domain_perf_record_count and domain_gap_record_count must both be unsigned integers"
                .to_string(),
        ),
    }
    if options.gap_acceptance == GapAcceptanceProfile::ZeroLossRequired && gap_count != Some(0) {
        errors.push(format!(
            "compact_stream.domain_gap_record_count is {gap_count:?}, but the explicit zero-loss profile requires 0"
        ));
    }
    let data_count = compact
        .get("domain_data_record_count")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let maximum_data_count = frame_count.saturating_mul(
        expected_contracts
            .iter()
            .filter(|contract| contract.requires_data)
            .count() as u64,
    );
    if data_count > maximum_data_count {
        errors.push(format!(
            "compact_stream.domain_data_record_count {data_count} exceeds maximum accepted-frame cardinality {maximum_data_count}"
        ));
    }

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
    let record_count = domain
        .get("record_count")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    if record_count > frame_count {
        errors.push(format!(
            "domain {} record_count {record_count} exceeds frame_end_record_count {frame_count}",
            contract.name
        ));
    }
    if contract.requires_data {
        require_u64_eq(domain, "data_record_count", record_count, errors);
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
    if read_count != record_count {
        errors.push(format!(
            "domain {} read_time_us.count {read_count} != accepted DOMAIN_PERF record_count {record_count}",
            contract.name
        ));
    }
    let units = domain
        .get("data_units_total")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if options.strict_semantic_units && record_count != 0 {
        for key in contract.required_units {
            if !units.contains_key(*key) {
                errors.push(format!(
                    "domain {} data_units_total is missing required counter {}",
                    contract.name, key
                ));
            }
        }
    }
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
    if !contract.work_implies_rows_units.is_empty() {
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
    if options.gap_acceptance == GapAcceptanceProfile::ZeroLossRequired
        && summary.domain_gap_record_count != 0
    {
        errors.push(format!(
            "explicit zero-loss profile rejected {} independently decoded H4DG record(s)",
            summary.domain_gap_record_count
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
        "domain_gap_record_count",
        summary.domain_gap_record_count,
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
    validate_scanned_domains_seen(&summary, compact, errors);
    validate_scanned_domain_authority(&summary, compact, expected_contracts, errors);
    validate_domain57_stream_authority(&summary, compact, errors);
    validate_campaign_identity_stream_authority(&summary, manifest, identity_mode, errors);
    validate_freeze_census_stream_authority(&summary, raw, manifest, errors);

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

fn validate_scanned_domains_seen(
    summary: &HeaderScanSummary,
    compact: &Value,
    errors: &mut Vec<String>,
) {
    let scanned_domains_seen = summary
        .domains_seen
        .iter()
        .map(|domain_id| {
            if *domain_id == compact_contract::COMPACT_DOMAIN_FRAME {
                "frame".to_string()
            } else {
                compact_contract::assigned_descriptor_by_id(*domain_id)
                    .map(|descriptor| descriptor.wire_name.to_string())
                    .unwrap_or_else(|| format!("unknown_domain_{domain_id}"))
            }
        })
        .collect::<BTreeSet<_>>();
    let Some(manifest_rows) = compact.get("domains_seen").and_then(Value::as_array) else {
        errors.push("manifest compact_stream.domains_seen is not an array".to_string());
        return;
    };
    let mut manifest_domains_seen = BTreeSet::new();
    for (index, row) in manifest_rows.iter().enumerate() {
        let Some(name) = row.as_str() else {
            errors.push(format!(
                "manifest compact_stream.domains_seen[{index}] is not a string"
            ));
            continue;
        };
        if !manifest_domains_seen.insert(name.to_string()) {
            errors.push(format!(
                "manifest compact_stream.domains_seen contains duplicate {name:?}"
            ));
        }
    }
    if manifest_domains_seen != scanned_domains_seen {
        errors.push(format!(
            "independent stream domains_seen {scanned_domains_seen:?} != manifest compact_stream.domains_seen {manifest_domains_seen:?}"
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
            if summary.per_domain_perf_authority.contains_key(&contract.id) {
                errors.push(format!(
                    "domain {} has independently scanned accepted PERF authority but is missing from manifest compact_stream.per_domain",
                    contract.name
                ));
            }
            continue;
        };
        let scanned_perf_count = summary
            .per_hour_perf_domain_counts
            .values()
            .map(|domains| domains.get(&contract.id).copied().unwrap_or(0))
            .sum::<u64>();
        let scanned_data_count = summary
            .per_hour_data_domain_counts
            .values()
            .map(|domains| domains.get(&contract.id).copied().unwrap_or(0))
            .sum::<u64>();
        compare_manifest_authority_field(
            contract.name,
            "PERF cardinality",
            "record_count",
            scanned_perf_count,
            manifest_domain,
            errors,
        );
        compare_manifest_authority_field(
            contract.name,
            "DATA cardinality",
            "data_record_count",
            scanned_data_count,
            manifest_domain,
            errors,
        );
        match manifest_domain
            .get("read_time_us")
            .and_then(|value| value.get("count"))
            .and_then(Value::as_u64)
        {
            Some(declared) if declared == scanned_perf_count => {}
            Some(declared) => errors.push(format!(
                "domain {} independently scanned PERF timing cardinality {scanned_perf_count} != manifest read_time_us.count {declared}",
                contract.name
            )),
            None => errors.push(format!(
                "domain {} manifest read_time_us.count is missing or not an unsigned integer",
                contract.name
            )),
        }
        if scanned_perf_count != 0 {
            let scanned_units = summary
                .per_domain_combined_units
                .get(&contract.id)
                .cloned()
                .unwrap_or_default();
            for required in contract.required_units {
                if !scanned_units.contains_key(*required) {
                    errors.push(format!(
                        "domain {} independently scanned accepted outcomes are missing required semantic counter {}",
                        contract.name, required
                    ));
                }
            }
        }
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
    let accepted_hours = summary
        .per_hour_perf_domain_counts
        .iter()
        .filter_map(|(hour, domains)| (domains.get(&57) == Some(&1)).then_some(*hour))
        .collect::<BTreeSet<_>>();
    let exact_one_record_per_accepted_frame = records_by_hour.len() == accepted_hours.len()
        && accepted_hours
            .iter()
            .all(|hour| records_by_hour.get(hour) == Some(&1))
        && records_by_hour
            .keys()
            .all(|hour| accepted_hours.contains(hour));
    if !exact_one_record_per_accepted_frame {
        errors.push(
            "independent d57 authority does not have exactly one record per accepted PERF frame"
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
    let scanned_skipped = i64::from(summary.skipped_hours);
    let manifest_skipped = manifest
        .get("skipped_game_hours")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    if scanned_skipped != manifest_skipped {
        errors.push(format!(
            "independent stream skipped hour count {scanned_skipped} != manifest skipped_game_hours {manifest_skipped}"
        ));
    }
    if !matches!(
        scanned_unique.checked_add(scanned_skipped),
        Some(covered_span) if covered_span == scanned_span
    ) {
        errors.push(format!(
            "independent stream unique hours {scanned_unique} + typed discontinuity hours {scanned_skipped} != inclusive span {scanned_span}"
        ));
    }
    if scanned_span != manifest_span {
        errors.push(format!(
            "independent stream inclusive hour span {scanned_span} != manifest game_hours_advanced + 1 ({manifest_span})"
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
    stream_certification: StreamCertification,
    hour_coverage: recorder_host::hour_coverage::HourCoverageMap,
    details: Value,
}

#[derive(Debug)]
enum StreamCertification {
    ByteComparedAgainstPackagedStream {
        container_stream: Vec<u8>,
        container_stream_sha256: String,
    },
    ManifestAttestedThroughDeltaCache {
        attested_source_sha256: String,
        attested_source_bytes: u64,
    },
}

const STREAM_CERTIFICATION_BYTE_COMPARED: &str = "byte_compared_against_packaged_stream";
const STREAM_CERTIFICATION_MANIFEST_ATTESTED: &str = "manifest_attested_through_delta_cache";

impl StreamCertification {
    fn path_name(&self) -> &'static str {
        match self {
            Self::ByteComparedAgainstPackagedStream { .. } => STREAM_CERTIFICATION_BYTE_COMPARED,
            Self::ManifestAttestedThroughDeltaCache { .. } => {
                STREAM_CERTIFICATION_MANIFEST_ATTESTED
            }
        }
    }
}

const CACHE_ONLY_LINK_COMPOSITION: &str = "container_composition";
const CACHE_ONLY_LINK_NO_PACKAGED_STREAM: &str = "no_packaged_stream_entry";
const CACHE_ONLY_LINK_SOURCE_READABLE: &str = "run_dir_source_readable";
const CACHE_ONLY_LINK_STREAM_STORAGE: &str = "stream_storage_identity";
const CACHE_ONLY_LINK_HOUR_INDEX: &str = "h4ix_source_identity";
const CACHE_ONLY_LINK_CHECKPOINT_CACHE: &str = "h4cp_source_digest";
const CACHE_ONLY_LINK_SEALED_ARTIFACTS: &str = "sealed_artifact_receipt";

fn cache_only_link_refusal(link: &str, detail: impl AsRef<str>) -> String {
    format!(
        "cache-only replay certification failed at attestation link {link:?}: {}",
        detail.as_ref()
    )
}

const CACHE_ONLY_SOURCE_ABSENT_OUTCOME: &str = "run_dir_source_stream_absent";

fn cache_only_source_absent(path: &Path) -> String {
    format!(
        "cache-only replay certification is not applicable here ({CACHE_ONLY_SOURCE_ABSENT_OUTCOME}): \
         the recording declares the {:?} composition, so the raw collector stream lives only in the \
         capture run directory, and {} does not exist. Nothing disagreed — there were no local \
         source bytes to hash, so no attestation link was proved or broken. Run this gate against \
         the capture's own run directory; a sealed container alone cannot be certified by it.",
        snapshot::RawStreamPackaging::Omitted.as_str(),
        path.display(),
    )
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
        compact_contract::STATIC_ARTIFACT_FOCUS_DEFS_STATIC => "artifacts/focus_defs_static.json",
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

struct StaticBundleCopies<'a> {
    sealed: &'a serde_json::Map<String, Value>,
    run_dir: &'a serde_json::Map<String, Value>,
}

impl<'a> StaticBundleCopies<'a> {
    fn entry(bundle: &'a serde_json::Map<String, Value>, label: &str) -> Option<&'a Value> {
        let mut segments = label.split('.');
        let mut current = bundle.get(segments.next()?)?;
        for segment in segments {
            current = current.as_object()?.get(segment)?;
        }
        Some(current)
    }

    fn sealed_entry(&self, label: &str) -> Option<&'a Value> {
        Self::entry(self.sealed, label)
    }

    fn run_dir_entry(&self, label: &str) -> Option<&'a Value> {
        Self::entry(self.run_dir, label)
    }
}

fn verify_retained_file_evidence(
    bundles: &StaticBundleCopies<'_>,
    label: &str,
    require_bytes: bool,
) -> Result<RetainedFileEvidence, String> {
    let evidence = bundles
        .sealed_entry(label)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("manifest static_bundle {label} is missing or not an object"))?;
    let sealed_path = evidence
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| format!("manifest static_bundle {label}.path is missing or empty"))?;
    let located = bundles
        .run_dir_entry(label)
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!(
                "run-dir native_session_manifest.json static_bundle {label} is missing or not an \
                 object; the retained evidence file is located by the run-dir copy, which is the \
                 only copy allowed to carry an absolute path"
            )
        })?;
    let raw_path = located
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            format!("run-dir native_session_manifest.json static_bundle {label}.path is missing or empty")
        })?;
    let redacted = recorder_host::portable_manifest::redact_absolute_path_string(raw_path);
    if redacted == raw_path || !Path::new(raw_path).is_absolute() {
        return Err(format!(
            "run-dir native_session_manifest.json static_bundle {label}.path must be the retained \
             canonical absolute path, got {raw_path:?}"
        ));
    }
    if redacted != sealed_path {
        return Err(format!(
            "run-dir native_session_manifest.json static_bundle {label}.path {raw_path:?} does not \
             bind to the sealed static_bundle {label}.path {sealed_path:?}; the sealed copy must \
             carry exactly the run-dir path's final component"
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
            "production manifest portable_content_manifest is missing or not an object".to_string()
        })?;
    if portable_content.get("schema").and_then(Value::as_str)
        != Some("hoi4.portable-content-manifest.v2")
    {
        return Err(
            "production manifest portable_content_manifest is not v2 product-modset-bound".into(),
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
    run_dir_manifest: &Value,
) -> Result<VerifiedStaticManifestAuthority, String> {
    let artifact_hashes = required_static_manifest_hashes(manifest)?;
    let static_bundle = manifest
        .get("static_bundle")
        .and_then(Value::as_object)
        .expect("static bundle shape checked above");
    let run_dir_static_bundle = run_dir_manifest
        .get("static_bundle")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "run-dir native_session_manifest.json static_bundle is missing or not an object; it \
             is where the retained evidence files are located"
                .to_string()
        })?;
    let bundles = StaticBundleCopies {
        sealed: static_bundle,
        run_dir: run_dir_static_bundle,
    };
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

    let retained_manifest = verify_retained_file_evidence(&bundles, "manifest", false)?;
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
    if product_modset
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != product_fields
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
    let executable = verify_retained_file_evidence(&bundles, "bindings.executable", true)?;
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
    if product_modset
        .get("executable_sha256")
        .and_then(Value::as_str)
        != Some(executable.sha256.as_str())
    {
        return Err("product modset executable identity differs from retained executable".into());
    }
    for name in ["mod_state", "dlc_state", "load_order"] {
        let retained_file =
            verify_retained_file_evidence(&bundles, &format!("bindings.{name}"), true)?;
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
    let source_inventory =
        verify_retained_file_evidence(&bundles, "bindings.source_inventory", true)?;
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
    verify_retained_file_evidence(&bundles, "bindings.live_resolution_report", true)?;
    let verifier = verify_retained_file_evidence(&bundles, "verifier", false)?;

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
    run_dir_manifest: &Value,
    game_id: &str,
) -> Result<Value, String> {
    let manifest_authority = verify_static_manifest_authority(manifest, run_dir_manifest)?;
    let roles = compact_contract::REQUIRED_STATIC_ARTIFACTS
        .iter()
        .map(|role| (*role).to_string())
        .collect::<BTreeSet<_>>();
    let details = admit_packaged_static_artifacts(
        container,
        game_id,
        &roles,
        &manifest_authority.artifact_hashes,
        Some(&manifest_authority.bundle_artifacts),
    )?;
    Ok(json!({
        "required_count": compact_contract::REQUIRED_STATIC_ARTIFACTS.len(),
        "exact_required_roles": true,
        "bundle_id": manifest_authority.bundle_id,
        "product_modset_sha256": manifest_authority.product_modset_sha256,
        "retained_manifest_sha256": manifest_authority.retained_manifest_sha256,
        "retained_binding_count": manifest_authority.retained_binding_count,
        "verifier_sha256": manifest_authority.verifier_sha256,
        "artifacts": details,
    }))
}

fn admit_packaged_static_artifacts(
    container: &Container,
    game_id: &str,
    roles: &BTreeSet<String>,
    artifact_hashes: &BTreeMap<String, String>,
    retained: Option<&BTreeMap<String, StaticBundleArtifactBinding>>,
) -> Result<Vec<Value>, String> {
    let expected_count = roles.len();
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
    let canonical = compact_contract::REQUIRED_STATIC_ARTIFACTS
        .iter()
        .map(|role| (*role).to_string())
        .collect::<BTreeSet<_>>();
    let required = roles;
    let mut index_bindings = BTreeMap::<String, StaticArtifactIndexBinding>::new();
    let mut relative_paths = BTreeSet::new();
    let mut rostered = BTreeSet::new();
    for (row_index, row) in rows.iter().enumerate() {
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("replay index static_artifacts[{row_index}].name is invalid"))?;
        if !canonical.contains(name) {
            return Err(format!(
                "authoritative replay index contains unexpected static role {name:?}"
            ));
        }
        if !rostered.insert(name.to_string()) {
            return Err(format!(
                "authoritative replay index duplicates static role {name:?}"
            ));
        }
        let present = row.get("present").and_then(Value::as_bool);
        if !required.contains(name) {
            if present != Some(false) {
                return Err(format!(
                    "authoritative replay index static role {name:?} is not packaged by \n                     this recording but is not badged present=false"
                ));
            }
            if row
                .get("absence_reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
                .is_none()
            {
                return Err(format!(
                    "authoritative replay index static role {name:?} is absent but names no reason"
                ));
            }
            continue;
        }
        if present != Some(true) {
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
    if rostered != canonical {
        return Err(format!(
            "authoritative replay index rosters {rostered:?}, not the canonical static \n             roster {canonical:?}: a role missing from the index is a role no reader can \n             learn about"
        ));
    }
    for name in required {
        if !index_bindings.contains_key(name) {
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
    for name in required {
        let name = name.as_str();
        let binding = index_bindings
            .get(name)
            .expect("required index binding checked above");
        let manifest_sha256 = artifact_hashes.get(name).ok_or_else(|| {
            format!("manifest carries no static artifact digest for role {name:?}")
        })?;
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
        let bundle_binding = retained
            .map(|bundle| {
                bundle.get(name).ok_or_else(|| {
                    format!("retained static bundle carries no artifact for role {name:?}")
                })
            })
            .transpose()?;
        if let Some(bundle_binding) = bundle_binding {
            if observed_sha256 != bundle_binding.sha256 {
                return Err(format!(
                    "static role {name:?} admitted bytes sha256 {observed_sha256} != retained bundle artifact sha256 {}",
                    bundle_binding.sha256
                ));
            }
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
            || bundle_binding
                .is_some_and(|bundle| embedded_artifact_sha256 != bundle.artifact_sha256)
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
    Ok(details)
}

enum ContentProofKind {
    LaunchTimeStaticBundleV1,
    AttachTimeEffectiveVfsJoinV1,
}

fn read_content_proof_kind(embedded_manifest: &Value) -> Result<ContentProofKind, String> {
    let field = recorder_host::attach_content_proof::CONTENT_PROOF_KIND_FIELD;
    let declared = embedded_manifest
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "container-authenticated recorder_manifest carries no {field}; a recording states \
                 which content proof it took, and a consumer never infers one from a missing entry"
            )
        })?;
    if declared == recorder_host::attach_content_proof::KIND_LAUNCH_TIME_STATIC_BUNDLE_V1 {
        Ok(ContentProofKind::LaunchTimeStaticBundleV1)
    } else if declared
        == recorder_host::attach_content_proof::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1
    {
        Ok(ContentProofKind::AttachTimeEffectiveVfsJoinV1)
    } else {
        Err(format!(
            "recording declares {field} {declared:?}, which certifies no content: this loader \
             admits {:?} and {:?}",
            recorder_host::attach_content_proof::KIND_LAUNCH_TIME_STATIC_BUNDLE_V1,
            recorder_host::attach_content_proof::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1,
        ))
    }
}

fn certify_attach_time_content_proof(
    container: &Container,
    embedded_manifest: &Value,
    game_id: &str,
) -> Result<Value, String> {
    let field = recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD;
    let proof = embedded_manifest
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!("recording declares the attach-time proof but carries no {field}")
        })?;
    if proof.get("schema").and_then(Value::as_str)
        != Some(recorder_host::attach_content_proof::SCHEMA)
    {
        return Err(format!(
            "{field}.schema is not {:?}",
            recorder_host::attach_content_proof::SCHEMA
        ));
    }
    if proof.get("kind").and_then(Value::as_str)
        != Some(recorder_host::attach_content_proof::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1)
    {
        return Err(format!(
            "{field}.kind disagrees with the declared proof kind"
        ));
    }
    for pre_resume in [
        "early_render_launch_manifest",
        "early_render_capture_receipt",
    ] {
        if embedded_manifest
            .get(pre_resume)
            .is_some_and(|value| !value.is_null())
        {
            return Err(format!(
                "recording declares the attach-time content proof but carries pre-resume evidence \
                 {pre_resume:?}: a recording observed before the engine was resumed takes the \
                 launch-time route, and cannot certify under the attach-time one"
            ));
        }
    }
    if proof.get("window_settled").and_then(Value::as_bool) != Some(true) {
        return Err(format!(
            "{field} reports an unsettled attach window after {} attempt(s): the install changed \
             while the corpus was being fingerprinted, so no observation inside the window \
             describes one install",
            proof
                .get("window_attempts")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ));
    }
    let observed_at = proof
        .get("observed_at_unix")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{field}.observed_at_unix is missing or not an integer"))?;
    let completed_at = proof
        .get("completed_at_unix")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{field}.completed_at_unix is missing or not an integer"))?;
    if completed_at < observed_at {
        return Err(format!("{field} window closes before it opens"));
    }
    let game_process_start_unix = proof
        .get("game_process_start_unix")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            format!(
                "{field}.game_process_start_unix is missing or not an integer: without it the \
                 interval between the engine starting and the attach window opening — which this \
                 proof does not cover — cannot be read off the recording"
            )
        })?;
    if game_process_start_unix > observed_at {
        return Err(format!(
            "{field} reports the engine starting after the attach window opened, so the window is \
             not an observation of a running engine's install"
        ));
    }
    let executable_sha256 = canonical_sha256(
        proof.get("executable_sha256"),
        &format!("{field}.executable_sha256"),
    )?;
    let dlc_load_sha256 = canonical_sha256(
        proof.get("dlc_load_sha256"),
        &format!("{field}.dlc_load_sha256"),
    )?;
    let layer_rows = proof
        .get("layer_stack")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{field}.layer_stack is missing or not an array"))?;
    if layer_rows.is_empty() {
        return Err(format!(
            "{field}.layer_stack is empty: a proof that states no content layers states no world"
        ));
    }
    let mut layers = Vec::with_capacity(layer_rows.len());
    for (index, row) in layer_rows.iter().enumerate() {
        let ordinal = row
            .get("ordinal")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{field}.layer_stack[{index}].ordinal is invalid"))?;
        if ordinal != index as u64 {
            return Err(format!(
                "{field}.layer_stack ordinals are not contiguous from zero, so the load order the \
                 digest covers is not the order the rows are in"
            ));
        }
        let kind = row
            .get("kind")
            .and_then(Value::as_str)
            .and_then(recorder_host::attach_content_proof::layer_kind_from_wire)
            .ok_or_else(|| {
                format!("{field}.layer_stack[{index}].kind is not a known layer kind")
            })?;
        if (index == 0) != (kind == "base") {
            return Err(format!(
                "{field}.layer_stack must begin with ordinal-zero base content and carry no other"
            ));
        }
        let optional_sha256 = |key: &str| -> Result<Option<String>, String> {
            match row.get(key) {
                None | Some(Value::Null) => Ok(None),
                value => canonical_sha256(value, &format!("{field}.layer_stack[{index}].{key}"))
                    .map(Some),
            }
        };
        layers.push(recorder_host::attach_content_proof::ProofLayer {
            ordinal: u32::try_from(ordinal)
                .map_err(|_| format!("{field}.layer_stack[{index}].ordinal is out of range"))?,
            kind,
            layer_id: row
                .get("layer_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("{field}.layer_stack[{index}].layer_id is invalid"))?
                .to_string(),
            descriptor_sha256: optional_sha256("descriptor_sha256")?,
            descriptor_file_sha256: optional_sha256("descriptor_file_sha256")?,
            source_container_sha256: optional_sha256("source_container_sha256")?,
        });
    }
    let recomputed_stack_sha256 = recorder_host::attach_content_proof::layer_stack_sha256(&layers)?;
    let declared_stack_sha256 = canonical_sha256(
        proof.get("layer_stack_sha256"),
        &format!("{field}.layer_stack_sha256"),
    )?;
    if recomputed_stack_sha256 != declared_stack_sha256 {
        return Err(format!(
            "{field}.layer_stack_sha256 {declared_stack_sha256} is not the digest of the layer \
             rows it travels with ({recomputed_stack_sha256})"
        ));
    }

    let canonical = compact_contract::REQUIRED_STATIC_ARTIFACTS
        .iter()
        .map(|role| (*role).to_string())
        .collect::<BTreeSet<_>>();
    let rows = embedded_manifest
        .get("replay_packaging")
        .and_then(|value| value.get("static_artifacts"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "manifest replay_packaging.static_artifacts is missing or not an array".to_string()
        })?;
    let mut declared = BTreeSet::new();
    let mut artifact_hashes = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                format!("manifest replay_packaging.static_artifacts[{index}].name is invalid")
            })?;
        if !canonical.contains(name) {
            return Err(format!(
                "manifest replay_packaging.static_artifacts contains unexpected role {name:?}"
            ));
        }
        if !declared.insert(name.to_string()) {
            return Err(format!(
                "manifest replay_packaging.static_artifacts duplicates role {name:?}"
            ));
        }
        artifact_hashes.insert(
            name.to_string(),
            canonical_sha256(
                row.get("sha256"),
                &format!("manifest static artifact {name:?} sha256"),
            )?,
        );
    }
    let missing_rows = embedded_manifest
        .get("production_profile")
        .and_then(|profile| profile.get("missing_required_static_artifacts"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "manifest production_profile.missing_required_static_artifacts is missing or not an \
             array"
                .to_string()
        })?;
    let mut missing = BTreeSet::new();
    for (index, row) in missing_rows.iter().enumerate() {
        let name = row.as_str().ok_or_else(|| {
            format!(
                "manifest production_profile.missing_required_static_artifacts[{index}] is not a \
                 string"
            )
        })?;
        if !missing.insert(name.to_string()) {
            return Err(format!(
                "manifest production_profile.missing_required_static_artifacts duplicates {name:?}"
            ));
        }
    }
    if !declared.is_disjoint(&missing) {
        return Err(format!(
            "roles {:?} are declared both packaged and missing",
            declared.intersection(&missing).collect::<Vec<_>>()
        ));
    }
    let union = declared.union(&missing).cloned().collect::<BTreeSet<_>>();
    if union != canonical {
        return Err(format!(
            "packaged roles plus missing roles {union:?} != the canonical roster {canonical:?}, so \
             a role could be omitted from the recording without being named anywhere"
        ));
    }

    if declared.is_empty() {
        return Err(format!(
            "{field}: the recording packages no static artifacts at all, so its attach-time proof \
             binds nothing to the captured content"
        ));
    }
    let admitted =
        admit_packaged_static_artifacts(container, game_id, &declared, &artifact_hashes, None)?;

    let refused_rows = proof
        .get("refused_artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{field}.refused_artifacts is missing or not an array"))?;
    let mut refused = BTreeMap::new();
    for row in refused_rows {
        let role = row
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{field}.refused_artifacts row names no role"))?;
        let reason = row
            .get("reason")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{field}.refused_artifacts row for {role:?} names no reason"))?;
        refused.insert(role.to_string(), reason.to_string());
    }
    let join_rows = proof
        .get("artifact_joins")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{field}.artifact_joins is missing or not an array"))?;
    let mut joins = BTreeMap::new();
    for row in join_rows {
        let role = row
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{field}.artifact_joins row names no role"))?;
        if refused.contains_key(role) {
            return Err(format!(
                "{field} both joins and refuses static role {role:?}"
            ));
        }
        let artifact_sha256 = canonical_sha256(
            row.get("artifact_sha256"),
            &format!("{field}.artifact_joins {role:?} artifact_sha256"),
        )?;
        let claimed_paths = row
            .get("claimed_paths")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{field}.artifact_joins {role:?} claimed_paths is invalid"))?;
        if claimed_paths == 0 {
            return Err(format!(
                "{field}.artifact_joins {role:?} joined no paths at all, so it proves nothing about \
                 the captured content"
            ));
        }
        if joins
            .insert(
                role.to_string(),
                (artifact_sha256, claimed_paths, row.clone()),
            )
            .is_some()
        {
            return Err(format!("{field}.artifact_joins duplicates role {role:?}"));
        }
    }
    let mut joined_details = Vec::with_capacity(admitted.len());
    for artifact in &admitted {
        let name = artifact
            .get("name")
            .and_then(Value::as_str)
            .expect("admitted static artifact carries its role");
        let observed_sha256 = artifact
            .get("sha256")
            .and_then(Value::as_str)
            .expect("admitted static artifact carries its digest");
        let Some((artifact_sha256, claimed_paths, row)) = joins.get(name) else {
            return Err(format!(
                "static role {name:?} is packaged but has no attach-time join receipt{}",
                refused
                    .get(name)
                    .map(|reason| format!(": {reason}"))
                    .unwrap_or_default()
            ));
        };
        if artifact_sha256 != observed_sha256 {
            return Err(format!(
                "static role {name:?} was joined as artifact {artifact_sha256}, but the container \
                 admitted {observed_sha256}: the receipt covers a different build of the artifact"
            ));
        }
        joined_details.push(json!({
            "name": name,
            "path": artifact.get("path"),
            "bytes": artifact.get("bytes"),
            "sha256": observed_sha256,
            "claimed_paths": claimed_paths,
            "joined_bytes": row.get("joined_bytes"),
            "proven_absent_paths": row.get("proven_absent_paths"),
            "localisation_source_sha256": row.get("localisation_source_sha256"),
        }));
    }

    let scheduled = embedded_manifest
        .get("scheduled_domains")
        .and_then(Value::as_array)
        .ok_or_else(|| "manifest scheduled_domains is missing or not an array".to_string())?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let mut expected_required_content = BTreeSet::new();
    if scheduled.contains(compact_contract::rail_replay::DOMAIN_NAME) {
        expected_required_content
            .insert(recorder_host::attach_content_proof::D81_PROVINCE_CATALOG_PATH.to_string());
    }
    let joined_required = proof
        .get("required_content")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{field}.required_content is missing or not an array"))?;
    let mut joined_required_paths = BTreeSet::new();
    for row in joined_required {
        let logical_path = row
            .get("logical_path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| format!("{field}.required_content row names no logical_path"))?;
        canonical_sha256(
            row.get("sha256"),
            &format!("{field}.required_content {logical_path:?} sha256"),
        )?;
        row.get("bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{field}.required_content {logical_path:?} bytes is invalid"))?;
        let winning_layer_id = row
            .get("winning_layer_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("{field}.required_content {logical_path:?} names no winning layer")
            })?;
        if !layers
            .iter()
            .any(|layer| layer.layer_id == winning_layer_id)
        {
            return Err(format!(
                "{field}.required_content {logical_path:?} was won by layer {winning_layer_id:?}, \
                 which is not in the layer stack the proof states"
            ));
        }
        if !joined_required_paths.insert(logical_path.to_string()) {
            return Err(format!(
                "{field}.required_content duplicates {logical_path:?}"
            ));
        }
    }
    if !expected_required_content.is_subset(&joined_required_paths) {
        return Err(format!(
            "the scheduled roster consumes playback-required content that the attach-time proof \
             does not join: {:?}",
            expected_required_content
                .difference(&joined_required_paths)
                .collect::<Vec<_>>()
        ));
    }
    let refused_required = proof
        .get("refused_required_content")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{field}.refused_required_content is missing or not an array"))?;
    if !refused_required.is_empty() {
        return Err(format!(
            "{field} could not resolve {} playback-required content file(s) through the \
             attach-time effective VFS: {}",
            refused_required.len(),
            refused_required
                .iter()
                .map(|row| format!(
                    "{} ({}): {}",
                    row.get("logical_path")
                        .and_then(Value::as_str)
                        .unwrap_or("?"),
                    row.get("purpose").and_then(Value::as_str).unwrap_or("?"),
                    row.get("reason").and_then(Value::as_str).unwrap_or("?"),
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }

    Ok(json!({
        "content_proof_kind":
            recorder_host::attach_content_proof::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1,
        "static_artifacts_joined": true,
        "required_count": compact_contract::REQUIRED_STATIC_ARTIFACTS.len(),
        "exact_required_roles": declared.len() == canonical.len(),
        "packaged_roles": declared.iter().collect::<Vec<_>>(),
        "missing_required_static_artifacts": missing.iter().collect::<Vec<_>>(),
        "attach_window_observed_at_unix": observed_at,
        "attach_window_completed_at_unix": completed_at,
        "attach_window_seconds": completed_at.saturating_sub(observed_at),
        "game_process_start_unix": game_process_start_unix,
        "seconds_running_before_the_window_opened": observed_at
            .saturating_sub(game_process_start_unix),
        "attach_window_attempts": proof.get("window_attempts"),
        "executable_sha256": executable_sha256,
        "dlc_load_sha256": dlc_load_sha256,
        "layer_stack_sha256": declared_stack_sha256,
        "layer_count": layers.len(),
        "required_content": proof.get("required_content"),
        "artifacts": joined_details,
        "portable_content_manifest":
            "absent: certified by attach_time_effective_vfs_join_v1 instead of a pre-resume epoch",
        "static_bundle":
            "absent: the attach-time proof binds the artifacts to engine content directly",
    }))
}

const RUN_DIR_ONLY_POST_SEAL_HOST_FINALIZE_FIELDS: &[&str] = &[
    "finalize_seconds",
    "peak_private_bytes",
    "peak_working_set_bytes",
    "memory_account",
    "verify",
];

const SEALED_ONLY_PRE_PACK_HOST_FINALIZE_FIELDS: &[&str] = &[
    "embedded_copy_phase",
    "standalone_manifest_is_authoritative",
    "post_pack_fields_in_standalone_manifest_only",
    "excluded_domain_data",
    recorder_host::hour_coverage::HOUR_COVERAGE_FIELD,
];

/// `spool_trim` is the storage-release outcome of
/// `recorder_host::release_sealed_spool_storage`, recorded by
/// `{"released": false, "reason": "--keep-spool-after-seal"}` against a sealed
const RUN_DIR_ONLY_POST_SEAL_TOP_LEVEL_FIELDS: &[&str] = &["spool_trim"];

fn post_seal_mutable_host_finalize_fields() -> Vec<&'static str> {
    RUN_DIR_ONLY_POST_SEAL_HOST_FINALIZE_FIELDS
        .iter()
        .chain(SEALED_ONLY_PRE_PACK_HOST_FINALIZE_FIELDS)
        .copied()
        .collect()
}

struct SealedManifestBinding {
    path_redaction_applied: bool,
    portability_degraded: bool,
}

///     report, or the spool storage-release outcome. Those are written back
fn bind_run_dir_manifest_to_sealed_authority(
    external_manifest: &Value,
    embedded_manifest: &Value,
) -> Result<SealedManifestBinding, String> {
    recorder_host::portable_manifest::reject_absolute_paths_in_json(embedded_manifest, "$")
        .map_err(|error| {
            format!(
                "authoritative sealed recorder_manifest is not portable: {error}; a shareable \
                 container must carry no personal filesystem path, in a value or in an object \
                 key, and the recorder's seal-time gate refuses to write one — so a container \
                 that carries one was not sealed by that gate"
            )
        })?;

    let mut sealed = embedded_manifest.clone();
    let mut run_dir = external_manifest.clone();

    let sealed_host_finalize_is_object = sealed.get("host_finalize").is_some_and(Value::is_object);
    if !sealed_host_finalize_is_object {
        return Err(format!(
            "authoritative sealed recorder_manifest has no object-valued \"host_finalize\" block (found {}); \
             every compact capture this gate can certify seals one, so its absence would unbind \
             host_finalize.stream_sha256 and all of host_finalize.fold.*",
            truncate_for_refusal(sealed.get("host_finalize").unwrap_or(&Value::Null))
        ));
    }
    if !run_dir.get("host_finalize").is_some_and(Value::is_object) {
        return Err(
            "run-dir native_session_manifest.json has no object-valued \"host_finalize\" block but \
             the authoritative sealed recorder_manifest does; a deleted block is not a sanctioned \
             post-seal divergence"
                .to_string(),
        );
    }

    take_directional_fields(
        &mut run_dir,
        &sealed,
        Some("host_finalize"),
        RUN_DIR_ONLY_POST_SEAL_HOST_FINALIZE_FIELDS,
        "run-dir",
        "sealed",
    )?;
    take_directional_fields(
        &mut sealed,
        &run_dir,
        Some("host_finalize"),
        SEALED_ONLY_PRE_PACK_HOST_FINALIZE_FIELDS,
        "sealed",
        "run-dir",
    )?;
    take_directional_fields(
        &mut run_dir,
        &sealed,
        None,
        RUN_DIR_ONLY_POST_SEAL_TOP_LEVEL_FIELDS,
        "run-dir",
        "sealed",
    )?;

    let (excluded_run_dir, _) =
        recorder_host::portable_manifest::apply_portability_exclusions(&run_dir);
    run_dir = excluded_run_dir;
    let sealed_ledger = sealed
        .get(recorder_host::portable_manifest::PORTABILITY_EXCLUSIONS_FIELD)
        .cloned();
    let expected_ledger = run_dir
        .get(recorder_host::portable_manifest::PORTABILITY_EXCLUSIONS_FIELD)
        .cloned();
    if sealed_ledger != expected_ledger {
        let describe = |ledger: Option<&Value>| {
            ledger
                .and_then(|ledger| ledger.get("excluded"))
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .map(truncate_for_refusal)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_else(|| "none".to_string())
        };
        return Err(format!(
            "authoritative sealed recorder_manifest accounts for portability \
             exclusion(s) [{}] but its run-dir native_session_manifest.json \
             produces [{}]; the ledger is what accounts for every value the seal \
             had to remove, so a ledger that does not follow from the recording \
             is not that accounting",
            describe(sealed_ledger.as_ref()),
            describe(expected_ledger.as_ref()),
        ));
    }
    let portability_degraded = sealed_ledger.is_some();

    let mut redactions = 0usize;
    if let Some(violation) = first_binding_violation(&sealed, &run_dir, "$", &mut redactions) {
        return Err(format!(
            "run-dir native_session_manifest.json contradicts the authoritative sealed \
             recorder_manifest at {violation}; the only sanctioned post-seal divergences are \
             absolute-path redaction of a run-dir value, host_finalize {:?}, and top-level {:?}",
            post_seal_mutable_host_finalize_fields(),
            RUN_DIR_ONLY_POST_SEAL_TOP_LEVEL_FIELDS
        ));
    }
    Ok(SealedManifestBinding {
        path_redaction_applied: redactions > 0,
        portability_degraded,
    })
}

fn take_directional_fields(
    owner: &mut Value,
    forbidden: &Value,
    block: Option<&str>,
    fields: &[&str],
    owner_label: &str,
    forbidden_label: &str,
) -> Result<(), String> {
    let qualified = |field: &str| match block {
        Some(name) => format!("{name}.{field}"),
        None => field.to_string(),
    };
    let forbidden_object = match block {
        Some(name) => forbidden.get(name).and_then(Value::as_object),
        None => forbidden.as_object(),
    };
    if let Some(object) = forbidden_object {
        if let Some(found) = fields.iter().find(|field| object.contains_key(**field)) {
            return Err(format!(
                "{} is a {owner_label}-copy-only post-seal field but the {forbidden_label} copy \
                 carries it; the sanctioned divergence is directional",
                qualified(found)
            ));
        }
    }
    let owner_object = match block {
        Some(name) => owner.get_mut(name).and_then(Value::as_object_mut),
        None => owner.as_object_mut(),
    };
    if let Some(object) = owner_object {
        for field in fields {
            object.remove(*field);
        }
    }
    Ok(())
}

fn first_binding_violation(
    sealed: &Value,
    run_dir: &Value,
    path: &str,
    redactions: &mut usize,
) -> Option<String> {
    use recorder_host::portable_manifest::redact_absolute_path_string;

    match (sealed, run_dir) {
        (Value::Object(left), Value::Object(right)) => {
            let mut bound = BTreeMap::<String, (&String, &Value)>::new();
            for (key, value) in right {
                let name = redact_absolute_path_string(key);
                if bound.insert(name.clone(), (key, value)).is_some() {
                    return Some(format!(
                        "{path}.{name} (two run-dir keys collide onto one sealed key after \
                         absolute-path redaction)"
                    ));
                }
            }
            let mut consumed = BTreeSet::<&str>::new();
            for (key, sealed_value) in left {
                let child = format!("{path}.{key}");
                let matched = right
                    .get_key_value(key)
                    .or_else(|| bound.get(key).map(|(k, v)| (*k, *v)));
                match matched {
                    None => {
                        return Some(format!(
                            "{child} (present in the sealed copy, absent from the run-dir copy)"
                        ))
                    }
                    Some((run_dir_key, run_dir_value)) => {
                        if run_dir_key != key {
                            *redactions += 1;
                        }
                        consumed.insert(run_dir_key.as_str());
                        if let Some(found) =
                            first_binding_violation(sealed_value, run_dir_value, &child, redactions)
                        {
                            return Some(found);
                        }
                    }
                }
            }
            right
                .keys()
                .find(|key| !consumed.contains(key.as_str()))
                .map(|key| {
                    format!(
                        "{path}.{key} (absent from the sealed copy, present in the run-dir copy)"
                    )
                })
        }
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                return Some(format!(
                    "{path} (sealed copy has {} elements, run-dir copy has {})",
                    left.len(),
                    right.len()
                ));
            }
            left.iter()
                .zip(right)
                .enumerate()
                .find_map(|(index, (l, r))| {
                    first_binding_violation(l, r, &format!("{path}[{index}]"), redactions)
                })
        }
        (left, right) if left == right => None,
        (Value::String(sealed_text), Value::String(run_dir_text)) => {
            let reduced = redact_absolute_path_string(run_dir_text);
            if &reduced != run_dir_text && &reduced == sealed_text {
                *redactions += 1;
                return None;
            }
            Some(format!(
                "{path} (sealed copy {}, run-dir copy {})",
                truncate_for_refusal(sealed),
                truncate_for_refusal(run_dir)
            ))
        }
        (left, right) => Some(format!(
            "{path} (sealed copy {}, run-dir copy {})",
            truncate_for_refusal(left),
            truncate_for_refusal(right)
        )),
    }
}

fn truncate_for_refusal(value: &Value) -> String {
    let rendered = value.to_string();
    if rendered.chars().count() <= 120 {
        return rendered;
    }
    let head: String = rendered.chars().take(120).collect();
    format!("{head}…")
}

#[cfg(test)]
fn scanner_guards(run_dir: &Path) -> Vec<std::fs::File> {
    fn walk(directory: &Path, guards: &mut Vec<std::fs::File>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => walk(&path, guards),
                Ok(kind) if kind.is_file() => {
                    if let Ok(file) = portable_test_fs_retry::open(&path) {
                        guards.push(file);
                    }
                }
                _ => {}
            }
        }
    }
    let mut guards = Vec::new();
    walk(run_dir, &mut guards);
    guards
}

fn load_verified_replay_authority(
    external_manifest: &Value,
    external_manifest_bytes: &[u8],
    run_dir: &Path,
    max_stream_bytes: u64,
) -> Result<VerifiedReplayAuthority, String> {
    #[cfg(test)]
    let _scanner_guards = scanner_guards(run_dir);
    let projected_external_manifest =
        recorder_host::portable_manifest::portable_manifest_projection(external_manifest).map_err(
            |err| format!("run-dir native_session_manifest.json has no portable projection: {err}"),
        )?;
    let declared_object = projected_external_manifest
        .as_object()
        .ok_or_else(|| "external native_session_manifest.json is not a JSON object".to_string())?;
    let declared_packaging =
        snapshot::read_raw_stream_packaging_with_legacy_default(declared_object)
            .map_err(|error| format!("external native_session_manifest.json: {error}"))?;
    let replay_path = required_run_file(run_dir, external_manifest, "replay_file")?;
    let expected = ExpectedIdentity {
        schema_version: Some(REPLAY_CONTAINER_SCHEMA_VERSION.to_string()),
        ..ExpectedIdentity::default()
    };
    let container = Container::open(&replay_path, &expected).map_err(|err| {
        format!(
            "cannot open replay container {} (container manifest.json schema_version must be {REPLAY_CONTAINER_SCHEMA_VERSION:?}): {err}",
            replay_path.display()
        )
    })?;
    let entry_report = container.verify_all_streaming_report().map_err(|err| {
        format!(
            "replay container per-entry verification failed for {}: {err}",
            replay_path.display()
        )
    })?;
    {
        let aliased: Vec<String> = entry_report
            .iter()
            .filter(|entry| {
                entry.hour_index.is_some()
                    && matches!(
                        entry.role.as_deref(),
                        Some(snapshot::CHECKPOINT_CACHE_ROLE) | Some(snapshot::HOUR_INDEX_ROLE)
                    )
            })
            .map(|entry| {
                format!(
                    "{} (role {:?}) is also indexed as the snapshot for hour_index {}",
                    entry.path,
                    entry.role.as_deref().unwrap_or("none"),
                    entry.hour_index.unwrap_or_default()
                )
            })
            .collect();
        if !aliased.is_empty() {
            return Err(format!(
                "replay container {} indexes {} replay_decoder artifact(s) as per-hour \
                 snapshots, so damage to them could not be attributed to the hours they really \
                 serve: {}",
                replay_path.display(),
                aliased.len(),
                aliased.join("; ")
            ));
        }
    }

    {
        let defer_to_per_record_attribution = |entry: &snapshot::container::EntryVerification| {
            matches!(declared_packaging, snapshot::RawStreamPackaging::Omitted)
                && entry.role.as_deref() == Some(snapshot::CHECKPOINT_CACHE_ROLE)
        };
        let unisolatable: Vec<String> = entry_report
            .iter()
            .filter(|entry| {
                entry.error.is_some()
                    && entry.hour_index.is_none()
                    && !defer_to_per_record_attribution(entry)
            })
            .map(|entry| {
                format!(
                    "{} (role {:?}): {}",
                    entry.path,
                    entry.role.as_deref().unwrap_or("none"),
                    entry.error.as_deref().unwrap_or("")
                )
            })
            .collect();
        if !unisolatable.is_empty() {
            return Err(format!(
                "replay container {} carries {} damaged entr(ies) that serve no single game \
                 hour, so the damage cannot be confined to one hour: {}",
                replay_path.display(),
                unisolatable.len(),
                unisolatable.join("; ")
            ));
        }
    }

    let replay_game_id = required_nonempty_str(external_manifest, "replay_game_id")?;
    if container.game_id() != replay_game_id {
        return Err(format!(
            "replay container game_id {:?} != recorder replay_game_id {:?}",
            container.game_id(),
            replay_game_id
        ));
    }
    let layout = replay_layout(external_manifest, declared_packaging)?;

    let manifest_entries = container.entries_by_role("recorder_manifest");
    if manifest_entries.len() != 1 {
        return Err(format!(
            "replay container has {} recorder_manifest entries, expected 1",
            manifest_entries.len()
        ));
    }
    let expected_manifest_path = format!("payload/{replay_game_id}/{}", layout.session_manifest);
    let manifest_entry = &manifest_entries[0];
    if manifest_entry.path != expected_manifest_path {
        return Err(format!(
            "recorder_manifest path {:?} != replay_entries.session_manifest authority {:?}",
            manifest_entry.path, expected_manifest_path
        ));
    }
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
    let embedded_manifest = snapshot::parse_json_strict(&embedded_manifest_bytes)
        .map_err(|err| format!("embedded recorder_manifest is not strict JSON: {err}"))?;
    let manifest_binding =
        bind_run_dir_manifest_to_sealed_authority(external_manifest, &embedded_manifest)?;
    let embedded_schema = embedded_manifest.get("schema").and_then(Value::as_str);
    if embedded_schema != Some(RECORDER_MANIFEST_SCHEMA) {
        return Err(format!(
            "container-authenticated recorder_manifest field \"schema\" is {embedded_schema:?}, expected {RECORDER_MANIFEST_SCHEMA:?}; \
             this is the recorder manifest's session-format identifier, not the container's schema_version"
        ));
    }
    if required_nonempty_str(&embedded_manifest, "replay_game_id")? != container.game_id() {
        return Err(
            "embedded recorder_manifest replay_game_id changed after identity binding".into(),
        );
    }
    let stream_entries = container.entries_by_role("collector_compact_stream");
    let (stream_certification, mut stream_details) = match declared_packaging {
        snapshot::RawStreamPackaging::Packaged => {
            if stream_entries.len() != 1 {
                return Err(format!(
                    "replay container has {} collector_compact_stream entries, expected 1",
                    stream_entries.len()
                ));
            }
            let stream_entry = &stream_entries[0];
            let Some(stream_relative) = layout.stream else {
                return Err(
                    "packaged composition resolved no replay_entries.stream layout".to_string(),
                );
            };
            let expected_stream_path = format!("payload/{replay_game_id}/{stream_relative}");
            if stream_entry.path != expected_stream_path {
                return Err(format!(
                    "collector_compact_stream path {:?} != replay_entries.stream authority {:?}",
                    stream_entry.path, expected_stream_path
                ));
            }
            let container_stream =
                container
                    .read_entry_bytes(&stream_entry.path)
                    .map_err(|err| {
                        format!(
                            "cannot read collector_compact_stream {} from replay container: {err}",
                            stream_entry.path
                        )
                    })?;
            let independently_observed_stream_sha256 = Sha256::hex(&container_stream);
            if independently_observed_stream_sha256 != stream_entry.sha256 {
                return Err(format!(
                    "verified collector_compact_stream bytes independently hash to {independently_observed_stream_sha256}, not container entry digest {}",
                    stream_entry.sha256
                ));
            }
            let details = json!({
                "collector_compact_stream": stream_entry.path,
                "collector_compact_stream_bytes": stream_entry.bytes,
                "collector_compact_stream_sha256": stream_entry.sha256,
                "independently_observed_stream_sha256": independently_observed_stream_sha256,
                "h4cr_stream_digest_bound": true,
                "h4cr_stream_bytes_compared": true,
            });
            (
                StreamCertification::ByteComparedAgainstPackagedStream {
                    container_stream,
                    container_stream_sha256: independently_observed_stream_sha256,
                },
                details,
            )
        }
        snapshot::RawStreamPackaging::Omitted => certify_cache_only_recording(
            &container,
            &embedded_manifest,
            &stream_entries,
            run_dir,
            max_stream_bytes,
        )?,
    };

    let content_proof_kind = read_content_proof_kind(&embedded_manifest)?;
    let static_artifacts = match content_proof_kind {
        ContentProofKind::LaunchTimeStaticBundleV1 => {
            let attach_field = recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD;
            if embedded_manifest
                .get(attach_field)
                .is_some_and(|value| !value.is_null())
            {
                return Err(format!(
                    "recording declares the launch-time content epoch but carries an \
                     {attach_field}: the two describe different observation points and a \
                     recording certifies under exactly one"
                ));
            }
            let static_artifacts = validate_required_static_artifacts(
                &container,
                &embedded_manifest,
                external_manifest,
                container.game_id(),
            )?;
            let container_content = container.content_manifest().ok_or_else(|| {
                "production replay container has no top-level portable content manifest".to_string()
            })?;
            let container_content_value =
                serde_json::to_value(container_content).map_err(|error| {
                    format!("serialize admitted container content manifest failed: {error}")
                })?;
            if embedded_manifest.get("portable_content_manifest") != Some(&container_content_value)
            {
                return Err(
                    "container top-level portable content manifest differs from embedded recorder \
                     authority"
                        .into(),
                );
            }
            static_artifacts
        }
        ContentProofKind::AttachTimeEffectiveVfsJoinV1 => {
            if let Some(container_content) = container.content_manifest() {
                let container_content_value =
                    serde_json::to_value(container_content).map_err(|error| {
                        format!("serialize admitted container content manifest failed: {error}")
                    })?;
                if embedded_manifest.get("portable_content_manifest")
                    != Some(&container_content_value)
                {
                    return Err(
                        "container top-level portable content manifest differs from embedded \
                         recorder authority"
                            .into(),
                    );
                }
            }
            certify_attach_time_content_proof(&container, &embedded_manifest, container.game_id())?
        }
    };
    let mut hour_coverage = read_sealed_hour_coverage(&embedded_manifest)?;
    let mut served_hours = container.published_hour_indexes();
    served_hours.extend(
        stream_details
            .get("indexed_hour_indexes")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(Value::as_i64).collect::<Vec<i64>>())
            .unwrap_or_default(),
    );
    served_hours.sort_unstable();
    served_hours.dedup();
    refuse_hours_the_coverage_map_does_not_vouch_for(&served_hours, &hour_coverage, &replay_path)?;
    bind_index_holes_to_sealed_coverage(&stream_details, &hour_coverage, &replay_path)?;

    let confined_by_cache_damage: Vec<Value> = stream_details
        .get(CACHE_DAMAGE_CONFINED_HOURS_FIELD)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for confined in &confined_by_cache_damage {
        let hour_index = confined
            .get("hour_index")
            .and_then(Value::as_i64)
            .ok_or_else(|| "confined hour row has no hour_index".to_string())?;
        let reason = confined
            .get("reason")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("confined hour {hour_index} names no reason"))?;
        let game_hour_raw = i32::try_from(hour_index)
            .ok()
            .and_then(|hour| hour.checked_add(compact_contract::calendar::DATE_EPOCH_CONST))
            .ok_or_else(|| {
                format!("cache damage confines an out-of-range hour_index {hour_index}")
            })?;
        let first_byte = confined
            .get("h4cr_start_offset")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let end = confined
            .get("h4cr_end_offset")
            .and_then(Value::as_u64)
            .unwrap_or(first_byte);
        hour_coverage.record_excluded(
            game_hour_raw,
            first_byte,
            end.saturating_sub(first_byte),
            reason,
        );
    }

    for entry in &entry_report {
        let Some(error) = entry.error.as_deref() else {
            continue;
        };
        let Some(game_hour_raw) = entry
            .hour_index
            .and_then(|hour| i32::try_from(hour).ok())
            .and_then(|hour| hour.checked_add(compact_contract::calendar::DATE_EPOCH_CONST))
        else {
            continue;
        };
        hour_coverage.record_excluded(
            game_hour_raw,
            0,
            0,
            format!(
                "the container entry serving this hour ({}) failed verification: {error}",
                entry.path
            ),
        );
    }
    let excluded_hours: Vec<Value> = hour_coverage
        .excluded_hours()
        .into_iter()
        .map(|(hour, reason)| json!({ "game_hour_raw": hour, "reason": reason }))
        .collect();

    let mut details = json!({
        "file": replay_path.display().to_string(),
        "game_id": container.game_id(),
        recorder_host::hour_coverage::HOUR_COVERAGE_FIELD: json!({
            "verified_hours": hour_coverage.verified_count(),
            "excluded_hours": excluded_hours,
            "unattributed_losses": hour_coverage.unattributed_losses(),
        }),
        "recorder_manifest": manifest_entry.path,
        "recorder_manifest_bytes": embedded_manifest_bytes.len(),
        "recorder_manifest_sha256": embedded_sha256,
        "external_manifest_bytes": external_manifest_bytes.len(),
        "external_manifest_sha256": external_sha256,
        "external_manifest_bound_to_sealed_authority": true,
        "external_manifest_binding": "portable_projection_with_named_post_seal_allowlist",
        "external_manifest_path_redaction_applied": manifest_binding.path_redaction_applied,
        "recorder_manifest_portability_degraded": manifest_binding.portability_degraded,
        "run_dir_only_post_seal_host_finalize_fields": RUN_DIR_ONLY_POST_SEAL_HOST_FINALIZE_FIELDS,
        "sealed_only_pre_pack_host_finalize_fields": SEALED_ONLY_PRE_PACK_HOST_FINALIZE_FIELDS,
        "run_dir_only_post_seal_top_level_fields": RUN_DIR_ONLY_POST_SEAL_TOP_LEVEL_FIELDS,
        "static_artifacts": static_artifacts,
        "signed": container.signed,
        "unsigned": container.unsigned,
        snapshot::RAW_STREAM_PACKAGING_FIELD: declared_packaging.as_str(),
        "stream_certification_path": stream_certification.path_name(),
    });
    details
        .as_object_mut()
        .expect("replay authority details is an object")
        .append(
            stream_details
                .as_object_mut()
                .expect("stream certification details is an object"),
        );
    Ok(VerifiedReplayAuthority {
        manifest: embedded_manifest,
        stream_certification,
        hour_coverage,
        details,
    })
}

fn refuse_hours_the_coverage_map_does_not_vouch_for(
    published_hour_indexes: &[i64],
    coverage: &recorder_host::hour_coverage::HourCoverageMap,
    replay_path: &Path,
) -> Result<(), String> {
    for hour_index in published_hour_indexes {
        let game_hour_raw = i32::try_from(*hour_index)
            .ok()
            .and_then(|hour| hour.checked_add(compact_contract::calendar::DATE_EPOCH_CONST))
            .ok_or_else(|| {
                format!("replay container publishes an out-of-range hour_index {hour_index}")
            })?;
        if !coverage.is_hour_verified(game_hour_raw) {
            return Err(format!(
                "replay container {} publishes game hour {game_hour_raw} (hour_index \
                 {hour_index}), which its own verified-coverage map does not vouch for: {}",
                replay_path.display(),
                coverage
                    .row(game_hour_raw)
                    .map(|row| match &row.coverage {
                        recorder_host::hour_coverage::HourCoverage::Excluded { reason } =>
                            reason.clone(),
                        recorder_host::hour_coverage::HourCoverage::Verified { .. } =>
                            "verified".to_string(),
                    })
                    .unwrap_or_else(|| "the map names this hour nowhere at all".to_string())
            ));
        }
    }
    Ok(())
}

const INDEX_EXCLUDED_HOURS_FIELD: &str = "index_excluded_hours";

const CACHE_DAMAGE_CONFINED_HOURS_FIELD: &str = "cache_damage_confined_hours";

fn sealed_artifact_digest(embedded_manifest: &Value, field: &str) -> Result<String, String> {
    embedded_manifest
        .get("host_finalize")
        .and_then(|finalize| finalize.get("fold"))
        .and_then(|fold| fold.get(field))
        .and_then(Value::as_str)
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_string)
        .ok_or_else(|| {
            cache_only_link_refusal(
                CACHE_ONLY_LINK_SEALED_ARTIFACTS,
                format!(
                    "the container-authenticated recorder manifest carries no 64-character \
                     host_finalize.fold.{field}; without it nothing binds the packaged artifact \
                     to the finalize that certified its hours and its holes"
                ),
            )
        })
}

fn bind_index_holes_to_sealed_coverage(
    stream_details: &Value,
    coverage: &recorder_host::hour_coverage::HourCoverageMap,
    replay_path: &Path,
) -> Result<(), String> {
    let Some(holes) = stream_details
        .get(INDEX_EXCLUDED_HOURS_FIELD)
        .and_then(Value::as_array)
    else {
        return Ok(());
    };
    for hole in holes {
        let hour_index = hole
            .get("hour_index")
            .and_then(Value::as_i64)
            .ok_or_else(|| "packaged index hole row has no hour_index".to_string())?;
        let reason = hole
            .get("reason")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("packaged index hole for hour {hour_index} names no reason"))?;
        let game_hour_raw = i32::try_from(hour_index)
            .ok()
            .and_then(|hour| hour.checked_add(compact_contract::calendar::DATE_EPOCH_CONST))
            .ok_or_else(|| {
                format!("packaged index excludes an out-of-range hour_index {hour_index}")
            })?;
        match coverage.row(game_hour_raw).map(|row| &row.coverage) {
            Some(recorder_host::hour_coverage::HourCoverage::Excluded { reason: sealed })
                if sealed == reason => {}
            Some(recorder_host::hour_coverage::HourCoverage::Excluded { reason: sealed }) => {
                return Err(format!(
                    "replay container {} excludes game hour {game_hour_raw} in its packaged index \
                     for {reason:?}, but the recording sealed {sealed:?}; a recording names each \
                     injured hour once",
                    replay_path.display()
                ))
            }
            Some(recorder_host::hour_coverage::HourCoverage::Verified { .. }) => {
                return Err(format!(
                    "replay container {} refuses game hour {game_hour_raw} in its packaged index \
                     for {reason:?}, but the recording's sealed coverage map VERIFIES that hour; \
                     a good hour must not be dropped on an excuse the seal never made",
                    replay_path.display()
                ))
            }
            None => {
                return Err(format!(
                    "replay container {} refuses game hour {game_hour_raw} in its packaged index \
                     for {reason:?}, but the recording's sealed coverage map names that hour \
                     nowhere at all",
                    replay_path.display()
                ))
            }
        }
    }
    Ok(())
}

fn read_sealed_hour_coverage(
    embedded_manifest: &Value,
) -> Result<recorder_host::hour_coverage::HourCoverageMap, String> {
    let field = recorder_host::hour_coverage::HOUR_COVERAGE_FIELD;
    let value = embedded_manifest
        .get("host_finalize")
        .and_then(|finalize| finalize.get(field))
        .ok_or_else(|| {
            format!(
                "container-authenticated recorder_manifest carries no \
                 host_finalize.{field}; a sealed recording must state which game hours it \
                 verifies and which it excludes"
            )
        })?;
    recorder_host::hour_coverage::HourCoverageMap::from_json(value).map_err(|error| {
        format!("container-authenticated recorder_manifest host_finalize.{field}: {error}")
    })
}

fn certify_cache_only_recording(
    container: &Container,
    embedded_manifest: &Value,
    stream_entries: &[snapshot::container::ContainerEntry],
    run_dir: &Path,
    max_stream_bytes: u64,
) -> Result<(StreamCertification, Value), String> {
    let container_packaging = snapshot::container_raw_stream_packaging(container)
        .map_err(|error| cache_only_link_refusal(CACHE_ONLY_LINK_COMPOSITION, error))?;
    if container_packaging != snapshot::RawStreamPackaging::Omitted {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_COMPOSITION,
            format!(
                "the run manifest routes this recording as {:?}, but the container's own recorder \
                 manifest declares {:?}",
                snapshot::RawStreamPackaging::Omitted.as_str(),
                container_packaging.as_str(),
            ),
        ));
    }

    if !stream_entries.is_empty() {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_NO_PACKAGED_STREAM,
            format!(
                "the recording declares the {:?} composition but the container carries {} \
                 collector_compact_stream entr{}; a packaged stream must be byte-compared, not \
                 manifest-attested",
                snapshot::RawStreamPackaging::Omitted.as_str(),
                stream_entries.len(),
                if stream_entries.len() == 1 {
                    "y"
                } else {
                    "ies"
                },
            ),
        ));
    }

    let storage = embedded_manifest
        .get("stream_storage")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            cache_only_link_refusal(
                CACHE_ONLY_LINK_STREAM_STORAGE,
                "the container-authenticated recorder manifest has no stream_storage object, so \
                 the cache has no source identity to be bound to",
            )
        })?;
    let attested_sha256 = storage
        .get("decoded_sha256")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            cache_only_link_refusal(
                CACHE_ONLY_LINK_STREAM_STORAGE,
                "stream_storage.decoded_sha256 is missing or empty",
            )
        })?
        .to_string();
    let attested_bytes = storage
        .get("decoded_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            cache_only_link_refusal(
                CACHE_ONLY_LINK_STREAM_STORAGE,
                "stream_storage.decoded_bytes is missing or not a u64",
            )
        })?;
    if attested_bytes > max_stream_bytes {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_STREAM_STORAGE,
            format!(
                "stream_storage.decoded_bytes {attested_bytes} exceeds the configured maximum \
                 {max_stream_bytes}"
            ),
        ));
    }

    let relative = validate_relative_path(
        required_nonempty_str(embedded_manifest, "raw_stream").map_err(|error| {
            cache_only_link_refusal(
                CACHE_ONLY_LINK_SOURCE_READABLE,
                format!("{error}; a cache-only recording must name its run-dir source stream"),
            )
        })?,
        "raw_stream",
    )
    .map_err(|error| cache_only_link_refusal(CACHE_ONLY_LINK_SOURCE_READABLE, error))?;
    let candidate = run_dir.join(&relative);
    match fs::metadata(&candidate) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(cache_only_source_absent(&candidate))
        }
        Err(error) => {
            return Err(cache_only_link_refusal(
                CACHE_ONLY_LINK_SOURCE_READABLE,
                format!(
                    "cannot stat run-dir source stream {}: {error}",
                    candidate.display()
                ),
            ))
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(cache_only_link_refusal(
                CACHE_ONLY_LINK_SOURCE_READABLE,
                format!(
                    "run-dir source stream {} is not a regular file",
                    candidate.display()
                ),
            ))
        }
        Ok(_) => {}
    }
    let stream_path = required_run_file(run_dir, embedded_manifest, "raw_stream")
        .map_err(|error| cache_only_link_refusal(CACHE_ONLY_LINK_SOURCE_READABLE, error))?;
    let compression_name = storage
        .get("compression")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            cache_only_link_refusal(
                CACHE_ONLY_LINK_SOURCE_READABLE,
                "stream_storage.compression is missing",
            )
        })?;
    let compression = match compression_name {
        "none" => compact_stream::StreamCompression::None,
        "zstd" => compact_stream::StreamCompression::Zstd,
        other => {
            return Err(cache_only_link_refusal(
                CACHE_ONLY_LINK_SOURCE_READABLE,
                format!("unsupported manifest stream compression {other:?}"),
            ))
        }
    };
    let detected = compact_stream::detect(&stream_path).map_err(|error| {
        cache_only_link_refusal(
            CACHE_ONLY_LINK_SOURCE_READABLE,
            format!(
                "detect compact stream storage {} failed: {error}",
                stream_path.display()
            ),
        )
    })?;
    if detected != compression {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_SOURCE_READABLE,
            format!(
                "manifest stream compression {} does not match stored file detection {}",
                compression.as_str(),
                detected.as_str()
            ),
        ));
    }

    let (observed_sha256, observed_bytes) =
        hash_decoded_source_stream(&stream_path, compression, attested_bytes)
            .map_err(|error| cache_only_link_refusal(CACHE_ONLY_LINK_SOURCE_READABLE, error))?;
    if observed_bytes > attested_bytes {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_STREAM_STORAGE,
            format!(
                "the run-dir source stream {} decodes to more than the {attested_bytes} bytes the \
                 container-authenticated recorder manifest states",
                stream_path.display(),
            ),
        ));
    }
    if observed_sha256 != attested_sha256 || observed_bytes != attested_bytes {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_STREAM_STORAGE,
            format!(
                "the run-dir source stream {} independently hashes to {observed_sha256} over \
                 {observed_bytes} decoded bytes, but the container-authenticated recorder manifest \
                 states {attested_sha256} over {attested_bytes}",
                stream_path.display(),
            ),
        ));
    }

    let index_entry = single_cache_entry(
        container,
        snapshot::HOUR_INDEX_ROLE,
        snapshot::HOUR_INDEX_FILE,
        CACHE_ONLY_LINK_HOUR_INDEX,
    )?;
    let index_bytes = container
        .read_entry_bytes(&index_entry.path)
        .map_err(|error| {
            cache_only_link_refusal(
                CACHE_ONLY_LINK_HOUR_INDEX,
                format!(
                    "cannot read {} from the container: {error}",
                    index_entry.path
                ),
            )
        })?;
    let hour_index = snapshot::ReplayHourIndex::from_bytes(&index_bytes).map_err(|error| {
        cache_only_link_refusal(
            CACHE_ONLY_LINK_HOUR_INDEX,
            format!("{} is not a readable H4IX: {error}", index_entry.path),
        )
    })?;
    if hour_index.source_sha256() != attested_sha256 {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_HOUR_INDEX,
            format!(
                "H4IX was folded over source {} but the attested run source is {attested_sha256}",
                hour_index.source_sha256(),
            ),
        ));
    }
    if hour_index.source_len() != attested_bytes {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_HOUR_INDEX,
            format!(
                "H4IX states source length {} but the attested run source is {attested_bytes} \
                 bytes; H4CP carries the source digest only, so this length link rests on H4IX \
                 alone",
                hour_index.source_len(),
            ),
        ));
    }
    if hour_index.game_id() != container.game_id() {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_HOUR_INDEX,
            format!(
                "H4IX game_id {:?} != container game_id {:?}",
                hour_index.game_id(),
                container.game_id(),
            ),
        ));
    }

    let sealed_index_sha256 = sealed_artifact_digest(embedded_manifest, "hour_index_sha256")?;
    let observed_index_sha256 = Sha256::hex(&index_bytes);
    if observed_index_sha256 != sealed_index_sha256 {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_SEALED_ARTIFACTS,
            format!(
                "the packaged {} hashes to {observed_index_sha256}, but the \
                 container-authenticated recorder manifest sealed \
                 host_finalize.fold.hour_index_sha256 {sealed_index_sha256}; this index is not \
                 the one the seal certified, so nothing vouches for which hours it serves and \
                 which it names as holes",
                index_entry.path
            ),
        ));
    }

    let cache_entry = single_cache_entry(
        container,
        snapshot::CHECKPOINT_CACHE_ROLE,
        snapshot::CHECKPOINT_CACHE_FILE,
        CACHE_ONLY_LINK_CHECKPOINT_CACHE,
    )?;
    let mut audit = snapshot::CacheAudit::new(&hour_index);
    let copy_outcome = container.copy_entry_to_unpublished_writer_bounded(
        &cache_entry.path,
        &mut audit,
        snapshot::zip::STREAMING_MAX_CONTAINER_ENTRY_BYTES,
    );
    let verdict = audit.finish().map_err(|error| {
        cache_only_link_refusal(
            CACHE_ONLY_LINK_CHECKPOINT_CACHE,
            format!(
                "the packaged {} cannot be attributed record by record: {error}{}",
                cache_entry.path,
                match &copy_outcome {
                    Err(copy_error) => format!(" (the container also refused it: {copy_error})"),
                    Ok(_) => String::new(),
                }
            ),
        )
    })?;
    snapshot::validate_checkpoint_cache_header(verdict.header(), &attested_sha256).map_err(
        |error| {
            cache_only_link_refusal(
                CACHE_ONLY_LINK_CHECKPOINT_CACHE,
                format!(
                    "the packaged H4CP header is not bound to source {attested_sha256}: {error}"
                ),
            )
        },
    )?;
    let confined_hours = hour_index.confine_damaged_records(&verdict);
    match (&copy_outcome, verdict.is_intact()) {
        (Ok(_), true) => {}
        (Err(_), false) => {}
        (Ok(receipt), false) => {
            return Err(cache_only_link_refusal(
                CACHE_ONLY_LINK_CHECKPOINT_CACHE,
                format!(
                    "the container verified {} against its own manifest digest {}, yet {} of its \
                     records do not match the per-record digests the sealed H4IX publishes; the \
                     two accounts of the same bytes disagree, so neither is trusted",
                    cache_entry.path,
                    receipt.sha256(),
                    verdict.damaged_positions().len(),
                ),
            ))
        }
        (Err(copy_error), true) => {
            return Err(cache_only_link_refusal(
                CACHE_ONLY_LINK_CHECKPOINT_CACHE,
                format!(
                    "the container refused {} ({copy_error}), but every one of its {} records \
                     matches the per-record digest the sealed H4IX publishes; the damage is in \
                     the container's own account of the entry, which serves no single game hour",
                    cache_entry.path,
                    hour_index.hours().len(),
                ),
            ))
        }
    }

    let sealed_cache_sha256 = sealed_artifact_digest(embedded_manifest, "checkpoint_cache_sha256")?;
    if hour_index.cache_sha256() != sealed_cache_sha256 {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_SEALED_ARTIFACTS,
            format!(
                "the packaged H4IX is bound to a {} cache, but the container-authenticated \
                 recorder manifest sealed host_finalize.fold.checkpoint_cache_sha256 \
                 {sealed_cache_sha256}; the index and the seal describe different caches",
                hour_index.cache_sha256(),
            ),
        ));
    }
    let cache_matches_seal = verdict.whole_cache_sha256() == sealed_cache_sha256;
    if !cache_matches_seal && confined_hours.is_empty() {
        return Err(cache_only_link_refusal(
            CACHE_ONLY_LINK_SEALED_ARTIFACTS,
            format!(
                "the packaged {} hashes to {}, but the container-authenticated recorder manifest \
                 sealed host_finalize.fold.checkpoint_cache_sha256 {sealed_cache_sha256}, and no \
                 record of it disagrees with the sealed H4IX; the artifact moved somewhere the \
                 per-record account does not reach",
                cache_entry.path,
                verdict.whole_cache_sha256(),
            ),
        ));
    }

    let details = json!({
        "stream_certification_chain": [
            CACHE_ONLY_LINK_COMPOSITION,
            CACHE_ONLY_LINK_NO_PACKAGED_STREAM,
            CACHE_ONLY_LINK_SOURCE_READABLE,
            CACHE_ONLY_LINK_STREAM_STORAGE,
            CACHE_ONLY_LINK_HOUR_INDEX,
            CACHE_ONLY_LINK_CHECKPOINT_CACHE,
            CACHE_ONLY_LINK_SEALED_ARTIFACTS,
        ],
        "h4cr_stream_digest_bound": false,
        "h4cr_stream_bytes_compared": false,
        "h4cr_identity_is_manifest_attested": true,
        "raw_stream_absence_reason": snapshot::RAW_STREAM_OMITTED_REASON,
        "attested_source_sha256": attested_sha256,
        "attested_source_bytes": attested_bytes,
        "run_dir_source_stream": stream_path.display().to_string(),
        "hour_index_entry": index_entry.path,
        "hour_index_sha256": index_entry.sha256,
        "checkpoint_cache_entry": cache_entry.path,
        "checkpoint_cache_sha256": cache_entry.sha256,
        "checkpoint_cache_matches_seal_whole": cache_matches_seal,
        "checkpoint_cache_damaged_records": verdict.damaged_positions().len(),
        "indexed_hours": hour_index.hours().len(),
        "indexed_hour_indexes": hour_index
            .hours()
            .iter()
            .map(|entry| entry.hour_index())
            .collect::<Vec<i64>>(),
        INDEX_EXCLUDED_HOURS_FIELD: hour_index
            .excluded_hours()
            .iter()
            .map(|hole| json!({
                "hour_index": hole.hour_index(),
                "reason": hole.reason(),
                "h4cr_start_offset": hole.h4cr_start_offset(),
                "h4cr_end_offset": hole.h4cr_end_offset(),
            }))
            .collect::<Vec<Value>>(),
        CACHE_DAMAGE_CONFINED_HOURS_FIELD: confined_hours
            .iter()
            .map(|confined| json!({
                "hour_index": confined.hour_index(),
                "reason": confined.reason(),
                "h4cr_start_offset": confined.h4cr_start_offset(),
                "h4cr_end_offset": confined.h4cr_end_offset(),
                "damaged_record_hour": confined.damaged_record_hour(),
                "checkpoint_hour": confined.checkpoint_hour(),
            }))
            .collect::<Vec<Value>>(),
    });
    Ok((
        StreamCertification::ManifestAttestedThroughDeltaCache {
            attested_source_sha256: attested_sha256,
            attested_source_bytes: attested_bytes,
        },
        details,
    ))
}

fn bind_run_stream_to_certification(
    certification: &StreamCertification,
    artifact: &StreamArtifact,
    errors: &mut Vec<String>,
) {
    let independently_scanned_sha256 = Sha256::hex(&artifact.decoded);
    match certification {
        StreamCertification::ByteComparedAgainstPackagedStream {
            container_stream,
            container_stream_sha256,
        } => {
            if independently_scanned_sha256 != *container_stream_sha256 {
                errors.push(format!(
                    "independently hashed decoded H4CR stream {independently_scanned_sha256} is not bound to verified container entry digest {}",
                    container_stream_sha256
                ));
            }
            if container_stream != &artifact.decoded {
                errors.push(format!(
                    "replay container decoded collector stream differs from decoded run stream (container sha256 {}, run sha256 {})",
                    Sha256::hex(container_stream),
                    artifact.decoded_sha256,
                ));
            }
        }
        StreamCertification::ManifestAttestedThroughDeltaCache {
            attested_source_sha256,
            attested_source_bytes,
        } => {
            if independently_scanned_sha256 != *attested_source_sha256 {
                errors.push(format!(
                    "independently hashed decoded H4CR stream {independently_scanned_sha256} is not the manifest-attested source identity {attested_source_sha256} the delta cache is bound to"
                ));
            }
            if artifact.decoded.len() as u64 != *attested_source_bytes {
                errors.push(format!(
                    "decoded run stream is {} bytes, not the manifest-attested source length {attested_source_bytes} the delta cache is bound to",
                    artifact.decoded.len()
                ));
            }
        }
    }
}

fn single_cache_entry(
    container: &Container,
    role: &str,
    canonical_file: &str,
    link: &str,
) -> Result<snapshot::container::ContainerEntry, String> {
    let entries = container.entries_by_role(role);
    let entry = match entries.as_slice() {
        [entry] => entry.clone(),
        entries => {
            return Err(cache_only_link_refusal(
                link,
                format!(
                    "replay container has {} entries with role {role:?}, expected exactly 1; a \
                     cache-only recording replays from this artifact alone",
                    entries.len()
                ),
            ))
        }
    };
    let expected = format!("payload/{}/{canonical_file}", container.game_id());
    if entry.path != expected {
        return Err(cache_only_link_refusal(
            link,
            format!(
                "role {role:?} is packaged at {:?}, not the canonical {expected:?}",
                entry.path
            ),
        ));
    }
    Ok(entry)
}

fn hash_decoded_source_stream(
    path: &Path,
    compression: compact_stream::StreamCompression,
    attested_bytes: u64,
) -> Result<(String, u64), String> {
    let reader = compact_stream::open_with_compression(path, compression).map_err(|error| {
        format!(
            "open decoded compact stream {} failed: {error}",
            path.display()
        )
    })?;
    let mut reader = reader.take(attested_bytes.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("decode compact stream {} failed: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((snapshot::sha256::to_hex(&hasher.finalize()), total))
}

#[derive(Debug)]
struct ReplayLayout<'a> {
    stream: Option<&'a str>,
    session_manifest: &'a str,
}

fn replay_layout(
    manifest: &Value,
    packaging: snapshot::RawStreamPackaging,
) -> Result<ReplayLayout<'_>, String> {
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
    let stream = match packaging {
        snapshot::RawStreamPackaging::Packaged => {
            let stream = string("stream")?;
            validate_relative_path(stream, "replay_entries.stream")?;
            Some(stream)
        }
        snapshot::RawStreamPackaging::Omitted => {
            if entries.contains_key("stream") {
                return Err(format!(
                    "manifest declares the {:?} composition but replay_entries.stream still names a packaged entry",
                    snapshot::RawStreamPackaging::Omitted.as_str()
                ));
            }
            None
        }
    };
    let session_manifest = string("session_manifest")?;
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
    #[cfg(test)]
    let _scanner_guard = portable_test_fs_retry::open(path).ok();
    // storage prefix was released after the checkpoint cache took ownership of
    recorder_host::spool_trim::require_whole_spool(path, "compact replay verification gate")?;
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
    domain_gap_record_count: u64,
    frame_end_record_count: u64,
    total_payload_bytes: u64,
    max_payload_len: u64,
    max_record_len: u64,
    framed_max_payload_len: u64,
    framed_max_record_len: u64,
    global_status_flags_or: u32,
    per_domain_status_flags_or: BTreeMap<u16, u32>,
    per_domain_perf_authority: BTreeMap<u16, PerfAuthority>,
    per_domain_data_authority: BTreeMap<u16, DataAuthority>,
    per_domain_combined_units: BTreeMap<u16, BTreeMap<String, u64>>,
    per_hour_perf_authority: BTreeMap<(i32, u16), PerfAuthority>,
    per_hour_data_authority: BTreeMap<(i32, u16), DataAuthority>,
    per_hour_gap_authority: BTreeMap<(i32, u16), Vec<compact_contract::DomainGapRecordV1>>,
    domain57_frames: Vec<recorder_host::domain_data_authority::Domain57FrameEvidence>,
    session_header_identity: recorder_host::campaign_identity::SessionHeaderIdentityObservation,
    validated_cid1: Option<ValidatedCid1>,
    frame_status_flags_or_raw: u32,
    per_hour_frame_authority: BTreeMap<i32, FrameEndAuthority>,
    domains_seen: BTreeSet<u16>,
    frame_end_hour_counts: BTreeMap<i32, u64>,
    per_hour_perf_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    per_hour_data_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    per_hour_gap_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    unique_hours: Vec<i32>,
    skipped_hours: i32,
    parse_errors: Vec<String>,
    ordering_errors: Vec<String>,
    freeze_census_records: Vec<(
        u64,
        compact_contract::freeze_census_record::FreezeCensusRecordV1,
    )>,
}

fn validate_freeze_census_stream_authority(
    summary: &HeaderScanSummary,
    raw: &[u8],
    manifest: &Value,
    errors: &mut Vec<String>,
) {
    let mut hasher = Sha256::new();
    hasher.update(raw);
    let decoded_sha256 = snapshot::sha256::to_hex(&hasher.finalize());
    let derived = freeze_census_manifest_from_scan(summary, &decoded_sha256);
    match manifest.get("freeze_census") {
        Some(declared) if declared == &derived => {}
        Some(declared) => errors.push(format!(
            "freeze_census manifest section does not equal the independently decoded sealed H4CR authority: declared={declared} derived={derived}"
        )),
        None => errors.push(
            "recorder manifest is missing the stream-derived freeze_census section".to_string(),
        ),
    }
}

fn freeze_census_manifest_from_scan(summary: &HeaderScanSummary, decoded_sha256: &str) -> Value {
    use compact_contract::freeze_census_record::{FreezeCensusPhase, FreezeCensusQuality};
    let quality_json = |quality: FreezeCensusQuality| {
        json!({
            "bits": quality.bits(),
            "attempts_unrepresented": quality.attempts_unrepresented(),
            "census_overflowed_before_phase": quality.census_overflowed_before_phase(),
            "census_dropped_during_phase": quality.census_dropped_during_phase(),
            "process_thread_attempts_differ": quality.process_thread_attempts_differ(),
            "counter_reconciliation_issue": quality.counter_reconciliation_issue(),
            "encoder_bound_truncated": quality.encoder_bound_truncated(),
        })
    };
    let mut rows_present = 0u64;
    let mut attempts_unrepresented = 0u64;
    let mut window_sum = 0u64;
    let mut acquire_sum = 0u64;
    let mut minhook_sum = 0u64;
    let mut refused_rows = 0u64;
    let mut install_records = 0usize;
    let mut problems = Vec::<String>::new();
    let records = summary
        .freeze_census_records
        .iter()
        .map(|(sequence, record)| {
            if record.phase() == FreezeCensusPhase::Install {
                install_records += 1;
            }
            rows_present = rows_present.saturating_add(record.rows().len() as u64);
            attempts_unrepresented =
                attempts_unrepresented.saturating_add(record.attempts_unrepresented_total());
            window_sum = window_sum.saturating_add(record.present_window_us_sum());
            acquire_sum = acquire_sum.saturating_add(record.present_acquire_us_sum());
            minhook_sum = minhook_sum.saturating_add(record.present_minhook_us_sum());
            refused_rows = refused_rows.saturating_add(record.present_refused_row_count());
            let quality = record.quality();
            if record.attempts_unrepresented_total() != 0 {
                problems.push(format!(
                    "sequence {sequence} leaves {} freeze attempt(s) without a carried row",
                    record.attempts_unrepresented_total()
                ));
            }
            if quality.census_overflowed_before_phase() {
                problems.push(format!(
                    "sequence {sequence} began after the process census had already overflowed"
                ));
            }
            if quality.census_dropped_during_phase() {
                problems.push(format!(
                    "sequence {sequence} dropped census rows during the phase"
                ));
            }
            if quality.encoder_bound_truncated() {
                problems.push(format!(
                    "sequence {sequence} was truncated by the H4FC row or label bound"
                ));
            }
            if quality.counter_reconciliation_issue() {
                problems.push(format!(
                    "sequence {sequence} has a named census-counter reconciliation issue"
                ));
            }
            if record.has_missing_non_refused_labels() {
                problems.push(format!(
                    "sequence {sequence} carries at least one non-refused row without a label"
                ));
            }
            let rows = record
                .rows()
                .iter()
                .map(|row| {
                    json!({
                        "ordinal": row.ordinal,
                        "entry_point": row.entry_point.map(|entry| entry.canonical_name()),
                        "entry_point_code": row.entry_point.map(|entry| entry as u8),
                        "label": row.label.as_str(),
                        "target_count": row.target_count,
                        "window_us_upper_bound": row.window_us,
                        "acquire_us": row.acquire_us,
                        "minhook_us": row.minhook_us,
                        "passes": row.passes,
                        "refused": row.refused,
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "sequence": sequence,
                "phase": record.phase().canonical_name(),
                "quality": quality_json(quality),
                "attempt_ordinal_start_exclusive": record.attempt_ordinal_start_exclusive(),
                "attempt_ordinal_end_inclusive": record.attempt_ordinal_end_inclusive(),
                "process_attempts_total": record.process_attempts_total(),
                "producer_thread_attempts_total": record.producer_thread_attempts_total(),
                "process_refusals_before": record.process_refusals_before(),
                "process_refusals_after": record.process_refusals_after(),
                "process_refusals_total": record.process_refusals_total(),
                "census_recorded_before": record.census_recorded_before(),
                "census_recorded_after": record.census_recorded_after(),
                "census_dropped_before": record.census_dropped_before(),
                "census_dropped_after": record.census_dropped_after(),
                "census_dropped_delta": record.census_dropped_delta(),
                "attempts_unrepresented_total": record.attempts_unrepresented_total(),
                "present_window_us_upper_bound_sum": record.present_window_us_sum(),
                "present_acquire_us_sum": record.present_acquire_us_sum(),
                "present_minhook_us_sum": record.present_minhook_us_sum(),
                "present_refused_rows": record.present_refused_row_count(),
                "rows": rows,
            })
        })
        .collect::<Vec<_>>();
    if summary.freeze_census_records.is_empty() {
        problems
            .push("freeze census telemetry is absent from this sealed logical stream".to_string());
    } else if install_records == 0 {
        problems.push("no install-phase freeze census record is present".to_string());
    } else if install_records > 1 {
        problems.push(format!(
            "{install_records} install-phase freeze census records are present; expected one"
        ));
    }
    let phase_section = |phase: FreezeCensusPhase, problems: &mut Vec<String>| {
        let mut record_count = 0u64;
        let mut phase_rows = 0u64;
        let mut phase_unrepresented = 0u64;
        let mut phase_window = 0u64;
        let mut phase_acquire = 0u64;
        let mut phase_minhook = 0u64;
        let mut phase_refused = 0u64;
        let mut lossy = false;
        let mut sequences = Vec::<u64>::new();
        for (sequence, record) in summary
            .freeze_census_records
            .iter()
            .filter(|(_, record)| record.phase() == phase)
        {
            let quality = record.quality();
            record_count += 1;
            sequences.push(*sequence);
            phase_rows = phase_rows.saturating_add(record.rows().len() as u64);
            phase_unrepresented =
                phase_unrepresented.saturating_add(record.attempts_unrepresented_total());
            phase_window = phase_window.saturating_add(record.present_window_us_sum());
            phase_acquire = phase_acquire.saturating_add(record.present_acquire_us_sum());
            phase_minhook = phase_minhook.saturating_add(record.present_minhook_us_sum());
            phase_refused = phase_refused.saturating_add(record.present_refused_row_count());
            lossy |= quality.census_dropped_during_phase()
                || quality.encoder_bound_truncated()
                || quality.census_overflowed_before_phase()
                || quality.counter_reconciliation_issue();
        }
        let status = if record_count == 0 {
            "absent"
        } else if record_count == 1 && phase_unrepresented == 0 && !lossy {
            "complete"
        } else {
            "partial"
        };
        if record_count == 0 {
            problems.push(format!(
                "no {}-phase freeze census record is present; those freeze durations were not recorded in this logical stream",
                phase.canonical_name()
            ));
        } else if record_count > 1 {
            problems.push(format!(
                "{record_count} {}-phase freeze census records are present; expected one",
                phase.canonical_name()
            ));
        }
        json!({
            "status": status,
            "record_count": record_count,
            "sequences": sequences,
            "rows_present": phase_rows,
            "attempts_unrepresented": phase_unrepresented,
            "present_window_us_upper_bound_sum": phase_window,
            "present_acquire_us_sum": phase_acquire,
            "present_minhook_us_sum": phase_minhook,
            "present_refused_rows": phase_refused,
        })
    };
    let install_phase = phase_section(FreezeCensusPhase::Install, &mut problems);
    let release_phase = phase_section(FreezeCensusPhase::Release, &mut problems);
    let phase_complete = |phase: &Value| phase.get("status") == Some(&json!("complete"));
    let status = if summary.freeze_census_records.is_empty() {
        "absent"
    } else if phase_complete(&install_phase) && phase_complete(&release_phase) {
        "complete"
    } else {
        "partial"
    };
    json!({
        "schema": "hoi4.freeze-census.v2",
        "source": {
            "kind": "sealed_h4cr",
            "record_type": compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            "decoded_stream_sha256": decoded_sha256,
        },
        "status": status,
        "duration_semantics": "every *_window_us_upper_bound_* value is an upper bound on stopped time, not a stopwatch reading: the census clock opens before the first suspension and closes after the last handle is released",
        "phases": {
            "install": install_phase,
            "release": release_phase,
        },
        "recording_totals": {
            "record_count": summary.freeze_census_records.len(),
            "rows_present": rows_present,
            "attempts_unrepresented": attempts_unrepresented,
            "present_window_us_upper_bound_sum": window_sum,
            "present_acquire_us_sum": acquire_sum,
            "present_minhook_us_sum": minhook_sum,
            "present_refused_rows": refused_rows,
        },
        "records": records,
        "problems": problems,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GapSequenceAuthority {
    next_ordinal: u64,
    last_range_end: i32,
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
    let mut gap_sequences = BTreeMap::<(u16, u16), GapSequenceAuthority>::new();
    let mut final_record_is_clean_world_boundary = false;
    let mut release_epilogue_seen = false;
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
        let awaiting_identity =
            identity_mode == StreamIdentityMode::IdentityV1 && summary.validated_cid1.is_none();
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
        let previous_physical_was_clean_world_boundary = final_record_is_clean_world_boundary;
        final_record_is_clean_world_boundary = false;
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
            Some(Err(error)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: stateful stream validation failed before dispatch: {error}"
                ));
            }
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
        if record_type == compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS {
            summary.record_count += 1;
            summary.total_payload_bytes = summary
                .total_payload_bytes
                .saturating_add(payload_len as u64);
            summary.max_payload_len = summary.max_payload_len.max(payload_len as u64);
            summary.max_record_len = summary.max_record_len.max(record_len as u64);
            if envelope_ok {
                match compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload)
                {
                    Ok(record) => {
                        let phase = record.phase();
                        let release = phase
                            == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                        summary.freeze_census_records.push((sequence, record));
                        final_record_is_clean_world_boundary = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(
                            previous_physical_was_clean_world_boundary,
                            phase,
                            release_epilogue_seen,
                        );
                        release_epilogue_seen |= release;
                    }
                    Err(error) => summary.parse_errors.push(format!(
                        "offset {offset}: FREEZE_CENSUS self-verification failed: {error}"
                    )),
                }
            }
            offset = offset.saturating_add(record_len);
            continue;
        }
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
        let known_record_type = match compact_contract::KnownRecordType::try_from(record_type) {
            Ok(record_type) => record_type,
            Err(unknown) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unknown record type {}",
                    unknown.0
                ));
                offset = offset.saturating_add(record_len);
                continue;
            }
        };
        match known_record_type {
            compact_contract::KnownRecordType::DomainGap => {
                summary.domain_gap_record_count += 1;
                *summary
                    .per_hour_gap_domain_counts
                    .entry(game_hour_raw)
                    .or_default()
                    .entry(domain_id)
                    .or_default() += 1;
                match compact_contract::DomainGapRecordV1::decode(payload) {
                    Ok(gap) => {
                        if gap.target_hour_raw != game_hour_raw {
                            summary.parse_errors.push(format!(
                                "offset {offset}: DOMAIN_GAP header hour {game_hour_raw} does not equal target_hour_raw {}",
                                gap.target_hour_raw
                            ));
                        }
                        let key = (domain_id, gap.part.code());
                        let expected_ordinal = gap_sequences
                            .get(&key)
                            .map_or(0, |state| state.next_ordinal);
                        if gap.outcome_ordinal != expected_ordinal {
                            summary.parse_errors.push(format!(
                                "offset {offset}: DOMAIN_GAP domain {domain_id} part {} outcome_ordinal {} != zero-based contiguous expected {expected_ordinal}",
                                gap.part.code(),
                                gap.outcome_ordinal
                            ));
                        }
                        if let Some(prior) = gap_sequences.get(&key) {
                            if gap.range_start_hour_raw <= prior.last_range_end {
                                summary.parse_errors.push(format!(
                                    "offset {offset}: DOMAIN_GAP domain {domain_id} part {} range {}..={} overlaps or regresses after prior end {}",
                                    gap.part.code(),
                                    gap.range_start_hour_raw,
                                    gap.range_end_hour_raw,
                                    prior.last_range_end
                                ));
                            }
                        }
                        match gap.outcome_ordinal.checked_add(1) {
                            Some(next_ordinal) => {
                                gap_sequences.insert(
                                    key,
                                    GapSequenceAuthority {
                                        next_ordinal,
                                        last_range_end: gap.range_end_hour_raw,
                                    },
                                );
                            }
                            None => summary.parse_errors.push(format!(
                                "offset {offset}: DOMAIN_GAP outcome ordinal is exhausted"
                            )),
                        }
                        summary
                            .per_hour_gap_authority
                            .entry((game_hour_raw, domain_id))
                            .or_default()
                            .push(gap);
                    }
                    Err(error) => summary.parse_errors.push(format!(
                        "offset {offset}: DOMAIN_GAP failed independent H4DG decode: {error}"
                    )),
                }
                match open_frame_hour {
                    Some(open) if open != game_hour_raw => summary.parse_errors.push(format!(
                        "offset {offset}: DOMAIN_GAP hour {game_hour_raw} does not match open frame hour {open}"
                    )),
                    Some(_) => {}
                    None => open_frame_hour = Some(game_hour_raw),
                }
            }
            compact_contract::KnownRecordType::DomainPerf => {
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
            compact_contract::KnownRecordType::DomainData => {
                summary.domain_data_record_count += 1;
                *summary
                    .per_hour_data_domain_counts
                    .entry(game_hour_raw)
                    .or_default()
                    .entry(domain_id)
                    .or_default() += 1;
                let data_valid = match recorder_host::stream_fold::validate_domain_data_payload_at(
                    domain_id,
                    payload,
                    header.record_flags,
                    sequence,
                    game_hour_raw,
                    offset as u64,
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
            compact_contract::KnownRecordType::FrameEnd => {
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
                    let observed_gap_records = summary
                        .per_hour_gap_domain_counts
                        .get(&game_hour_raw)
                        .map(|domains| domains.values().copied().sum::<u64>())
                        .unwrap_or(0);
                    let observed_terminal_records =
                        observed_perf_records.saturating_add(observed_gap_records);
                    if u64::from(authority.emitted_domains) != observed_terminal_records {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END emitted_domains {} != independently observed terminal record count {observed_terminal_records} (DOMAIN_PERF {observed_perf_records} + H4DG {observed_gap_records})",
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
                            "offset {offset}: FRAME_END hour {game_hour_raw} has no open terminal frame"
                        ));
                    }
                }
                if let Some(closed) = last_closed_hour {
                    match game_hour_raw.cmp(&closed) {
                        std::cmp::Ordering::Greater => {}
                        _ => {
                            clean = false;
                            summary.parse_errors.push(format!(
                                "offset {offset}: closed frame hour {game_hour_raw} does not strictly advance after {closed}"
                            ));
                        }
                    }
                }
                if clean {
                    open_frame_hour = None;
                    pending_perf_hour = None;
                    last_closed_hour = Some(game_hour_raw);
                    final_record_is_clean_world_boundary = true;
                }
            }
            compact_contract::KnownRecordType::FrameRefused => {
                let mut clean = envelope_ok;
                match compact_contract::frame_refusal::FrameRefusalRecordV2::decode(payload) {
                    Ok(record) if record.refused_hour == game_hour_raw => {}
                    Ok(record) => {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_REFUSED envelope hour {game_hour_raw} disagrees with payload hour {}",
                            record.refused_hour
                        ));
                    }
                    Err(error) => {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_REFUSED failed independent decode: {error}"
                        ));
                    }
                }
                match open_frame_hour.or(pending_perf_hour) {
                    Some(open) if open == game_hour_raw => {}
                    Some(open) => {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_REFUSED hour {game_hour_raw} does not close open frame hour {open}"
                        ));
                    }
                    None => {}
                }
                if last_closed_hour.is_some_and(|closed| game_hour_raw <= closed) {
                    clean = false;
                    summary.parse_errors.push(format!(
                        "offset {offset}: FRAME_REFUSED hour {game_hour_raw} does not strictly advance"
                    ));
                }
                if clean {
                    open_frame_hour = None;
                    pending_perf_hour = None;
                    last_closed_hour = Some(game_hour_raw);
                    final_record_is_clean_world_boundary = true;
                }
            }
            compact_contract::KnownRecordType::FreezeCensus
            | compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StateTopologyCensus
            | compact_contract::KnownRecordType::StreamHeader => {}
            compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => {
                summary.parse_errors.push(format!(
                    "offset {offset}: record type {} is outside the independent world-scan grammar",
                    known_record_type.canonical_name()
                ))
            }
        }
        offset = offset.saturating_add(record_len);
    }
    if let Some(open) = open_frame_hour {
        summary
            .parse_errors
            .push(format!("stream ended with open terminal frame hour {open}"));
    }
    if framing_complete && !raw.is_empty() && !final_record_is_clean_world_boundary {
        summary.parse_errors.push(
            "stream is not closed by a final canonical FRAME_END or FRAME_REFUSED boundary"
                .to_string(),
        );
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
    let expected_set = expected_domains.iter().copied().collect::<BTreeSet<_>>();
    let frame_hours: BTreeSet<i32> = summary.frame_end_hour_counts.keys().copied().collect();
    let mut all_record_hours = frame_hours.clone();
    all_record_hours.extend(summary.per_hour_perf_domain_counts.keys().copied());
    all_record_hours.extend(summary.per_hour_data_domain_counts.keys().copied());
    all_record_hours.extend(summary.per_hour_gap_domain_counts.keys().copied());

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
        let observed_terminal_set = summary
            .per_hour_perf_domain_counts
            .get(hour)
            .into_iter()
            .flat_map(|domains| domains.keys().copied())
            .chain(
                summary
                    .per_hour_gap_domain_counts
                    .get(hour)
                    .into_iter()
                    .flat_map(|domains| domains.keys().copied()),
            )
            .filter(|domain| *domain != RC_PROBE_DOMAIN_ID)
            .collect::<BTreeSet<_>>();
        if observed_terminal_set != expected_set {
            errors.push(format!(
                "game_hour_raw {hour}: independently observed terminal roster {observed_terminal_set:?} != exact scheduled roster {expected_set:?}"
            ));
        }
        let observed_data_set = summary
            .per_hour_data_domain_counts
            .get(hour)
            .into_iter()
            .flat_map(|domains| domains.keys().copied())
            .collect::<BTreeSet<_>>();
        let unexpected_data = observed_data_set
            .difference(&expected_set)
            .copied()
            .collect::<Vec<_>>();
        if !unexpected_data.is_empty() {
            errors.push(format!(
                "game_hour_raw {hour}: DATA records contain unscheduled domains {unexpected_data:?}"
            ));
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
            let gap_count = summary
                .per_hour_gap_domain_counts
                .get(hour)
                .and_then(|domains| domains.get(expected))
                .copied()
                .unwrap_or(0);
            if !matches!((perf_count, gap_count), (1, 0) | (0, 1)) {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} must have exactly one terminal outcome PERF XOR H4DG, observed PERF={perf_count} H4DG={gap_count}",
                    contract.name
                ));
            }
            let data_count = summary
                .per_hour_data_domain_counts
                .get(hour)
                .and_then(|domains| domains.get(expected))
                .copied()
                .unwrap_or(0);
            let expected_data_count = if perf_count == 1 && contract.requires_data {
                1
            } else {
                0
            };
            if data_count != expected_data_count {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} DATA is nonterminal and must be present exactly once only with an accepted PERF outcome; observed DATA={data_count}, expected {expected_data_count}",
                    contract.name
                ));
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
            let data_count = domain_counts.get(expected).copied().unwrap_or(0);
            if data_count != 0 {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} data_record_count {data_count} on non-frame hour",
                    contract.name
                ));
            }
        }
    }
    for (hour, domain_counts) in &summary.per_hour_gap_domain_counts {
        if frame_hours.contains(hour) {
            continue;
        }
        for (domain, gap_count) in domain_counts {
            if *gap_count != 0 {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} H4DG record_count {gap_count} on non-frame hour",
                    domain_name(*domain)
                ));
            }
        }
    }

    for pair in summary.unique_hours.windows(2) {
        let previous = pair[0];
        let next = pair[1];
        if next <= previous.saturating_add(1) {
            continue;
        }
        let Some(required_start) = previous.checked_add(1) else {
            errors.push(format!(
                "FRAME_END discontinuity after exhausted hour {previous} cannot be represented"
            ));
            continue;
        };
        for domain in expected_domains {
            let matching = summary
                .per_hour_gap_authority
                .get(&(next, *domain))
                .into_iter()
                .flat_map(|values| values.iter())
                .filter(|gap| {
                    gap.range_start_hour_raw == required_start
                        && gap.range_end_hour_raw == next
                        && gap.target_hour_raw == next
                })
                .count();
            if matching != 1 {
                errors.push(format!(
                    "FRAME_END discontinuity {previous}->{next}: domain {} must have one H4DG with exact range {required_start}..={next}, observed {matching}",
                    domain_name(*domain)
                ));
            }
        }
    }

    for ((target_hour, domain), gaps) in &summary.per_hour_gap_authority {
        for gap in gaps {
            for closed_hour in summary.unique_hours.iter().copied().filter(|hour| {
                gap.range_start_hour_raw <= *hour
                    && *hour <= gap.range_end_hour_raw
                    && *hour != *target_hour
            }) {
                let perf = summary
                    .per_hour_perf_domain_counts
                    .get(&closed_hour)
                    .and_then(|domains| domains.get(domain))
                    .copied()
                    .unwrap_or(0);
                let other_gap = summary
                    .per_hour_gap_domain_counts
                    .get(&closed_hour)
                    .and_then(|domains| domains.get(domain))
                    .copied()
                    .unwrap_or(0);
                if perf != 0 || other_gap != 0 {
                    errors.push(format!(
                        "H4DG domain {} target {target_hour} range {}..={} overlaps closed terminal outcome at hour {closed_hour}",
                        domain_name(*domain),
                        gap.range_start_hour_raw,
                        gap.range_end_hour_raw
                    ));
                }
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

    /// One release-phase H4FC payload, bounded and self-consistent.
    fn release_census_payload() -> Vec<u8> {
        use compact_contract::freeze_census_record::{
            FreezeCensusCounterSnapshot, FreezeCensusPhase, FreezeCensusRecordV1,
        };
        FreezeCensusRecordV1::encode_bounded(
            FreezeCensusPhase::Release,
            FreezeCensusCounterSnapshot::default(),
            FreezeCensusCounterSnapshot::default(),
            &[],
        )
        .unwrap()
    }

    fn append_release_census(raw: &mut Vec<u8>, sequence: u64) {
        append_record(
            raw,
            sequence,
            0,
            DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            &release_census_payload(),
        );
    }

    fn one_closed_hour() -> Vec<u8> {
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
        raw
    }

    /// The scanner must hand each DOMAIN_DATA payload the header it arrived
    /// under, not a placeholder.
    ///
    /// d57 cross-checks its payload hour against the enclosing H4CR hour, so a
    /// call site that substitutes `0` rejects every well-formed d57 record and
    /// then never decodes it into `domain57_frames`. That is not theoretical:
    /// lane OB's live capture had all 366 of its d57 records excluded as
    /// `d57 payload hour <hour> != H4CR hour 0`, which emptied `domain57_frames`
    /// and failed the seal gate with `d57_record_count_per_closed_frame_invalid`,
    /// producing a recovery-only replay the viewer refuses to open.
    ///
    /// This pins the CALL SITE rather than the validator: the record below is
    /// well formed, so the only way to fail is for the scan to validate it
    /// against something other than its own header.
    ///
    /// kills: passing a literal `0` (or any placeholder) for `game_hour_raw` at
    /// the `validate_domain_data_payload_at` call in `scan_compact_stream`.
    #[test]
    fn the_scan_validates_a_domain_data_payload_against_its_own_header_hour() {
        use session_multiplayer_metadata_live as session57;

        let hour = 100i32;
        let payload = session57::encode_payload(&session57::SessionMetadataFrame {
            game_hour_raw: hour,
            status_flags: 0,
            source_flags: 0,
            schema_flags: 0,
            root_evidence_mask: 0,
            world_tension_fixed5: None,
            world_tension_details: None,
            player_country_rows: Vec::new(),
            event_rows: Vec::new(),
        });

        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            hour,
            session57::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut raw,
            2,
            hour,
            session57::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        append_record(
            &mut raw,
            3,
            hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        assert!(
            summary.parse_errors.is_empty(),
            "a d57 record whose payload hour equals its header hour must scan clean, got: {:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.domain57_frames.len(),
            1,
            "the scan must decode the d57 record it just accepted"
        );
        assert_eq!(summary.domain57_frames[0].header_hour_raw, hour);
        assert_eq!(summary.domain57_frames[0].payload_hour_raw, hour);
    }

    /// release census behind the final FRAME_END, and EOF.
    #[test]
    fn the_gate_accepts_one_release_census_after_the_final_boundary() {
        let mut raw = one_closed_hour();
        append_release_census(&mut raw, 4);
        let summary = scan_compact_stream(&raw);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.freeze_census_records.len(), 1);
    }

    #[test]
    fn the_gate_refuses_every_other_shape_after_the_final_boundary() {
        let mut two_censuses = one_closed_hour();
        append_release_census(&mut two_censuses, 4);
        append_release_census(&mut two_censuses, 5);
        assert!(
            !scan_compact_stream(&two_censuses).parse_errors.is_empty(),
            "a second release census was accepted"
        );

        let mut census_then_data = one_closed_hour();
        append_release_census(&mut census_then_data, 4);
        append_record(
            &mut census_then_data,
            5,
            101,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        assert!(
            !scan_compact_stream(&census_then_data)
                .parse_errors
                .is_empty(),
            "a record after the epilogue was accepted"
        );

        let mut data_after_end = one_closed_hour();
        append_record(
            &mut data_after_end,
            4,
            101,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        assert!(
            !scan_compact_stream(&data_after_end).parse_errors.is_empty(),
            "an ordinary record was allowed to close the stream"
        );

        let mut no_boundary = Vec::new();
        append_record(
            &mut no_boundary,
            1,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_release_census(&mut no_boundary, 2);
        assert!(
            !scan_compact_stream(&no_boundary).parse_errors.is_empty(),
            "a release census without a preceding closed hour was accepted"
        );

        let mut oversize = one_closed_hour();
        let mut payload = release_census_payload();
        payload.resize(
            compact_contract::freeze_census_record::FREEZE_CENSUS_MAX_PAYLOAD_LEN + 1,
            0,
        );
        append_record(
            &mut oversize,
            4,
            0,
            DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            &payload,
        );
        assert!(
            !scan_compact_stream(&oversize).parse_errors.is_empty(),
            "an oversize release census was accepted"
        );

        let mut hour_bearing = one_closed_hour();
        append_record(
            &mut hour_bearing,
            4,
            100,
            DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            &release_census_payload(),
        );
        assert!(
            !scan_compact_stream(&hour_bearing).parse_errors.is_empty(),
            "a release census carrying a game hour was accepted"
        );
    }

    #[test]
    fn freeze_census_manifest_join_uses_independent_stream_authority() {
        use compact_contract::campaign_identity::{
            encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
        };
        use compact_contract::freeze_census_record::{
            FreezeCensusCounterSnapshot, FreezeCensusPhase, FreezeCensusRecordV1,
        };
        let hour = compact_contract::calendar::DATE_EPOCH_CONST + (1936 * 365 * 24);
        let observation = Cid1Observation {
            game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_string(),
            game_unique_seed: 7,
            game_hour_raw: hour,
        };
        let fields = Cid1Fields::from_stable_observations(
            observation.clone(),
            observation,
            "gate-test".to_string(),
            Cid1Witnesses::default(),
        )
        .unwrap();
        let cid1 = encode_cid1(&fields).unwrap();
        let counters = FreezeCensusCounterSnapshot {
            process_attempts: 4,
            producer_thread_attempts: 4,
            process_refusals: 0,
            census_recorded: 4,
            census_dropped: 0,
        };
        let census = FreezeCensusRecordV1::encode_bounded(
            FreezeCensusPhase::Install,
            counters,
            counters,
            &[],
        )
        .unwrap();
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            0,
            hour,
            COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            &cid1,
        );
        append_record(
            &mut raw,
            1,
            0,
            compact_contract::COMPACT_DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            &census,
        );
        let summary = scan_compact_stream_with_identity_mode(&raw, StreamIdentityMode::IdentityV1);
        assert_eq!(summary.freeze_census_records.len(), 1);
        let mut hasher = Sha256::new();
        hasher.update(&raw);
        let hash = snapshot::sha256::to_hex(&hasher.finalize());
        let declared = freeze_census_manifest_from_scan(&summary, &hash);
        let manifest = json!({"freeze_census": declared});
        let mut errors = Vec::new();
        validate_freeze_census_stream_authority(&summary, &raw, &manifest, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");

        let mut contradictory = manifest;
        contradictory["freeze_census"]["status"] = json!("absent");
        validate_freeze_census_stream_authority(&summary, &raw, &contradictory, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("independently decoded sealed H4CR"));
    }

    #[test]
    fn lane_local_badges_are_not_unknown_bits_for_their_own_domain() {
        let d53_word = 0x0053_0200;
        assert_eq!(super::domain_status_unknown_bits(53, d53_word), 0);
        let d60_word = (1 << 16) | (1 << 17);
        assert_eq!(super::domain_status_unknown_bits(60, d60_word), 0);
        assert_eq!(super::domain_status_unknown_bits(52, d53_word), 0x0053_0000);
        assert_eq!(super::domain_status_unknown_bits(61, d60_word), d60_word);
        assert_eq!(super::domain_status_unknown_bits(53, 1 << 30), 1 << 30);
        assert_eq!(super::domain_status_unknown_bits(60, 1 << 23), 1 << 23);
    }

    const TEST_MAX_STREAM_BYTES: u64 = 128 * 1024 * 1024;

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
            per_hour_perf_domain_counts: BTreeMap::from([(100, BTreeMap::from([(57, 1)]))]),
            domain57_frames: vec![
                recorder_host::domain_data_authority::Domain57FrameEvidence {
                    sequence: 2,
                    header_hour_raw: 100,
                    payload_hour_raw: 100,
                    root_evidence_mask: 0x7,
                    speed_rows: vec![
                        recorder_host::domain_data_authority::Domain57ScalarEvidence {
                            event_hour_raw: 100,
                            value_i32: 4,
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
    fn gap_acceptance_profiles_are_explicit_and_zero_loss_is_opt_in() {
        let defaults = GateOptions::default();
        assert_eq!(
            defaults.gap_acceptance,
            GapAcceptanceProfile::TypedGapsAllowed
        );
        assert_eq!(defaults.gap_acceptance.as_str(), "typed_gaps_allowed");
        assert_eq!(
            GapAcceptanceProfile::ZeroLossRequired.as_str(),
            "require_zero_loss"
        );
    }

    #[test]
    fn manifest_domains_seen_is_bound_to_independent_h4cr_roster() {
        let summary = HeaderScanSummary {
            domains_seen: BTreeSet::from([10, compact_contract::COMPACT_DOMAIN_FRAME]),
            ..HeaderScanSummary::default()
        };
        let mut errors = Vec::new();
        validate_scanned_domains_seen(
            &summary,
            &json!({"domains_seen": ["fuel_live", "frame"]}),
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        validate_scanned_domains_seen(&summary, &json!({"domains_seen": ["frame"]}), &mut errors);
        assert!(errors
            .iter()
            .any(|error| error.contains("independent stream domains_seen")));

        let mut malformed_errors = Vec::new();
        validate_scanned_domains_seen(
            &summary,
            &json!({"domains_seen": ["fuel_live", "frame", "frame", 10]}),
            &mut malformed_errors,
        );
        assert!(malformed_errors
            .iter()
            .any(|error| error.contains("duplicate")));
        assert!(malformed_errors
            .iter()
            .any(|error| error.contains("not a string")));
    }

    fn canonical_gap_hour(offset: i32) -> i32 {
        compact_contract::calendar::DATE_EPOCH_CONST + (1936 * 365 * 24) + offset
    }

    fn gap_payload(target: i32, range_start: i32, range_end: i32, ordinal: u64) -> Vec<u8> {
        let width = u64::try_from(i64::from(range_end) - i64::from(range_start) + 1).unwrap();
        let (reason, tick_before, tick_after) = if range_start == range_end {
            (compact_contract::DomainGapReason::Torn, 100, 101)
        } else {
            (compact_contract::DomainGapReason::Overrun, 100, 100 + width)
        };
        compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason,
            target_hour_raw: target,
            range_start_hour_raw: range_start,
            range_end_hour_raw: range_end,
            tick_before,
            tick_after,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: ordinal,
        }
        .encode()
        .unwrap()
    }

    #[test]
    fn header_scan_accepts_gap_only_and_mixed_perf_gap_frames() {
        let hour = canonical_gap_hour(0);
        let mut gap_only = Vec::new();
        append_record(
            &mut gap_only,
            1,
            hour,
            10,
            RECORD_TYPE_DOMAIN_GAP,
            &gap_payload(hour, hour, hour, 0),
        );
        append_record(
            &mut gap_only,
            2,
            hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload_with_fields(1, 1, 0, 0),
        );
        let gap_summary = scan_compact_stream(&gap_only);
        assert!(
            gap_summary.parse_errors.is_empty(),
            "{:?}",
            gap_summary.parse_errors
        );
        assert_eq!(gap_summary.domain_gap_record_count, 1);
        assert!(compact_domain_coverage_errors(&gap_summary, &[10]).is_empty());

        let mut mixed = Vec::new();
        append_record(
            &mut mixed,
            1,
            hour,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut mixed,
            2,
            hour,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut mixed,
            3,
            hour,
            12,
            RECORD_TYPE_DOMAIN_GAP,
            &gap_payload(hour, hour, hour, 0),
        );
        append_record(
            &mut mixed,
            4,
            hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload_with_fields(2, 2, 0, 0),
        );
        let mixed_summary = scan_compact_stream(&mixed);
        assert!(
            mixed_summary.parse_errors.is_empty(),
            "{:?}",
            mixed_summary.parse_errors
        );
        assert!(
            compact_domain_coverage_errors(&mixed_summary, &[10, 12]).is_empty(),
            "{:?}",
            compact_domain_coverage_errors(&mixed_summary, &[10, 12])
        );
    }

    #[test]
    fn header_scan_rejects_h4dg_ordinal_holes_overlap_and_target_mismatch() {
        let hour = canonical_gap_hour(0);

        let mut ordinal_hole = Vec::new();
        append_record(
            &mut ordinal_hole,
            1,
            hour,
            10,
            RECORD_TYPE_DOMAIN_GAP,
            &gap_payload(hour, hour, hour, 1),
        );
        let summary = scan_compact_stream(&ordinal_hole);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("zero-based contiguous expected 0")));

        let mut overlap = Vec::new();
        append_record(
            &mut overlap,
            1,
            hour,
            10,
            RECORD_TYPE_DOMAIN_GAP,
            &gap_payload(hour, hour, hour, 0),
        );
        append_record(
            &mut overlap,
            2,
            hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload_with_fields(1, 1, 0, 0),
        );
        append_record(
            &mut overlap,
            3,
            hour + 1,
            10,
            RECORD_TYPE_DOMAIN_GAP,
            &gap_payload(hour + 1, hour, hour + 1, 1),
        );
        append_record(
            &mut overlap,
            4,
            hour + 1,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload_with_fields(1, 1, 0, 0),
        );
        let summary = scan_compact_stream(&overlap);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("overlaps or regresses")));

        let mut target_mismatch = Vec::new();
        append_record(
            &mut target_mismatch,
            1,
            hour + 1,
            10,
            RECORD_TYPE_DOMAIN_GAP,
            &gap_payload(hour, hour, hour, 0),
        );
        let summary = scan_compact_stream(&target_mismatch);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("does not equal target_hour_raw")));
    }

    #[test]
    fn header_scan_requires_exact_forward_discontinuity_h4dg_ranges() {
        let hour = canonical_gap_hour(0);
        let mut exact = Vec::new();
        append_record(
            &mut exact,
            1,
            hour,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut exact,
            2,
            hour,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut exact,
            3,
            hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload_with_fields(1, 1, 0, 0),
        );
        append_record(
            &mut exact,
            4,
            hour + 3,
            10,
            RECORD_TYPE_DOMAIN_GAP,
            &gap_payload(hour + 3, hour + 1, hour + 3, 0),
        );
        append_record(
            &mut exact,
            5,
            hour + 3,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload_with_fields(1, 1, 0, 0),
        );
        let exact_summary = scan_compact_stream(&exact);
        assert!(
            exact_summary.parse_errors.is_empty(),
            "{:?}",
            exact_summary.parse_errors
        );
        assert!(
            compact_domain_coverage_errors(&exact_summary, &[10]).is_empty(),
            "{:?}",
            compact_domain_coverage_errors(&exact_summary, &[10])
        );

        let mut hole = exact;
        let replacement = gap_payload(hour + 3, hour + 2, hour + 3, 0);
        let gap_offset = (RECORD_HEADER_LEN + 40)
            + (RECORD_HEADER_LEN + fuel_empty_payload().len())
            + (RECORD_HEADER_LEN + 32)
            + RECORD_HEADER_LEN;
        hole[gap_offset..gap_offset + replacement.len()].copy_from_slice(&replacement);
        let hole_summary = scan_compact_stream(&hole);
        assert!(compact_domain_coverage_errors(&hole_summary, &[10])
            .iter()
            .any(|error| error.contains("exact range")));

        let mut initial_coalesced = Vec::new();
        append_record(
            &mut initial_coalesced,
            1,
            hour + 1,
            10,
            RECORD_TYPE_DOMAIN_GAP,
            &gap_payload(hour + 1, hour, hour + 1, 0),
        );
        append_record(
            &mut initial_coalesced,
            2,
            hour + 1,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload_with_fields(1, 1, 0, 0),
        );
        let first_summary = scan_compact_stream(&initial_coalesced);
        assert!(
            compact_domain_coverage_errors(&first_summary, &[10]).is_empty(),
            "the first authenticated H4DG range_start is the domain's session origin"
        );
    }

    #[test]
    fn header_scan_rejects_unknown_type_and_aggregate_count_bypass() {
        let hour = canonical_gap_hour(0);
        let mut unknown = Vec::new();
        append_record(&mut unknown, 1, hour, 10, 99, &[]);
        let summary = scan_compact_stream(&unknown);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("unknown record type 99")));

        let mut bypass = Vec::new();
        append_record(
            &mut bypass,
            1,
            hour,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut bypass,
            2,
            hour,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0),
        );
        append_record(
            &mut bypass,
            3,
            hour,
            10,
            RECORD_TYPE_DOMAIN_DATA,
            &fuel_empty_payload(),
        );
        append_record(
            &mut bypass,
            4,
            hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload_with_fields(2, 2, 0, 0),
        );
        let summary = scan_compact_stream(&bypass);
        assert!(
            summary.parse_errors.is_empty(),
            "aggregate fixture itself should reach exact-roster checks: {:?}",
            summary.parse_errors
        );
        let errors = compact_domain_coverage_errors(&summary, &[10, 12]);
        assert!(errors.iter().any(|error| error.contains("terminal roster")));
        assert!(errors.iter().any(|error| error.contains("PERF XOR H4DG")));
    }

    #[test]
    fn probe_domain_records_are_excluded_from_all_stream_accounting() {
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
            "status_flags_or": (1u64 << 8) | (1u64 << 9),
            "read_time_us": {"count": 1},
            "data_units_total": units,
        })
    }

    #[test]
    fn d47_adapter_work_with_zero_emission_is_fatal_even_when_partial_tolerated() {
        let contract = contract_by_id(47).expect("domain 47 contract exists");
        assert!(!contract.work_implies_rows_units.is_empty());
        let domain = d47_domain_manifest(
            0,
            json!({
                "domain47_adapter_character_rows": 39_553_062u64,
                "domain47_adapter_advisor_roster_rows": 29_790_670u64,
                "domain47_frame_parse_errors": 1560,
            }),
        );
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
        let domain = d47_domain_manifest(
            2_360_421,
            json!({
                "domain47_adapter_character_rows": 39_553_062u64,
            }),
        );
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
    fn d47_no_work_and_no_rows_does_not_trip_work_implies_rows() {
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
        assert_eq!(
            errors,
            vec![
                "game_hour_raw 100: compact domain fuel_live DATA is nonterminal and must be present exactly once only with an accepted PERF outcome; observed DATA=0, expected 1"
                    .to_string()
            ]
        );
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
                error.contains("domain 10 DATA payload")
                    && error.contains("unassigned status flag(s) 0x80000000")
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
            .any(|error| error.contains("exactly one record per accepted PERF frame")));

        let gapped = HeaderScanSummary {
            frame_end_record_count: 1,
            frame_end_hour_counts: BTreeMap::from([(100, 1)]),
            per_hour_gap_domain_counts: BTreeMap::from([(100, BTreeMap::from([(57, 1)]))]),
            ..HeaderScanSummary::default()
        };
        let gapped_compact = json!({
            "domain57_frame_evidence": domain57_frame_evidence_json(&gapped),
        });
        let mut gapped_errors = Vec::new();
        validate_domain57_stream_authority(&gapped, &gapped_compact, &mut gapped_errors);
        assert!(gapped_errors.is_empty(), "{gapped_errors:?}");

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
            encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
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

        let summary = scan_compact_stream_with_identity_mode(&raw, StreamIdentityMode::IdentityV1);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(
            summary.ordering_errors.is_empty(),
            "{:?}",
            summary.ordering_errors
        );
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
        let summary = scan_compact_stream_with_identity_mode(&raw, StreamIdentityMode::IdentityV1);
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
                    && error.contains("independently observed terminal record count 1")
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
            "skipped_game_hours": 0,
        });
        let mut errors = Vec::new();
        validate_scanned_hours(&summary, &honest, 1, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");

        let dishonest = json!({
            "unique_game_hour_count": 3,
            "game_hours_advanced": 2,
            "skipped_game_hours": 0,
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
            "skipped_game_hours": 0,
        });
        let mut minimum_errors = Vec::new();
        validate_scanned_hours(&one_hour, &manifest, 1, &mut minimum_errors);
        assert_eq!(
            minimum_errors
                .iter()
                .filter(|error| error.contains("below required"))
                .count(),
            1,
            "{minimum_errors:?}"
        );

        let discontinuous = HeaderScanSummary {
            unique_hours: vec![100, 103],
            skipped_hours: 2,
            ..HeaderScanSummary::default()
        };
        let discontinuous_manifest = json!({
            "unique_game_hour_count": 2,
            "game_hours_advanced": 3,
            "skipped_game_hours": 2,
        });
        let mut discontinuity_errors = Vec::new();
        validate_scanned_hours(
            &discontinuous,
            &discontinuous_manifest,
            3,
            &mut discontinuity_errors,
        );
        assert!(discontinuity_errors.is_empty(), "{discontinuity_errors:?}");
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
        validate_hourly_manifest(
            &honest,
            3,
            GapAcceptanceProfile::TypedGapsAllowed,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let mut mismatched_request = honest.clone();
        mismatched_request["hourly_requirement"]["requested_game_hours"] = json!(4);
        validate_hourly_manifest(
            &mismatched_request,
            3,
            GapAcceptanceProfile::TypedGapsAllowed,
            &mut errors,
        );
        assert!(errors.iter().any(|error| {
            error.contains("requested_game_hours 4") && error.contains("--min-game-hours 3")
        }));

        let mut missing_closed_frame = honest;
        missing_closed_frame["unique_game_hour_count"] = json!(3);
        validate_hourly_manifest(
            &missing_closed_frame,
            3,
            GapAcceptanceProfile::TypedGapsAllowed,
            &mut errors,
        );
        assert!(errors.iter().any(|error| {
            error.contains("unique_game_hour_count 3")
                && error.contains("game_hours_advanced + 1 (4)")
        }));

        let typed_gap = json!({
            "hourly_requirement": {
                "required": true,
                "met": true,
                "problems": [],
                "requested_game_hours": 3,
            },
            "game_hours_advanced": 3,
            "unique_game_hour_count": 2,
            "skipped_game_hours": 2,
        });
        let mut typed_errors = Vec::new();
        validate_hourly_manifest(
            &typed_gap,
            3,
            GapAcceptanceProfile::TypedGapsAllowed,
            &mut typed_errors,
        );
        assert!(typed_errors.is_empty(), "{typed_errors:?}");
        validate_hourly_manifest(
            &typed_gap,
            3,
            GapAcceptanceProfile::ZeroLossRequired,
            &mut typed_errors,
        );
        assert!(typed_errors
            .iter()
            .any(|error| error.contains("zero-loss profile requires 0")));
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
    fn a_measured_empty_domain_certifies_without_a_failure_override() {
        let empty = compact_contract::COMPACT_STATUS_SOURCE_EMPTY;
        assert_eq!(domain_status_unknown_bits(36, empty), 0);
        assert_eq!(rejected_domain_failure_bits(36, empty, false), 0);
        assert_eq!(rejected_global_failure_bits(empty, false), 0);

        let contract = test_contract(36, "air_variant_reserves");
        let manifest = test_domain_manifest(36, json!(empty));
        let mut errors = Vec::new();
        validate_domain_manifest(&manifest, contract, 1, &GateOptions::default(), &mut errors);
        assert!(
            errors.is_empty(),
            "a positively measured empty source was rejected: {errors:?}"
        );
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
            let stored = portable_test_fs_retry::read(&path).unwrap();
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
        let sealed_bytes = write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let sealed: Value = serde_json::from_slice(&sealed_bytes).unwrap();
        assert_ne!(
            sealed, manifest,
            "the sealed copy is the portable projection"
        );
        assert_eq!(
            sealed["static_bundle"]["manifest"]["path"],
            json!("static_bundle_manifest.json"),
            "the sealed evidence entry carries a basename"
        );
        let authority = load_verified_replay_authority(
            &manifest,
            &manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .unwrap();
        assert_eq!(authority.manifest, sealed);
        assert_eq!(
            authority.details["static_artifacts"]["required_count"],
            json!(compact_contract::REQUIRED_STATIC_ARTIFACTS.len())
        );
        assert_eq!(
            authority.details["static_artifacts"]["exact_required_roles"],
            json!(true)
        );

        assert_eq!(
            authority.details["external_manifest_bound_to_sealed_authority"],
            json!(true)
        );

        let mut rewritten = manifest.clone();
        rewritten["duration_ms"] = json!(999_999);
        let rewritten_bytes = serde_json::to_vec_pretty(&rewritten).unwrap();
        let error = load_verified_replay_authority(
            &rewritten,
            &rewritten_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("mutable disk rewrite must fail");
        assert!(
            error.contains("contradicts the authoritative sealed recorder_manifest"),
            "{error}"
        );
        assert!(error.contains("$.duration_ms"), "{error}");
    }

    fn live_shaped_manifest_pair(dir: &Path, game_id: &str) -> (Value, Value) {
        live_shaped_manifest_pair_with(dir, game_id, |_| {})
    }

    fn live_shaped_manifest_pair_with(
        dir: &Path,
        game_id: &str,
        adjust_base: impl FnOnce(&mut Value),
    ) -> (Value, Value) {
        let mut base = replay_test_manifest(game_id, dir);
        base["dll"] = json!(r"\\?\.cargo-target\release\collector.dll");
        base["modset"] = json!({
            "game": {
                "exe_path": r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe",
            },
            "pdx_user_dir": r"\\?\C:\Users\Example\Documents\Paradox Interactive\Hearts of Iron IV",
        });
        adjust_base(&mut base);
        let shared_host_finalize = base["host_finalize"].clone();

        let mut sealed = recorder_host::portable_manifest::portable_manifest_projection(&base)
            .expect("the fixture run-dir manifest has a portable projection");
        let mut sealed_host_finalize = shared_host_finalize.clone();
        let object = sealed_host_finalize.as_object_mut().unwrap();
        object.insert("embedded_copy_phase".into(), json!("pre_pack"));
        object.insert("standalone_manifest_is_authoritative".into(), json!(true));
        object.insert(
            "post_pack_fields_in_standalone_manifest_only".into(),
            json!([
                "finalize_seconds",
                "peak_private_bytes",
                "peak_working_set_bytes",
                "verify"
            ]),
        );
        object.insert(
            "excluded_domain_data".into(),
            json!({"complete": true, "per_domain": {}, "records": [], "total_records": 0}),
        );
        object.insert(
            recorder_host::hour_coverage::HOUR_COVERAGE_FIELD.into(),
            fixture_hour_coverage(&[0, 1, 2], &[]).to_json(),
        );
        sealed["host_finalize"] = sealed_host_finalize;

        let mut run_dir = base;
        let mut run_dir_host_finalize = shared_host_finalize;
        let object = run_dir_host_finalize.as_object_mut().unwrap();
        object.insert("finalize_seconds".into(), json!(970.986_574_5));
        object.insert("peak_private_bytes".into(), json!(2_909_249_536u64));
        object.insert("peak_working_set_bytes".into(), json!(2_870_587_392u64));
        object.insert(
            "verify".into(),
            json!({
                "entries_verified": 11,
                "stream_entry_bytes": 0,
                "total_uncompressed": 2_196_756_222u64,
                "stream_sha256_matches_capture": false,
            }),
        );
        run_dir["host_finalize"] = run_dir_host_finalize;

        (run_dir, sealed)
    }

    #[test]
    fn a_live_shaped_sealed_and_rewritten_manifest_pair_loads() {
        let dir = TmpDir::new("live_shaped_manifest");
        let (run_dir_manifest, sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        let sealed_bytes = serde_json::to_vec_pretty(&sealed_manifest).unwrap();
        let run_dir_bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        assert_ne!(
            sealed_bytes, run_dir_bytes,
            "the fixture must actually be the divergent live shape"
        );
        write_replay_fixture(&dir.0, "g", &sealed_bytes, true);

        let authority = load_verified_replay_authority(
            &run_dir_manifest,
            &run_dir_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect("a live-shaped recording must be admissible");
        assert_eq!(
            authority.manifest, sealed_manifest,
            "the SEALED copy is authority"
        );
        assert_eq!(
            authority.details["external_manifest_bound_to_sealed_authority"],
            json!(true)
        );
        assert_eq!(
            authority.details["external_manifest_path_redaction_applied"],
            json!(true)
        );
        assert_eq!(
            authority.details["external_manifest_sha256"],
            json!(Sha256::hex(&run_dir_bytes))
        );
        assert_eq!(
            authority.details["recorder_manifest_sha256"],
            json!(Sha256::hex(&sealed_bytes))
        );
        assert_ne!(
            authority.details["external_manifest_sha256"],
            authority.details["recorder_manifest_sha256"],
            "the two identities must be reported separately, not conflated"
        );
    }

    #[test]
    fn a_non_allowlisted_rewrite_inside_host_finalize_still_refuses_by_name() {
        let dir = TmpDir::new("host_finalize_tamper");
        let (run_dir_manifest, sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        write_replay_fixture(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );

        for (pointer, value, named) in [
            (
                "/host_finalize/stream_sha256",
                json!("b".repeat(64)),
                "$.host_finalize.stream_sha256",
            ),
            (
                "/host_finalize/fold/hours_emitted",
                json!(9_999),
                "$.host_finalize.fold.hours_emitted",
            ),
            (
                "/host_finalize/fold/excluded_domain_data/complete",
                json!(false),
                "$.host_finalize.fold.excluded_domain_data.complete",
            ),
            ("/replay_game_id", json!("g"), "$.replay_game_id"),
        ] {
            let mut tampered = run_dir_manifest.clone();
            if pointer == "/replay_game_id" {
                tampered["production_profile"]["enabled"] = json!(false);
            } else {
                *tampered.pointer_mut(pointer).expect(pointer) = value;
            }
            let expected = if pointer == "/replay_game_id" {
                "$.production_profile.enabled"
            } else {
                named
            };
            let bytes = serde_json::to_vec_pretty(&tampered).unwrap();
            let error =
                load_verified_replay_authority(&tampered, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                    .expect_err("a non-allowlisted rewrite must refuse");
            assert!(
                error.contains("contradicts the authoritative sealed recorder_manifest"),
                "{error}"
            );
            assert!(
                error.contains(expected),
                "refusal must name {expected}: {error}"
            );
        }
    }

    #[test]
    fn every_run_dir_only_post_seal_field_may_diverge() {
        let dir = TmpDir::new("allowlist_permits");
        let (run_dir_manifest, sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        write_replay_fixture(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );

        for field in RUN_DIR_ONLY_POST_SEAL_HOST_FINALIZE_FIELDS {
            let mut rewritten = run_dir_manifest.clone();
            rewritten["host_finalize"][*field] = json!("post-seal-only");
            let bytes = serde_json::to_vec_pretty(&rewritten).unwrap();
            load_verified_replay_authority(&rewritten, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                .unwrap_or_else(|error| {
                    panic!("run-dir-only field {field:?} must be permitted to diverge: {error}")
                });

            let mut removed = run_dir_manifest.clone();
            removed["host_finalize"]
                .as_object_mut()
                .unwrap()
                .remove(*field);
            let bytes = serde_json::to_vec_pretty(&removed).unwrap();
            load_verified_replay_authority(&removed, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                .unwrap_or_else(|error| {
                    panic!("run-dir-only field {field:?} must be permitted to be absent: {error}")
                });
        }
    }

    #[test]
    fn a_sealed_only_field_appearing_in_the_run_dir_copy_refuses() {
        let dir = TmpDir::new("allowlist_direction");
        let (run_dir_manifest, sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        write_replay_fixture(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );

        for field in SEALED_ONLY_PRE_PACK_HOST_FINALIZE_FIELDS {
            let mut forged = run_dir_manifest.clone();
            forged["host_finalize"][*field] = json!("forged");
            let bytes = serde_json::to_vec_pretty(&forged).unwrap();
            let error =
                load_verified_replay_authority(&forged, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                    .expect_err("a sealed-only field in the run-dir copy must refuse");
            assert!(
                error.contains(&format!("host_finalize.{field}")),
                "refusal must name the field: {error}"
            );
            assert!(error.contains("directional"), "{error}");
        }

        for field in RUN_DIR_ONLY_POST_SEAL_HOST_FINALIZE_FIELDS {
            let dir = TmpDir::new("allowlist_direction_sealed");
            let (run_dir_manifest, mut forged_seal) = live_shaped_manifest_pair(&dir.0, "g");
            forged_seal["host_finalize"][*field] = json!("forged");
            write_replay_fixture(
                &dir.0,
                "g",
                &serde_json::to_vec_pretty(&forged_seal).unwrap(),
                true,
            );
            let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
            let error = load_verified_replay_authority(
                &run_dir_manifest,
                &bytes,
                &dir.0,
                TEST_MAX_STREAM_BYTES,
            )
            .expect_err("a run-dir-only field in the sealed copy must refuse");
            assert!(
                error.contains(&format!("host_finalize.{field}")),
                "refusal must name the field: {error}"
            );
        }
    }

    #[test]
    fn a_keep_spool_after_seal_manifest_pair_binds_without_unbinding_the_root() {
        const SPOOL_TRIM: &str = "spool_trim";
        // Byte-for-byte the summary `release_sealed_spool_storage` records for
        let keep_spool = || json!({ "released": false, "reason": "--keep-spool-after-seal" });

        let dir = TmpDir::new("keep_spool_binds");
        let (mut run_dir_manifest, sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        run_dir_manifest[SPOOL_TRIM] = keep_spool();
        assert!(
            sealed_manifest.get(SPOOL_TRIM).is_none(),
            "the sealed copy is built before the container exists, so it cannot carry {SPOOL_TRIM}"
        );
        write_replay_fixture(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );
        let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        let authority = load_verified_replay_authority(
            &run_dir_manifest,
            &bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect("a --keep-spool-after-seal capture must be admissible");
        assert_eq!(
            authority.manifest, sealed_manifest,
            "the SEALED copy is still the authority handed downstream"
        );
        assert!(
            authority.manifest.get(SPOOL_TRIM).is_none(),
            "admitting the divergence must not smuggle the run-dir field into the authority"
        );
        assert_eq!(
            authority.details["run_dir_only_post_seal_top_level_fields"],
            json!(RUN_DIR_ONLY_POST_SEAL_TOP_LEVEL_FIELDS),
            "the report names what it unbound instead of leaving a reader to infer it"
        );

        let mut unlisted = run_dir_manifest.clone();
        unlisted["spool_trim_extra"] = keep_spool();
        let bytes = serde_json::to_vec_pretty(&unlisted).unwrap();
        let error =
            load_verified_replay_authority(&unlisted, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                .expect_err("an unlisted root key must still contradict the sealed copy");
        assert!(
            error.contains("$.spool_trim_extra"),
            "the refusal must name the unlisted key: {error}"
        );

        let dir = TmpDir::new("keep_spool_direction");
        let (run_dir_manifest, mut forged_seal) = live_shaped_manifest_pair(&dir.0, "g");
        forged_seal[SPOOL_TRIM] = keep_spool();
        write_replay_fixture(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&forged_seal).unwrap(),
            true,
        );
        let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        let error = load_verified_replay_authority(
            &run_dir_manifest,
            &bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a run-dir-only root field in the sealed copy must refuse");
        assert!(
            error.contains(SPOOL_TRIM),
            "refusal must name the field: {error}"
        );
        assert!(error.contains("directional"), "{error}");

        let dir = TmpDir::new("keep_spool_still_refuses");
        let (mut run_dir_manifest, sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        write_replay_fixture(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );
        run_dir_manifest[SPOOL_TRIM] = keep_spool();
        run_dir_manifest["host_finalize"]["stream_sha256"] = json!("0".repeat(64));
        let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        let error = load_verified_replay_authority(
            &run_dir_manifest,
            &bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a contradicted bound field must refuse even on a keep-spool capture");
        assert!(
            error.contains("$.host_finalize.stream_sha256"),
            "the refusal must name the contradicted field: {error}"
        );
    }

    #[test]
    fn a_sealed_manifest_carrying_an_absolute_path_refuses() {
        let dir = TmpDir::new("sealed_absolute_value");
        let (run_dir_manifest, mut sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        sealed_manifest["dll"] = json!(r"\\?\C:\somewhere\else\collector.dll");
        write_replay_fixture_sealed_verbatim(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );
        let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        let error = load_verified_replay_authority(
            &run_dir_manifest,
            &bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("an absolute path in the sealed copy must refuse");
        assert!(error.contains("is not portable"), "{error}");
        assert!(
            error.contains("still contains an absolute path at $.dll"),
            "the refusal must name the path: {error}"
        );

        let dir = TmpDir::new("sealed_absolute_key");
        let (run_dir_manifest, mut sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        let absolute_key = r"\\?\C:\somewhere\else\dll";
        let object = sealed_manifest.as_object_mut().unwrap();
        let value = object.remove("dll").unwrap();
        object.insert(absolute_key.to_string(), value);
        write_replay_fixture_sealed_verbatim(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );
        let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        let error = load_verified_replay_authority(
            &run_dir_manifest,
            &bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("an absolute key in the sealed copy must refuse");
        assert!(error.contains("is not portable"), "{error}");
        assert!(
            error.contains(&format!("absolute path key {absolute_key:?}")),
            "the refusal must name the offending key: {error}"
        );
    }

    #[test]
    fn a_degraded_sealed_manifest_binds_and_reports_its_exclusions() {
        let dir = TmpDir::new("degraded_portability");
        let poisoned = concat!(
            r"settings.txt launch settings unresolved: ",
            r"\\?\C:\Users\Example\Documents\Paradox Interactive\Hearts of Iron IV\settings.txt",
            r" mtime is at/after game process start",
        );
        let (run_dir_manifest, sealed_manifest) =
            live_shaped_manifest_pair_with(&dir.0, "g", |base| {
                base["modset"] = json!({
                    "schema": "hoi4_modset_binding_v2",
                    "resolved": false,
                    "errors": [poisoned],
                });
            });
        assert_eq!(
            sealed_manifest["modset"]["errors"][0],
            json!(recorder_host::portable_manifest::PORTABILITY_EXCLUDED_VALUE),
            "the recorder's own projection must cost the field, not the seal"
        );
        assert_eq!(
            sealed_manifest[recorder_host::portable_manifest::PORTABILITY_EXCLUSIONS_FIELD]
                ["excluded"][0]["location"],
            json!(["modset", "errors", 0])
        );
        write_replay_fixture_sealed_verbatim(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );
        let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        let authority = load_verified_replay_authority(
            &run_dir_manifest,
            &bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect("a degraded container must still verify");
        assert_eq!(
            authority.details["recorder_manifest_portability_degraded"],
            json!(true)
        );
    }

    #[test]
    fn a_ledger_the_recording_does_not_produce_refuses() {
        let dir = TmpDir::new("forged_portability_ledger");
        let (run_dir_manifest, mut sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        sealed_manifest[recorder_host::portable_manifest::PORTABILITY_EXCLUSIONS_FIELD] = json!({
            "schema": recorder_host::portable_manifest::PORTABILITY_EXCLUSIONS_SCHEMA,
            "status": "degraded",
            "excluded": [{
                "location": ["modset"],
                "kind": "value",
                "reason": "absolute_path_fragment",
            }],
        });
        write_replay_fixture_sealed_verbatim(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );
        let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        let error = load_verified_replay_authority(
            &run_dir_manifest,
            &bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a ledger the recording does not produce must refuse");
        assert!(
            error.contains("portability exclusion"),
            "the refusal must name the ledger: {error}"
        );
    }

    #[test]
    fn a_sealed_absolute_path_is_not_reduced_away() {
        let mut redactions = 0;
        let violation = first_binding_violation(
            &json!({"dll": r"\\?\C:\somewhere\else\collector.dll"}),
            &json!({"dll": r"\\?\release\collector.dll"}),
            "$",
            &mut redactions,
        )
        .expect("a same-basename different-prefix pair must not bind");
        assert!(violation.contains("$.dll"), "{violation}");
        assert_eq!(
            redactions, 0,
            "nothing was reduced, so nothing may be counted"
        );

        let violation = first_binding_violation(
            &json!({r"\\?\C:\somewhere\else\dll": "collector.dll"}),
            &json!({r"\\?\release\dll": "collector.dll"}),
            "$",
            &mut redactions,
        )
        .expect("an absolute sealed key must not bind to its basename");
        assert!(violation.contains("dll"), "{violation}");
    }

    #[test]
    fn object_keys_bind_by_equality_first_and_collisions_refuse() {
        let mut redactions = 0;
        let absolute_key = r"\\?\release\dll";
        assert!(
            first_binding_violation(
                &json!({absolute_key: "collector.dll"}),
                &json!({absolute_key: "collector.dll"}),
                "$",
                &mut redactions,
            )
            .is_none(),
            "an identical absolute key must bind to itself"
        );
        assert_eq!(redactions, 0, "binding to itself is not a redaction");

        let dir = TmpDir::new("absolute_key_collision");
        let (mut run_dir_manifest, sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        run_dir_manifest
            .as_object_mut()
            .unwrap()
            .insert(r"\\?\C:\elsewhere\dll".to_string(), json!("collector.dll"));
        write_replay_fixture(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );
        let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        let error = load_verified_replay_authority(
            &run_dir_manifest,
            &bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("colliding run-dir keys must refuse");
        assert!(error.contains("key_collision"), "{error}");
        assert!(
            error.contains("does not follow from the recording"),
            "{error}"
        );

        let mut redactions = 0;
        let violation = first_binding_violation(
            &json!({"dll": "collector.dll"}),
            &json!({"dll": "collector.dll", r"\\?\C:\elsewhere\dll": "collector.dll"}),
            "$",
            &mut redactions,
        )
        .expect("colliding run-dir keys must not bind");
        assert!(
            violation.contains("collide onto one sealed key"),
            "{violation}"
        );
    }

    #[test]
    fn redaction_does_not_bind_in_the_reverse_direction() {
        let mut redactions = 0;
        let violation = first_binding_violation(
            &json!({"dll": r"\\?\release\collector.dll"}),
            &json!({"dll": "collector.dll"}),
            "$",
            &mut redactions,
        )
        .expect("reverse-direction redaction must not bind");
        assert!(violation.contains("$.dll"), "{violation}");
        assert_eq!(redactions, 0);

        let dir = TmpDir::new("reverse_redaction");
        let (mut run_dir_manifest, mut sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        sealed_manifest["dll"] = json!(r"\\?\release\collector.dll");
        run_dir_manifest["dll"] = json!("collector.dll");
        write_replay_fixture_sealed_verbatim(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );
        let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        let error = load_verified_replay_authority(
            &run_dir_manifest,
            &bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a sealed absolute path must refuse");
        assert!(error.contains("$.dll"), "{error}");
    }

    #[test]
    fn deleting_the_run_dir_host_finalize_block_refuses() {
        let dir = TmpDir::new("host_finalize_deleted");
        let (run_dir_manifest, sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        write_replay_fixture(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );
        let mut stripped = run_dir_manifest;
        stripped.as_object_mut().unwrap().remove("host_finalize");
        let bytes = serde_json::to_vec_pretty(&stripped).unwrap();
        let error =
            load_verified_replay_authority(&stripped, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                .expect_err("a deleted host_finalize block must refuse");
        assert!(error.contains("host_finalize"), "{error}");
        assert!(
            error.contains("not a sanctioned post-seal divergence"),
            "{error}"
        );
    }

    #[test]
    fn a_sealed_copy_without_host_finalize_refuses() {
        for sealed_block in [None, Some(json!(null)), Some(json!("pre_pack"))] {
            let dir = TmpDir::new("sealed_without_host_finalize");
            let (run_dir_manifest, mut sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
            let object = sealed_manifest.as_object_mut().unwrap();
            match sealed_block.clone() {
                None => {
                    object.remove("host_finalize");
                }
                Some(value) => {
                    object.insert("host_finalize".to_string(), value);
                }
            }
            write_replay_fixture(
                &dir.0,
                "g",
                &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
                true,
            );
            let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
            let error = load_verified_replay_authority(
                &run_dir_manifest,
                &bytes,
                &dir.0,
                TEST_MAX_STREAM_BYTES,
            )
            .expect_err("a sealed copy without an object host_finalize must refuse");
            assert!(
                error.contains("object-valued \"host_finalize\""),
                "{sealed_block:?}: {error}"
            );
        }
    }

    #[test]
    fn an_absolute_path_key_cannot_hide_the_composition_from_routing() {
        let dir = TmpDir::new("composition_key_rename");
        let recording = write_cache_only_replay_fixture(&dir.0, "g", CacheOnlyBreak::default());
        let mut renamed_key = recording.manifest.clone();
        let object = renamed_key.as_object_mut().unwrap();
        let declared = object
            .remove(snapshot::RAW_STREAM_PACKAGING_FIELD)
            .expect("the fixture declares its composition");
        assert_eq!(
            declared,
            json!(snapshot::RawStreamPackaging::Omitted.as_str()),
            "the fixture must be the cache-only composition for this test to discriminate"
        );
        object.insert(
            format!(r"\\?\C:\x\{}", snapshot::RAW_STREAM_PACKAGING_FIELD),
            declared,
        );
        let bytes = serde_json::to_vec_pretty(&renamed_key).unwrap();

        let authority =
            load_verified_replay_authority(&renamed_key, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                .expect("routing must see the composition the binding will bind");
        assert_eq!(
            authority.details[snapshot::RAW_STREAM_PACKAGING_FIELD],
            json!(snapshot::RawStreamPackaging::Omitted.as_str()),
            "the cache-only route must have run"
        );
        assert_eq!(
            authority.details["external_manifest_path_redaction_applied"],
            json!(true)
        );
    }

    #[test]
    fn an_absolute_replay_file_is_refused_not_reduced_to_a_basename() {
        let dir = TmpDir::new("absolute_replay_file");
        let (run_dir_manifest, sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        write_replay_fixture(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            true,
        );
        let mut absolute = run_dir_manifest;
        absolute["replay_file"] = json!(dir
            .0
            .join("native_session.battleplan")
            .display()
            .to_string());
        assert!(
            absolute["replay_file"].as_str().unwrap().len() > "native_session.battleplan".len(),
            "the fixture must actually be an absolute path"
        );
        let bytes = serde_json::to_vec_pretty(&absolute).unwrap();
        let error =
            load_verified_replay_authority(&absolute, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                .expect_err("an absolute replay_file must refuse");
        assert!(
            error.contains("must be a run-directory-contained relative path"),
            "{error}"
        );
    }

    #[test]
    fn an_absent_sealed_recorder_manifest_refuses() {
        let dir = TmpDir::new("sealed_manifest_absent");
        let (run_dir_manifest, sealed_manifest) = live_shaped_manifest_pair(&dir.0, "g");
        write_replay_fixture(
            &dir.0,
            "g",
            &serde_json::to_vec_pretty(&sealed_manifest).unwrap(),
            false,
        );
        let bytes = serde_json::to_vec_pretty(&run_dir_manifest).unwrap();
        let error = load_verified_replay_authority(
            &run_dir_manifest,
            &bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a container with no sealed recorder_manifest must refuse");
        assert!(error.contains("recorder_manifest entries"), "{error}");
    }

    #[test]
    fn a_recorder_sealed_container_shape_loads() {
        let dir = TmpDir::new("recorder_sealed_shape");
        let manifest = replay_test_manifest("g", &dir.0);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);

        let sealed = portable_test_fs_retry::read(dir.0.join("native_session.battleplan")).unwrap();
        let sealed_manifest: Value = serde_json::from_slice(
            &Container::from_bytes(sealed, &ExpectedIdentity::default())
                .unwrap()
                .read_entry_bytes("payload/g/recorder/native_session_manifest.json")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            sealed_manifest["schema"],
            json!(RECORDER_MANIFEST_SCHEMA),
            "the recorder manifest states the SESSION format in its own `schema` field"
        );
        assert_ne!(
            RECORDER_MANIFEST_SCHEMA, REPLAY_CONTAINER_SCHEMA_VERSION,
            "the two vocabularies are distinct; a test that let them coincide would prove nothing"
        );

        load_verified_replay_authority(&manifest, &manifest_bytes, &dir.0, TEST_MAX_STREAM_BYTES)
            .expect("a container sealed the way the recorder seals one must load");
    }

    #[test]
    fn a_container_schema_version_from_the_wrong_vocabulary_still_refuses() {
        for wrong in [RECORDER_MANIFEST_SCHEMA, "2", ""] {
            let error = cache_only_authority_error(
                "container_schema_wrong",
                CacheOnlyBreak {
                    container_schema_version: Some(wrong.to_string()),
                    ..CacheOnlyBreak::default()
                },
            );
            assert!(
                error.contains("schema_version")
                    && error.contains(&format!("{wrong:?}"))
                    && error.contains(&format!("{REPLAY_CONTAINER_SCHEMA_VERSION:?}")),
                "the refusal must name the field and BOTH values: {error}"
            );
        }
    }

    #[test]
    fn a_recorder_manifest_schema_from_the_wrong_vocabulary_still_refuses() {
        for wrong in [
            REPLAY_CONTAINER_SCHEMA_VERSION,
            "native_collector_session_v2",
        ] {
            let error = cache_only_authority_error(
                "manifest_schema_wrong",
                CacheOnlyBreak {
                    manifest_schema: Some(wrong.to_string()),
                    ..CacheOnlyBreak::default()
                },
            );
            assert!(
                error.contains("recorder_manifest")
                    && error.contains("schema")
                    && error.contains(&format!("{wrong:?}"))
                    && error.contains(&format!("{RECORDER_MANIFEST_SCHEMA:?}")),
                "the refusal must name the field and BOTH values: {error}"
            );
        }

        let dir = TmpDir::new("manifest_schema_absent");
        let mut manifest = replay_test_manifest("g", &dir.0);
        manifest.as_object_mut().unwrap().remove("schema");
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let error = load_verified_replay_authority(
            &manifest,
            &manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a recorder manifest with no schema must refuse");
        assert!(
            error.contains("schema") && error.contains("None"),
            "{error}"
        );
    }

    #[test]
    fn cache_only_capture_certifies_through_the_manifest_attested_chain() {
        let dir = TmpDir::new("cache_only_authority");
        let recording = write_cache_only_replay_fixture(&dir.0, "g", CacheOnlyBreak::default());
        let authority = load_verified_replay_authority(
            &recording.manifest,
            &recording.manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect("a default cache-only recording must certify");
        assert!(matches!(
            authority.stream_certification,
            StreamCertification::ManifestAttestedThroughDeltaCache { .. }
        ));
        assert_eq!(
            authority.details["stream_certification_path"],
            json!(STREAM_CERTIFICATION_MANIFEST_ATTESTED)
        );
        assert_eq!(
            authority.details[snapshot::RAW_STREAM_PACKAGING_FIELD],
            json!(snapshot::RawStreamPackaging::Omitted.as_str())
        );
        assert_eq!(
            authority.details["h4cr_stream_bytes_compared"],
            json!(false)
        );
        assert_eq!(authority.details["h4cr_stream_digest_bound"], json!(false));
        assert_eq!(
            authority.details["h4cr_identity_is_manifest_attested"],
            json!(true)
        );
        assert!(authority.details["collector_compact_stream"].is_null());
        assert_eq!(
            authority.details["attested_source_sha256"],
            json!(recording.stream_sha256)
        );
        assert_eq!(
            authority.details["attested_source_bytes"],
            json!(recording.stream_bytes)
        );
    }

    #[test]
    fn packaged_capture_still_certifies_by_byte_comparison_and_says_so() {
        let dir = TmpDir::new("packaged_authority_path");
        let manifest = replay_test_manifest("g", &dir.0);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let authority = load_verified_replay_authority(
            &manifest,
            &manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .unwrap();
        let StreamCertification::ByteComparedAgainstPackagedStream {
            container_stream, ..
        } = &authority.stream_certification
        else {
            panic!("a packaged recording must be byte-compared");
        };
        assert_eq!(container_stream, b"stream");
        assert_eq!(
            authority.details["stream_certification_path"],
            json!(STREAM_CERTIFICATION_BYTE_COMPARED)
        );
        assert_eq!(
            authority.details[snapshot::RAW_STREAM_PACKAGING_FIELD],
            json!(snapshot::RawStreamPackaging::Packaged.as_str())
        );
        assert_eq!(authority.details["h4cr_stream_bytes_compared"], json!(true));
        assert_eq!(authority.details["h4cr_stream_digest_bound"], json!(true));
        assert_eq!(
            authority.details["independently_observed_stream_sha256"],
            json!(Sha256::hex(b"stream"))
        );
    }

    #[test]
    fn a_tampered_packaged_stream_still_refuses() {
        let dir = TmpDir::new("packaged_tampered");
        let manifest = replay_test_manifest("g", &dir.0);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture_with_static_mutation(&dir.0, "g", &manifest_bytes, true, |_, _| {});
        let replay_path = dir.0.join("native_session.battleplan");
        let mut packed = portable_test_fs_retry::read(&replay_path).unwrap();
        let position = packed
            .windows(6)
            .enumerate()
            .find(|(offset, window)| {
                *window == b"stream"
                    && *offset >= 10
                    && &packed[offset - 10..*offset] != b"collector_"
            })
            .map(|(offset, _)| offset)
            .expect("the fixture packages the literal stream payload");
        packed[position] ^= 0xff;
        fs::write(&replay_path, &packed).unwrap();
        let error = load_verified_replay_authority(
            &manifest,
            &manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a tampered packaged stream must refuse");
        assert!(
            error.contains("verify_all failed")
                || error.contains("independently hash")
                || error.contains("not container entry digest")
                || (error.contains("serve no single game hour")
                    && error.contains("collector_compact_stream")),
            "{error}"
        );
        assert!(
            !error.contains(STREAM_CERTIFICATION_MANIFEST_ATTESTED),
            "a packaged container must never fall back to the weaker route: {error}"
        );
    }

    #[test]
    fn a_packaged_stream_can_never_take_the_manifest_attested_route() {
        let error = cache_only_authority_error(
            "cache_only_with_a_stream",
            CacheOnlyBreak {
                package_stream_entry: true,
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            error.contains(CACHE_ONLY_LINK_NO_PACKAGED_STREAM),
            "{error}"
        );
        assert!(
            error.contains("byte-compared, not manifest-attested"),
            "{error}"
        );
    }

    #[test]
    fn every_broken_attestation_link_refuses_by_its_own_name() {
        let error = cache_only_authority_error(
            "cache_only_storage_hash",
            CacheOnlyBreak {
                stream_storage: Some(json!({
                    "compression": "none",
                    "stored_bytes": 140,
                    "decoded_bytes": 140,
                    "decoded_sha256": "b".repeat(64),
                })),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(error.contains(CACHE_ONLY_LINK_STREAM_STORAGE), "{error}");
        assert!(error.contains("independently hashes to"), "{error}");

        let mut longer = b"H4CRcache-only-run-directory-source".repeat(4);
        longer.extend_from_slice(b"and then some more");
        let error = cache_only_authority_error(
            "cache_only_storage_overlong",
            CacheOnlyBreak {
                run_dir_stream_override: Some(longer),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(error.contains(CACHE_ONLY_LINK_STREAM_STORAGE), "{error}");
        assert!(error.contains("decodes to more than"), "{error}");

        let mut other = b"H4CRcache-only-run-directory-source".repeat(4);
        other[0] ^= 0xff;
        let error = cache_only_authority_error(
            "cache_only_index_hash",
            CacheOnlyBreak {
                cache_folded_from: Some(other),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(error.contains(CACHE_ONLY_LINK_HOUR_INDEX), "{error}");
        assert!(error.contains("was folded over source"), "{error}");

        let error = cache_only_authority_error(
            "cache_only_index_length",
            CacheOnlyBreak {
                index_source_len: Some(139),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(error.contains(CACHE_ONLY_LINK_HOUR_INDEX), "{error}");
        assert!(error.contains("states source length"), "{error}");
        assert!(
            error.contains("H4CP carries the source digest only"),
            "the length refusal must say why H4IX alone carries it: {error}"
        );

        let error = cache_only_authority_error(
            "cache_only_cache_bytes",
            CacheOnlyBreak {
                corrupt_cache_bytes: true,
                ..CacheOnlyBreak::default()
            },
        );
        assert!(error.contains(CACHE_ONLY_LINK_CHECKPOINT_CACHE), "{error}");
        assert!(
            error.contains("do not match the per-record digests"),
            "{error}"
        );
        assert!(
            error.contains("two accounts of the same bytes disagree"),
            "{error}"
        );

        let error = cache_only_authority_error(
            "cache_only_cache_header",
            CacheOnlyBreak {
                cache_header_digest_of: Some(b"a different canonical H4CR".to_vec()),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(error.contains(CACHE_ONLY_LINK_CHECKPOINT_CACHE), "{error}");
        assert!(error.contains("header is not bound to source"), "{error}");
    }

    #[test]
    fn the_attested_length_is_bounded_by_the_gates_own_maximum() {
        let dir = TmpDir::new("cache_only_absurd_length");
        let recording = write_cache_only_replay_fixture(&dir.0, "g", CacheOnlyBreak::default());
        let error = load_verified_replay_authority(
            &recording.manifest,
            &recording.manifest_bytes,
            &dir.0,
            recording.stream_bytes - 1,
        )
        .expect_err("an attested length above the configured maximum must refuse");
        assert!(error.contains(CACHE_ONLY_LINK_STREAM_STORAGE), "{error}");
        assert!(error.contains("exceeds the configured maximum"), "{error}");
    }

    #[test]
    fn a_non_file_at_the_source_path_breaks_the_link_it_is_not_absence() {
        let dir = TmpDir::new("cache_only_source_is_a_dir");
        let recording = write_cache_only_replay_fixture(&dir.0, "g", CacheOnlyBreak::default());
        let stream_path = dir.0.join("collector_stream.bin");
        portable_test_fs_retry::remove_file(&stream_path).unwrap();
        fs::create_dir(&stream_path).unwrap();
        let error = load_verified_replay_authority(
            &recording.manifest,
            &recording.manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a directory at the source path must refuse");
        assert!(error.contains(CACHE_ONLY_LINK_SOURCE_READABLE), "{error}");
        assert!(error.contains("not a regular file"), "{error}");
        assert!(
            !error.contains(CACHE_ONLY_SOURCE_ABSENT_OUTCOME),
            "something IS there, so this is not the absent outcome: {error}"
        );
    }

    #[test]
    fn declared_compression_belongs_to_the_readable_link() {
        for (label, compression) in [
            ("cache_only_compression_missing", None),
            ("cache_only_compression_unsupported", Some("brotli")),
        ] {
            let mut storage = json!({
                "stored_bytes": 140,
                "decoded_bytes": 140,
                "decoded_sha256": "c".repeat(64),
            });
            if let Some(compression) = compression {
                storage["compression"] = json!(compression);
            }
            let error = cache_only_authority_error(
                label,
                CacheOnlyBreak {
                    stream_storage: Some(storage),
                    ..CacheOnlyBreak::default()
                },
            );
            assert!(error.contains(CACHE_ONLY_LINK_SOURCE_READABLE), "{error}");
            assert!(
                !error.contains(CACHE_ONLY_LINK_STREAM_STORAGE),
                "no digest comparison ran, so the identity link must not be blamed: {error}"
            );
        }
    }

    #[test]
    fn a_missing_run_dir_source_is_its_own_named_outcome() {
        let error = cache_only_authority_error(
            "cache_only_no_source",
            CacheOnlyBreak {
                omit_run_dir_stream: true,
                ..CacheOnlyBreak::default()
            },
        );
        assert!(error.contains(CACHE_ONLY_SOURCE_ABSENT_OUTCOME), "{error}");
        assert!(error.contains("Nothing disagreed"), "{error}");
        for link in [
            CACHE_ONLY_LINK_STREAM_STORAGE,
            CACHE_ONLY_LINK_HOUR_INDEX,
            CACHE_ONLY_LINK_CHECKPOINT_CACHE,
            CACHE_ONLY_LINK_SOURCE_READABLE,
        ] {
            assert!(
                !error.contains(link),
                "the absent-source outcome must not alias link {link}: {error}"
            );
        }
    }

    #[test]
    fn a_rewritten_disk_composition_cannot_choose_the_proof() {
        let dir = TmpDir::new("cache_only_composition_rewrite");
        let recording = write_cache_only_replay_fixture(&dir.0, "g", CacheOnlyBreak::default());
        let mut rewritten = recording.manifest.clone();
        rewritten[snapshot::RAW_STREAM_PACKAGING_FIELD] =
            json!(snapshot::RawStreamPackaging::Packaged.as_str());
        let rewritten_bytes = serde_json::to_vec_pretty(&rewritten).unwrap();
        let error = load_verified_replay_authority(
            &rewritten,
            &rewritten_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a rewritten composition must not certify");
        assert!(
            error.contains("replay_entries.stream is missing"),
            "{error}"
        );

        let dir = TmpDir::new("packaged_composition_rewrite");
        let manifest = replay_test_manifest("g", &dir.0);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let mut rewritten = manifest.clone();
        rewritten[snapshot::RAW_STREAM_PACKAGING_FIELD] =
            json!(snapshot::RawStreamPackaging::Omitted.as_str());
        rewritten["replay_entries"]
            .as_object_mut()
            .unwrap()
            .remove("stream");
        let rewritten_bytes = serde_json::to_vec_pretty(&rewritten).unwrap();
        let error = load_verified_replay_authority(
            &rewritten,
            &rewritten_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a packaged container must never take the weaker route");
        assert!(
            error.contains("contradicts the authoritative sealed recorder_manifest"),
            "{error}"
        );
        assert!(
            error.contains(snapshot::RAW_STREAM_PACKAGING_FIELD)
                || error.contains("replay_entries"),
            "the refusal must name the rewritten field: {error}"
        );
        assert!(
            !error.contains(STREAM_CERTIFICATION_MANIFEST_ATTESTED),
            "the weaker route must not be reported as having run: {error}"
        );
    }

    #[test]
    fn the_run_stream_is_bound_back_to_the_proof_that_certified_it() {
        let decoded = b"H4CRthe run directory stream".to_vec();
        let artifact = StreamArtifact {
            path: PathBuf::from("collector_stream.bin"),
            compression: compact_stream::StreamCompression::None,
            stored: decoded.clone(),
            stored_sha256: Sha256::hex(&decoded),
            decoded_sha256: Sha256::hex(&decoded),
            decoded: decoded.clone(),
        };

        let mut errors = Vec::new();
        bind_run_stream_to_certification(
            &StreamCertification::ByteComparedAgainstPackagedStream {
                container_stream: decoded.clone(),
                container_stream_sha256: Sha256::hex(&decoded),
            },
            &artifact,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let mut errors = Vec::new();
        let mut other = decoded.clone();
        other[4] ^= 0xff;
        bind_run_stream_to_certification(
            &StreamCertification::ByteComparedAgainstPackagedStream {
                container_stream: other.clone(),
                container_stream_sha256: Sha256::hex(&other),
            },
            &artifact,
            &mut errors,
        );
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].contains("verified container entry digest"),
            "{errors:?}"
        );
        assert!(
            errors[1].contains("differs from decoded run stream"),
            "{errors:?}"
        );

        let mut errors = Vec::new();
        bind_run_stream_to_certification(
            &StreamCertification::ManifestAttestedThroughDeltaCache {
                attested_source_sha256: Sha256::hex(&decoded),
                attested_source_bytes: decoded.len() as u64,
            },
            &artifact,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let mut errors = Vec::new();
        bind_run_stream_to_certification(
            &StreamCertification::ManifestAttestedThroughDeltaCache {
                attested_source_sha256: Sha256::hex(&other),
                attested_source_bytes: decoded.len() as u64 + 1,
            },
            &artifact,
            &mut errors,
        );
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].contains("manifest-attested source identity"),
            "{errors:?}"
        );
        assert!(
            errors[1].contains("manifest-attested source length"),
            "{errors:?}"
        );
        assert!(
            errors.iter().all(|error| !error.contains("container")),
            "{errors:?}"
        );
    }

    #[test]
    fn an_unassigned_composition_spelling_refuses_at_the_gate() {
        let dir = TmpDir::new("gate_unassigned_composition");
        let mut manifest = replay_test_manifest("g", &dir.0);
        manifest[snapshot::RAW_STREAM_PACKAGING_FIELD] = json!("sometimes");
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let error = load_verified_replay_authority(
            &manifest,
            &manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("an unassigned composition must refuse");
        assert!(error.contains("unassigned"), "{error}");
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
                manifest["static_bundle"]["product_modset_sha256"] = json!("9".repeat(64));
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
    fn retained_static_bundle_evidence_is_located_by_the_run_dir_copy() {
        let dir = TmpDir::new("retained_evidence_located");
        let run_dir_manifest = replay_test_manifest("g", &dir.0);
        let sealed_manifest =
            recorder_host::portable_manifest::portable_manifest_projection(&run_dir_manifest)
                .expect("the fixture manifest has a portable projection");

        for label in [
            "manifest",
            "verifier",
            "bindings.executable",
            "bindings.mod_state",
            "bindings.dlc_state",
            "bindings.load_order",
            "bindings.source_inventory",
            "bindings.live_resolution_report",
        ] {
            let pointer = format!("/static_bundle/{}/path", label.replace('.', "/"));
            let run_dir_path = run_dir_manifest
                .pointer(&pointer)
                .unwrap()
                .as_str()
                .unwrap();
            let sealed_path = sealed_manifest.pointer(&pointer).unwrap().as_str().unwrap();
            assert!(
                Path::new(run_dir_path).is_absolute(),
                "{label}: the run-dir copy locates the evidence: {run_dir_path:?}"
            );
            assert!(
                !Path::new(sealed_path).is_absolute() && !sealed_path.contains('\\'),
                "{label}: the sealed copy must carry a bare basename, got {sealed_path:?}"
            );
            assert_eq!(
                sealed_path,
                recorder_host::portable_manifest::redact_absolute_path_string(run_dir_path),
                "{label}: the sealed basename must bind to the run-dir path"
            );
        }

        verify_static_manifest_authority(&sealed_manifest, &run_dir_manifest)
            .expect("evidence present at the run-dir path, bound to the sealed basename, loads");
    }

    #[test]
    fn the_relocated_retained_evidence_check_refuses_by_name() {
        let dir = TmpDir::new("retained_evidence_refusals");
        let base_run_dir = replay_test_manifest("g", &dir.0);
        let sealed_manifest =
            recorder_host::portable_manifest::portable_manifest_projection(&base_run_dir)
                .expect("the fixture manifest has a portable projection");

        let mut relative = base_run_dir.clone();
        relative["static_bundle"]["verifier"]["path"] = json!("static_bundle.py");
        let error = verify_static_manifest_authority(&sealed_manifest, &relative)
            .expect_err("a relative run-dir evidence path must refuse");
        assert!(
            error.contains(
                "static_bundle verifier.path must be the retained canonical absolute path"
            ),
            "{error}"
        );
        assert!(
            error.contains("run-dir native_session_manifest.json"),
            "{error}"
        );

        let decoy = dir.0.join("decoy_verifier.py");
        fs::write(&decoy, b"# fixture static bundle verifier\n").unwrap();
        let mut unbound = base_run_dir.clone();
        unbound["static_bundle"]["verifier"]["path"] =
            json!(fs::canonicalize(&decoy).unwrap().display().to_string());
        let error = verify_static_manifest_authority(&sealed_manifest, &unbound)
            .expect_err("a run-dir evidence path that does not bind must refuse");
        assert!(error.contains("does not bind to the sealed"), "{error}");
        assert!(
            error.contains("decoy_verifier.py"),
            "the refusal names the path: {error}"
        );
        assert!(
            error.contains("static_bundle.py"),
            "and the sealed entry: {error}"
        );

        let verifier_path = PathBuf::from(
            base_run_dir["static_bundle"]["verifier"]["path"]
                .as_str()
                .unwrap(),
        );
        portable_test_fs_retry::remove_file(&verifier_path).unwrap();
        let error = verify_static_manifest_authority(&sealed_manifest, &base_run_dir)
            .expect_err("missing retained evidence must refuse");
        assert!(
            error.contains("cannot resolve retained static_bundle verifier file"),
            "{error}"
        );
        assert!(
            error.contains("static_bundle.py"),
            "the refusal names the path it could not resolve: {error}"
        );
    }

    #[test]
    fn the_retained_evidence_claims_come_from_the_sealed_copy_only() {
        let dir = TmpDir::new("retained_evidence_claim_authority");
        let base_run_dir = replay_test_manifest("g", &dir.0);
        let sealed_manifest =
            recorder_host::portable_manifest::portable_manifest_projection(&base_run_dir)
                .expect("the fixture manifest has a portable projection");

        let mut lying_run_dir = base_run_dir.clone();
        lying_run_dir["static_bundle"]["verifier"]["sha256"] = json!("c".repeat(64));
        lying_run_dir["static_bundle"]["bindings"]["mod_state"]["sha256"] = json!("d".repeat(64));
        lying_run_dir["static_bundle"]["bindings"]["mod_state"]["bytes"] = json!(1);
        verify_static_manifest_authority(&sealed_manifest, &lying_run_dir)
            .expect("the run-dir copy's claims about the evidence are not the authority");

        let mut lying_seal = sealed_manifest.clone();
        lying_seal["static_bundle"]["verifier"]["sha256"] = json!("c".repeat(64));
        let error = verify_static_manifest_authority(&lying_seal, &base_run_dir)
            .expect_err("a wrong sealed hash must refuse");
        assert!(error.contains("static_bundle verifier.sha256"), "{error}");

        let mut lying_seal = sealed_manifest;
        lying_seal["static_bundle"]["bindings"]["mod_state"]["bytes"] = json!(1);
        let error = verify_static_manifest_authority(&lying_seal, &base_run_dir)
            .expect_err("a wrong sealed byte count must refuse");
        assert!(
            error.contains("static_bundle bindings.mod_state.bytes 1 !="),
            "{error}"
        );
    }

    #[test]
    fn an_old_style_container_sealing_absolute_evidence_paths_refuses_on_portability() {
        let dir = TmpDir::new("old_style_absolute_evidence");
        let manifest = replay_test_manifest("g", &dir.0);
        let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture_sealed_verbatim(&dir.0, "g", &bytes, true);
        let error =
            load_verified_replay_authority(&manifest, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                .expect_err("an old-style sealed copy must refuse");
        assert!(error.contains("is not portable"), "{error}");
        assert!(
            error.contains("still contains an absolute path at $.static_bundle."),
            "the refusal must name the offending path: {error}"
        );
        assert!(
            !error.contains("must be the retained canonical absolute path"),
            "the old sealed-side requirement must be gone: {error}"
        );
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
        let retained_bytes = portable_test_fs_retry::read(&retained_path).unwrap();
        let mut retained = snapshot::parse_json_strict(&retained_bytes).unwrap();
        let arbitrary = "a".repeat(64);
        retained["bundle_id"] = json!(arbitrary);
        let forged_bytes = serde_json::to_vec_pretty(&retained).unwrap();
        fs::write(&retained_path, &forged_bytes).unwrap();
        manifest["static_bundle"]["bundle_id"] = json!(arbitrary);
        manifest["static_bundle"]["manifest"]["sha256"] = json!(Sha256::hex(&forged_bytes));
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let error = load_verified_replay_authority(
            &manifest,
            &manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
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
        assert!(error.contains("not the canonical static"), "{error}");

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
        assert!(
            error.contains("unexpected static role \"forged_extra\""),
            "{error}"
        );

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
        let error =
            load_verified_replay_authority(&manifest, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                .unwrap_err();
        assert!(error.contains("game_id"), "{error}");

        write_replay_fixture(&dir.0, "expected", &bytes, false);
        let error =
            load_verified_replay_authority(&manifest, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
                .unwrap_err();
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
        for packaging in [
            snapshot::RawStreamPackaging::Packaged,
            snapshot::RawStreamPackaging::Omitted,
        ] {
            assert!(
                replay_layout(&manifest, packaging).is_err(),
                "{packaging:?}"
            );
        }
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
        let d47_legacy_status =
            0x1234u32 & !characters_leaders_cabinet_live_lane::STATUS_PARTIAL_DATA;
        let d47 =
            characters_leaders_cabinet_live_lane::encode_payload(0, d47_legacy_status, 0, &[]);
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
        assert_eq!(d47.status_flags, Some(d47_legacy_status));

        let d47_declared = characters_leaders_cabinet_live_lane::encode_payload_with_families(
            0,
            0,
            characters_leaders_cabinet_live_lane::SCHEMA_ROOTS_VALIDATED
                | characters_leaders_cabinet_live_lane::SCHEMA_STATIC_NAME_CLOSURE
                | characters_leaders_cabinet_live_lane::SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
            characters_leaders_cabinet_live_lane::FAMILY_CENSUS_ONLY,
            &[],
        );
        let d47_declared = recorder_host::domain_data_authority::summarize_domain_data_payload(
            47,
            &d47_declared,
            recorder_host::domain_data_authority::DomainDataContext::default(),
        )
        .unwrap();
        assert_eq!(
            d47_declared.status_flags,
            Some(characters_leaders_cabinet_live_lane::STATUS_PARTIAL_DATA)
        );

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
        summary
            .per_hour_perf_domain_counts
            .insert(100, BTreeMap::from([(10, 1)]));
        summary
            .per_hour_data_domain_counts
            .insert(100, BTreeMap::from([(10, 1)]));
        let manifest_domain = json!({
            "record_count": 1,
            "data_record_count": 1,
            "read_time_us": {"count": 1},
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
            "record_count",
            "data_record_count",
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

        let mut forged_timing = compact.clone();
        forged_timing["per_domain"]["fuel_live"]["read_time_us"]["count"] = json!(0);
        let mut timing_errors = Vec::new();
        validate_scanned_domain_authority(
            &summary,
            &forged_timing,
            &[contract],
            &mut timing_errors,
        );
        assert!(timing_errors
            .iter()
            .any(|error| error.contains("timing cardinality")));

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
            let result = recorder_host::stream_fold::validate_domain_data_payload_at(
                descriptor.id,
                &[],
                0,
                0,
                0,
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
            recorder_local_settings: BTreeMap::new(),
            required_content: Vec::new(),
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
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
            recorder_host::attach_content_proof::CONTENT_PROOF_KIND_FIELD:
                recorder_host::attach_content_proof::KIND_LAUNCH_TIME_STATIC_BUNDLE_V1,
            recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD: Value::Null,
            "replay_entries": {
                "stream": "collector_raw/collector_stream.bin",
                "stream_role": "collector_compact_stream",
                "session_manifest": "recorder/native_session_manifest.json",
                "session_manifest_role": "recorder_manifest"
            },
            "host_finalize": {
                "streaming": true,
                "stream_bytes": 4096,
                "stream_stored_bytes": 4096,
                "stream_sha256": "a".repeat(64),
                "fold": {
                    "hours_emitted": 7,
                    "records_total": 70,
                    "excluded_domain_data": {
                        "complete": true,
                        "per_domain": {},
                        "records": [],
                        "total_records": 0,
                    },
                },
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

    fn attach_time_manifest(game_id: &str, evidence_root: &Path, packaged: &[&str]) -> Value {
        let mut manifest = replay_test_manifest(game_id, evidence_root);
        let object = manifest.as_object_mut().unwrap();
        object.insert("static_bundle".to_string(), Value::Null);
        object.insert("portable_content_manifest".to_string(), Value::Null);
        object.insert(
            recorder_host::attach_content_proof::CONTENT_PROOF_KIND_FIELD.to_string(),
            json!(recorder_host::attach_content_proof::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1),
        );
        object["production_profile"]["enabled"] = json!(false);
        object.insert(
            "scheduled_domains".to_string(),
            json!([compact_contract::rail_replay::DOMAIN_NAME, "fuel_live"]),
        );
        object["production_profile"]["missing_required_static_artifacts"] =
            json!(compact_contract::REQUIRED_STATIC_ARTIFACTS
                .iter()
                .filter(|role| !packaged.contains(role))
                .collect::<Vec<_>>());
        object["replay_packaging"]["static_artifacts"] = json!(packaged
            .iter()
            .map(|role| json!({
                "name": role,
                "sha256": Sha256::hex(&replay_test_static_artifact_bytes(role)),
            }))
            .collect::<Vec<_>>());
        let layers = vec![recorder_host::attach_content_proof::ProofLayer {
            ordinal: 0,
            kind: "base",
            layer_id: "base".into(),
            descriptor_sha256: None,
            descriptor_file_sha256: None,
            source_container_sha256: None,
        }];
        object.insert(
            recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD.to_string(),
            json!({
                "schema": recorder_host::attach_content_proof::SCHEMA,
                "kind": recorder_host::attach_content_proof::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1,
                "game_process_start_unix": 1_786_652_129i64,
                "observed_at_unix": 1_786_653_129i64,
                "completed_at_unix": 1_786_653_193i64,
                "window_attempts": 1,
                "window_settled": true,
                "executable_sha256": "8".repeat(64),
                "executable_size_bytes": 55_680_632u64,
                "dlc_load_sha256": "7".repeat(64),
                "layer_stack": [{
                    "ordinal": 0,
                    "kind": "base",
                    "layer_id": "base",
                    "descriptor_sha256": Value::Null,
                    "descriptor_file_sha256": Value::Null,
                    "source_container_sha256": Value::Null,
                }],
                "layer_stack_sha256":
                    recorder_host::attach_content_proof::layer_stack_sha256(&layers).unwrap(),
                "artifact_joins": packaged
                    .iter()
                    .map(|role| json!({
                        "role": role,
                        "artifact_sha256": Sha256::hex(&replay_test_static_artifact_bytes(role)),
                        "claimed_paths": 11u64,
                        "joined_bytes": 2_048u64,
                        "proven_absent_paths": 0u64,
                        "localisation_source_sha256": Value::Null,
                    }))
                    .collect::<Vec<_>>(),
                "refused_artifacts": [],
                "required_content": [{
                    "purpose": "D81 province catalog",
                    "logical_path": "map/definition.csv",
                    "sha256": "6".repeat(64),
                    "bytes": 1_234u64,
                    "winning_layer_id": "base",
                }],
                "refused_required_content": [],
            }),
        );
        manifest
    }

    fn write_attach_time_replay_fixture(
        dir: &Path,
        game_id: &str,
        manifest_bytes: &[u8],
        packaged: &[&str],
    ) -> Vec<u8> {
        write_attach_time_replay_fixture_with_index_mutation(
            dir,
            game_id,
            manifest_bytes,
            packaged,
            |_| {},
        )
    }

    fn write_attach_time_replay_fixture_with_index_mutation<F>(
        dir: &Path,
        game_id: &str,
        manifest_bytes: &[u8],
        packaged: &[&str],
        mutate_index: F,
    ) -> Vec<u8>
    where
        F: FnOnce(&mut Vec<Value>),
    {
        let sealed_bytes = sealed_manifest_bytes(manifest_bytes);
        let mut descriptors = Vec::new();
        let mut entries = vec![
            snapshot::PackEntry {
                path: format!("payload/{game_id}/collector_raw/collector_stream.bin"),
                bytes: b"stream".to_vec(),
                role: "collector_compact_stream",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game_id}/recorder/native_session_manifest.json"),
                bytes: sealed_bytes.clone(),
                role: "recorder_manifest",
                hour_index: None,
            },
        ];
        for role in compact_contract::REQUIRED_STATIC_ARTIFACTS {
            if !packaged.contains(role) {
                descriptors.push(json!({
                    "name": role,
                    "present": false,
                    "role": "static_artifact",
                    "absence_reason": "not supplied to the recorder for this recording",
                }));
                continue;
            }
            let bytes = replay_test_static_artifact_bytes(role);
            let relative_path = required_static_artifact_relative_path(role).unwrap();
            descriptors.push(json!({
                "name": role,
                "present": true,
                "role": "static_artifact",
                "path": relative_path.clone(),
                "sha256": Sha256::hex(&bytes),
                "bytes": bytes.len(),
            }));
            entries.push(snapshot::PackEntry {
                path: format!("payload/{game_id}/{relative_path}"),
                bytes,
                role: "static_artifact",
                hour_index: None,
            });
        }
        mutate_index(&mut descriptors);
        entries.insert(
            0,
            snapshot::PackEntry {
                path: format!("payload/{game_id}/index.json"),
                bytes: serde_json::to_vec(&json!({
                    "game_id": game_id,
                    "schema_version": "1",
                    "snapshots": [],
                    "static_artifacts": descriptors,
                }))
                .unwrap(),
                role: "index",
                hour_index: None,
            },
        );
        let packed = snapshot::pack(
            game_id,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some(REPLAY_CONTAINER_SCHEMA_VERSION.to_string()),
                content_manifest: None,
                ..snapshot::PackIdentity::default()
            },
            false,
        )
        .unwrap();
        fs::write(dir.join("native_session.battleplan"), packed).unwrap();
        sealed_bytes
    }

    #[test]
    fn an_attach_time_recording_certifies_and_every_break_refuses_by_name() {
        let packaged = [
            compact_contract::STATIC_ARTIFACT_TEMPLATES_EQUIPMENT_CATALOG,
            compact_contract::STATIC_ARTIFACT_LOCALISATION_JOIN,
            compact_contract::STATIC_ARTIFACT_DECISIONS_STATIC_DEFINITIONS,
            compact_contract::STATIC_ARTIFACT_IDEA_TECH_LOC_XREF_STATIC,
            compact_contract::STATIC_ARTIFACT_SHIP_DEFINITIONS_STATIC,
            compact_contract::STATIC_ARTIFACT_GFX_ICON_MANIFEST,
        ];
        let load = |label: &str, mutate: &dyn Fn(&mut Value)| -> Result<Value, String> {
            let dir = TmpDir::new(label);
            let mut manifest = attach_time_manifest("g", &dir.0, &packaged);
            mutate(&mut manifest);
            let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
            write_attach_time_replay_fixture(&dir.0, "g", &manifest_bytes, &packaged);
            load_verified_replay_authority(
                &manifest,
                &manifest_bytes,
                &dir.0,
                TEST_MAX_STREAM_BYTES,
            )
            .map(|authority| authority.details)
        };

        let details = load("attach_ok", &|_| {}).expect("an attach-time recording certifies");
        let proof = &details["static_artifacts"];
        assert_eq!(
            proof["content_proof_kind"],
            json!(recorder_host::attach_content_proof::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1)
        );
        assert_eq!(proof["static_artifacts_joined"], json!(true));
        assert_eq!(proof["exact_required_roles"], json!(false));
        assert_eq!(proof["attach_window_seconds"], json!(64));
        assert_eq!(proof["artifacts"].as_array().unwrap().len(), packaged.len());
        assert!(
            proof["portable_content_manifest"]
                .as_str()
                .unwrap()
                .starts_with("absent:"),
            "the missing launch-time epoch must be NAMED, not inferred: {proof}"
        );

        let breaks: Vec<(&str, Box<dyn Fn(&mut Value)>, &str)> = vec![
            (
                "attach_kind_none",
                Box::new(|manifest: &mut Value| {
                    manifest[recorder_host::attach_content_proof::CONTENT_PROOF_KIND_FIELD] =
                        json!(recorder_host::attach_content_proof::KIND_NONE);
                }),
                "certifies no content",
            ),
            (
                "attach_unsettled_window",
                Box::new(|manifest: &mut Value| {
                    manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["window_settled"] = json!(false);
                }),
                "unsettled attach window",
            ),
            (
                "attach_forged_stack_digest",
                Box::new(|manifest: &mut Value| {
                    manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["layer_stack_sha256"] = json!("1".repeat(64));
                }),
                "is not the digest of the layer rows",
            ),
            (
                "attach_unjoined_role",
                Box::new(|manifest: &mut Value| {
                    let joins = manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["artifact_joins"]
                        .as_array_mut()
                        .unwrap();
                    joins.pop();
                }),
                "is packaged but has no attach-time join receipt",
            ),
            (
                "attach_receipt_for_other_build",
                Box::new(|manifest: &mut Value| {
                    manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["artifact_joins"][0]["artifact_sha256"] = json!("2".repeat(64));
                }),
                "the receipt covers a different build of the artifact",
            ),
            (
                "attach_joined_and_refused",
                Box::new(|manifest: &mut Value| {
                    let role = manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["artifact_joins"][0]["role"]
                        .clone();
                    manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["refused_artifacts"] =
                        json!([{ "role": role, "reason": "fixture contradiction" }]);
                }),
                "both joins and refuses static role",
            ),
            (
                "attach_role_in_neither_list",
                Box::new(|manifest: &mut Value| {
                    manifest["production_profile"]["missing_required_static_artifacts"] = json!([]);
                }),
                "!= the canonical roster",
            ),
            (
                "attach_role_in_both_lists",
                Box::new(|manifest: &mut Value| {
                    manifest["production_profile"]["missing_required_static_artifacts"] =
                        json!(compact_contract::REQUIRED_STATIC_ARTIFACTS
                            .iter()
                            .collect::<Vec<_>>());
                }),
                "declared both packaged and missing",
            ),
            (
                "attach_required_content_refused",
                Box::new(|manifest: &mut Value| {
                    manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["refused_required_content"] = json!([{
                        "purpose": "D81 province catalog",
                        "logical_path": "map/definition.csv",
                        "reason": "absent from the attach-time effective VFS",
                    }]);
                }),
                "playback-required content file",
            ),
            (
                "attach_empty_join",
                Box::new(|manifest: &mut Value| {
                    manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["artifact_joins"][0]["claimed_paths"] = json!(0);
                }),
                "joined no paths at all",
            ),
            (
                "attach_no_proof_block",
                Box::new(|manifest: &mut Value| {
                    manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD] =
                        Value::Null;
                }),
                "declares the attach-time proof but carries no",
            ),
            (
                "attach_empty_layer_stack",
                Box::new(|manifest: &mut Value| {
                    let layers: Vec<recorder_host::attach_content_proof::ProofLayer> = Vec::new();
                    let proof = &mut manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD];
                    proof["layer_stack"] = json!([]);
                    proof["layer_stack_sha256"] = json!(
                        recorder_host::attach_content_proof::layer_stack_sha256(&layers).unwrap()
                    );
                }),
                "states no content layers",
            ),
            (
                "attach_packages_nothing",
                Box::new(|manifest: &mut Value| {
                    manifest["replay_packaging"]["static_artifacts"] = json!([]);
                    manifest["production_profile"]["missing_required_static_artifacts"] =
                        json!(compact_contract::REQUIRED_STATIC_ARTIFACTS
                            .iter()
                            .collect::<Vec<_>>());
                }),
                "packages no static artifacts at all",
            ),
            (
                "attach_required_content_silently_absent",
                Box::new(|manifest: &mut Value| {
                    manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["required_content"] = json!([]);
                }),
                "consumes playback-required content that the attach-time proof does not join",
            ),
            (
                "attach_required_content_won_by_an_unstated_layer",
                Box::new(|manifest: &mut Value| {
                    manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["required_content"][0]["winning_layer_id"] = json!("mod/ghost.mod");
                }),
                "is not in the layer stack the proof states",
            ),
            (
                "attach_kind_over_pre_resume_evidence",
                Box::new(|manifest: &mut Value| {
                    manifest["early_render_launch_manifest"] =
                        json!({ "fixture": "pre-resume evidence" });
                }),
                "carries pre-resume evidence",
            ),
            (
                "attach_no_process_start",
                Box::new(|manifest: &mut Value| {
                    manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD]
                        ["game_process_start_unix"] = Value::Null;
                }),
                "game_process_start_unix is missing or not an integer",
            ),
            (
                "attach_process_started_after_the_window",
                Box::new(|manifest: &mut Value| {
                    let proof = &mut manifest
                        [recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD];
                    let observed = proof["observed_at_unix"].as_i64().unwrap();
                    proof["game_process_start_unix"] = json!(observed + 1);
                }),
                "the engine starting after the attach window opened",
            ),
        ];
        for (label, mutate, expected) in breaks {
            let error = load(label, mutate.as_ref())
                .map(|details| format!("unexpectedly certified: {details}"))
                .unwrap_err();
            assert!(
                error.contains(expected),
                "{label}: expected a refusal naming {expected:?}, got {error}"
            );
        }
    }

    #[test]
    fn an_unpackaged_role_must_be_badged_absent_with_a_reason() {
        let packaged = [
            compact_contract::STATIC_ARTIFACT_TEMPLATES_EQUIPMENT_CATALOG,
            compact_contract::STATIC_ARTIFACT_LOCALISATION_JOIN,
            compact_contract::STATIC_ARTIFACT_DECISIONS_STATIC_DEFINITIONS,
            compact_contract::STATIC_ARTIFACT_IDEA_TECH_LOC_XREF_STATIC,
            compact_contract::STATIC_ARTIFACT_SHIP_DEFINITIONS_STATIC,
            compact_contract::STATIC_ARTIFACT_GFX_ICON_MANIFEST,
        ];
        let cases: Vec<(&str, Box<dyn Fn(&mut Vec<Value>)>, &str)> = vec![
            (
                "index_absent_row_claims_present",
                Box::new(|rows: &mut Vec<Value>| {
                    for row in rows.iter_mut() {
                        if row["present"] == json!(false) {
                            row["present"] = json!(true);
                        }
                    }
                }),
                "is not badged present=false",
            ),
            (
                "index_absent_row_without_a_reason",
                Box::new(|rows: &mut Vec<Value>| {
                    for row in rows.iter_mut() {
                        if row["present"] == json!(false) {
                            row.as_object_mut().unwrap().remove("absence_reason");
                        }
                    }
                }),
                "is absent but names no reason",
            ),
            (
                "index_drops_the_absent_rows_entirely",
                Box::new(|rows: &mut Vec<Value>| {
                    rows.retain(|row| row["present"] == json!(true));
                }),
                "not the canonical static",
            ),
        ];
        for (label, mutate, expected) in cases {
            let dir = TmpDir::new(label);
            let manifest = attach_time_manifest("g", &dir.0, &packaged);
            let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
            write_attach_time_replay_fixture_with_index_mutation(
                &dir.0,
                "g",
                &manifest_bytes,
                &packaged,
                mutate.as_ref(),
            );
            let error = load_verified_replay_authority(
                &manifest,
                &manifest_bytes,
                &dir.0,
                TEST_MAX_STREAM_BYTES,
            )
            .map(|authority| format!("unexpectedly certified: {}", authority.details))
            .unwrap_err();
            assert!(
                error.contains(expected),
                "{label}: expected a refusal naming {expected:?}, got {error}"
            );
        }
    }

    #[test]
    fn the_content_proof_route_is_read_positively_and_never_inferred() {
        let dir = TmpDir::new("content_proof_route");
        let mut manifest = replay_test_manifest("g", &dir.0);
        manifest
            .as_object_mut()
            .unwrap()
            .remove(recorder_host::attach_content_proof::CONTENT_PROOF_KIND_FIELD);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let error = load_verified_replay_authority(
            &manifest,
            &manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .unwrap_err();
        assert!(
            error.contains("carries no content_proof_kind"),
            "an absent declaration must refuse, never default to the launch-time route: {error}"
        );

        let dir = TmpDir::new("content_proof_route_swap");
        let mut manifest = replay_test_manifest("g", &dir.0);
        manifest[recorder_host::attach_content_proof::CONTENT_PROOF_KIND_FIELD] =
            json!(recorder_host::attach_content_proof::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let error = load_verified_replay_authority(
            &manifest,
            &manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .unwrap_err();
        assert!(
            error.contains("declares the attach-time proof but carries no"),
            "{error}"
        );

        let dir = TmpDir::new("content_proof_route_both");
        let mut manifest = replay_test_manifest("g", &dir.0);
        manifest[recorder_host::attach_content_proof::ATTACH_TIME_CONTENT_PROOF_FIELD] = json!({
            "schema": recorder_host::attach_content_proof::SCHEMA,
            "kind": recorder_host::attach_content_proof::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1,
        });
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_replay_fixture(&dir.0, "g", &manifest_bytes, true);
        let error = load_verified_replay_authority(
            &manifest,
            &manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .unwrap_err();
        assert!(
            error.contains("declares the launch-time content epoch but carries an"),
            "{error}"
        );
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

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum SealedCopy {
        Portable,
        Verbatim,
    }

    fn fixture_hour_coverage(
        verified: &[i64],
        excluded: &[(i64, &str)],
    ) -> recorder_host::hour_coverage::HourCoverageMap {
        let mut map = recorder_host::hour_coverage::HourCoverageMap::new();
        for hour in verified {
            let raw = compact_contract::calendar::DATE_EPOCH_CONST + *hour as i32;
            map.record_verified(raw, 0, 64, Sha256::hex(format!("hour {hour}").as_bytes()));
        }
        for (hour, reason) in excluded {
            let raw = compact_contract::calendar::DATE_EPOCH_CONST + *hour as i32;
            map.record_excluded(raw, 0, 0, *reason);
        }
        map
    }

    fn sealed_manifest_bytes(manifest_bytes: &[u8]) -> Vec<u8> {
        let mut manifest: Value = serde_json::from_slice(manifest_bytes).unwrap();
        if let Some(finalize) = manifest
            .get_mut("host_finalize")
            .and_then(Value::as_object_mut)
        {
            let field = recorder_host::hour_coverage::HOUR_COVERAGE_FIELD;
            if !finalize.contains_key(field) {
                finalize.insert(
                    field.to_string(),
                    fixture_hour_coverage(&[0, 1, 2], &[]).to_json(),
                );
            }
        }
        recorder_host::portable_manifest::portable_session_manifest_bytes(&manifest)
            .expect("a fixture run-dir manifest must have a portable projection")
    }

    #[test]
    fn a_container_may_not_publish_an_hour_its_coverage_map_refuses() {
        let path = Path::new("fixture.battleplan");
        let clean = fixture_hour_coverage(&[0, 1, 2], &[]);
        refuse_hours_the_coverage_map_does_not_vouch_for(&[0, 1, 2], &clean, path)
            .expect("every published hour is verified");

        let injured = fixture_hour_coverage(&[0, 2], &[(1, "framing damage at byte 421")]);
        let error = refuse_hours_the_coverage_map_does_not_vouch_for(&[0, 1, 2], &injured, path)
            .unwrap_err();
        assert!(
            error.contains("does not vouch for") && error.contains("framing damage at byte 421"),
            "the refusal must name the hour AND why it was excluded: {error}"
        );

        let silent = fixture_hour_coverage(&[0, 2], &[]);
        let error = refuse_hours_the_coverage_map_does_not_vouch_for(&[0, 1, 2], &silent, path)
            .unwrap_err();
        assert!(
            error.contains("names this hour nowhere at all"),
            "an unmentioned hour is not covered: {error}"
        );
    }

    #[test]
    fn a_cache_only_container_serves_around_a_named_interior_hole() {
        const SEALED_REASON: &str = "sequence discontinuity at byte 421: 3 -> 7";
        let dir = TmpDir::new("gz_interior_hole_authority");
        let recording = write_cache_only_replay_fixture(
            &dir.0,
            "g",
            CacheOnlyBreak {
                index_hole_reason: Some(SEALED_REASON.to_string()),
                sealed_coverage: Some((vec![0, 2], vec![(1, SEALED_REASON.to_string())])),
                ..CacheOnlyBreak::default()
            },
        );
        let authority = load_verified_replay_authority(
            &recording.manifest,
            &recording.manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect("an interior hole must not cost the whole recording");

        assert_eq!(
            authority.details["indexed_hour_indexes"],
            json!([0, 2]),
            "every verified hour is still served"
        );
        assert_eq!(
            authority.details[INDEX_EXCLUDED_HOURS_FIELD][0]["hour_index"],
            json!(1)
        );
        assert_eq!(
            authority.details[INDEX_EXCLUDED_HOURS_FIELD][0]["reason"],
            json!(SEALED_REASON),
            "the excluded hour is refused BY NAME, in the same words the seal used"
        );
        let excluded =
            &authority.details[recorder_host::hour_coverage::HOUR_COVERAGE_FIELD]["excluded_hours"];
        assert_eq!(excluded[0]["reason"], json!(SEALED_REASON));

        let paraphrased = cache_only_authority_error(
            "gz_hole_paraphrased",
            CacheOnlyBreak {
                index_hole_reason: Some("something went wrong".to_string()),
                sealed_coverage: Some((vec![0, 2], vec![(1, SEALED_REASON.to_string())])),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            paraphrased.contains("names each injured hour once"),
            "a paraphrase is a second author: {paraphrased}"
        );

        let invented = cache_only_authority_error(
            "gz_hole_invented",
            CacheOnlyBreak {
                index_hole_reason: Some(SEALED_REASON.to_string()),
                sealed_coverage: Some((vec![0, 1, 2], vec![])),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            invented.contains("sealed coverage map VERIFIES that hour"),
            "an invented hole must be refused: {invented}"
        );

        let unmentioned = cache_only_authority_error(
            "gz_hole_unmentioned",
            CacheOnlyBreak {
                index_hole_reason: Some(SEALED_REASON.to_string()),
                sealed_coverage: Some((vec![0, 2], vec![])),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            unmentioned.contains("names that hour nowhere at all"),
            "absence is not a claim, in this direction either: {unmentioned}"
        );
    }

    #[test]
    fn post_seal_cache_record_damage_costs_its_checkpoint_window() {
        let dir = TmpDir::new("ha_post_seal_record_rot");
        let recording = write_cache_only_replay_fixture(
            &dir.0,
            "g",
            CacheOnlyBreak {
                cache_hours: Some(4),
                checkpoint_interval: Some(2),
                rot_cache_record: Some(2),
                sealed_coverage: Some((vec![0, 1, 2, 3], vec![])),
                ..CacheOnlyBreak::default()
            },
        );
        let authority = load_verified_replay_authority(
            &recording.manifest,
            &recording.manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect("one damaged record must not cost the container");

        assert_eq!(
            authority.details["indexed_hour_indexes"],
            json!([0, 1, 2, 3])
        );
        assert_eq!(
            authority.details["checkpoint_cache_matches_seal_whole"],
            json!(false),
            "the whole-artifact claim must report itself false rather than be omitted"
        );
        assert_eq!(
            authority.details["checkpoint_cache_damaged_records"],
            json!(1)
        );

        let confined = authority.details[CACHE_DAMAGE_CONFINED_HOURS_FIELD]
            .as_array()
            .expect("confined hours are reported")
            .clone();
        let hours: Vec<i64> = confined
            .iter()
            .map(|row| row["hour_index"].as_i64().unwrap())
            .collect();
        assert_eq!(hours, vec![2, 3], "the second window, and only it");
        for row in &confined {
            assert_eq!(row["damaged_record_hour"], json!(2));
            assert_eq!(row["checkpoint_hour"], json!(2));
            assert!(
                row["reason"]
                    .as_str()
                    .unwrap()
                    .contains("one damaged record costs its checkpoint window"),
                "{row}"
            );
        }

        let coverage = &authority.details[recorder_host::hour_coverage::HOUR_COVERAGE_FIELD];
        assert_eq!(coverage["verified_hours"], json!(2));
        let excluded: Vec<i64> = coverage["excluded_hours"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                row["game_hour_raw"].as_i64().unwrap()
                    - i64::from(compact_contract::calendar::DATE_EPOCH_CONST)
            })
            .collect();
        assert_eq!(excluded, vec![2, 3]);
        assert!(
            coverage["excluded_hours"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("game hour 2"),
            "the exclusion names the record that rotted: {coverage}"
        );
    }

    #[test]
    fn unattributable_cache_damage_still_refuses_the_whole_container() {
        let header = cache_only_authority_error(
            "ha_post_seal_header_rot",
            CacheOnlyBreak {
                cache_hours: Some(4),
                checkpoint_interval: Some(2),
                rot_cache_header: true,
                sealed_coverage: Some((vec![0, 1, 2, 3], vec![])),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            header.contains(CACHE_ONLY_LINK_CHECKPOINT_CACHE),
            "{header}"
        );
        assert!(
            header.contains("header is not bound to source"),
            "the header refusal names the identity it broke: {header}"
        );

        let substituted = cache_only_authority_error(
            "ha_index_and_seal_disagree",
            CacheOnlyBreak {
                cache_hours: Some(4),
                checkpoint_interval: Some(2),
                sealed_cache_digest: Some("c".repeat(64)),
                sealed_coverage: Some((vec![0, 1, 2, 3], vec![])),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            substituted.contains(CACHE_ONLY_LINK_SEALED_ARTIFACTS),
            "{substituted}"
        );
        assert!(
            substituted.contains("describe different caches"),
            "{substituted}"
        );
    }

    #[test]
    fn a_replay_artifact_indexed_as_an_hours_snapshot_refuses_the_container() {
        let damaged_alias = cache_only_authority_error(
            "ha_aliased_cache_behind_damaged",
            CacheOnlyBreak {
                cache_hours: Some(4),
                checkpoint_interval: Some(2),
                rot_cache_record: Some(2),
                alias_cache_as_snapshot_hour: Some(1),
                sealed_coverage: Some((vec![0, 1, 2, 3], vec![])),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            damaged_alias.contains("indexes 1 replay_decoder artifact(s) as per-hour snapshots"),
            "{damaged_alias}"
        );
        assert!(damaged_alias.contains("hour_index 1"), "{damaged_alias}");

        let clean_alias = cache_only_authority_error(
            "ha_aliased_cache_behind_clean",
            CacheOnlyBreak {
                cache_hours: Some(4),
                checkpoint_interval: Some(2),
                alias_cache_as_snapshot_hour: Some(1),
                sealed_coverage: Some((vec![0, 1, 2, 3], vec![])),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            clean_alias.contains("indexes 1 replay_decoder artifact(s) as per-hour snapshots"),
            "{clean_alias}"
        );

        let first = cache_only_authority_error(
            "ha_aliased_cache_first",
            CacheOnlyBreak {
                cache_hours: Some(4),
                checkpoint_interval: Some(2),
                alias_cache_as_first_snapshot_hour: Some(9),
                sealed_coverage: Some((vec![0, 1, 2, 3], vec![])),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            first.contains("cannot open replay container") && first.contains("container json"),
            "an aliased artifact must not decode as the representative snapshot: {first}"
        );
    }

    #[test]
    fn a_packaged_artifact_must_be_the_one_the_seal_certified() {
        let swapped_index = cache_only_authority_error(
            "gz_index_not_sealed",
            CacheOnlyBreak {
                sealed_index_digest: Some("ab".repeat(32)),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            swapped_index.contains(CACHE_ONLY_LINK_SEALED_ARTIFACTS)
                && swapped_index.contains("not the one the seal certified")
                && swapped_index.contains("hour_index_sha256"),
            "a packaged index the seal never certified must refuse by this link's own name, \
             naming the index receipt: {swapped_index}"
        );

        let swapped_cache = cache_only_authority_error(
            "gz_cache_not_sealed",
            CacheOnlyBreak {
                sealed_cache_digest: Some("cd".repeat(32)),
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            swapped_cache.contains(CACHE_ONLY_LINK_SEALED_ARTIFACTS)
                && swapped_cache.contains("checkpoint_cache_sha256"),
            "a packaged cache the seal never certified must refuse by this link's own name, \
             naming the cache receipt: {swapped_cache}"
        );

        let absent = cache_only_authority_error(
            "gz_no_artifact_receipt",
            CacheOnlyBreak {
                omit_artifact_receipts: true,
                ..CacheOnlyBreak::default()
            },
        );
        assert!(
            absent.contains(CACHE_ONLY_LINK_SEALED_ARTIFACTS)
                && absent.contains("carries no 64-character"),
            "an artifact-to-seal binding a consumer has to assume is not a binding: {absent}"
        );
    }

    #[test]
    fn a_sealed_recorder_manifest_without_a_coverage_map_is_refused() {
        let error = read_sealed_hour_coverage(&json!({ "host_finalize": {} })).unwrap_err();
        assert!(
            error.contains("carries no host_finalize.hour_coverage"),
            "{error}"
        );

        let mut forged = fixture_hour_coverage(&[0], &[]).to_json();
        forged["hours"][0]
            .as_object_mut()
            .unwrap()
            .insert("basis_sha256".to_string(), json!(""));
        let error =
            read_sealed_hour_coverage(&json!({ "host_finalize": { "hour_coverage": forged } }))
                .unwrap_err();
        assert!(
            error.contains("without a digest-backed basis"),
            "a verified hour with no basis must be refused at the decoder: {error}"
        );

        let good = fixture_hour_coverage(&[0, 1], &[(2, "the producer refused this frame")]);
        let read = read_sealed_hour_coverage(
            &json!({ "host_finalize": { "hour_coverage": good.to_json() } }),
        )
        .expect("an admissible map round-trips through the sealed manifest");
        assert_eq!(read, good);
        assert_eq!(read.excluded_hours().len(), 1);
    }

    fn write_replay_fixture(
        dir: &Path,
        game_id: &str,
        manifest_bytes: &[u8],
        include_manifest_role: bool,
    ) -> Vec<u8> {
        write_replay_fixture_with_static_mutation(
            dir,
            game_id,
            manifest_bytes,
            include_manifest_role,
            |_, _| {},
        )
    }

    fn write_replay_fixture_sealed_verbatim(
        dir: &Path,
        game_id: &str,
        sealed_bytes: &[u8],
        include_manifest_role: bool,
    ) {
        write_replay_fixture_full(
            dir,
            game_id,
            sealed_bytes,
            include_manifest_role,
            REPLAY_CONTAINER_SCHEMA_VERSION,
            SealedCopy::Verbatim,
            |_, _| {},
        );
    }

    fn write_replay_fixture_with_static_mutation<F>(
        dir: &Path,
        game_id: &str,
        manifest_bytes: &[u8],
        include_manifest_role: bool,
        mutate: F,
    ) -> Vec<u8>
    where
        F: FnOnce(&mut Vec<Value>, &mut Vec<snapshot::PackEntry>),
    {
        write_replay_fixture_full(
            dir,
            game_id,
            manifest_bytes,
            include_manifest_role,
            REPLAY_CONTAINER_SCHEMA_VERSION,
            SealedCopy::Portable,
            mutate,
        )
    }

    fn write_replay_fixture_full<F>(
        dir: &Path,
        game_id: &str,
        manifest_bytes: &[u8],
        include_manifest_role: bool,
        container_schema_version: &str,
        sealed_copy: SealedCopy,
        mutate: F,
    ) -> Vec<u8>
    where
        F: FnOnce(&mut Vec<Value>, &mut Vec<snapshot::PackEntry>),
    {
        let sealed_bytes = match sealed_copy {
            SealedCopy::Portable => sealed_manifest_bytes(manifest_bytes),
            SealedCopy::Verbatim => manifest_bytes.to_vec(),
        };
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
            bytes: sealed_bytes.clone(),
            role: if include_manifest_role {
                "recorder_manifest"
            } else {
                "other"
            },
            hour_index: None,
        });
        entries.append(&mut static_entries);
        let session_manifest: Value = serde_json::from_slice(&sealed_bytes).unwrap();
        let product_modset_sha256 = session_manifest
            .pointer("/portable_content_manifest/product_modset_sha256")
            .and_then(Value::as_str)
            .expect("fixture recorder manifest product-modset key");
        let packed = snapshot::pack(
            game_id,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some(container_schema_version.to_string()),
                content_manifest: Some(replay_test_content_manifest(product_modset_sha256)),
                ..snapshot::PackIdentity::default()
            },
            false,
        )
        .unwrap();
        fs::write(dir.join("native_session.battleplan"), packed).unwrap();
        sealed_bytes
    }

    struct CacheOnlyRecording {
        manifest: Value,
        manifest_bytes: Vec<u8>,
        stream_sha256: String,
        stream_bytes: u64,
    }

    #[derive(Default)]
    struct CacheOnlyBreak {
        cache_folded_from: Option<Vec<u8>>,
        index_source_len: Option<u64>,
        cache_header_digest_of: Option<Vec<u8>>,
        stream_storage: Option<Value>,
        package_stream_entry: bool,
        omit_run_dir_stream: bool,
        corrupt_cache_bytes: bool,
        run_dir_stream_override: Option<Vec<u8>>,
        container_schema_version: Option<String>,
        manifest_schema: Option<String>,
        index_hole_reason: Option<String>,
        sealed_coverage: Option<(Vec<i64>, Vec<(i64, String)>)>,
        sealed_index_digest: Option<String>,
        sealed_cache_digest: Option<String>,
        omit_artifact_receipts: bool,
        cache_hours: Option<usize>,
        checkpoint_interval: Option<u32>,
        rot_cache_record: Option<usize>,
        rot_cache_header: bool,
        alias_cache_as_snapshot_hour: Option<i64>,
        alias_cache_as_first_snapshot_hour: Option<i64>,
    }

    fn replace_unique_digest(bytes: &mut [u8], from_hex: &str, to_hex: &str) {
        let from = hex32(from_hex);
        let to = hex32(to_hex);
        let positions = bytes
            .windows(32)
            .enumerate()
            .filter(|(_, window)| *window == from.as_slice())
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        assert_eq!(
            positions.len(),
            1,
            "the fixture digest must appear exactly once in the H4IX image"
        );
        bytes[positions[0]..positions[0] + 32].copy_from_slice(&to);
    }

    fn rot_sealed_cache_byte(packed: &mut [u8], sealed_cache: &[u8], offset: usize) {
        let positions: Vec<usize> = packed
            .windows(sealed_cache.len())
            .enumerate()
            .filter(|(_, window)| *window == sealed_cache)
            .map(|(position, _)| position)
            .collect();
        assert_eq!(
            positions.len(),
            1,
            "the sealed cache must appear exactly once in the packed container"
        );
        packed[positions[0] + offset] ^= 0xff;
    }

    fn hex32(value: &str) -> Vec<u8> {
        assert_eq!(value.len(), 64, "sha256 hex is 64 characters");
        (0..32)
            .map(|index| u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap())
            .collect()
    }

    fn write_cache_only_replay_fixture(
        dir: &Path,
        game_id: &str,
        breakage: CacheOnlyBreak,
    ) -> CacheOnlyRecording {
        let stream = b"H4CRcache-only-run-directory-source".repeat(4);
        let stream_sha256 = Sha256::hex(&stream);

        let mut manifest = replay_test_manifest(game_id, dir);
        {
            let object = manifest.as_object_mut().unwrap();
            if let Some(schema) = &breakage.manifest_schema {
                object.insert("schema".to_string(), json!(schema));
            }
            object.insert(
                snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                json!(snapshot::RawStreamPackaging::Omitted.as_str()),
            );
            object.insert(
                "stream_storage".to_string(),
                breakage.stream_storage.clone().unwrap_or_else(|| {
                    json!({
                        "compression": "none",
                        "stored_bytes": stream.len(),
                        "decoded_bytes": stream.len(),
                        "decoded_sha256": stream_sha256,
                    })
                }),
            );
            let entries = object
                .get_mut("replay_entries")
                .unwrap()
                .as_object_mut()
                .unwrap();
            entries.remove("stream");
            entries.insert(
                "raw_stream_absence_reason".to_string(),
                json!(snapshot::RAW_STREAM_OMITTED_REASON),
            );
        }
        let folded_from = breakage
            .cache_folded_from
            .clone()
            .unwrap_or_else(|| stream.clone());
        let folded_sha256 = Sha256::hex(&folded_from);
        let header_sha256 = breakage
            .cache_header_digest_of
            .as_ref()
            .map(|bytes| Sha256::hex(bytes))
            .unwrap_or_else(|| folded_sha256.clone());
        let source_len = breakage
            .index_source_len
            .unwrap_or(folded_from.len() as u64);
        let mut encoder = snapshot::ReplayCacheEncoder::new(
            Vec::new(),
            &header_sha256,
            breakage.checkpoint_interval.unwrap_or(168),
        )
        .unwrap();
        match &breakage.index_hole_reason {
            None => {
                let hours = breakage.cache_hours.unwrap_or(1) as u64;
                let span = source_len / hours;
                for hour in 0..hours {
                    let start = span * hour;
                    let end = if hour + 1 == hours {
                        source_len
                    } else {
                        span * (hour + 1)
                    };
                    let date = format!("1936.1.1.{}", hour + 12);
                    let world = serde_json::to_vec(&json!({
                        "date": date,
                        "hour_index": hour,
                    }))
                    .unwrap();
                    encoder
                        .push(snapshot::CacheBuildHour {
                            hour_index: hour as i64,
                            date,
                            h4cr_start_offset: start,
                            h4cr_end_offset: end,
                            world_json: world,
                        })
                        .unwrap();
                }
            }
            Some(reason) => {
                let third = source_len / 3;
                for hour in 0..3i64 {
                    let start = third * hour as u64;
                    let end = if hour == 2 {
                        source_len
                    } else {
                        third * (hour as u64 + 1)
                    };
                    if hour == 1 {
                        encoder
                            .exclude_hour(snapshot::ReplayExcludedHour::new(
                                hour,
                                start,
                                end,
                                reason.clone(),
                            ))
                            .unwrap();
                        continue;
                    }
                    let date = format!("1936.1.1.{}", hour + 12);
                    let world = serde_json::to_vec(&json!({
                        "date": date,
                        "hour_index": hour,
                    }))
                    .unwrap();
                    encoder
                        .push(snapshot::CacheBuildHour {
                            hour_index: hour,
                            date,
                            h4cr_start_offset: start,
                            h4cr_end_offset: end,
                            world_json: world,
                        })
                        .unwrap();
                }
            }
        }
        let (mut cache_bytes, replay_index) = encoder
            .finish(
                game_id,
                "1",
                snapshot::CacheIdentityMode::Legacy,
                source_len,
            )
            .unwrap();
        let mut index_bytes = replay_index.to_bytes().unwrap();
        if breakage.cache_header_digest_of.is_some() {
            replace_unique_digest(&mut index_bytes, &header_sha256, &folded_sha256);
        }
        if breakage.corrupt_cache_bytes {
            *cache_bytes.last_mut().unwrap() ^= 0xff;
        }
        let sealed_cache_bytes = cache_bytes.clone();
        let record_windows: Vec<(u64, u64)> = replay_index
            .hours()
            .iter()
            .map(|row| (row.cache_record_offset(), row.cache_record_len()))
            .collect();

        if !breakage.omit_artifact_receipts {
            let fold = manifest
                .get_mut("host_finalize")
                .and_then(|finalize| finalize.get_mut("fold"))
                .and_then(Value::as_object_mut)
                .unwrap();
            fold.insert(
                "hour_index_sha256".to_string(),
                json!(breakage
                    .sealed_index_digest
                    .clone()
                    .unwrap_or_else(|| Sha256::hex(&index_bytes))),
            );
            fold.insert(
                "checkpoint_cache_sha256".to_string(),
                json!(breakage
                    .sealed_cache_digest
                    .clone()
                    .unwrap_or_else(|| Sha256::hex(&cache_bytes))),
            );
        }
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        let sealed_manifest_bytes = match &breakage.sealed_coverage {
            None => sealed_manifest_bytes(&manifest_bytes),
            Some((verified, excluded)) => {
                let mut sealed: Value = serde_json::from_slice(&manifest_bytes).unwrap();
                let excluded: Vec<(i64, &str)> = excluded
                    .iter()
                    .map(|(hour, reason)| (*hour, reason.as_str()))
                    .collect();
                sealed["host_finalize"].as_object_mut().unwrap().insert(
                    recorder_host::hour_coverage::HOUR_COVERAGE_FIELD.to_string(),
                    fixture_hour_coverage(verified, &excluded).to_json(),
                );
                let sealed_bytes = serde_json::to_vec_pretty(&sealed).unwrap();
                sealed_manifest_bytes(&sealed_bytes)
            }
        };

        let (static_artifacts, mut static_entries) = replay_test_static_artifacts(game_id);
        const FIXTURE_SNAPSHOT_FILE: &str = "snapshots/000000000000.snapshot.json";
        let mut snapshots: Vec<Value> = Vec::new();
        if let Some(hour) = breakage.alias_cache_as_first_snapshot_hour {
            snapshots.push(json!({
                "hour_index": hour,
                "file": snapshot::CHECKPOINT_CACHE_FILE,
            }));
        }
        if let Some(hour) = breakage.alias_cache_as_snapshot_hour {
            snapshots.push(json!({ "hour_index": 0, "file": FIXTURE_SNAPSHOT_FILE }));
            snapshots.push(json!({
                "hour_index": hour,
                "file": snapshot::CHECKPOINT_CACHE_FILE,
            }));
        }
        let container_index = serde_json::to_vec(&json!({
            "game_id": game_id,
            "schema_version": "1",
            "snapshots": snapshots,
            "static_artifacts": static_artifacts,
            snapshot::RAW_STREAM_PACKAGING_FIELD: snapshot::RawStreamPackaging::Omitted.as_str(),
            "raw_stream_absence_reason": snapshot::RAW_STREAM_OMITTED_REASON,
        }))
        .unwrap();
        let mut entries = vec![
            snapshot::PackEntry {
                path: format!("payload/{game_id}/index.json"),
                bytes: container_index,
                role: "index",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game_id}/recorder/native_session_manifest.json"),
                bytes: sealed_manifest_bytes.clone(),
                role: "recorder_manifest",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game_id}/{}", snapshot::HOUR_INDEX_FILE),
                bytes: index_bytes,
                role: snapshot::HOUR_INDEX_ROLE,
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game_id}/{}", snapshot::CHECKPOINT_CACHE_FILE),
                bytes: cache_bytes,
                role: snapshot::CHECKPOINT_CACHE_ROLE,
                hour_index: None,
            },
        ];
        if breakage.alias_cache_as_snapshot_hour.is_some() {
            entries.push(snapshot::PackEntry {
                path: format!("payload/{game_id}/{FIXTURE_SNAPSHOT_FILE}"),
                bytes: b"{}".to_vec(),
                role: "snapshot",
                hour_index: Some(0),
            });
        }
        if breakage.package_stream_entry {
            entries.push(snapshot::PackEntry {
                path: format!("payload/{game_id}/collector_raw/collector_stream.bin"),
                bytes: stream.clone(),
                role: "collector_compact_stream",
                hour_index: None,
            });
        }
        entries.append(&mut static_entries);
        let product_modset_sha256 = manifest
            .pointer("/portable_content_manifest/product_modset_sha256")
            .and_then(Value::as_str)
            .expect("fixture recorder manifest product-modset key");
        let packed = snapshot::pack(
            game_id,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some(
                    breakage
                        .container_schema_version
                        .clone()
                        .unwrap_or_else(|| REPLAY_CONTAINER_SCHEMA_VERSION.to_string()),
                ),
                content_manifest: Some(replay_test_content_manifest(product_modset_sha256)),
                ..snapshot::PackIdentity::default()
            },
            false,
        )
        .unwrap();
        let mut packed = packed;
        if let Some(position) = breakage.rot_cache_record {
            let (offset, len) = record_windows[position];
            rot_sealed_cache_byte(
                &mut packed,
                &sealed_cache_bytes,
                (offset + len - 1) as usize,
            );
        }
        if breakage.rot_cache_header {
            rot_sealed_cache_byte(&mut packed, &sealed_cache_bytes, 8);
        }
        fs::write(dir.join("native_session.battleplan"), packed).unwrap();
        if !breakage.omit_run_dir_stream {
            let on_disk = breakage.run_dir_stream_override.as_ref().unwrap_or(&stream);
            fs::write(dir.join("collector_stream.bin"), on_disk).unwrap();
        }
        fs::write(dir.join("native_session_manifest.json"), &manifest_bytes).unwrap();
        CacheOnlyRecording {
            manifest,
            manifest_bytes,
            stream_sha256,
            stream_bytes: stream.len() as u64,
        }
    }

    fn cache_only_authority_error(label: &str, breakage: CacheOnlyBreak) -> String {
        let dir = TmpDir::new(label);
        let recording = write_cache_only_replay_fixture(&dir.0, "g", breakage);
        load_verified_replay_authority(
            &recording.manifest,
            &recording.manifest_bytes,
            &dir.0,
            TEST_MAX_STREAM_BYTES,
        )
        .expect_err("a broken attestation link must refuse")
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
        load_verified_replay_authority(&manifest, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
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
        load_verified_replay_authority(&manifest, &bytes, &dir.0, TEST_MAX_STREAM_BYTES)
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
