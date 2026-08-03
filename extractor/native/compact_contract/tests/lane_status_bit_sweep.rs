//! Anti-drift sweep: every lane-local status-bit DECLARATION in
//! `extractor/native/lanes/` must be registered in
//! `compact_contract::lane_local_status_mask`, or provably never emitted.
//!
//! The failure class this pins: the contract's registry is a table maintained
//! next to (but apart from) the lane declarations, and such tables drift
//! silently — domain 53 declared badge bits 16..=22 and emitted them on 379 of
//! 514 hours of the batch-3a capture while the registry knew only domains 49
//! and 51, which made every such record an unassigned-status-bit stream
//! rejection under the contract's own validity rule
//! (`compact_status_unknown_flags_v1` docs). Same drift class as the
//! `OWNED_EMITTED_DOMAIN_DEPENDENCIES` fix: the cure is a consumer that reads
//! the declarations, so the two can no longer disagree unobserved.
//!
//! What counts as a lane-local declaration: a `const STATUS_*: u32 = <bit>;`
//! whose literal value is a status-word bit and whose NAME is not the global
//! v1 name for that bit. Mirrors of the global vocabulary (`STATUS_PARTIAL_DATA
//! = 1 << 9` and friends) are the lane convention for keeping the shared bits
//! nameable inside a lane module and are exempt by exact name match.
//! Declarations that alias `compact_contract::DOMAIN*` constants carry no
//! literal and are exempt by construction — the compiler pins those.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use compact_contract::{assigned_descriptor_by_id, lane_local_status_mask, CompactDomainStatus};

/// Global v1 status-bit names, indexed by bit position
/// (`status_flags_base.rs`). A lane constant named `STATUS_<name>` declaring
/// exactly `1 << bit` is a mirror, not a lane-local badge.
const GLOBAL_V1_BIT_NAMES: [&str; 15] = [
    "STATE_UNAVAILABLE",
    "PROFILE_MISMATCH",
    "NO_CLOCK",
    "NO_SCALAR_SOURCE",
    "WAR_BRIDGE_APPLIED",
    "EXTRACTION_PERF_GATE",
    "WORKER_PANIC",
    "BRIDGE_UNAVAILABLE",
    "SOURCE_UNAVAILABLE",
    "PARTIAL_DATA",
    "DELTA_DATA",
    "TRANSITION_BASELINE_RESET",
    "CONTROL_INVALID",
    "WAR_SEGMENT_ABSENT",
    "SOURCE_EMPTY",
];

/// This table IS a local copy of the contract's vocabulary, which is the drift
/// this file exists to catch elsewhere. Pin its length against the contract so
/// the sweep cannot quietly start reading an assigned global bit as a
/// lane-local badge: a `STATUS_SOURCE_EMPTY = 1 << 14` mirror in a lane would
/// have been reported as an unregistered lane-local declaration purely because
/// this array stopped one entry short.
#[test]
fn the_global_bit_name_table_covers_the_whole_assigned_vocabulary() {
    assert_eq!(
        compact_contract::COMPACT_STATUS_V1_KNOWN_MASK.count_ones() as usize,
        GLOBAL_V1_BIT_NAMES.len(),
        "the contract assigned a status bit this table does not name"
    );
    assert_eq!(
        compact_contract::COMPACT_STATUS_V1_KNOWN_MASK,
        (1u32 << GLOBAL_V1_BIT_NAMES.len()) - 1,
        "the assigned vocabulary is no longer a contiguous run from bit 0, so \
         indexing this table by bit position is wrong"
    );
}

/// Lane-local declarations that are deliberately NOT registered because the
/// production decode path rejects them outright — a stream carrying one is
/// refused by the lane's own gate, so no consumer ever has to classify it.
/// Each entry carries the source marker whose presence the sweep re-verifies;
/// if the retirement is ever lifted, the marker disappears and this sweep
/// fails, forcing a re-triage instead of silently keeping the amnesty.
struct NeverEmitted {
    domain_id: u16,
    bits: &'static [u32],
    marker_file_suffix: &'static str,
    marker: &'static str,
}

/// Explicit constant-to-domain bindings for files that declare MORE THAN ONE
/// assigned domain id (shared modules). Without a binding, "registered for
/// any candidate" would let a bit declared for one domain ride on a sibling
/// domain's registration — e.g. a hypothetical d50-local bit 11 passing via
/// d49's mask in the shared naval module. A lane-local declaration in a
/// multi-domain file MUST have an entry here; the sweep fails otherwise.
const MULTI_DOMAIN_BINDINGS: &[(&str, u16)] = &[
    // The shared 49/50 naval module's badge belongs to domain 49
    // (naval_combat_lifecycle_live): only lifecycle rows carry the
    // location/region promotion the badge reports.
    ("STATUS_LOCATION_CANDIDATE_UNVALIDATED", 49),
];

const NEVER_EMITTED: &[NeverEmitted] = &[
    // d56's four fixture badges are schema-v1 positions retained for
    // fixed-width decoding but forbidden in production; the lane rejects
    // streams carrying them ("retired fixture status bits", see
    // `compact_stream_gate_rejects_retired_lane_fixture_badges`).
    NeverEmitted {
        domain_id: 56,
        bits: &[16, 17, 18, 19],
        marker_file_suffix: "naval_missions_status_live/src/domain56_compact.rs",
        marker: "pub const RETIRED_SOURCE_STATUS_MASK: u32 = STATUS_MISSION_FIXTURE_REQUIRED",
    },
    // d61's quartet (bits 0..=3: SOURCE_UNAVAILABLE / BINDING_UNVERIFIED /
    // PARTIAL / VALUE_VALIDATION_FIXTURE_REQUIRED) is the pre-promotion
    // packet's self-contained vocabulary, retired when the lane was promoted
    // onto the shared status word: the production emitter
    // (`attach_log.rs` `read_compact_air_loss_stats_live`) writes only global
    // vocabulary into the payload status field, and the production fold
    // (`world_fold.rs` `fold_air_loss_stats`) classifies that field with the
    // global `handle_compact_failure`. Registering these would be actively
    // wrong — at bits 0..=3 they alias STATE_UNAVAILABLE / PROFILE_MISMATCH /
    // NO_CLOCK / NO_SCALAR_SOURCE, and masking those out of d61 would erase
    // real failure classification.
    NeverEmitted {
        domain_id: 61,
        bits: &[0, 1, 2, 3],
        marker_file_suffix: "air_loss_stats_live/src/lib.rs",
        marker: "Legacy lane-local status from the pre-promotion packet",
    },
];

#[derive(Debug)]
struct Finding {
    file: PathBuf,
    line_no: usize,
    name: String,
    value: u32,
}

fn lanes_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../lanes")
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("file type");
        if file_type.is_dir() {
            // Build output only; every source dir is swept.
            if entry.file_name() != "target" {
                collect_rs_files(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Parses `[pub] const <NAME>: u32 = <literal>;` where `<literal>` is
/// `1 << N`, `1u32 << N`, or a hex literal. Returns `(name, value)`.
/// Identifier-valued and OR-composed declarations return `None` — those are
/// aliases or masks whose components are checked at their own declaration.
fn parse_const_u32_literal(line: &str) -> Option<(String, u32)> {
    let trimmed = line.trim_start();
    let after_vis = trimmed
        .strip_prefix("pub const ")
        .or_else(|| trimmed.strip_prefix("pub(crate) const "))
        .or_else(|| trimmed.strip_prefix("const "))?;
    let (name, rest) = after_vis.split_once(':')?;
    let name = name.trim();
    let rest = rest.trim_start();
    let rest = rest.strip_prefix("u32")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    let expr = rest.strip_suffix(';')?.trim();
    if let Some((lhs, shift)) = expr.split_once("<<") {
        let lhs = lhs.trim();
        if lhs != "1" && lhs != "1u32" && lhs != "1_u32" {
            return None;
        }
        let shift: u32 = shift.trim().parse().ok()?;
        if shift >= 32 {
            return None;
        }
        return Some((name.to_string(), 1u32 << shift));
    }
    if let Some(hex) = expr.strip_prefix("0x") {
        let cleaned: String = hex.chars().filter(|c| *c != '_').collect();
        let value = u32::from_str_radix(&cleaned, 16).ok()?;
        return Some((name.to_string(), value));
    }
    None
}

/// Every literal `const DOMAIN*: u16 = N;` in the file — the lane conventions
/// are `DOMAIN_ID`, `DOMAIN_ID_PROPOSED`, and per-domain names such as
/// `DOMAIN_NAVAL_COMBAT_LIFECYCLE`. Non-domain `u16` consts (record types,
/// `DOMAIN_FRAME`) may slip in; they carry an empty mask and never satisfy a
/// registration check, so they only ever make the sweep stricter to pass.
fn domain_id_candidates(text: &str) -> BTreeSet<u16> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(after_vis) = trimmed
            .strip_prefix("pub const ")
            .or_else(|| trimmed.strip_prefix("pub(crate) const "))
            .or_else(|| trimmed.strip_prefix("const "))
        else {
            continue;
        };
        let Some((name, rest)) = after_vis.split_once(':') else {
            continue;
        };
        if !name.trim().starts_with("DOMAIN") {
            continue;
        }
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix("u16") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        if let Some(value) = rest.trim().strip_suffix(';') {
            if let Ok(value) = value.trim().parse::<u16>() {
                out.insert(value);
            }
        }
    }
    out
}

/// The file's OWN domain id: the exact `const DOMAIN_ID: u16 = N;` (or
/// `DOMAIN_ID_PROPOSED`) declaration, as opposed to the broader candidate
/// set which also collects cross-referenced `DOMAIN_*` ids.
fn own_domain_id(text: &str) -> Option<u16> {
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(after_vis) = trimmed
            .strip_prefix("pub const ")
            .or_else(|| trimmed.strip_prefix("pub(crate) const "))
            .or_else(|| trimmed.strip_prefix("const "))
        else {
            continue;
        };
        let Some((name, rest)) = after_vis.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name != "DOMAIN_ID" && name != "DOMAIN_ID_PROPOSED" {
            continue;
        }
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix("u16") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        if let Some(value) = rest.trim().strip_suffix(';') {
            if let Ok(value) = value.trim().parse::<u16>() {
                return Some(value);
            }
        }
    }
    None
}

fn is_global_mirror(name: &str, value: u32) -> bool {
    if value.count_ones() != 1 {
        return false;
    }
    let bit = value.trailing_zeros() as usize;
    if bit >= GLOBAL_V1_BIT_NAMES.len() {
        return false;
    }
    name == format!("STATUS_{}", GLOBAL_V1_BIT_NAMES[bit])
}

#[test]
fn every_declared_lane_local_status_bit_is_registered_or_provably_never_emitted() {
    let root = lanes_root();
    assert!(
        root.is_dir(),
        "lanes root {} not found; this sweep must run from the repo tree",
        root.display()
    );
    let mut files = Vec::new();
    collect_rs_files(&root, &mut files);
    assert!(
        files.len() > 50,
        "suspiciously few lane sources ({}) — sweep coverage broken",
        files.len()
    );

    // (finding, candidate domain ids, never-scheduled own-domain) for every
    // literal STATUS_* declaration that is not a global mirror.
    let mut lane_local: Vec<(Finding, BTreeSet<u16>, bool)> = Vec::new();
    let mut violations: Vec<String> = Vec::new();

    for file in &files {
        let text = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
        let mut findings = Vec::new();
        for (idx, line) in text.lines().enumerate() {
            let Some((name, value)) = parse_const_u32_literal(line) else {
                continue;
            };
            if !name.starts_with("STATUS_") || value == 0 {
                continue;
            }
            if is_global_mirror(&name, value) {
                continue;
            }
            findings.push(Finding {
                file: file.clone(),
                line_no: idx + 1,
                name,
                value,
            });
        }
        if findings.is_empty() {
            continue;
        }
        // A file whose OWN domain descriptor says "never scheduled in
        // production" (d58's Reserved badge lane) cannot put its vocabulary
        // on a production stream at all; its declarations are proof-artifact
        // vocabulary. The check re-fires if the domain is ever promoted to
        // schedulable.
        let never_scheduled = own_domain_id(&text)
            .and_then(assigned_descriptor_by_id)
            .is_some_and(|descriptor| !descriptor.schedulable);
        // Resolve the lane's domain id(s): this file first, then every file
        // in the same lane directory tree. Ids without a contract descriptor
        // (record-type constants, `DOMAIN_FRAME = 1000`) are not domains and
        // are dropped so they cannot inflate a file into the multi-domain
        // rule or satisfy any check.
        let mut candidates = domain_id_candidates(&text);
        if candidates.is_empty() {
            let lane_dir = file
                .strip_prefix(&root)
                .ok()
                .and_then(|rel| rel.components().next())
                .map(|c| root.join(c.as_os_str()))
                .unwrap_or_else(|| root.clone());
            let mut lane_files = Vec::new();
            collect_rs_files(&lane_dir, &mut lane_files);
            for sibling in lane_files {
                let sibling_text = fs::read_to_string(&sibling)
                    .unwrap_or_else(|e| panic!("reading {}: {e}", sibling.display()));
                candidates.extend(domain_id_candidates(&sibling_text));
            }
        }
        candidates.retain(|id| {
            assigned_descriptor_by_id(*id)
                .is_some_and(|descriptor| descriptor.status != CompactDomainStatus::Infrastructure)
        });
        for finding in findings {
            if candidates.is_empty() {
                violations.push(format!(
                    "{}:{}: lane-local status bit {} = {:#x} found but no literal \
                     `const DOMAIN*: u16 = N;` resolves this lane's domain id — \
                     declare one or extend the sweep's resolution rule",
                    finding.file.display(),
                    finding.line_no,
                    finding.name,
                    finding.value
                ));
            } else {
                lane_local.push((finding, candidates.clone(), never_scheduled));
            }
        }
    }

    // The sweep must SEE the known declarations, or its parsing has rotted
    // into a vacuous pass. d49's overlap badge and d56's retired badges are
    // literal declarations today; d53's and d60's are contract aliases (no
    // literal), pinned by the compiler instead.
    for (expected_name, expected_value) in [
        ("STATUS_LOCATION_CANDIDATE_UNVALIDATED", 1u32 << 11),
        ("STATUS_MISSION_FIXTURE_REQUIRED", 1 << 16),
        ("STATUS_HOME_BASE_FIXTURE_REQUIRED", 1 << 17),
        ("STATUS_REPAIR_RETREAT_FIXTURE_REQUIRED", 1 << 18),
        ("STATUS_CONVOY_CONTEXT_FIXTURE_REQUIRED", 1 << 19),
    ] {
        assert!(
            lane_local
                .iter()
                .any(|(f, _, _)| f.name == expected_name && f.value == expected_value),
            "sweep self-check: expected to find {expected_name} = {expected_value:#x}; \
             the declaration moved or the parser broke"
        );
    }

    for (finding, candidates, never_scheduled) in &lane_local {
        if *never_scheduled {
            continue;
        }
        // A multi-domain (shared) module is ambiguous by construction: the
        // declaration must be explicitly bound to its owning domain, and all
        // checks then run against that domain alone.
        let owners: Vec<u16> = if candidates.len() > 1 {
            match MULTI_DOMAIN_BINDINGS
                .iter()
                .find(|(name, _)| *name == finding.name)
            {
                Some((_, domain)) if candidates.contains(domain) => vec![*domain],
                Some((_, domain)) => {
                    violations.push(format!(
                        "{}:{}: {} is bound to domain {domain} in \
                         MULTI_DOMAIN_BINDINGS, but that id is not among this \
                         file's candidates {candidates:?}",
                        finding.file.display(),
                        finding.line_no,
                        finding.name
                    ));
                    continue;
                }
                None => {
                    violations.push(format!(
                        "{}:{}: {} is declared in a file with multiple domain \
                         candidates {candidates:?}; add an explicit \
                         MULTI_DOMAIN_BINDINGS entry naming its owning domain",
                        finding.file.display(),
                        finding.line_no,
                        finding.name
                    ));
                    continue;
                }
            }
        } else {
            candidates.iter().copied().collect()
        };
        let bits = (0..32).filter(|bit| finding.value & (1 << bit) != 0);
        for bit in bits {
            let registered = owners
                .iter()
                .any(|domain| lane_local_status_mask(*domain) & (1 << bit) != 0);
            let allowlisted = NEVER_EMITTED.iter().any(|entry| {
                entry.bits.contains(&bit) && owners.contains(&entry.domain_id)
            });
            if registered || allowlisted {
                continue;
            }
            violations.push(format!(
                "{}:{}: {} declares status bit {bit} (value {:#x}) for domain(s) \
                 {owners:?}, but the bit is neither registered in \
                 `compact_contract::lane_local_status_mask` nor allowlisted as \
                 provably-never-emitted. Register it (mirror the DOMAIN53 pattern) \
                 or add a NEVER_EMITTED entry with a source marker proving the \
                 production path rejects it.",
                finding.file.display(),
                finding.line_no,
                finding.name,
                finding.value
            ));
        }
    }

    // Re-verify every never-emitted claim against its source marker.
    for entry in NEVER_EMITTED {
        let marker_file = files
            .iter()
            .find(|file| {
                file.to_string_lossy()
                    .replace('\\', "/")
                    .ends_with(entry.marker_file_suffix)
            })
            .unwrap_or_else(|| {
                panic!(
                    "NEVER_EMITTED marker file …/{} not found in lanes sweep",
                    entry.marker_file_suffix
                )
            });
        let text = fs::read_to_string(marker_file)
            .unwrap_or_else(|e| panic!("reading {}: {e}", marker_file.display()));
        assert!(
            text.contains(entry.marker),
            "domain {} bits {:?} are allowlisted as never-emitted, but the \
             retirement marker is gone from {} — re-triage: either the bits are \
             emittable again (register them) or update the marker",
            entry.domain_id,
            entry.bits,
            marker_file.display()
        );
        // An allowlisted bit must not ALSO be registered — the two treatments
        // are mutually exclusive by definition.
        for bit in entry.bits {
            assert_eq!(
                lane_local_status_mask(entry.domain_id) & (1 << bit),
                0,
                "domain {} bit {bit} is both registered and allowlisted",
                entry.domain_id
            );
        }
    }

    assert!(
        violations.is_empty(),
        "unregistered lane-local status bits:\n{}",
        violations.join("\n")
    );
}
