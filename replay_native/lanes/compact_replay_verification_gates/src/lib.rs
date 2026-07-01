use serde_json::{json, Value};
use snapshot::{Container, ExpectedIdentity};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const RECORD_MAGIC: &[u8; 4] = b"H4CR";
const RECORD_VERSION: u16 = 1;
const RECORD_HEADER_LEN: usize = 32;
const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
const RECORD_TYPE_FRAME_END: u16 = 2;
const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
const DOMAIN_FRAME: u16 = 1000;

const STATUS_WORKER_PANIC: u32 = 1 << 6;
const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
const STATUS_PARTIAL_DATA: u32 = 1 << 9;
const FAIL_STATUS_MASK: u32 = STATUS_WORKER_PANIC
    | STATUS_BRIDGE_UNAVAILABLE
    | STATUS_SOURCE_UNAVAILABLE
    | STATUS_PARTIAL_DATA;

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

#[derive(Debug, Clone, Copy)]
struct DomainContract {
    id: u16,
    name: &'static str,
    requires_data: bool,
    required_units: &'static [&'static str],
    shared_recorder_known: bool,
}

const DOMAIN_CONTRACTS: &[DomainContract] = &[
    DomainContract {
        id: 10,
        name: "fuel_live",
        requires_data: true,
        required_units: &["fuel_country_rows"],
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
        shared_recorder_known: true,
    },
    DomainContract {
        id: 12,
        name: "focus_live",
        requires_data: true,
        required_units: &["focus_country_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 13,
        name: "tech_live",
        requires_data: true,
        required_units: &["tech_country_rows", "active_research_slot_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 14,
        name: "politics_live",
        requires_data: true,
        required_units: &["politics_country_rows", "popularity_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 15,
        name: "politics_residuals",
        requires_data: true,
        required_units: &["politics_residual_country_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 16,
        name: "decisions_live",
        requires_data: true,
        required_units: &["decision_country_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 17,
        name: "diplomacy_live",
        requires_data: true,
        required_units: &["diplomacy_country_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 20,
        name: "divisions",
        requires_data: true,
        required_units: &["division_rows", "division_root_rows", "division_owner_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 21,
        name: "land_combats",
        requires_data: true,
        required_units: &["land_combat_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 22,
        name: "map_control",
        requires_data: true,
        required_units: &["map_control_state_rows", "map_control_province_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 23,
        name: "map_detail",
        requires_data: true,
        required_units: &["map_detail_state_rows", "map_detail_province_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 24,
        name: "weather_live",
        requires_data: true,
        required_units: &["weather_region_rows", "weather_observed_region_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 26,
        name: "division_equipment",
        requires_data: true,
        required_units: &["division_equipment_rows", "division_equipment_root_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 27,
        name: "army_orders",
        requires_data: true,
        required_units: &["army_order_rows", "army_order_root_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 28,
        name: "oob_command",
        requires_data: true,
        required_units: &["oob_command_rows", "oob_command_root_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 29,
        name: "ships",
        requires_data: true,
        required_units: &["ship_rows", "ship_observed_ship_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 30,
        name: "air_wings",
        requires_data: true,
        required_units: &["air_wing_rows", "air_wing_observed_rows"],
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
        shared_recorder_known: true,
    },
    DomainContract {
        id: 32,
        name: "air_wing_loadouts",
        requires_data: true,
        required_units: &["air_wing_loadout_rows", "air_wing_loadout_equipment_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 33,
        name: "air_variant_defs",
        requires_data: true,
        required_units: &["air_variant_def_rows", "air_variant_def_module_token_rows"],
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
        shared_recorder_known: true,
    },
    DomainContract {
        id: 41,
        name: "projects_live",
        requires_data: true,
        required_units: &["project_rows"],
        shared_recorder_known: true,
    },
    DomainContract {
        id: 42,
        name: "air_combat_losses_live",
        requires_data: true,
        required_units: &["air_strength_delta_rows", "air_strength_observed_wing_rows"],
        shared_recorder_known: false,
    },
    DomainContract {
        id: 43,
        name: "market_trade_live",
        requires_data: true,
        required_units: &[],
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
        shared_recorder_known: true,
    },
];

pub fn validate_run_dir(options: &GateOptions) -> GateReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut details = serde_json::Map::new();

    let manifest_path = options.run_dir.join("native_session_manifest.json");
    let manifest = match read_json(&manifest_path) {
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
    let required_domains = if options.required_domains.is_empty() {
        scheduled_domains.clone()
    } else {
        options.required_domains.clone()
    };
    details.insert("scheduled_domains".to_string(), json!(scheduled_domains));
    details.insert("required_domains".to_string(), json!(required_domains));

    let mut expected_contracts = Vec::new();
    for domain in &required_domains {
        match contract_by_name(domain) {
            Some(contract) => expected_contracts.push(contract),
            None => errors.push(format!("unknown required compact domain {domain:?}")),
        }
    }
    for contract in &expected_contracts {
        if !scheduled_domains
            .iter()
            .any(|domain| domain == contract.name)
        {
            errors.push(format!(
                "required compact domain {} is not present in scheduled_domains",
                contract.name
            ));
        }
        if !contract.shared_recorder_known {
            warnings.push(format!(
                "compact domain {} ({}) is lane-known but not yet in shared recorder_host decoder maps",
                contract.name, contract.id
            ));
        }
    }

    let min_game_hours = options.min_game_hours.unwrap_or_else(|| {
        manifest
            .get("hourly_requirement")
            .and_then(|value| value.get("requested_game_hours"))
            .and_then(Value::as_i64)
            .unwrap_or(1)
    });
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

    let stream_name = manifest
        .get("raw_stream")
        .and_then(Value::as_str)
        .unwrap_or("collector_stream.bin");
    let stream_path = options.run_dir.join(stream_name);
    let stream_bytes = match fs::read(&stream_path) {
        Ok(bytes) => {
            details.insert(
                "stream_file".to_string(),
                json!(stream_path.display().to_string()),
            );
            details.insert("stream_file_bytes".to_string(), json!(bytes.len()));
            Some(bytes)
        }
        Err(err) => {
            errors.push(format!(
                "cannot read compact stream {}: {err}",
                stream_path.display()
            ));
            None
        }
    };

    if let Some(bytes) = stream_bytes.as_deref() {
        if bytes.len() as u64 > options.max_stream_bytes {
            errors.push(format!(
                "collector stream bytes {} exceed limit {}",
                bytes.len(),
                options.max_stream_bytes
            ));
        }
        validate_stream_file_against_manifest(
            bytes,
            &manifest,
            &expected_contracts,
            options,
            &mut errors,
        );
    }

    if options.require_replay_verify {
        match verify_replay_container(&manifest, &options.run_dir, stream_bytes.as_deref()) {
            Ok(value) => {
                details.insert("replay_verify".to_string(), value);
            }
            Err(err) => errors.push(err),
        }
    }

    GateReport {
        passed: errors.is_empty(),
        errors,
        warnings,
        details: Value::Object(details),
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|err| err.to_string())?;
    serde_json::from_slice(&bytes).map_err(|err| err.to_string())
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
    if advanced >= 0 && unique_hours >= 0 && unique_hours != advanced + 1 {
        errors.push(format!(
            "unique_game_hour_count {unique_hours} != game_hours_advanced + 1 ({})",
            advanced + 1
        ));
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
    if !options.allow_partial_or_source_failure {
        require_bool_eq(compact, "worker_panic_seen", false, errors);
        require_bool_eq(compact, "bridge_unavailable_seen", false, errors);
        require_bool_eq(compact, "source_unavailable_seen", false, errors);
        require_bool_eq(compact, "partial_data_seen", false, errors);
    } else {
        for key in [
            "worker_panic_seen",
            "bridge_unavailable_seen",
            "source_unavailable_seen",
            "partial_data_seen",
        ] {
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
    let status = domain
        .get("status_flags_or")
        .and_then(Value::as_u64)
        .unwrap_or(FAIL_STATUS_MASK as u64 + 1);
    if !options.allow_partial_or_source_failure && (status as u32 & FAIL_STATUS_MASK) != 0 {
        errors.push(format!(
            "domain {} status_flags_or {status} contains failure bits",
            contract.name
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
    let summary = scan_compact_stream(raw);
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
    if !options.allow_partial_or_source_failure && (summary.status_flags_or & FAIL_STATUS_MASK) != 0
    {
        errors.push(format!(
            "independent stream scan status_flags_or {} contains failure bits",
            summary.status_flags_or
        ));
    }
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

    if summary.max_record_len > options.max_record_bytes {
        errors.push(format!(
            "independent stream scan max_record_len {} exceeds limit {}",
            summary.max_record_len, options.max_record_bytes
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

fn verify_replay_container(
    manifest: &Value,
    run_dir: &Path,
    stream_bytes: Option<&[u8]>,
) -> Result<Value, String> {
    let replay_name = manifest
        .get("replay_file")
        .and_then(Value::as_str)
        .unwrap_or("native_session.battleplan");
    let replay_path = run_dir.join(replay_name);
    let container = Container::open(&replay_path, &ExpectedIdentity::default()).map_err(|err| {
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
    let stream_entries = container.entries_by_role("collector_compact_stream");
    if stream_entries.len() != 1 {
        return Err(format!(
            "replay container has {} collector_compact_stream entries, expected 1",
            stream_entries.len()
        ));
    }
    let entry = &stream_entries[0];
    let stored_stream = container.read_entry_bytes(&entry.path).map_err(|err| {
        format!(
            "cannot read collector_compact_stream {} from replay container: {err}",
            entry.path
        )
    })?;
    if let Some(raw) = stream_bytes {
        if stored_stream.as_slice() != raw {
            return Err(
                "replay container collector_compact_stream bytes differ from collector_stream.bin"
                    .to_string(),
            );
        }
    }
    Ok(json!({
        "file": replay_path.display().to_string(),
        "game_id": container.game_id(),
        "collector_compact_stream": entry.path,
        "collector_compact_stream_bytes": stored_stream.len(),
        "signed": container.signed,
        "unsigned": container.unsigned,
    }))
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
    status_flags_or: u32,
    domains_seen: BTreeSet<u16>,
    frame_end_hour_counts: BTreeMap<i32, u64>,
    per_hour_perf_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    per_hour_data_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    unique_hours: Vec<i32>,
    skipped_hours: i32,
    parse_errors: Vec<String>,
    ordering_errors: Vec<String>,
}

fn scan_compact_stream(raw: &[u8]) -> HeaderScanSummary {
    let mut summary = HeaderScanSummary::default();
    let mut offset = 0usize;
    let mut previous_sequence = None;
    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        if remaining < RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: truncated header with {remaining} byte(s) remaining"
            ));
            break;
        }
        let header = &raw[offset..offset + RECORD_HEADER_LEN];
        if &header[0..4] != RECORD_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: invalid record magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        let header_len = read_u16_le(header, 6) as usize;
        let sequence = read_u64_le(header, 8);
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        if version != RECORD_VERSION {
            summary.parse_errors.push(format!(
                "offset {offset}: record version {version} != {RECORD_VERSION}"
            ));
            break;
        }
        if header_len != RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: header length {header_len} != {RECORD_HEADER_LEN}"
            ));
            break;
        }
        let record_len = RECORD_HEADER_LEN.saturating_add(payload_len);
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining bytes {remaining}"
            ));
            break;
        }
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} did not advance past {previous}"
                ));
            }
        }
        previous_sequence = Some(sequence);
        let payload = &raw[offset + RECORD_HEADER_LEN..offset + record_len];
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
                    summary.status_flags_or |= read_u32_le(payload, 32);
                } else {
                    summary.parse_errors.push(format!(
                        "offset {offset}: domain perf payload is {} byte(s), expected at least 36",
                        payload.len()
                    ));
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
            }
            RECORD_TYPE_FRAME_END => {
                summary.frame_end_record_count += 1;
                *summary
                    .frame_end_hour_counts
                    .entry(game_hour_raw)
                    .or_default() += 1;
                if domain_id != DOMAIN_FRAME {
                    summary.parse_errors.push(format!(
                        "offset {offset}: frame-end domain_id {domain_id} != {DOMAIN_FRAME}"
                    ));
                }
                if payload.len() >= 28 {
                    summary.status_flags_or |= read_u32_le(payload, 24);
                } else {
                    summary.parse_errors.push(format!(
                        "offset {offset}: frame-end payload is {} byte(s), expected at least 28",
                        payload.len()
                    ));
                }
            }
            other => summary
                .parse_errors
                .push(format!("offset {offset}: unknown record type {other}")),
        }
        offset = offset.saturating_add(record_len);
    }
    summary.unique_hours = summary.frame_end_hour_counts.keys().copied().collect();
    summary.skipped_hours = skipped_hours(&summary.unique_hours);
    summary
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
        for expected in expected_domains {
            let Some(contract) = contract_by_id(*expected) else {
                errors.push(format!("unknown compact domain id {expected}"));
                continue;
            };
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
            let Some(contract) = contract_by_id(*expected) else {
                continue;
            };
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

fn domain_name(id: u16) -> &'static str {
    contract_by_id(id)
        .map(|contract| contract.name)
        .unwrap_or("unknown")
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

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
        append_record(&mut raw, 2, 100, 10, RECORD_TYPE_DOMAIN_DATA, &[0; 24]);
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
        append_record(&mut raw, 5, 101, 10, RECORD_TYPE_DOMAIN_DATA, &[0; 24]);
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
    fn header_scan_rejects_sequence_regression_and_failure_flags() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            5,
            100,
            10,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(STATUS_WORKER_PANIC),
        );
        append_record(
            &mut raw,
            5,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let summary = scan_compact_stream(&raw);
        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("did not advance")));
        assert_ne!(summary.status_flags_or & STATUS_WORKER_PANIC, 0);
    }

    #[test]
    fn contract_knows_pending_domain_42_but_marks_shared_decoder_pending() {
        let contract = contract_by_name("air_combat_losses_live").unwrap();
        assert_eq!(contract.id, 42);
        assert!(contract.requires_data);
        assert!(!contract.shared_recorder_known);
    }

    fn append_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) {
        out.extend_from_slice(RECORD_MAGIC);
        out.extend_from_slice(&RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn perf_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = vec![0; 40];
        payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn frame_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = vec![0; 32];
        payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }
}
