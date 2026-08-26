//! Fail-closed real-capture measurement for d81 (`rail_replay_state`).
//!
//! The input is a complete production H4CR stream plus the exact portable
//! content manifest and effective-VFS `map/definition.csv` used by the capture.
//! The definition is authenticated against the manifest before its province
//! ids can become wire keys. Every H4CR record is sequence/framing checked and
//! every closed hour must contain exactly one D81 terminal outcome: a
//! status-bound PERF+DATA pair or an authenticated H4DG state carrier. D81
//! DATA carriers are completely decoded before their status is classified;
//! known failures and H4DG ranges are excluded and force the next usable
//! observation to be a baseline.
//!
//! The accepted observations are encoded as four shaped collections (frame,
//! provinces, ordered directed neighbors, hub simulation rows), in 48-hour blocks.
//! Every emitted baseline or delta is decoded against the reader's own prior
//! and compared with the source fold at stored precision. No output is created
//! until the complete stream and the clean, nonempty v4 evidence gate have
//! passed. UI-era v2 hub frames are never accepted as production evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

use compact_contract::rail_replay as compact_rail;
use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_GAP,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END,
};
use portable_content_manifest::symbol_manifest::{
    ContentIdSource, SymbolManifestBuilder, SymbolObservation,
};
use portable_content_manifest::{ContentManifest, RequiredContentKind, UsedContentKey};
use sha2::{Digest, Sha256};
use wire_format::body::{
    apply_shaped, collection_stored_eq, decode_shaped_baseline, decode_shaped_delta, diff_shaped,
    encode_shaped_baseline, encode_shaped_delta, CollectionDelta, CollectionState, KeyedState, Row,
};
use wire_format::d81::{self, FoldOutcome};

const D81: u16 = 81;
const D18: u16 = 18;
const BLOCK_HOURS: usize = 48;
const PROBE_DOMAIN_ID: u16 = 250;
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
const DEFINITION_PATH: &str = "map/definition.csv";

#[derive(Clone, Debug)]
struct D81Hour {
    game_hour: i32,
    states: Vec<CollectionState>,
    ship_span: Range<usize>,
    follows_previous: bool,
    simulation_by_pov: BTreeMap<u32, BTreeMap<(u32, u8), compact_rail::RailHubSimulationState>>,
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D81Hour>,
    failure_records: usize,
    closed_hours: usize,
    cid1_sha256: Option<String>,
    coverage: Coverage,
}

#[derive(Clone, Debug, Default)]
struct Coverage {
    v1_failure_carrier: bool,
    v4_complete_empty_simulation: bool,
    active_terminal: [bool; 2],
    anchor_variant: [bool; 2],
    rail_level: [bool; 5],
    hub_subtype: [bool; 2],
    foreign_route: [bool; 2],
    fallback_record: [bool; 2],
}

#[derive(Debug)]
struct CaptureAuthority {
    session_sha256: String,
    stream_sha256: String,
    content_manifest_sha256: String,
}

#[derive(Clone, Copy, Debug, Default)]
struct PovMeasurement {
    observations: usize,
    logical_rows: usize,
    /// Exact public simulation ABI payload bytes before compact framing:
    /// 3*u32 + 3*u8 + u32 + 3*i64 + 2*bool = 45.
    simulation_abi_bytes: usize,
    adjacent_transitions: usize,
    changed_rows: usize,
}

fn measure_pov(hours: &[D81Hour]) -> BTreeMap<u32, PovMeasurement> {
    let mut out = BTreeMap::<u32, PovMeasurement>::new();
    for hour in hours {
        for (pov, rows) in &hour.simulation_by_pov {
            let measurement = out.entry(*pov).or_default();
            measurement.observations += 1;
            measurement.logical_rows += rows.len();
            measurement.simulation_abi_bytes += rows.len() * 45;
        }
    }
    for pair in hours.windows(2) {
        let [prior_hour, next_hour] = pair else {
            unreachable!()
        };
        if !next_hour.follows_previous {
            continue;
        }
        let povs = prior_hour
            .simulation_by_pov
            .keys()
            .chain(next_hour.simulation_by_pov.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        for pov in povs {
            let empty = BTreeMap::new();
            let prior = prior_hour.simulation_by_pov.get(&pov).unwrap_or(&empty);
            let next = next_hour.simulation_by_pov.get(&pov).unwrap_or(&empty);
            let identities = prior
                .keys()
                .chain(next.keys())
                .copied()
                .collect::<BTreeSet<_>>();
            let measurement = out.entry(pov).or_default();
            measurement.adjacent_transitions += 1;
            measurement.changed_rows += identities
                .into_iter()
                .filter(|identity| prior.get(identity) != next.get(identity))
                .count();
        }
    }
    out
}

fn opens_baseline(index: usize, follows_previous: bool) -> bool {
    index % BLOCK_HOURS == 0 || !follows_previous
}

fn decode_hex_sha256(text: &str) -> Result<[u8; 32], String> {
    if text.len() != 64 {
        return Err("SHA-256 is not 64 hexadecimal characters".into());
    }
    let mut out = [0u8; 32];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        out[index] = u8::from_str_radix(
            std::str::from_utf8(pair).map_err(|_| "SHA-256 is not ASCII")?,
            16,
        )
        .map_err(|_| "SHA-256 is not hexadecimal")?;
    }
    Ok(out)
}

fn lower_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Join all caller-supplied bytes through the recorder's retained session
/// authority. This closes the otherwise-valid cross-modset substitution where
/// a stream is paired with some other manifest/definition pair.
fn validate_capture_authority(
    session_bytes: &[u8],
    stream: &[u8],
    supplied_manifest_bytes: &[u8],
    identity_mode: StreamIdentityMode,
) -> Result<(ContentManifest, CaptureAuthority), String> {
    let session: serde_json::Value = serde_json::from_slice(session_bytes)
        .map_err(|error| format!("native session manifest rejected: {error}"))?;
    if session.get("schema").and_then(serde_json::Value::as_str)
        != Some("native_collector_session_v1")
        || session
            .get("stream_format")
            .and_then(serde_json::Value::as_str)
            != Some("compact")
    {
        return Err("native session manifest is not a compact capture authority".into());
    }
    if session
        .pointer("/production_profile/enabled")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return Err("native session is not a production-profile capture".into());
    }
    let profile = session
        .pointer("/production_profile/profile_domains")
        .and_then(serde_json::Value::as_array)
        .ok_or("production profile has no domain roster")?;
    let scheduled = session
        .get("scheduled_domains")
        .and_then(serde_json::Value::as_array)
        .ok_or("native session has no scheduled-domain roster")?;
    let string_set = |values: &[serde_json::Value], label: &str| {
        values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{label} contains a non-string domain"))
            })
            .collect::<Result<BTreeSet<_>, _>>()
    };
    let profile = string_set(profile, "production profile")?;
    let scheduled = string_set(scheduled, "scheduled domains")?;
    let canonical = compact_contract::production_profile_scheduled_names()
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    if profile != canonical || scheduled != canonical {
        return Err("capture does not schedule the canonical full production roster".into());
    }
    for pointer in [
        "/production_profile/profile_domains_not_scheduled",
        "/production_profile/scheduled_domains_not_in_profile",
        "/production_profile/mandatory_domains_not_production_complete",
        "/production_profile/missing_required_static_artifacts",
    ] {
        if !session
            .pointer(pointer)
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            return Err(format!("production receipt {pointer} is not empty"));
        }
    }
    if session
        .pointer("/replay_artifact/recovery")
        .and_then(serde_json::Value::as_bool)
        != Some(false)
        || !session
            .pointer("/replay_artifact/gate_problems")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
        || session
            .pointer("/replay_packaging/container_emitted")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err("native session did not publish a clean non-recovery replay artifact".into());
    }
    if session
        .pointer("/acceptance_telemetry/complete")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
        || !session
            .pointer("/acceptance_telemetry/problems")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
    {
        return Err("native session acceptance telemetry is not complete and clean".into());
    }
    let expected_mode = match identity_mode {
        StreamIdentityMode::Legacy => "legacy",
        StreamIdentityMode::IdentityV1 => "identity_v1",
    };
    if session
        .get("compact_stream_identity_mode")
        .and_then(serde_json::Value::as_str)
        != Some(expected_mode)
    {
        return Err(
            "native session manifest identity mode disagrees with the requested protocol".into(),
        );
    }
    let stream_sha256 = lower_hex(&Sha256::digest(stream));
    if session
        .pointer("/stream_storage/decoded_sha256")
        .and_then(serde_json::Value::as_str)
        != Some(stream_sha256.as_str())
    {
        return Err("H4CR digest disagrees with the native session manifest".into());
    }
    if session
        .pointer("/stream_storage/decoded_bytes")
        .and_then(serde_json::Value::as_u64)
        != Some(stream.len() as u64)
    {
        return Err("H4CR byte length disagrees with the native session manifest".into());
    }
    let embedded_value = session
        .get("portable_content_manifest")
        .ok_or("native session manifest omits portable_content_manifest")?;
    if embedded_value.is_null() {
        return Err("native session manifest has no finalized portable content authority".into());
    }
    let embedded_bytes = serde_json::to_vec(embedded_value)
        .map_err(|error| format!("encode embedded content manifest: {error}"))?;
    let embedded = ContentManifest::decode_json(&embedded_bytes)
        .map_err(|error| format!("embedded content manifest rejected: {error}"))?;
    if embedded.manifest_digest_sha256.is_none() {
        return Err("session content manifest is not finalized with a digest".into());
    }
    let supplied = ContentManifest::decode_json(supplied_manifest_bytes)
        .map_err(|error| format!("supplied content manifest rejected: {error}"))?;
    if embedded != supplied {
        return Err("supplied content manifest is not the session's embedded authority".into());
    }
    let content_manifest_sha256 = supplied.sha256()?;
    Ok((
        supplied,
        CaptureAuthority {
            session_sha256: lower_hex(&Sha256::digest(session_bytes)),
            stream_sha256,
            content_manifest_sha256,
        },
    ))
}

/// Construct the only resolver this harness accepts: the retained definition
/// bytes must be the exact payload admitted by the portable content manifest,
/// and the manifest's selected layer must agree with the resolution receipt.
fn authenticated_province_symbols(
    manifest_bytes: &[u8],
    resolution_bytes: &[u8],
    definition_bytes: &[u8],
    country_bindings: &[(u32, String)],
) -> Result<(SymbolManifestBuilder, Vec<(u32, String)>), String> {
    let manifest = ContentManifest::decode_json(manifest_bytes)
        .map_err(|error| format!("content manifest rejected: {error}"))?;
    let resolution: UsedContentKey = serde_json::from_slice(resolution_bytes)
        .map_err(|error| format!("definition.csv resolution receipt rejected: {error}"))?;
    resolution
        .validate()
        .map_err(|error| format!("definition.csv VFS resolution rejected: {error}"))?;
    if resolution.normalized_game_relative_path != DEFINITION_PATH {
        return Err("definition.csv resolution receipt names another VFS path".into());
    }
    if resolution.effective_vfs_manifest_sha256 != manifest.effective_vfs_manifest_sha256 {
        return Err("definition.csv resolution belongs to another effective VFS".into());
    }
    let matching = manifest
        .required_content
        .iter()
        .filter(|required| required.content.normalized_game_relative_path == DEFINITION_PATH)
        .collect::<Vec<_>>();
    let [required] = matching.as_slice() else {
        return Err("content manifest must admit map/definition.csv exactly once".into());
    };
    if required.content_key != DEFINITION_PATH
        || required.kind != RequiredContentKind::Definition
        || required.content != resolution
        || !required
            .consumers
            .iter()
            .any(|consumer| consumer == "wire_product.d81")
    {
        return Err(
            "content manifest D81 definition admission disagrees with the retained resolution"
                .into(),
        );
    }
    let layer = manifest
        .ordered_layers
        .iter()
        .find(|layer| layer.ordinal == resolution.winning_layer_ordinal)
        .ok_or("definition.csv winning layer is absent from the manifest")?;
    if layer.layer_kind != resolution.layer_kind || layer.layer_id != resolution.layer_id {
        return Err("definition.csv winning layer identity disagrees with the manifest".into());
    }
    if resolution.payload_bytes != definition_bytes.len() as u64 {
        return Err("definition.csv byte length disagrees with its resolution receipt".into());
    }
    let actual: [u8; 32] = Sha256::digest(definition_bytes).into();
    if actual != decode_hex_sha256(&resolution.payload_sha256)? {
        return Err("definition.csv digest disagrees with its resolution receipt".into());
    }

    let text =
        std::str::from_utf8(definition_bytes).map_err(|_| "definition.csv is not valid UTF-8")?;
    let mut by_id = BTreeMap::<u32, String>::new();
    let mut rgbs = BTreeSet::new();
    for (line_index, raw_line) in text.lines().enumerate() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            continue;
        }
        let fields = line.split(';').collect::<Vec<_>>();
        if fields.len() < 8 {
            return Err(format!(
                "definition.csv line {} has fewer than eight fields",
                line_index + 1
            ));
        }
        let number = |index: usize, label: &str| {
            fields[index]
                .parse::<u32>()
                .map_err(|_| format!("definition.csv line {} has invalid {label}", line_index + 1))
        };
        let province_id = number(0, "province id")?;
        let red = number(1, "red")?;
        let green = number(2, "green")?;
        let blue = number(3, "blue")?;
        if province_id == 0 || red > 255 || green > 255 || blue > 255 {
            return Err(format!(
                "definition.csv line {} has an out-of-range identity",
                line_index + 1
            ));
        }
        if !rgbs.insert((red, green, blue)) {
            return Err(format!(
                "definition.csv line {} repeats RGB {red},{green},{blue}",
                line_index + 1
            ));
        }
        if by_id
            .insert(province_id, format!("province/rgb/{red}/{green}/{blue}"))
            .is_some()
        {
            return Err(format!("definition.csv repeats province id {province_id}"));
        }
    }
    if by_id.is_empty() {
        return Err("definition.csv contains no province definitions".into());
    }

    let mut symbols = SymbolManifestBuilder::from_base(
        by_id
            .values()
            .cloned()
            .chain(country_bindings.iter().map(|(_, symbol)| symbol.clone())),
    )
    .map_err(|error| format!("build province symbols: {error:?}"))?;
    let bindings = by_id.into_iter().collect::<Vec<_>>();
    symbols
        .bind_existing_sources(bindings.iter().map(|(province_id, symbol)| {
            SymbolObservation::content_id(
                d81::PROVINCE_CONTENT_ID_NAMESPACE,
                u64::from(*province_id),
                symbol.clone(),
            )
        }))
        .map_err(|error| format!("bind authenticated province symbols: {error:?}"))?;
    symbols
        .bind_existing_sources(country_bindings.iter().map(|(country_id, symbol)| {
            SymbolObservation::content_id(
                d81::COUNTRY_CONTENT_ID_NAMESPACE,
                u64::from(*country_id),
                symbol.clone(),
            )
        }))
        .map_err(|error| format!("bind authenticated country symbols: {error:?}"))?;
    Ok((symbols, bindings))
}

fn authenticated_country_bindings(stream: &[u8]) -> Result<Vec<(u32, String)>, String> {
    let mut bindings = BTreeMap::<u32, String>::new();
    let mut source_by_symbol = BTreeMap::<String, u32>::new();
    let mut saw_complete = false;
    let mut open_hour = None;
    let mut last_closed = None;
    let mut perf_domains = BTreeMap::<u16, u32>::new();
    let mut terminal_domains = BTreeSet::<u16>::new();
    let mut pending_d18_data: Option<(u32, Option<Vec<(u32, String)>>)> = None;
    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|error| format!("country symbol scan at byte {pos}: {error}"))?;
        let payload_start = pos
            .checked_add(COMPACT_RECORD_HEADER_LEN)
            .ok_or("country symbol scan offset overflow")?;
        let end = payload_start
            .checked_add(header.payload_len)
            .ok_or("country symbol scan length overflow")?;
        let payload = stream
            .get(payload_start..end)
            .ok_or_else(|| format!("country symbol scan ends inside record at byte {pos}"))?;
        if matches!(
            header.record_type,
            COMPACT_RECORD_TYPE_DOMAIN_PERF
                | COMPACT_RECORD_TYPE_DOMAIN_DATA
                | COMPACT_RECORD_TYPE_DOMAIN_GAP
        ) {
            match open_hour {
                None => {
                    if last_closed.is_some_and(|hour| header.game_hour_raw <= hour) {
                        return Err(format!(
                            "country symbol scan reopens closed hour at byte {pos}"
                        ));
                    }
                    open_hour = Some(header.game_hour_raw);
                }
                Some(hour) if hour == header.game_hour_raw => {}
                Some(hour) => {
                    return Err(format!(
                    "country symbol scan interleaves hour {} into open hour {hour} at byte {pos}",
                    header.game_hour_raw
                ))
                }
            }
        }
        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF if header.domain_id != PROBE_DOMAIN_ID => {
                let status = u32::from_le_bytes(
                    payload
                        .get(32..36)
                        .ok_or_else(|| format!("PERF status truncated at byte {pos}"))?
                        .try_into()
                        .unwrap(),
                );
                if perf_domains.insert(header.domain_id, status).is_some()
                    || !terminal_domains.insert(header.domain_id)
                {
                    return Err(format!(
                        "country symbol scan repeats terminal domain {} at byte {pos}",
                        header.domain_id
                    ));
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_GAP => {
                compact_contract::DomainGapRecordV1::decode(payload)
                    .map_err(|error| format!("H4DG at byte {pos}: {error}"))?;
                if !terminal_domains.insert(header.domain_id) {
                    return Err(format!(
                        "country symbol scan repeats terminal domain {} at byte {pos}",
                        header.domain_id
                    ));
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA if header.domain_id == D18 => {
                if pending_d18_data.is_some() {
                    return Err(format!(
                        "country symbol scan repeats D18 DATA at byte {pos}"
                    ));
                }
                pending_d18_data = Some(decode_d18_country_carrier(payload, pos)?);
            }
            COMPACT_RECORD_TYPE_FRAME_END => {
                if open_hour != Some(header.game_hour_raw) {
                    return Err(format!(
                        "country symbol FRAME_END at byte {pos} has no matching open hour"
                    ));
                }
                let (emitted, scheduled, frame_status) = check_frame_end(payload, pos)?;
                let observed = u32::try_from(terminal_domains.len())
                    .map_err(|_| "country symbol terminal count exceeds u32")?;
                let expected_status =
                    perf_domains
                        .iter()
                        .fold(0u32, |combined, (domain, status)| {
                            combined
                                | compact_contract::compact_status_for_global_interpretation(
                                    *domain, *status,
                                )
                        });
                if emitted != observed || scheduled != observed || frame_status != expected_status {
                    return Err(format!(
                        "country symbol FRAME_END at byte {pos} does not seal its exact terminal/status set"
                    ));
                }
                // Same equation as `wire_product`'s D18 binding: PERF equals the
                // outer DATA roster word OR the three CIM1 family words. The
                // outer word alone is one term of it, and comparing against
                // that term refuses producer-emittable CIM1 failures while
                // admitting payloads whose CIM1 words contradict their PERF.
                // (Codex reviews 01a03ba4 and 01a03bb5, finding 1.)
                match (
                    perf_domains.get(&D18).copied(),
                    pending_d18_data.take(),
                ) {
                    (None, None) => {}
                    (Some(perf), Some((data, rows))) if perf == data => {
                        if let Some(rows) = rows {
                            for (country_id, symbol) in rows {
                                if let Some(previous) =
                                    bindings.insert(country_id, symbol.clone())
                                {
                                    if previous != symbol {
                                        return Err(format!(
                                            "d18 source country {country_id} changes symbol from {previous} to {symbol}"
                                        ));
                                    }
                                }
                                if let Some(previous) =
                                    source_by_symbol.insert(symbol.clone(), country_id)
                                {
                                    if previous != country_id {
                                        return Err(format!(
                                            "d18 country symbol {symbol} aliases source ids {previous} and {country_id}"
                                        ));
                                    }
                                }
                            }
                            saw_complete = true;
                        }
                    }
                    (perf, data) => {
                        return Err(format!(
                            "country symbol D18 PERF/DATA binding differs in hour {}: PERF={perf:?}, DATA={:?}",
                            header.game_hour_raw,
                            data.as_ref().map(|(status, _)| status)
                        ))
                    }
                }
                open_hour = None;
                last_closed = Some(header.game_hour_raw);
                perf_domains.clear();
                terminal_domains.clear();
            }
            _ => {}
        }
        pos = end;
    }
    if open_hour.is_some() {
        return Err("country symbol scan ends with an open hour".into());
    }
    if !saw_complete {
        return Err("stream contains no complete d18 country-identity roster".into());
    }
    Ok(bindings.into_iter().collect())
}

fn decode_d18_country_carrier(
    payload: &[u8],
    at: usize,
) -> Result<(u32, Option<Vec<(u32, String)>>), String> {
    if payload.len() < 20 {
        return Err(format!("d18 DATA at byte {at} is shorter than its header"));
    }
    let row_count = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
    let changed = u32::from_le_bytes(payload[12..16].try_into().unwrap()) as usize;
    let status = u32::from_le_bytes(payload[16..20].try_into().unwrap());
    let unknown = compact_contract::compact_status_unknown_flags_v1(status);
    if unknown != 0 {
        return Err(format!(
            "d18 DATA at byte {at} has unknown status {unknown:#x}"
        ));
    }
    // Same as `wire_product`: a failure carrier still carries its CIM1 block,
    // so returning the outer word alone here refuses producer-sealed hours and
    // skips the extension's validation entirely. (Codex review 01a03bcf.)
    let complete = !compact_contract::compact_status_is_failure(status);
    if complete && (row_count == 0 || changed != row_count) {
        return Err(format!(
            "d18 DATA at byte {at} is not a complete nonempty roster"
        ));
    }
    let mut cursor = 20usize;
    let mut local = BTreeMap::new();
    let mut source_by_symbol = BTreeMap::<String, u32>::new();
    for row_index in 0..row_count {
        let country_id = read_u32_slice(payload, &mut cursor, "country id")?;
        let tag = read_u16_string(payload, &mut cursor, "country tag")?;
        let symbol = format!("country/tag/{tag}");
        if country_id == 0
            || tag.len() != 3
            || !tag
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
            || local.insert(country_id, symbol.clone()).is_some()
        {
            return Err(format!("d18 DATA at byte {at} row {row_index} is invalid"));
        }
        if let Some(previous) = source_by_symbol.insert(symbol.clone(), country_id) {
            return Err(format!(
                "d18 country symbol {symbol} aliases source ids {previous} and {country_id}"
            ));
        }
        let marker = *payload
            .get(cursor)
            .ok_or_else(|| format!("d18 DATA at byte {at} truncates cosmetic marker"))?;
        cursor += 1;
        match marker {
            0 | 1 => {}
            2 => {
                let _ = read_u16_string(payload, &mut cursor, "cosmetic country tag")?;
            }
            _ => {
                return Err(format!(
                    "d18 DATA at byte {at} has invalid cosmetic marker {marker}"
                ))
            }
        }
    }
    let family_status = validate_country_identity_extension(payload, cursor, at)?;
    Ok((
        status | family_status,
        complete.then(|| local.into_iter().collect()),
    ))
}

fn read_u32_slice(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<u32, String> {
    let end = cursor
        .checked_add(4)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let value = u32::from_le_bytes(
        bytes
            .get(*cursor..end)
            .ok_or_else(|| format!("{label} truncated"))?
            .try_into()
            .unwrap(),
    );
    *cursor = end;
    Ok(value)
}

fn read_u16_string(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<String, String> {
    let len_end = cursor
        .checked_add(2)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let len = u16::from_le_bytes(
        bytes
            .get(*cursor..len_end)
            .ok_or_else(|| format!("{label} length truncated"))?
            .try_into()
            .unwrap(),
    ) as usize;
    let end = len_end
        .checked_add(len)
        .ok_or_else(|| format!("{label} length overflow"))?;
    let text = std::str::from_utf8(
        bytes
            .get(len_end..end)
            .ok_or_else(|| format!("{label} bytes truncated"))?,
    )
    .map_err(|_| format!("{label} is not UTF-8"))?
    .to_string();
    *cursor = end;
    Ok(text)
}

/// Validates the CIM1 extent and returns the OR of its three family status
/// words, which is the rest of the D18 data authority.
fn validate_country_identity_extension(
    payload: &[u8],
    cursor: usize,
    at: usize,
) -> Result<u32, String> {
    use compact_contract::country_identity_membership as cim;
    let header = payload
        .get(cursor..cursor.saturating_add(cim::CIM1_HEADER_LEN))
        .ok_or_else(|| format!("d18 DATA at byte {at} truncates CIM1 extension"))?;
    if &header[..4] != cim::CIM1_MAGIC
        || u16::from_le_bytes(header[4..6].try_into().unwrap()) != cim::CIM1_VERSION
        || u16::from_le_bytes(header[6..8].try_into().unwrap()) as usize != cim::CIM1_HEADER_LEN
        || header[44..48] != [0; 4]
    {
        return Err(format!("d18 DATA at byte {at} has invalid CIM1 header"));
    }
    let family_status = u32::from_le_bytes(header[8..12].try_into().unwrap())
        | u32::from_le_bytes(header[12..16].try_into().unwrap())
        | u32::from_le_bytes(header[16..20].try_into().unwrap());
    let state_count = u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
    let claims = u32::from_le_bytes(header[24..28].try_into().unwrap()) as usize;
    let cores = u32::from_le_bytes(header[28..32].try_into().unwrap()) as usize;
    let majors = u32::from_le_bytes(header[32..36].try_into().unwrap()) as usize;
    let unmeasured = (u32::from_le_bytes(header[36..40].try_into().unwrap()) as usize)
        .checked_add(u32::from_le_bytes(header[40..44].try_into().unwrap()) as usize)
        .ok_or("CIM1 unmeasured count overflow")?;
    let mut body = cursor + cim::CIM1_HEADER_LEN;
    let mut seen_claims = 0usize;
    let mut seen_cores = 0usize;
    for _ in 0..state_count {
        let row = payload
            .get(body..body.saturating_add(12))
            .ok_or_else(|| format!("d18 DATA at byte {at} truncates CIM1 state row"))?;
        let row_claims = u32::from_le_bytes(row[4..8].try_into().unwrap()) as usize;
        let row_cores = u32::from_le_bytes(row[8..12].try_into().unwrap()) as usize;
        seen_claims = seen_claims
            .checked_add(row_claims)
            .ok_or("CIM1 claims overflow")?;
        seen_cores = seen_cores
            .checked_add(row_cores)
            .ok_or("CIM1 cores overflow")?;
        let claim_bytes = row_claims
            .checked_mul(4)
            .ok_or("CIM1 claim bytes overflow")?;
        let core_bytes = row_cores.checked_mul(4).ok_or("CIM1 core bytes overflow")?;
        let row_bytes = 12usize
            .checked_add(claim_bytes)
            .and_then(|value| value.checked_add(core_bytes))
            .ok_or("CIM1 state extent overflow")?;
        body = body
            .checked_add(row_bytes)
            .ok_or("CIM1 state extent overflow")?;
    }
    if seen_claims != claims || seen_cores != cores {
        return Err(format!(
            "d18 DATA at byte {at} CIM1 membership totals disagree"
        ));
    }
    body = body
        .checked_add(majors.checked_mul(8).ok_or("CIM1 major extent overflow")?)
        .ok_or("CIM1 extent overflow")?;
    body = body
        .checked_add(
            unmeasured
                .checked_mul(cim::CIM1_UNMEASURED_ENTRY_LEN)
                .ok_or("CIM1 unmeasured extent overflow")?,
        )
        .ok_or("CIM1 extent overflow")?;
    if body != payload.len() {
        return Err(format!(
            "d18 DATA at byte {at} CIM1 does not exactly cover payload"
        ));
    }
    Ok(family_status)
}

fn check_frame_end(payload: &[u8], at: usize) -> Result<(u32, u32, u32), String> {
    if payload.len() != COMPACT_FRAME_END_PAYLOAD_LEN {
        return Err(format!("FRAME_END at byte {at} has wrong payload length"));
    }
    let emitted = u32::from_le_bytes(payload[16..20].try_into().unwrap());
    let scheduled = u32::from_le_bytes(payload[20..24].try_into().unwrap());
    let status = u32::from_le_bytes(payload[24..28].try_into().unwrap());
    let reserved = u32::from_le_bytes(payload[28..32].try_into().unwrap());
    if emitted != scheduled || reserved != 0 {
        return Err(format!("FRAME_END at byte {at} has an invalid seal"));
    }
    let unknown = status & !FRAME_END_KNOWN_STATUS_MASK;
    if unknown != 0 {
        return Err(format!(
            "FRAME_END at byte {at} has rejected status {status:#x}"
        ));
    }
    Ok((emitted, scheduled, status))
}

fn keyed(
    label: &str,
    rows: Vec<(wire_format::body::Key, wire_format::Record)>,
) -> Result<CollectionState, String> {
    let mut state = KeyedState::new();
    for (key, record) in rows {
        if state.insert(key, record).is_some() {
            return Err(format!("d81 {label} snapshot has a duplicate portable key"));
        }
    }
    Ok(CollectionState::Keyed(state))
}

fn states_from_snapshot(snapshot: d81::DecodedPayload) -> Result<Vec<CollectionState>, String> {
    Ok(vec![
        keyed("frame", vec![snapshot.frame])?,
        keyed("province", snapshot.provinces)?,
        CollectionState::OrderedKeyed(
            snapshot
                .directed_neighbors
                .into_iter()
                .map(|(key, record)| Row { key, record })
                .collect(),
        ),
        keyed("hub_simulation", snapshot.hub_simulation_rows)?,
    ])
}

fn scan_stream(
    stream: &[u8],
    symbols: &SymbolManifestBuilder,
    identity_mode: StreamIdentityMode,
    expected_build_profile: &str,
) -> Result<StreamScan, String> {
    let schema = d81::schema();
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };
    let mut span_delta = compact_contract::payload_span_delta::PayloadSpanDeltaConsumer::new();
    let mut hours = Vec::new();
    let mut failures = 0usize;
    let mut closed_hours = 0usize;
    let mut open_hour = None;
    let mut last_closed = None;
    let mut last_record_type = None;
    let mut pending_perf = None;
    let mut perf_domains = BTreeMap::new();
    let mut terminal_domains = BTreeSet::new();
    let mut d81_data_in_open_hour = false;
    let mut d81_gap_in_open_hour: Option<(i32, i32)> = None;
    let mut next_d81_gap_ordinal = 0u64;
    let mut last_d81_coverage_end = None;
    let mut last_d81_hour = None;
    let mut last_usable_hour = None;
    let mut saw_clean_v4 = false;
    let mut coverage = Coverage::default();

    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|error| format!("record at byte {pos}: {error}"))?;
        let payload_start = pos
            .checked_add(COMPACT_RECORD_HEADER_LEN)
            .ok_or("record offset overflow")?;
        let end = payload_start
            .checked_add(header.payload_len)
            .ok_or("record length overflow")?;
        if end > stream.len() {
            return Err(format!("stream ends inside record at byte {pos}"));
        }
        let payload = &stream[payload_start..end];
        validator
            .validate_record(&header, payload)
            .map_err(|error| format!("record at byte {pos}: {error:?}"))?;

        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                let status = u32::from_le_bytes(
                    payload
                        .get(32..36)
                        .ok_or_else(|| format!("PERF status truncated at byte {pos}"))?
                        .try_into()
                        .unwrap(),
                );
                if header.domain_id != PROBE_DOMAIN_ID
                    && perf_domains.insert(header.domain_id, status).is_some()
                {
                    return Err(format!("duplicate PERF domain at byte {pos}"));
                }
                if header.domain_id != PROBE_DOMAIN_ID && !terminal_domains.insert(header.domain_id)
                {
                    return Err(format!("duplicate terminal domain at byte {pos}"));
                }
                if let Some(open) = open_hour {
                    if open != header.game_hour_raw {
                        return Err(format!("PERF hour interleaving at byte {pos}"));
                    }
                } else {
                    if last_closed.is_some_and(|hour| header.game_hour_raw <= hour) {
                        return Err(format!("non-monotonic PERF at byte {pos}"));
                    }
                    if pending_perf.is_some_and(|hour| hour != header.game_hour_raw) {
                        return Err(format!("conflicting pending PERF hour at byte {pos}"));
                    }
                    pending_perf = Some(header.game_hour_raw);
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                if open_hour.is_some_and(|hour| hour != header.game_hour_raw) {
                    return Err(format!("DATA hour interleaving at byte {pos}"));
                }
                if open_hour.is_none() {
                    if pending_perf.is_some_and(|hour| hour != header.game_hour_raw) {
                        return Err(format!("DATA disagrees with pending PERF at byte {pos}"));
                    }
                    if last_closed.is_some_and(|hour| header.game_hour_raw <= hour) {
                        return Err(format!("DATA reopens a closed hour at byte {pos}"));
                    }
                    open_hour = Some(header.game_hour_raw);
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_GAP => {
                if open_hour.is_some_and(|hour| hour != header.game_hour_raw) {
                    return Err(format!("H4DG hour interleaving at byte {pos}"));
                }
                if open_hour.is_none() {
                    if pending_perf.is_some_and(|hour| hour != header.game_hour_raw) {
                        return Err(format!("H4DG disagrees with pending PERF at byte {pos}"));
                    }
                    if last_closed.is_some_and(|hour| header.game_hour_raw <= hour) {
                        return Err(format!("H4DG reopens a closed hour at byte {pos}"));
                    }
                    open_hour = Some(header.game_hour_raw);
                }
                if !terminal_domains.insert(header.domain_id) {
                    return Err(format!("duplicate terminal domain at byte {pos}"));
                }
                if header.domain_id == D81 {
                    if d81_data_in_open_hour
                        || perf_domains.contains_key(&D81)
                        || d81_gap_in_open_hour.is_some()
                    {
                        return Err(format!("duplicate D81 terminal at byte {pos}"));
                    }
                    let gap = compact_contract::DomainGapRecordV1::decode(payload)
                        .map_err(|error| format!("d81 H4DG at byte {pos}: {error}"))?;
                    if gap.part != compact_contract::DomainGapPart::State
                        || gap.target_hour_raw != header.game_hour_raw
                        || gap.range_end_hour_raw != header.game_hour_raw
                        || gap.outcome_ordinal != next_d81_gap_ordinal
                    {
                        return Err(format!(
                            "d81 H4DG at byte {pos} has invalid state/range/ordinal semantics"
                        ));
                    }
                    next_d81_gap_ordinal = next_d81_gap_ordinal
                        .checked_add(1)
                        .ok_or("d81 H4DG ordinal overflow")?;
                    let expected_start = last_d81_coverage_end
                        .and_then(|hour: i32| hour.checked_add(1))
                        .unwrap_or(gap.range_start_hour_raw);
                    if gap.range_start_hour_raw != expected_start {
                        return Err(format!(
                            "d81 H4DG at byte {pos} coverage starts {}, expected {expected_start}",
                            gap.range_start_hour_raw
                        ));
                    }
                    d81_gap_in_open_hour = Some((gap.range_start_hour_raw, gap.range_end_hour_raw));
                }
            }
            COMPACT_RECORD_TYPE_FRAME_END => {
                let (emitted, scheduled, frame_status) = check_frame_end(payload, pos)?;
                let observed = terminal_domains.len().min(u32::MAX as usize) as u32;
                if emitted != observed || scheduled != observed {
                    return Err(format!(
                        "FRAME_END at byte {pos} disagrees with PERF domains"
                    ));
                }
                let expected_status =
                    perf_domains
                        .iter()
                        .fold(0u32, |combined, (domain, status)| {
                            combined
                                | compact_contract::compact_status_for_global_interpretation(
                                    *domain, *status,
                                )
                        });
                if frame_status != expected_status {
                    return Err(format!(
                        "FRAME_END at byte {pos} status {frame_status:#x} != PERF-derived {expected_status:#x}"
                    ));
                }
                if open_hour != Some(header.game_hour_raw) {
                    return Err(format!(
                        "FRAME_END at byte {pos} does not close its open hour"
                    ));
                }
                let has_d81_perf_data = perf_domains.contains_key(&D81) && d81_data_in_open_hour;
                if has_d81_perf_data == d81_gap_in_open_hour.is_some() {
                    return Err(format!(
                        "closed hour {} must have exactly one D81 PERF+DATA or H4DG terminal",
                        header.game_hour_raw
                    ));
                }
                if last_closed.is_some_and(|hour| header.game_hour_raw <= hour) {
                    return Err(format!("non-monotonic FRAME_END at byte {pos}"));
                }
                open_hour = None;
                pending_perf = None;
                perf_domains.clear();
                terminal_domains.clear();
                d81_data_in_open_hour = false;
                if let Some((start, end)) = d81_gap_in_open_hour.take() {
                    failures += 1;
                    last_d81_coverage_end = Some(end);
                    if start > end {
                        return Err(format!(
                            "closed hour {} has reversed D81 H4DG range",
                            header.game_hour_raw
                        ));
                    }
                }
                last_closed = Some(header.game_hour_raw);
                closed_hours += 1;
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA && header.domain_id == D81 {
            if d81_data_in_open_hour {
                return Err(format!(
                    "closed hour {} has more than one d81 DATA carrier",
                    header.game_hour_raw
                ));
            }
            d81_data_in_open_hour = true;
            if d81_gap_in_open_hour.is_some() {
                return Err(format!("D81 DATA and H4DG coexist at byte {pos}"));
            }
            if last_d81_hour.is_some_and(|hour| header.game_hour_raw <= hour) {
                return Err(format!("non-monotonic d81 DATA at byte {pos}"));
            }
            last_d81_hour = Some(header.game_hour_raw);
            let expected_hour = last_d81_coverage_end
                .and_then(|hour: i32| hour.checked_add(1))
                .unwrap_or(header.game_hour_raw);
            if header.game_hour_raw != expected_hour {
                return Err(format!(
                    "d81 DATA at byte {pos} coverage starts {}, expected {expected_hour}",
                    header.game_hour_raw
                ));
            }
            last_d81_coverage_end = Some(header.game_hour_raw);

            // `fold_payload` first runs the complete compact decoder (including
            // every row and join), then rejects unknown status vocabulary, then
            // classifies known failure bits.
            //
            // UNWRAP BEFORE ANYTHING LOOKS AT THE PAYLOAD: a d81 record may ride
            // as a `PSD1` envelope (`compact_contract::payload_span_delta`),
            // and this bench must fold what the domain encoded, not refuse it.
            // A record that will not reconstruct is refused BY NAME — this is a
            // bench over an intact stream, so a refusal is worth stopping for.
            let reconstructed =
                compact_contract::payload_span_delta::reconstruct_if_enveloped(
                    &mut span_delta,
                    header.record_flags,
                    compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
                        domain_id: header.domain_id,
                        source_sequence: header.sequence,
                        game_hour_raw: header.game_hour_raw,
                    },
                    payload,
                )
                .map_err(|reason| format!("d81 DATA at byte {pos} reconstructs: {reason}"))?;
            let payload: &[u8] = match &reconstructed {
                Some(reconstructed) => reconstructed,
                None => payload,
            };
            let compact_decoded = compact_rail::decode_payload(payload)
                .map_err(|error| format!("d81 DATA at byte {pos}: {error}"))?;
            let perf_status = perf_domains.get(&D81).copied().ok_or_else(|| {
                format!(
                    "d81 DATA at byte {pos} has no matching D81 PERF in hour {}",
                    header.game_hour_raw
                )
            })?;
            if perf_status != compact_decoded.status_flags {
                return Err(format!(
                    "d81 DATA at byte {pos} status {:#x} differs from D81 PERF {perf_status:#x}",
                    compact_decoded.status_flags
                ));
            }
            match d81::fold_payload(&schema, payload, header.game_hour_raw as u64, symbols)
                .map_err(|error| format!("d81 DATA at byte {pos}: {error}"))?
            {
                FoldOutcome::Unavailable { .. } => {
                    failures += 1;
                    coverage.v1_failure_carrier |=
                        u16::from_le_bytes(payload[4..6].try_into().unwrap())
                            == d81::PAYLOAD_VERSION_V1;
                }
                FoldOutcome::Snapshot(snapshot) => {
                    let follows_previous = last_usable_hour
                        .and_then(|hour: i32| hour.checked_add(1))
                        == Some(header.game_hour_raw);
                    last_usable_hour = Some(header.game_hour_raw);
                    saw_clean_v4 |= snapshot.header.payload_version == d81::PAYLOAD_VERSION_V4
                        && snapshot.header.hub_simulation_complete
                        && !snapshot.provinces.is_empty()
                        && !snapshot.directed_neighbors.is_empty()
                        && !snapshot.hub_simulation_rows.is_empty();
                    coverage.v4_complete_empty_simulation |= snapshot.header.payload_version
                        == d81::PAYLOAD_VERSION_V4
                        && snapshot.header.hub_simulation_complete
                        && snapshot.hub_simulation_rows.is_empty();
                    for province in &compact_decoded.provinces {
                        coverage.active_terminal[province.active_terminal as usize] = true;
                        coverage.anchor_variant[province.anchor_variant as usize] = true;
                        for neighbor in &province.neighbors {
                            coverage.rail_level[(neighbor.edge_level - 1) as usize] = true;
                        }
                    }
                    let mut simulation_by_pov = BTreeMap::new();
                    for row in &compact_decoded.hub_simulation_rows {
                        coverage.hub_subtype[row.subtype as usize] = true;
                        coverage.foreign_route[row.foreign_route as usize] = true;
                        coverage.fallback_record[row.fallback_record as usize] = true;
                        simulation_by_pov
                            .entry(row.pov_country_id)
                            .or_insert_with(BTreeMap::new)
                            .insert((row.province_id, row.subtype), *row);
                    }
                    hours.push(D81Hour {
                        game_hour: header.game_hour_raw,
                        states: states_from_snapshot(snapshot)?,
                        ship_span: pos..end,
                        follows_previous,
                        simulation_by_pov,
                    });
                }
            }
        }
        pos = end;
    }

    if open_hour.is_some() {
        return Err("stream leaves a game hour open at EOF".into());
    }
    if last_closed.is_none() || last_record_type != Some(COMPACT_RECORD_TYPE_FRAME_END) {
        return Err("stream must end on a canonical FRAME_END".into());
    }
    if hours.is_empty() {
        return Err("stream has no usable d81 observations".into());
    }
    if !saw_clean_v4 {
        return Err(
            "capture lacks a clean v4 d81 snapshot with nonempty provinces, directed \
             neighbors, and nonempty complete simulation rows"
                .into(),
        );
    }
    let cid1_sha256 = validator
        .identity()
        .map(|identity| {
            if identity.source_build_profile() != expected_build_profile {
                return Err(format!(
                    "CID1 build profile {:?} disagrees with content manifest {:?}",
                    identity.source_build_profile(),
                    expected_build_profile
                ));
            }
            Ok(identity.payload_sha256_hex())
        })
        .transpose()?;
    if identity_mode == StreamIdentityMode::IdentityV1 && cid1_sha256.is_none() {
        return Err("identity-v1 stream ended without placement-authoritative CID1".into());
    }
    Ok(StreamScan {
        hours,
        failure_records: failures,
        closed_hours,
        cid1_sha256,
        coverage,
    })
}

struct Measurement {
    wire: Vec<u8>,
    offsets: Vec<usize>,
    baselines: usize,
    gap_baselines: usize,
    deltas: usize,
    unchanged: usize,
    baseline_bytes: usize,
    delta_bytes: usize,
    changed_rows: usize,
    tombstones: usize,
    collection_changes: [usize; d81::COLLECTION_COUNT],
    adjacent_changed_rows: usize,
    adjacent_tombstones: usize,
    adjacent_collection_changes: [usize; d81::COLLECTION_COUNT],
}

fn measure(schema: &wire_format::Schema, hours: &[D81Hour]) -> Result<Measurement, String> {
    let mut result = Measurement {
        wire: Vec::new(),
        offsets: vec![0],
        baselines: 0,
        gap_baselines: 0,
        deltas: 0,
        unchanged: 0,
        baseline_bytes: 0,
        delta_bytes: 0,
        changed_rows: 0,
        tombstones: 0,
        collection_changes: [0; d81::COLLECTION_COUNT],
        adjacent_changed_rows: 0,
        adjacent_tombstones: 0,
        adjacent_collection_changes: [0; d81::COLLECTION_COUNT],
    };
    let mut prior: Option<Vec<CollectionState>> = None;
    let mut reader: Option<Vec<CollectionState>> = None;

    for (index, hour) in hours.iter().enumerate() {
        if let Some(prior_states) = prior.as_ref() {
            for (collection_index, ((collection, before), after)) in schema
                .collections
                .iter()
                .zip(prior_states)
                .zip(&hour.states)
                .enumerate()
            {
                if let Some(change) = diff_shaped(collection, before, after)
                    .map_err(|error| format!("adjacent diff hour {}: {error:?}", hour.game_hour))?
                {
                    result.adjacent_collection_changes[collection_index] += 1;
                    match change {
                        CollectionDelta::Keyed(delta) => {
                            result.adjacent_changed_rows += delta.changed.len();
                            result.adjacent_tombstones += delta.tombstones.len();
                        }
                        CollectionDelta::OrderedKeyed(rows) => {
                            result.adjacent_changed_rows += rows.len();
                        }
                        CollectionDelta::OrderedOccurrence(rows) => {
                            result.adjacent_changed_rows += rows.len();
                        }
                    }
                }
            }
        }
        let baseline = opens_baseline(index, hour.follows_previous);
        let start = result.wire.len();
        let mut emitted = true;
        if baseline {
            encode_shaped_baseline(schema, &hour.states, &mut result.wire)
                .map_err(|error| format!("hour {} baseline encode: {error:?}", hour.game_hour))?;
            result.baselines += 1;
            result.baseline_bytes += result.wire.len() - start;
            if index % BLOCK_HOURS != 0 {
                result.gap_baselines += 1;
            }
        } else {
            let prior_states = prior.as_ref().ok_or("delta before baseline")?;
            let mut deltas = Vec::with_capacity(schema.collections.len());
            for (collection_index, ((collection, before), after)) in schema
                .collections
                .iter()
                .zip(prior_states)
                .zip(&hour.states)
                .enumerate()
            {
                let delta = diff_shaped(collection, before, after)
                    .map_err(|error| format!("diff hour {}: {error:?}", hour.game_hour))?;
                if let Some(ref change) = delta {
                    result.collection_changes[collection_index] += 1;
                    match change {
                        CollectionDelta::Keyed(delta) => {
                            result.changed_rows += delta.changed.len();
                            result.tombstones += delta.tombstones.len();
                        }
                        CollectionDelta::OrderedKeyed(rows) => {
                            result.changed_rows += rows.len();
                        }
                        CollectionDelta::OrderedOccurrence(rows) => {
                            result.changed_rows += rows.len();
                        }
                    }
                }
                deltas.push(delta);
            }
            if deltas.iter().all(Option::is_none) {
                emitted = false;
                result.unchanged += 1;
            } else {
                encode_shaped_delta(schema, prior_states, &deltas, &mut result.wire)
                    .map_err(|error| format!("hour {} delta encode: {error:?}", hour.game_hour))?;
                result.deltas += 1;
                result.delta_bytes += result.wire.len() - start;
            }
        }
        result.offsets.push(result.wire.len());

        if emitted {
            let bytes = &result.wire[start..];
            if baseline {
                let (states, end) = decode_shaped_baseline(schema, bytes, 0).map_err(|error| {
                    format!("hour {} baseline decode: {error:?}", hour.game_hour)
                })?;
                if end != bytes.len() {
                    return Err(format!(
                        "hour {} baseline has trailing bytes",
                        hour.game_hour
                    ));
                }
                reader = Some(states);
            } else {
                let mut states = reader.take().ok_or("reader delta before baseline")?;
                let (deltas, end) = decode_shaped_delta(schema, &states, bytes, 0)
                    .map_err(|error| format!("hour {} delta decode: {error:?}", hour.game_hour))?;
                if end != bytes.len() {
                    return Err(format!("hour {} delta has trailing bytes", hour.game_hour));
                }
                for (state, delta) in states.iter_mut().zip(&deltas) {
                    if let Some(delta) = delta {
                        apply_shaped(state, delta)
                            .map_err(|error| format!("hour {} apply: {error:?}", hour.game_hour))?;
                    }
                }
                reader = Some(states);
            }
        }
        let replay = reader.as_ref().ok_or("reader has no baseline")?;
        if replay.len() != hour.states.len() {
            return Err(format!("hour {} replay arity mismatch", hour.game_hour));
        }
        for ((collection, replayed), expected) in
            schema.collections.iter().zip(replay).zip(&hour.states)
        {
            if !collection_stored_eq(collection, replayed, expected)
                .map_err(|error| format!("hour {} parity: {error:?}", hour.game_hour))?
            {
                return Err(format!(
                    "hour {} reconstructed state differs from source fold",
                    hour.game_hour
                ));
            }
        }
        prior = Some(hour.states.clone());
    }
    Ok(result)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("write {}: {error}", path.display()))
}

fn write_offsets(path: &Path, offsets: &[usize]) -> Result<(), String> {
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    for offset in offsets {
        writeln!(file, "{offset}").map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    Ok(())
}

fn zstd3(input: &Path, output: &Path) -> Result<usize, String> {
    let script = "import pathlib,sys,zstandard\n\
data=pathlib.Path(sys.argv[1]).read_bytes()\n\
out=zstandard.ZstdCompressor(level=3).compress(data)\n\
pathlib.Path(sys.argv[2]).write_bytes(out)\n\
print(len(out))";
    let result = Command::new("python")
        .arg("-c")
        .arg(script)
        .arg(input)
        .arg(output)
        .output()
        .map_err(|error| format!("launch Python zstd-3: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "Python zstd-3 failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    String::from_utf8_lossy(&result.stdout)
        .trim()
        .parse::<usize>()
        .map_err(|error| format!("parse zstd-3 size: {error}"))
}

fn zstd3_blocks(
    input: &Path,
    output: &Path,
    offsets: &[usize],
    block_starts: &[usize],
) -> Result<usize, String> {
    let mut ranges = Vec::with_capacity(block_starts.len());
    for (index, start) in block_starts.iter().copied().enumerate() {
        let end = block_starts
            .get(index + 1)
            .copied()
            .unwrap_or(offsets.len() - 1);
        ranges.push(format!("{}:{}", offsets[start], offsets[end]));
    }
    let script = "import pathlib,sys,zstandard\n\
data=pathlib.Path(sys.argv[1]).read_bytes()\n\
ranges=[tuple(map(int,p.split(':'))) for p in sys.argv[3].split(',')]\n\
out=b''.join(zstandard.ZstdCompressor(level=3).compress(data[a:b]) for a,b in ranges)\n\
pathlib.Path(sys.argv[2]).write_bytes(out)\n\
print(len(out))";
    let result = Command::new("python")
        .arg("-c")
        .arg(script)
        .arg(input)
        .arg(output)
        .arg(ranges.join(","))
        .output()
        .map_err(|error| format!("launch block zstd-3: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "block zstd-3 failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    String::from_utf8_lossy(&result.stdout)
        .trim()
        .parse::<usize>()
        .map_err(|error| format!("parse block zstd-3 size: {error}"))
}

fn promotion_identity_mode(value: &str) -> Result<StreamIdentityMode, String> {
    match value {
        "identity-v1" => Ok(StreamIdentityMode::IdentityV1),
        "legacy" => Err(
            "legacy H4CR identity is retained only for internal compatibility; \
             D81 production measurement requires identity-v1 with CID1 authority"
                .into(),
        ),
        _ => Err("D81 production measurement identity mode must be identity-v1".into()),
    }
}

fn run() -> Result<(), String> {
    const USAGE: &str = "usage: bench_real_d81 <collector_stream.bin> <out-dir> \
                         <native_session_manifest.json> \
                         <portable_content_manifest.json> \
                         <definition_resolution.json> <definition.csv> \
                         <identity-v1>";
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 7 {
        return Err(USAGE.into());
    }
    let stream_path = PathBuf::from(&args[0]);
    let out_dir = PathBuf::from(&args[1]);
    let mode = promotion_identity_mode(&args[6]).map_err(|error| format!("{error}; {USAGE}"))?;
    let stream = std::fs::read(&stream_path)
        .map_err(|error| format!("read {}: {error}", stream_path.display()))?;
    let session =
        std::fs::read(&args[2]).map_err(|error| format!("read session manifest: {error}"))?;
    let manifest = std::fs::read(&args[3]).map_err(|error| format!("read manifest: {error}"))?;
    let definition_resolution =
        std::fs::read(&args[4]).map_err(|error| format!("read definition resolution: {error}"))?;
    let definition =
        std::fs::read(&args[5]).map_err(|error| format!("read definition.csv: {error}"))?;
    let (content_manifest, authority) =
        validate_capture_authority(&session, &stream, &manifest, mode)?;
    let country_bindings = authenticated_country_bindings(&stream)?;
    let (symbols, province_bindings) = authenticated_province_symbols(
        &manifest,
        &definition_resolution,
        &definition,
        &country_bindings,
    )?;
    let scan = scan_stream(
        &stream,
        &symbols,
        mode,
        &content_manifest.source_game.build_profile,
    )?;
    let schema = d81::schema();
    let measured = measure(&schema, &scan.hours)?;
    let pov_measurements = measure_pov(&scan.hours);

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    for hour in &scan.hours {
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
    }
    let block_starts = scan
        .hours
        .iter()
        .enumerate()
        .filter_map(|(index, hour)| opens_baseline(index, hour.follows_previous).then_some(index))
        .collect::<Vec<_>>();

    let mut unexercised = Vec::<String>::new();
    let mut missing = |condition: bool, description: &str| {
        if condition {
            unexercised.push(description.to_string());
        }
    };
    missing(
        scan.failure_records == 0,
        "known-failure exclusion and post-gap restart (needs a structurally valid failed hour)",
    );
    missing(
        measured.gap_baselines == 0,
        "gap-forced baseline (needs a usable-failed-usable sequence)",
    );
    missing(
        measured.baselines.saturating_sub(measured.gap_baselines) <= 1,
        "48-hour cadence baseline (needs at least 49 contiguous usable observations)",
    );
    missing(
        measured.deltas == 0,
        "delta encoding (needs adjacent changed snapshots)",
    );
    missing(
        measured.unchanged == 0,
        "UNCHANGED (needs adjacent stored-equal snapshots)",
    );
    missing(
        measured.adjacent_collection_changes[d81::FRAME_INDEX] == 0,
        "frame collection change (requires a future authoritative completeness-state transition)",
    );
    missing(
        measured.adjacent_collection_changes[d81::PROVINCES_INDEX] == 0,
        "province collection change (needs changed anchor/terminal state)",
    );
    missing(
        measured.adjacent_collection_changes[d81::DIRECTED_NEIGHBORS_INDEX] == 0,
        "directed-neighbor replacement/reorder (needs changed engine neighbor order/topology)",
    );
    missing(
        measured.adjacent_collection_changes[d81::HUB_SIMULATION_INDEX] == 0,
        "hub-simulation collection change (needs changed simulation state)",
    );
    missing(
        !scan.coverage.v1_failure_carrier,
        "failure-class v1 structural carrier (needs a fully valid known-failure v1 payload)",
    );
    missing(
        !scan.coverage.v4_complete_empty_simulation,
        "complete-empty v4 simulation snapshot (needs a measured zero-row simulation state)",
    );
    for value in 0..=1 {
        missing(
            !scan.coverage.active_terminal[value],
            &format!("active_terminal={value} (needs an anchor with that Boolean)"),
        );
        missing(
            !scan.coverage.anchor_variant[value],
            &format!("anchor_variant={value} (needs that authenticated selector leg)"),
        );
        missing(
            !scan.coverage.hub_subtype[value],
            &format!("hub_subtype={value} (needs a complete hub row of that subtype)"),
        );
        missing(
            !scan.coverage.foreign_route[value],
            &format!("foreign_route={value} (needs a simulation row with that Boolean)"),
        );
        missing(
            !scan.coverage.fallback_record[value],
            &format!("fallback_record={value} (needs a simulation row with that Boolean)"),
        );
    }
    for level in 1..=d81::MAX_RAIL_LEVEL as usize {
        missing(
            !scan.coverage.rail_level[level - 1],
            &format!("rail_edge_level={level} (needs a directed edge at that level)"),
        );
    }
    drop(missing);

    if out_dir.exists() {
        return Err(format!(
            "refusing to overwrite existing measurement directory {}",
            out_dir.display()
        ));
    }
    let parent = out_dir
        .parent()
        .ok_or("measurement output has no parent directory")?;
    let leaf = out_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("measurement output name is not UTF-8")?;
    let stage = parent.join(format!(".{leaf}.d81-stage-{}", std::process::id()));
    if stage.exists() {
        return Err(format!(
            "staging directory already exists: {}",
            stage.display()
        ));
    }
    std::fs::create_dir(&stage).map_err(|error| format!("create {}: {error}", stage.display()))?;
    let published = (|| -> Result<(usize, usize, usize, usize, String), String> {
        let ship_path = stage.join("ship_d81.bin");
        let wire_path = stage.join("wire_new.bin");
        write_bytes(&ship_path, &ship)?;
        write_bytes(&wire_path, &measured.wire)?;
        write_offsets(&stage.join("ship_d81.offsets"), &ship_offsets)?;
        write_offsets(&stage.join("wire_new.offsets"), &measured.offsets)?;
        let ship_zstd = zstd3(&ship_path, &stage.join("ship_d81.bin.zst"))?;
        let wire_zstd = zstd3(&wire_path, &stage.join("wire_new.bin.zst"))?;
        let ship_block_zstd = zstd3_blocks(
            &ship_path,
            &stage.join("ship_d81.blocks.zst"),
            &ship_offsets,
            &block_starts,
        )?;
        let wire_block_zstd = zstd3_blocks(
            &wire_path,
            &stage.join("wire_new.blocks.zst"),
            &measured.offsets,
            &block_starts,
        )?;
        let mut catalog = String::from("province_id\tportable_symbol\n");
        for (province_id, symbol) in &province_bindings {
            catalog.push_str(&format!("{province_id}\t{symbol}\n"));
        }
        write_bytes(&stage.join("province_symbols.tsv"), catalog.as_bytes())?;
        write_bytes(
            &stage.join("definition_resolution.json"),
            &definition_resolution,
        )?;
        let symbol_manifest = symbols
            .manifest()
            .encode_json()
            .map_err(|error| format!("encode province symbol manifest: {error:?}"))?;
        write_bytes(&stage.join("symbols.json"), &symbol_manifest)?;
        let mut pov_report = String::from(
            "portable_pov_country_id\tobservations\tlogical_rows\tsimulation_abi_bytes\tadjacent_transitions\tchanged_rows\tchanged_rows_per_transition\n",
        );
        for (raw_pov, measurement) in &pov_measurements {
            let portable = symbols
                .require_content_id(&ContentIdSource::new(
                    d81::COUNTRY_CONTENT_ID_NAMESPACE,
                    u64::from(*raw_pov),
                ))
                .map_err(|error| format!("resolve d81 POV country {raw_pov}: {error}"))?
                .get();
            pov_report.push_str(&format!(
                "{portable}\t{}\t{}\t{}\t{}\t{}\t{:.6}\n",
                measurement.observations,
                measurement.logical_rows,
                measurement.simulation_abi_bytes,
                measurement.adjacent_transitions,
                measurement.changed_rows,
                measurement.changed_rows as f64 / measurement.adjacent_transitions.max(1) as f64,
            ));
        }
        write_bytes(&stage.join("pov_measurement.tsv"), pov_report.as_bytes())?;
        let pov_report_sha256 = lower_hex(&Sha256::digest(pov_report.as_bytes()));
        // THE REPORT'S OWN SCHEMA MOVED TO v4 BECAUSE ONE OF ITS KEYS DID:
        // `coverage_v3_complete_empty_simulation` named the payload version it
        // gates on, and that version is now v4. Two unrelated things landing on
        // v4 at once is a coincidence of counting, not a shared version.
        let report = format!(
            "schema=battle-plan.d81_real_measurement.v4\n\
session_manifest_sha256={}\nstream_sha256={}\n\
content_manifest_sha256={}\ndefinition_csv_sha256={}\n\
definition_resolution_sha256={}\n\
symbol_manifest_sha256={}\n\
identity_mode={}\ncid1_payload_sha256={}\n\
hours_usable={}\nhours_closed={}\nexcluded_known_failure={}\nrejected=0\n\
hours_verified={}\nadjacent_transition_count={}\ndelta_transition_count={}\n\
baselines={}\ngap_forced_baselines={}\ndeltas={}\nunchanged={}\n\
baseline_bytes={}\ndelta_bytes={}\n\
adjacent_changed_rows={}\nadjacent_tombstones={}\n\
adjacent_changed_rows_per_transition={:.6}\nadjacent_tombstones_per_transition={:.6}\n\
delta_changed_rows={}\ndelta_tombstones={}\n\
delta_changed_rows_per_transition={:.6}\ndelta_tombstones_per_transition={:.6}\n\
ship_raw_bytes={}\nwire_raw_bytes={}\nship_zstd3_bytes={}\nwire_zstd3_bytes={}\n\
ship_48h_zstd3_bytes={}\nwire_48h_zstd3_bytes={}\n\
per_pov_count={}\nper_pov_measurement_sha256={}\n\
coverage_v1_failure_carrier={}\ncoverage_v4_complete_empty_simulation={}\n\
coverage_active_terminal={:?}\ncoverage_anchor_variant={:?}\n\
coverage_rail_level={:?}\ncoverage_hub_subtype={:?}\n\
coverage_foreign_route={:?}\ncoverage_fallback_record={:?}\n\
unexercised={}\n",
            authority.session_sha256,
            authority.stream_sha256,
            authority.content_manifest_sha256,
            lower_hex(&Sha256::digest(&definition)),
            lower_hex(&Sha256::digest(&definition_resolution)),
            lower_hex(&Sha256::digest(&symbol_manifest)),
            match mode {
                StreamIdentityMode::Legacy => "legacy",
                StreamIdentityMode::IdentityV1 => "identity_v1",
            },
            scan.cid1_sha256.as_deref().unwrap_or("none"),
            scan.hours.len(),
            scan.closed_hours,
            scan.failure_records,
            scan.hours.len(),
            scan.hours.len().saturating_sub(1),
            measured.deltas,
            measured.baselines,
            measured.gap_baselines,
            measured.deltas,
            measured.unchanged,
            measured.baseline_bytes,
            measured.delta_bytes,
            measured.adjacent_changed_rows,
            measured.adjacent_tombstones,
            measured.adjacent_changed_rows as f64
                / scan.hours.len().saturating_sub(1).max(1) as f64,
            measured.adjacent_tombstones as f64 / scan.hours.len().saturating_sub(1).max(1) as f64,
            measured.changed_rows,
            measured.tombstones,
            measured.changed_rows as f64 / measured.deltas.max(1) as f64,
            measured.tombstones as f64 / measured.deltas.max(1) as f64,
            ship.len(),
            measured.wire.len(),
            ship_zstd,
            wire_zstd,
            ship_block_zstd,
            wire_block_zstd,
            pov_measurements.len(),
            pov_report_sha256,
            scan.coverage.v1_failure_carrier,
            scan.coverage.v4_complete_empty_simulation,
            scan.coverage.active_terminal,
            scan.coverage.anchor_variant,
            scan.coverage.rail_level,
            scan.coverage.hub_subtype,
            scan.coverage.foreign_route,
            scan.coverage.fallback_record,
            if unexercised.is_empty() {
                "none".to_string()
            } else {
                unexercised.join(",")
            },
        );
        write_bytes(&stage.join("measurement_report.txt"), report.as_bytes())?;
        Ok((
            ship_zstd,
            wire_zstd,
            ship_block_zstd,
            wire_block_zstd,
            report,
        ))
    })();
    let (ship_zstd, wire_zstd, ship_block_zstd, wire_block_zstd, report) = match published {
        Ok(result) => result,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(error);
        }
    };
    if let Err(error) = std::fs::rename(&stage, &out_dir) {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(format!(
            "atomically publish {} -> {}: {error}",
            stage.display(),
            out_dir.display()
        ));
    }
    print!("{report}");
    println!(
        "wire_raw_ratio_percent={:.2}",
        100.0 * measured.wire.len() as f64 / ship.len().max(1) as f64
    );
    println!(
        "wire_zstd3_ratio_percent={:.2}",
        100.0 * wire_zstd as f64 / ship_zstd.max(1) as f64
    );
    println!(
        "wire_48h_zstd3_ratio_percent={:.2}",
        100.0 * wire_block_zstd as f64 / ship_block_zstd.max(1) as f64
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("bench_real_d81: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::rail_replay::{
        decode_payload, encode_payload, encode_payload_with_hub_simulation, RailHubSimulationState,
        RailNeighborState, RailProvinceState, PAYLOAD_FLAG_HUB_FRAMES_UNAVAILABLE,
    };
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_STATUS_STATE_UNAVAILABLE};
    use portable_content_manifest::{
        ContentLayerKind, LayerStorageKind, RecordedContentLayer, RequiredContent,
        SourceGameIdentity, SourceProfileIdentity, SCHEMA,
    };

    fn symbols() -> SymbolManifestBuilder {
        let mut symbols = SymbolManifestBuilder::from_base([
            "country/tag/GER",
            "country/tag/FRA",
            "province/rgb/1/2/3",
            "province/rgb/4/5/6",
        ])
        .unwrap();
        symbols
            .bind_existing_sources([
                SymbolObservation::content_id(
                    d81::COUNTRY_CONTENT_ID_NAMESPACE,
                    7,
                    "country/tag/GER",
                ),
                SymbolObservation::content_id(
                    d81::COUNTRY_CONTENT_ID_NAMESPACE,
                    8,
                    "country/tag/FRA",
                ),
                SymbolObservation::content_id(
                    d81::PROVINCE_CONTENT_ID_NAMESPACE,
                    1,
                    "province/rgb/1/2/3",
                ),
                SymbolObservation::content_id(
                    d81::PROVINCE_CONTENT_ID_NAMESPACE,
                    2,
                    "province/rgb/4/5/6",
                ),
            ])
            .unwrap();
        symbols
    }

    fn payload(status: u32, shift: f32) -> Vec<u8> {
        let provinces = [
            RailProvinceState {
                province_id: 1,
                active_terminal: true,
                anchor_variant: 0,
                anchor_bits: [shift.to_bits(), 2.0f32.to_bits(), 3.0f32.to_bits()],
                neighbors: vec![RailNeighborState {
                    province_id: 2,
                    edge_level: 4,
                }],
            },
            RailProvinceState {
                province_id: 2,
                active_terminal: false,
                anchor_variant: 1,
                anchor_bits: [4.0f32.to_bits(), 5.0f32.to_bits(), 6.0f32.to_bits()],
                neighbors: vec![RailNeighborState {
                    province_id: 1,
                    edge_level: 2,
                }],
            },
        ];
        encode_payload_with_hub_simulation(
            &provinces,
            &[RailHubSimulationState {
                pov_country_id: 7,
                province_id: 1,
                province_owner_country_id: 8,
                subtype: 0,
                route_class: 2,
                node_kind: 5,
                node_flag: 0,
                available_raw: 200_000,
                consumed_raw: 100_000,
                foreign_max_raw: 300_000,
                foreign_route: true,
                fallback_record: false,
            }],
            7,
            status,
            2,
            2,
        )
        .unwrap()
    }

    fn record(sequence: u64, hour: i32, domain: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn data(sequence: u64, hour: i32, payload: &[u8]) -> Vec<u8> {
        record(
            sequence,
            hour,
            D81,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        )
    }

    fn perf_domain(sequence: u64, hour: i32, domain: u16, status: u32) -> Vec<u8> {
        let mut payload = vec![0u8; 40];
        payload[32..36].copy_from_slice(&status.to_le_bytes());
        record(
            sequence,
            hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &payload,
        )
    }

    fn perf(sequence: u64, hour: i32, status: u32) -> Vec<u8> {
        perf_domain(sequence, hour, D81, status)
    }

    fn frame_end(sequence: u64, hour: i32) -> Vec<u8> {
        frame_end_with_status(sequence, hour, 0)
    }

    fn frame_end_with_status(sequence: u64, hour: i32, status: u32) -> Vec<u8> {
        let mut payload = [0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&1u32.to_le_bytes());
        payload[20..24].copy_from_slice(&1u32.to_le_bytes());
        payload[24..28].copy_from_slice(&status.to_le_bytes());
        record(
            sequence,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    /// A FRAME_END that declares its own emitted/scheduled domain count, for the
    /// cases that must reach a rule beyond the seal arithmetic.
    fn frame_end_with_domain_counts(sequence: u64, hour: i32, domains: u32) -> Vec<u8> {
        let mut payload = [0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&domains.to_le_bytes());
        payload[20..24].copy_from_slice(&domains.to_le_bytes());
        record(
            sequence,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    fn one_hour(payload: &[u8]) -> Vec<u8> {
        let status = u32::from_le_bytes(payload[12..16].try_into().unwrap());
        [perf(1, 10, status), data(2, 10, payload), frame_end(3, 10)].concat()
    }

    fn country_identity(sequence: u64, hour: i32, rows: &[(u32, &str)]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for (id, tag) in rows {
            payload.extend_from_slice(&id.to_le_bytes());
            payload.extend_from_slice(&(tag.len() as u16).to_le_bytes());
            payload.extend_from_slice(tag.as_bytes());
            payload.push(1);
        }
        payload.extend_from_slice(compact_contract::country_identity_membership::CIM1_MAGIC);
        payload.extend_from_slice(
            &compact_contract::country_identity_membership::CIM1_VERSION.to_le_bytes(),
        );
        payload.extend_from_slice(
            &(compact_contract::country_identity_membership::CIM1_HEADER_LEN as u16).to_le_bytes(),
        );
        payload.extend_from_slice(
            &[0; compact_contract::country_identity_membership::CIM1_HEADER_LEN - 8],
        );
        record(
            sequence,
            hour,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        )
    }

    fn country_identity_hour(sequence: u64, hour: i32, rows: &[(u32, &str)]) -> Vec<u8> {
        [
            perf_domain(sequence, hour, D18, 0),
            country_identity(sequence + 1, hour, rows),
            frame_end(sequence + 2, hour),
        ]
        .concat()
    }

    fn valid_hour(offset: i32) -> i32 {
        compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24 + offset
    }

    fn domain_gap(sequence: u64, hour: i32, range_start: i32, ordinal: u64) -> Vec<u8> {
        let payload = compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: compact_contract::DomainGapReason::Overrun,
            target_hour_raw: hour,
            range_start_hour_raw: range_start,
            range_end_hour_raw: hour,
            tick_before: 100,
            tick_after: 100 + u64::try_from(hour - range_start + 1).unwrap(),
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: ordinal,
        }
        .encode()
        .unwrap();
        record(
            sequence,
            hour,
            D81,
            COMPACT_RECORD_TYPE_DOMAIN_GAP,
            &payload,
        )
    }

    fn historical_v2_empty() -> Vec<u8> {
        let mut payload = vec![0u8; compact_rail::HEADER_LEN];
        payload[..4].copy_from_slice(b"RRL1");
        payload[4..6].copy_from_slice(&compact_rail::PAYLOAD_VERSION_V2.to_le_bytes());
        payload[6..8].copy_from_slice(&(compact_rail::HEADER_LEN as u16).to_le_bytes());
        payload[8..10].copy_from_slice(&(compact_rail::ROW_LEN as u16).to_le_bytes());
        payload[32..36]
            .copy_from_slice(&compact_rail::PAYLOAD_FLAG_HUB_FRAMES_COMPLETE.to_le_bytes());
        // THE DECODER REFUSES THIS, and that is the point of the fixture: it is
        // a historical v2 payload, and since the v4 change every pre-v4 payload
        // is refused by name. This used to `unwrap()` a successful decode to
        // prove the fixture was really a v2 payload; it now proves the same
        // thing by the refusal naming version 2.
        let error = compact_rail::decode_payload(&payload)
            .expect_err("a pre-v4 payload is refused by the decoder");
        assert!(
            error.contains("version 2 is refused") && error.contains("only v4"),
            "{error}"
        );
        payload
    }

    #[test]
    fn d18_country_symbols_are_authenticated_and_ambiguous_rebinding_fails() {
        let stream = country_identity_hour(1, 10, &[(7, "GER"), (8, "FRA")]);
        assert_eq!(
            authenticated_country_bindings(&stream).unwrap(),
            vec![
                (7, "country/tag/GER".to_string()),
                (8, "country/tag/FRA".to_string()),
            ]
        );
        let ambiguous = [
            country_identity_hour(1, 10, &[(7, "GER")]),
            country_identity_hour(4, 11, &[(7, "FRA")]),
        ]
        .concat();
        assert!(authenticated_country_bindings(&ambiguous)
            .unwrap_err()
            .contains("changes symbol"));
        let aliased = country_identity_hour(1, 10, &[(7, "GER"), (8, "GER")]);
        assert!(authenticated_country_bindings(&aliased)
            .unwrap_err()
            .contains("aliases source ids"));
    }

    #[test]
    fn d18_country_symbols_require_status_bound_perf_data_and_frame_seal() {
        let missing_perf = [
            perf_domain(1, 10, D81, 0),
            country_identity(2, 10, &[(7, "GER")]),
            frame_end(3, 10),
        ]
        .concat();
        assert!(authenticated_country_bindings(&missing_perf)
            .unwrap_err()
            .contains("PERF/DATA binding differs"));

        let status_mismatch = [
            perf_domain(1, 10, D18, COMPACT_STATUS_STATE_UNAVAILABLE),
            country_identity(2, 10, &[(7, "GER")]),
            frame_end_with_status(3, 10, COMPACT_STATUS_STATE_UNAVAILABLE),
        ]
        .concat();
        assert!(authenticated_country_bindings(&status_mismatch)
            .unwrap_err()
            .contains("PERF/DATA binding differs"));
    }

    #[test]
    fn clean_v4_stream_certifies_and_reader_reconstructs_source() {
        let stream = [
            perf(1, 10, 0),
            data(2, 10, &payload(0, 1.0)),
            frame_end(3, 10),
            perf(4, 11, 0),
            data(5, 11, &payload(0, 9.0)),
            frame_end(6, 11),
        ]
        .concat();
        let scan = scan_stream(&stream, &symbols(), StreamIdentityMode::Legacy, "test").unwrap();
        assert_eq!(scan.closed_hours, 2);
        assert_eq!(scan.hours.len(), 2);
        let measured = measure(&d81::schema(), &scan.hours).unwrap();
        assert_eq!(measured.baselines, 1);
        assert_eq!(measured.deltas, 1);
        assert!(measured.adjacent_changed_rows > 0);
    }

    #[test]
    fn sequence_eof_and_per_hour_completeness_fail_closed() {
        let clean = payload(0, 1.0);
        assert!(scan_stream(
            &[perf(1, 10, 0), data(2, 10, &clean)].concat(),
            &symbols(),
            StreamIdentityMode::Legacy,
            "test"
        )
        .unwrap_err()
        .contains("open at EOF"));
        // A FRAME_END with nothing open. The shared `frame_end` helper declares
        // emitted=scheduled=1, so a lone one of those is refused earlier and for
        // a different reason -- "disagrees with PERF domains" -- and never
        // reaches the open-hour check this line is here to pin. Declaring zero
        // domains satisfies the seal arithmetic and leaves the open-hour rule as
        // the only thing that can reject the record.
        assert_eq!(
            scan_stream(
                &frame_end_with_domain_counts(1, 10, 0),
                &symbols(),
                StreamIdentityMode::Legacy,
                "test"
            )
            .unwrap_err(),
            "FRAME_END at byte 0 does not close its open hour"
        );
        let skipped_sequence = [perf(2, 10, 0), data(3, 10, &clean), frame_end(4, 10)].concat();
        assert!(scan_stream(
            &skipped_sequence,
            &symbols(),
            StreamIdentityMode::Legacy,
            "test"
        )
        .is_err());
    }

    #[test]
    fn duplicate_success_failure_pair_is_rejected_before_exclusion() {
        let stream = [
            perf(1, 10, 0),
            data(2, 10, &payload(0, 1.0)),
            data(3, 10, &payload(COMPACT_STATUS_STATE_UNAVAILABLE, 1.0)),
            frame_end(4, 10),
        ]
        .concat();
        assert!(
            scan_stream(&stream, &symbols(), StreamIdentityMode::Legacy, "test")
                .unwrap_err()
                .contains("more than one")
        );
    }

    #[test]
    fn d81_success_requires_status_bound_perf_data_terminal() {
        let data_without_d81_perf = [
            perf_domain(1, 10, D18, 0),
            data(2, 10, &payload(0, 1.0)),
            frame_end(3, 10),
        ]
        .concat();
        assert!(scan_stream(
            &data_without_d81_perf,
            &symbols(),
            StreamIdentityMode::Legacy,
            "test"
        )
        .unwrap_err()
        .contains("no matching D81 PERF"));

        let status_mismatch = [
            perf(1, 10, COMPACT_STATUS_STATE_UNAVAILABLE),
            data(2, 10, &payload(0, 1.0)),
            frame_end_with_status(3, 10, COMPACT_STATUS_STATE_UNAVAILABLE),
        ]
        .concat();
        assert!(scan_stream(
            &status_mismatch,
            &symbols(),
            StreamIdentityMode::Legacy,
            "test"
        )
        .unwrap_err()
        .contains("differs from D81 PERF"));
    }

    #[test]
    fn malformed_failure_and_unknown_status_are_rejected() {
        let mut malformed = payload(COMPACT_STATUS_STATE_UNAVAILABLE, 1.0);
        malformed.push(0);
        assert!(scan_stream(
            &one_hour(&malformed),
            &symbols(),
            StreamIdentityMode::Legacy,
            "test"
        )
        .is_err());

        let mut unknown = payload(0, 1.0);
        unknown[12..16].copy_from_slice(&(1u32 << 31).to_le_bytes());
        assert!(scan_stream(
            &one_hour(&unknown),
            &symbols(),
            StreamIdentityMode::Legacy,
            "test"
        )
        .unwrap_err()
        .contains("unknown global bits"));
    }

    #[test]
    fn excluded_failure_forces_next_observation_to_baseline() {
        let stream = [
            perf(1, 10, 0),
            data(2, 10, &payload(0, 1.0)),
            frame_end(3, 10),
            perf(4, 11, COMPACT_STATUS_STATE_UNAVAILABLE),
            data(5, 11, &payload(COMPACT_STATUS_STATE_UNAVAILABLE, 1.0)),
            frame_end_with_status(6, 11, COMPACT_STATUS_STATE_UNAVAILABLE),
            perf(7, 12, 0),
            data(8, 12, &payload(0, 2.0)),
            frame_end(9, 12),
        ]
        .concat();
        let scan = scan_stream(&stream, &symbols(), StreamIdentityMode::Legacy, "test").unwrap();
        let measured = measure(&d81::schema(), &scan.hours).unwrap();
        assert_eq!(scan.failure_records, 1);
        assert_eq!(measured.baselines, 2);
        assert_eq!(measured.gap_baselines, 1);
    }

    #[test]
    fn h4dg_is_the_only_non_data_terminal_and_forces_rebaseline() {
        let h0 = valid_hour(0);
        let h1 = valid_hour(1);
        let h2 = valid_hour(2);
        let h3 = valid_hour(3);
        let stream = [
            perf(1, h0, 0),
            data(2, h0, &payload(0, 1.0)),
            frame_end(3, h0),
            domain_gap(4, h2, h1, 0),
            frame_end(5, h2),
            perf(6, h3, 0),
            data(7, h3, &payload(0, 2.0)),
            frame_end(8, h3),
        ]
        .concat();
        let scan = scan_stream(&stream, &symbols(), StreamIdentityMode::Legacy, "test").unwrap();
        let measured = measure(&d81::schema(), &scan.hours).unwrap();
        assert_eq!(scan.closed_hours, 3);
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(measured.baselines, 2);
        assert_eq!(measured.gap_baselines, 1);
    }

    #[test]
    fn historical_v1_success_is_rejected_but_v1_failure_is_typed_unavailable() {
        let provinces = decode_payload(&payload(0, 1.0)).unwrap().provinces;
        let v1_success =
            encode_payload(&provinces, 0, 0, PAYLOAD_FLAG_HUB_FRAMES_UNAVAILABLE, 2, 2).unwrap();
        let v1_failure = encode_payload(
            &provinces,
            0,
            COMPACT_STATUS_STATE_UNAVAILABLE,
            PAYLOAD_FLAG_HUB_FRAMES_UNAVAILABLE,
            2,
            2,
        )
        .unwrap();
        assert!(scan_stream(
            &one_hour(&v1_success),
            &symbols(),
            StreamIdentityMode::Legacy,
            "test",
        )
        .unwrap_err()
        .contains("historical UI-era/non-production"));
        let stream = [
            perf(1, 10, COMPACT_STATUS_STATE_UNAVAILABLE),
            data(2, 10, &v1_failure),
            frame_end_with_status(3, 10, COMPACT_STATUS_STATE_UNAVAILABLE),
            perf(4, 11, 0),
            data(5, 11, &payload(0, 2.0)),
            frame_end(6, 11),
        ]
        .concat();
        let scan = scan_stream(&stream, &symbols(), StreamIdentityMode::Legacy, "test").unwrap();
        assert!(scan.coverage.v1_failure_carrier);
    }

    #[test]
    fn historical_v2_success_is_explicitly_rejected() {
        assert!(scan_stream(
            &one_hour(&historical_v2_empty()),
            &symbols(),
            StreamIdentityMode::Legacy,
            "test",
        )
        .unwrap_err()
        .contains("historical UI-era/non-production"));
    }

    #[test]
    fn production_measurement_cli_rejects_legacy_identity_mode() {
        assert_eq!(
            promotion_identity_mode("identity-v1").unwrap(),
            StreamIdentityMode::IdentityV1
        );
        assert!(promotion_identity_mode("legacy")
            .unwrap_err()
            .contains("requires identity-v1"));
    }

    #[test]
    fn structurally_valid_but_empty_v4_cannot_satisfy_clean_capture_gate() {
        let empty = encode_payload_with_hub_simulation(&[], &[], 0, 0, 0, 0).unwrap();
        assert!(scan_stream(
            &one_hour(&empty),
            &symbols(),
            StreamIdentityMode::Legacy,
            "test"
        )
        .unwrap_err()
        .contains("lacks a clean v4"));
    }

    fn authority_fixture(
        stream: &[u8],
    ) -> (
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        ContentManifest,
        UsedContentKey,
    ) {
        let definition = b"1;1;2;3;land;false;plains;1\n2;4;5;6;land;false;plains;1\n".to_vec();
        let digest = lower_hex(&Sha256::digest(&definition));
        let resolution = UsedContentKey {
            effective_vfs_manifest_sha256: "f".repeat(64),
            normalized_game_relative_path: DEFINITION_PATH.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: digest.clone(),
            payload_sha256: digest,
            payload_bytes: definition.len() as u64,
        };
        let mut manifest = ContentManifest {
            schema: SCHEMA.into(),
            product_modset_sha256: "9".repeat(64),
            source_game: SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256: "8".repeat(64),
                build_profile: "test".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "profile".into(),
                playset_id: "playset".into(),
                selection_sha256: "0".repeat(64),
            },
            effective_vfs_manifest_sha256: "f".repeat(64),
            ordered_layers: vec![RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: vec![],
                required_for_playback: true,
            }],
            dlc_selection: vec![],
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            recorder_local_settings: BTreeMap::new(),
            required_content: vec![RequiredContent {
                content_key: DEFINITION_PATH.into(),
                kind: RequiredContentKind::Definition,
                content: resolution.clone(),
                observation_count: 1,
                consumers: vec!["wire_product.d81".into()],
            }],
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest = manifest.with_computed_digest().unwrap();
        let manifest_bytes = manifest.encode_json().unwrap();
        let roster = compact_contract::production_profile_scheduled_names();
        assert!(
            roster.contains(&"divisions"),
            "fixture pins wire names rather than canonical descriptor names"
        );
        let session = serde_json::to_vec(&serde_json::json!({
            "schema": "native_collector_session_v1",
            "stream_format": "compact",
            "compact_stream_identity_mode": "legacy",
            "production_profile": {
                "enabled": true,
                "profile_domains": roster.clone(),
                "profile_domains_not_scheduled": [],
                "scheduled_domains_not_in_profile": [],
                "mandatory_domains_not_production_complete": [],
                "missing_required_static_artifacts": []
            },
            "scheduled_domains": roster,
            "replay_artifact": {"recovery": false, "gate_problems": []},
            "replay_packaging": {"container_emitted": true},
            "acceptance_telemetry": {"complete": true, "problems": []},
            "stream_storage": {
                "decoded_bytes": stream.len(),
                "decoded_sha256": lower_hex(&Sha256::digest(stream))
            },
            "portable_content_manifest": serde_json::to_value(&manifest).unwrap()
        }))
        .unwrap();
        (
            session,
            manifest_bytes,
            serde_json::to_vec(&resolution).unwrap(),
            definition,
            manifest,
            resolution,
        )
    }

    #[test]
    fn capture_authority_pins_stream_manifest_finalization_and_definition_resolution() {
        let stream = one_hour(&payload(0, 1.0));
        let (session, manifest_bytes, resolution_bytes, definition, manifest, resolution) =
            authority_fixture(&stream);
        validate_capture_authority(
            &session,
            &stream,
            &manifest_bytes,
            StreamIdentityMode::Legacy,
        )
        .unwrap();
        authenticated_province_symbols(&manifest_bytes, &resolution_bytes, &definition, &[])
            .unwrap();

        let mut recovery_session: serde_json::Value = serde_json::from_slice(&session).unwrap();
        recovery_session["replay_artifact"]["recovery"] = serde_json::json!(true);
        assert!(validate_capture_authority(
            &serde_json::to_vec(&recovery_session).unwrap(),
            &stream,
            &manifest_bytes,
            StreamIdentityMode::Legacy
        )
        .unwrap_err()
        .contains("non-recovery"));

        let mut override_session: serde_json::Value = serde_json::from_slice(&session).unwrap();
        override_session["scheduled_domains"] = serde_json::json!([d81::DOMAIN.to_string()]);
        assert!(validate_capture_authority(
            &serde_json::to_vec(&override_session).unwrap(),
            &stream,
            &manifest_bytes,
            StreamIdentityMode::Legacy
        )
        .unwrap_err()
        .contains("canonical full production roster"));

        let mut changed_stream = stream.clone();
        changed_stream.push(0);
        assert!(validate_capture_authority(
            &session,
            &changed_stream,
            &manifest_bytes,
            StreamIdentityMode::Legacy
        )
        .unwrap_err()
        .contains("H4CR digest"));

        // A DIFFERENT but internally valid finalized manifest. The localisation
        // language is an input to `computed_selection_sha256`, so changing it
        // invalidates the recorded selection digest; `with_computed_digest`
        // validates that selection before computing the manifest digest and was
        // right to refuse. Recompute the selection first, so this arrives at
        // `validate_capture_authority` as a well-formed foreign manifest and the
        // refusal below is about session binding rather than a malformed input.
        let mut other = manifest.clone();
        other.localisation_language = "l_french".into();
        other.manifest_digest_sha256 = None;
        other.source_profile.selection_sha256 = other.computed_selection_sha256().unwrap();
        other = other.with_computed_digest().unwrap();
        assert!(validate_capture_authority(
            &session,
            &stream,
            &other.encode_json().unwrap(),
            StreamIdentityMode::Legacy
        )
        .unwrap_err()
        .contains("not the session"));

        let mut unfinalized_session: serde_json::Value = serde_json::from_slice(&session).unwrap();
        let mut unfinalized = manifest.clone();
        unfinalized.manifest_digest_sha256 = None;
        unfinalized_session["portable_content_manifest"] =
            serde_json::to_value(unfinalized).unwrap();
        assert!(validate_capture_authority(
            &serde_json::to_vec(&unfinalized_session).unwrap(),
            &stream,
            &manifest_bytes,
            StreamIdentityMode::Legacy
        )
        .unwrap_err()
        .contains("not finalized"));

        let mut substituted = resolution;
        substituted.payload_sha256 = "a".repeat(64);
        assert!(authenticated_province_symbols(
            &manifest_bytes,
            &serde_json::to_vec(&substituted).unwrap(),
            &definition,
            &[],
        )
        .unwrap_err()
        .contains("disagrees"));
    }
}
