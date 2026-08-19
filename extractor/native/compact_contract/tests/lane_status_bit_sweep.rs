use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use compact_contract::{assigned_descriptor_by_id, lane_local_status_mask, CompactDomainStatus};

fn global_v1_bit_names() -> Vec<String> {
    let mut by_bit: Vec<(u32, String)> = compact_contract::COMPACT_STATUS_V1_FLAG_SPECS
        .iter()
        .map(|spec| {
            assert_eq!(
                spec.bit.count_ones(),
                1,
                "contract v1 flag spec {} is not a single bit ({:#x}); it cannot \
                 have a bit position",
                spec.name,
                spec.bit
            );
            (spec.bit.trailing_zeros(), spec.name.to_ascii_uppercase())
        })
        .collect();
    by_bit.sort_by_key(|(bit, _)| *bit);
    for (index, (bit, name)) in by_bit.iter().enumerate() {
        assert_eq!(
            *bit as usize, index,
            "the assigned v1 vocabulary is no longer a contiguous run from bit 0 \
             ({name} sits at bit {bit}), so indexing this table by bit position \
             is wrong"
        );
    }
    by_bit.into_iter().map(|(_, name)| name).collect()
}

#[test]
fn the_global_bit_name_table_covers_the_whole_assigned_vocabulary() {
    let names = global_v1_bit_names();
    assert_eq!(
        compact_contract::COMPACT_STATUS_V1_KNOWN_MASK.count_ones() as usize,
        names.len(),
        "the contract assigned a status bit the v1 flag specs do not name"
    );
    assert_eq!(
        compact_contract::COMPACT_STATUS_V1_KNOWN_MASK,
        (1u32 << names.len()) - 1,
        "the assigned vocabulary is no longer a contiguous run from bit 0, so \
         indexing this table by bit position is wrong"
    );
}

struct NeverEmitted {
    domain_id: u16,
    bits: &'static [u32],
    marker_file_suffix: &'static str,
    marker: &'static str,
}

const MULTI_DOMAIN_BINDINGS: &[(&str, u16)] = &[("STATUS_LOCATION_CANDIDATE_UNVALIDATED", 49)];

const NEVER_EMITTED: &[NeverEmitted] = &[
    NeverEmitted {
        domain_id: 56,
        bits: &[16, 17, 18, 19],
        marker_file_suffix: "naval_missions_status_live/src/domain56_compact.rs",
        marker: "pub const RETIRED_SOURCE_STATUS_MASK: u32 = STATUS_MISSION_FIXTURE_REQUIRED",
    },
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
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("file type");
        if file_type.is_dir() {
            if entry.file_name() != "target" {
                collect_rs_files(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

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

fn is_global_mirror(global_bit_names: &[String], name: &str, value: u32) -> bool {
    if value.count_ones() != 1 {
        return false;
    }
    let bit = value.trailing_zeros() as usize;
    if bit >= global_bit_names.len() {
        return false;
    }
    name == format!("STATUS_{}", global_bit_names[bit])
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

    let mut lane_local: Vec<(Finding, BTreeSet<u16>, bool)> = Vec::new();
    let mut violations: Vec<String> = Vec::new();
    let global_bit_names = global_v1_bit_names();

    for file in &files {
        let text =
            fs::read_to_string(file).unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
        let mut findings = Vec::new();
        for (idx, line) in text.lines().enumerate() {
            let Some((name, value)) = parse_const_u32_literal(line) else {
                continue;
            };
            if !name.starts_with("STATUS_") || value == 0 {
                continue;
            }
            if is_global_mirror(&global_bit_names, &name, value) {
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
        let never_scheduled = own_domain_id(&text)
            .and_then(assigned_descriptor_by_id)
            .is_some_and(|descriptor| !descriptor.schedulable);
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

    for (expected_name, expected_value) in [
        (
            "STATUS_LOCATION_CANDIDATE_UNVALIDATED",
            compact_contract::DOMAIN49_STATUS_LOCATION_CANDIDATE_UNVALIDATED,
        ),
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
            let allowlisted = NEVER_EMITTED
                .iter()
                .any(|entry| entry.bits.contains(&bit) && owners.contains(&entry.domain_id));
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
