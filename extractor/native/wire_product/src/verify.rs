use crate::catalog::decode_artifact as decode_province_catalog_artifact;
use crate::codec::{decode_branches, decode_bundle_metadata, decode_schemas};
use crate::model::{
    ArtifactRole, BranchSpec, BundleBuildConfig, BundleEntry, BundleEntryDescriptor, D81LeadingGap,
    H4crSession, PreparedBundle, ProductError, VerifiedBranch, VerifiedBundle, BRANCHES_PATH,
    BUNDLE_PATH, PROVINCES_PATH, SCHEMAS_PATH, SYMBOLS_PATH,
};
use crate::AuthenticatedProvinceCatalog;
use portable_content_manifest::symbol_manifest::SymbolManifest;
use std::collections::{BTreeMap, BTreeSet};
use wire_format::body::{
    apply_shaped, decode_shaped_baseline, decode_shaped_delta, CollectionState,
};
use wire_format::d81;
use wire_format::frame::{read_block, Kind};
use wire_format::gap::{decode_record as decode_gap, GapPart, GapRecord};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Coverage {
    start: u64,
    end: u64,
    is_gap: bool,
}

struct ComparingSink<'a, F> {
    actual: &'a [BundleEntryDescriptor],
    read: &'a mut F,
    cursors: [RoleCursor; 7],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RoleCursor {
    next: usize,
    end: usize,
}

impl<'a, F> ComparingSink<'a, F> {
    fn new(actual: &'a [BundleEntryDescriptor], read: &'a mut F) -> Result<Self, ProductError> {
        let mut cursors = [RoleCursor::default(); 7];
        let mut seen = [false; 7];
        for (index, descriptor) in actual.iter().enumerate() {
            let role = descriptor.role as usize;
            if !seen[role] {
                cursors[role] = RoleCursor {
                    next: index,
                    end: index + 1,
                };
                seen[role] = true;
            } else if cursors[role].end == index {
                cursors[role].end = index + 1;
            } else {
                return Err(ProductError::new(format!(
                    "packed {} descriptors are not contiguous in canonical path order",
                    descriptor.role.as_str()
                )));
            }
        }
        Ok(Self {
            actual,
            read,
            cursors,
        })
    }

    fn finish(&self) -> Result<(), ProductError> {
        if self.cursors.iter().any(|cursor| cursor.next != cursor.end) {
            return Err(ProductError::new(
                "packed rebuilt-wire descriptor set has artifacts not emitted by the independent H4CR refold",
            ));
        }
        Ok(())
    }
}

impl<F> crate::build::ArtifactSink for ComparingSink<'_, F>
where
    F: FnMut(&str) -> Result<Vec<u8>, String>,
{
    fn push(&mut self, path: String, role: ArtifactRole, bytes: &[u8]) -> Result<(), ProductError> {
        let expected = BundleEntryDescriptor {
            path,
            role,
            len: u64::try_from(bytes.len())
                .map_err(|_| ProductError::new("rebuilt artifact length exceeds u64"))?,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        let cursor = &mut self.cursors[role as usize];
        if cursor.next >= cursor.end {
            return Err(ProductError::new(format!(
                "independent H4CR refold emitted unexpected artifact {}",
                expected.path
            )));
        }
        let actual_descriptor = &self.actual[cursor.next];
        if actual_descriptor != &expected {
            return Err(ProductError::new(format!(
                "packed descriptor {} differs from independently emitted {}",
                actual_descriptor.path, expected.path
            )));
        }
        let actual_bytes = read_verified(actual_descriptor, self.read)?;
        if actual_bytes != bytes {
            return Err(ProductError::new(format!(
                "packed artifact {} differs from independent H4CR refold",
                expected.path
            )));
        }
        cursor.next += 1;
        Ok(())
    }

    fn descriptors(&self) -> Vec<BundleEntryDescriptor> {
        self.actual
            .iter()
            .filter(|entry| entry.role != ArtifactRole::BundleMetadata)
            .cloned()
            .collect()
    }
}

pub fn verify_prepared_bundle(bundle: &PreparedBundle) -> Result<VerifiedBundle, ProductError> {
    let mut descriptors = bundle
        .entries
        .iter()
        .map(BundleEntryDescriptor::from)
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let files = bundle
        .entries
        .iter()
        .map(|entry| (entry.path.clone(), entry.file_path.clone()))
        .collect::<BTreeMap<_, _>>();
    verify_entry_source(&descriptors, |path| {
        let file = files
            .get(path)
            .ok_or_else(|| format!("prepared artifact {path} has no retained file"))?;
        std::fs::read(file)
            .map_err(|error| format!("read prepared artifact {}: {error}", file.display()))
    })
}

pub fn verify_entry_source_against_h4cr<F>(
    descriptors: &[BundleEntryDescriptor],
    mut read: F,
    config: BundleBuildConfig,
    branches: &[BranchSpec],
    catalog: &AuthenticatedProvinceCatalog,
    sessions: &[H4crSession<'_>],
    leading_gaps: &[D81LeadingGap],
) -> Result<VerifiedBundle, ProductError>
where
    F: FnMut(&str) -> Result<Vec<u8>, String>,
{
    let verified = verify_entry_source(descriptors, &mut read)?;
    let mut sink = ComparingSink::new(descriptors, &mut read)?;
    crate::build::build_d81_into(config, branches, catalog, sessions, leading_gaps, &mut sink)?;
    sink.finish()?;
    Ok(verified)
}

pub fn verify_entries(entries: &[BundleEntry]) -> Result<VerifiedBundle, ProductError> {
    for entry in entries {
        entry.validate_declared_integrity()?;
    }
    let mut descriptors = entries
        .iter()
        .map(BundleEntryDescriptor::from)
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let bytes = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry.bytes.as_slice()))
        .collect::<BTreeMap<_, _>>();
    verify_entry_source(&descriptors, |path| {
        bytes
            .get(path)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| format!("rebuilt-wire entry {path} disappeared"))
    })
}

pub fn verify_entry_source<F>(
    entries: &[BundleEntryDescriptor],
    mut read: F,
) -> Result<VerifiedBundle, ProductError>
where
    F: FnMut(&str) -> Result<Vec<u8>, String>,
{
    if entries.is_empty() {
        return Err(ProductError::new("rebuilt-wire entry set is empty"));
    }
    for entry in entries {
        crate::model::validate_entry_path(&entry.path)?;
    }
    if entries
        .windows(2)
        .any(|pair| pair[0].path.as_bytes() >= pair[1].path.as_bytes())
    {
        return Err(ProductError::new(format!(
            "rebuilt-wire entry descriptors are not strictly bytewise sorted"
        )));
    }
    let bundle_entry = require_entry(entries, BUNDLE_PATH, ArtifactRole::BundleMetadata)?;
    let bundle_bytes = read_verified(bundle_entry, &mut read)?;
    let metadata = decode_bundle_metadata(&bundle_bytes)?;
    if metadata.artifacts.len() + 1 != entries.len() {
        return Err(ProductError::new(
            "bundle metadata does not describe the exact packed entry cardinality",
        ));
    }
    let actual_artifacts = entries
        .iter()
        .filter(|entry| entry.path != BUNDLE_PATH)
        .collect::<Vec<_>>();
    for (descriptor, actual) in metadata.artifacts.iter().zip(actual_artifacts) {
        if actual.path != descriptor.path {
            return Err(ProductError::new(format!(
                "bundle metadata path {} differs from packed artifact {}",
                descriptor.path, actual.path
            )));
        }
        if actual.role != descriptor.role
            || actual.len != descriptor.len
            || actual.blake3 != descriptor.blake3
        {
            return Err(ProductError::new(format!(
                "packed artifact {} does not match authenticated descriptor",
                descriptor.path
            )));
        }
    }
    let branch_entry = require_entry(entries, BRANCHES_PATH, ArtifactRole::BranchTable)?;
    let branches = decode_branches(&read_verified(branch_entry, &mut read)?)?;
    let branch_by_id = branches
        .iter()
        .map(|branch| (branch.branch, branch))
        .collect::<BTreeMap<_, _>>();
    let schema_entry = require_entry(entries, SCHEMAS_PATH, ArtifactRole::SchemaRegistry)?;
    let schemas = decode_schemas(&read_verified(schema_entry, &mut read)?)?;
    if schemas.len() != 1
        || schemas[0].domain != d81::DOMAIN
        || schemas[0].schema_hash != d81::schema().schema_hash()
        || schemas[0].canonical_bytes != d81::schema().canonical_bytes()
    {
        return Err(ProductError::new(
            "schema registry is not the exact canonical d81 production declaration",
        ));
    }
    let symbol_entry = require_entry(entries, SYMBOLS_PATH, ArtifactRole::SymbolManifest)?;
    let symbol_bytes = read_verified(symbol_entry, &mut read)?;
    let symbols = SymbolManifest::decode_json(&symbol_bytes)
        .map_err(|error| ProductError::new(format!("symbol manifest rejected: {error:?}")))?;
    if symbols
        .batches
        .iter()
        .any(|batch| !batch.available_symbols.is_empty())
        || !symbols.overflow_symbols.is_empty()
    {
        return Err(ProductError::new(
            "d81 production symbols must be authenticated file-global base symbols",
        ));
    }
    for symbol in &symbols.base_symbols {
        if symbol.starts_with("province/rgb/") {
            validate_province_symbol(symbol)?;
        } else {
            validate_country_symbol(symbol)?;
        }
    }
    let province_entry = require_entry(entries, PROVINCES_PATH, ArtifactRole::ProvinceCatalog)?;
    let province_bytes = read_verified(province_entry, &mut read)?;
    let province_catalog = decode_province_catalog_artifact(&province_bytes)?;
    if province_catalog.content_manifest_sha256 != metadata.content_manifest_sha256
        || province_catalog.definition_csv_sha256 != metadata.definition_csv_sha256
    {
        return Err(ProductError::new(
            "province catalog provenance does not match bundle metadata",
        ));
    }
    let catalog_symbols = province_catalog
        .bindings
        .iter()
        .map(|binding| binding.symbol.as_str())
        .collect::<BTreeSet<_>>();
    let manifest_symbols = symbols
        .base_symbols
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if !catalog_symbols.is_subset(&manifest_symbols)
        || manifest_symbols
            .difference(&catalog_symbols)
            .any(|symbol| !symbol.starts_with("country/tag/"))
    {
        return Err(ProductError::new(
            "symbol manifest is not the exact province catalog plus authenticated D18 country vocabulary",
        ));
    }

    let mut coverages: BTreeMap<u32, Vec<Coverage>> = BTreeMap::new();
    let mut state_counts: BTreeMap<u32, u64> = BTreeMap::new();
    let mut block_counts: BTreeMap<u32, u64> = BTreeMap::new();
    let mut block_indices: BTreeMap<u32, Vec<u64>> = BTreeMap::new();
    let mut gap_indices: BTreeMap<u32, Vec<u64>> = BTreeMap::new();
    let mut gaps_by_branch: BTreeMap<u32, Vec<GapRecord>> = BTreeMap::new();

    for entry in entries {
        match entry.role {
            ArtifactRole::StateBlock => {
                let (path_branch, path_index) =
                    parse_indexed_path(&entry.path, "rebuilt_wire/blocks/d81-b")?;
                if !branch_by_id.contains_key(&path_branch) {
                    return Err(ProductError::new(format!(
                        "block names unknown branch {path_branch}"
                    )));
                }
                let entry_bytes = read_verified(entry, &mut read)?;
                let (header, frames) = read_block(&entry_bytes, 1).map_err(|error| {
                    ProductError::new(format!("reopen block {}: {error:?}", entry.path))
                })?;
                if header.domain != d81::DOMAIN
                    || header.branch != path_branch
                    || header.schema_hash != schemas[0].schema_hash
                    || header.first_tick != frames[0].tick
                {
                    return Err(ProductError::new(format!(
                        "block {} header does not match its authenticated identity",
                        entry.path
                    )));
                }
                if frames[0].kind != Kind::Baseline
                    || frames
                        .iter()
                        .skip(1)
                        .any(|frame| frame.kind == Kind::Baseline)
                {
                    return Err(ProductError::new(format!(
                        "block {} does not have exactly one opening baseline",
                        entry.path
                    )));
                }
                let mut state: Option<Vec<CollectionState>> = None;
                for frame in frames {
                    match frame.kind {
                        Kind::Baseline => {
                            let (decoded, end) =
                                decode_shaped_baseline(&d81::schema(), &frame.body, 0).map_err(
                                    |error| {
                                        ProductError::new(format!(
                                            "decode baseline in {}: {error:?}",
                                            entry.path
                                        ))
                                    },
                                )?;
                            if end != frame.body.len() {
                                return Err(ProductError::new(format!(
                                    "baseline in {} has trailing bytes",
                                    entry.path
                                )));
                            }
                            validate_d81_state(&decoded, &symbols)?;
                            state = Some(decoded);
                            push_coverage(
                                &mut coverages,
                                path_branch,
                                frame.tick,
                                frame
                                    .tick
                                    .checked_add(1)
                                    .ok_or_else(|| ProductError::new("baseline tick overflows"))?,
                                false,
                            );
                            add_count(&mut state_counts, path_branch, 1)?;
                        }
                        Kind::Delta => {
                            let prior = state.as_mut().ok_or_else(|| {
                                ProductError::new("delta reopened before baseline state")
                            })?;
                            let (deltas, end) =
                                decode_shaped_delta(&d81::schema(), prior, &frame.body, 0)
                                    .map_err(|error| {
                                        ProductError::new(format!(
                                            "decode delta in {}: {error:?}",
                                            entry.path
                                        ))
                                    })?;
                            if end != frame.body.len() {
                                return Err(ProductError::new(format!(
                                    "delta in {} has trailing bytes",
                                    entry.path
                                )));
                            }
                            for (collection, delta) in prior.iter_mut().zip(&deltas) {
                                if let Some(delta) = delta {
                                    apply_shaped(collection, delta).map_err(|error| {
                                        ProductError::new(format!(
                                            "apply delta in {}: {error:?}",
                                            entry.path
                                        ))
                                    })?;
                                }
                            }
                            validate_d81_state(prior, &symbols)?;
                            push_coverage(
                                &mut coverages,
                                path_branch,
                                frame.tick,
                                frame
                                    .tick
                                    .checked_add(1)
                                    .ok_or_else(|| ProductError::new("delta tick overflows"))?,
                                false,
                            );
                            add_count(&mut state_counts, path_branch, 1)?;
                        }
                        Kind::Unchanged => {
                            if state.is_none() || !frame.body.is_empty() {
                                return Err(ProductError::new(format!(
                                    "UNCHANGED in {} has no prior or carries a body",
                                    entry.path
                                )));
                            }
                            let end = frame
                                .tick
                                .checked_add(frame.run_len)
                                .ok_or_else(|| ProductError::new("UNCHANGED range overflows"))?;
                            push_coverage(&mut coverages, path_branch, frame.tick, end, false);
                            add_count(&mut state_counts, path_branch, frame.run_len)?;
                        }
                        _ => {
                            return Err(ProductError::new(format!(
                                "state block {} carries a non-state or reserved frame",
                                entry.path
                            )))
                        }
                    }
                }
                add_count(&mut block_counts, path_branch, 1)?;
                block_indices
                    .entry(path_branch)
                    .or_default()
                    .push(path_index);
            }
            ArtifactRole::GapRecord => {
                let (path_branch, path_index) =
                    parse_indexed_path(&entry.path, "rebuilt_wire/gaps/d81-b")?;
                let entry_bytes = read_verified(entry, &mut read)?;
                let gap = decode_gap(&entry_bytes).map_err(|error| {
                    ProductError::new(format!("reopen gap {}: {error:?}", entry.path))
                })?;
                if gap.branch != path_branch
                    || gap.domains != [d81::DOMAIN]
                    || gap.part != GapPart::State
                    || !branch_by_id.contains_key(&gap.branch)
                {
                    return Err(ProductError::new(format!(
                        "gap {} does not name exact d81 state authority",
                        entry.path
                    )));
                }
                validate_d81_gap(&gap)?;
                push_coverage(
                    &mut coverages,
                    gap.branch,
                    gap.tick_start,
                    gap.tick_end,
                    true,
                );
                gap_indices.entry(path_branch).or_default().push(path_index);
                gaps_by_branch.entry(gap.branch).or_default().push(gap);
            }
            ArtifactRole::BundleMetadata
            | ArtifactRole::BranchTable
            | ArtifactRole::SchemaRegistry
            | ArtifactRole::SymbolManifest
            | ArtifactRole::ProvinceCatalog => {
                let canonical = match entry.role {
                    ArtifactRole::BundleMetadata => BUNDLE_PATH,
                    ArtifactRole::BranchTable => BRANCHES_PATH,
                    ArtifactRole::SchemaRegistry => SCHEMAS_PATH,
                    ArtifactRole::SymbolManifest => SYMBOLS_PATH,
                    ArtifactRole::ProvinceCatalog => PROVINCES_PATH,
                    _ => unreachable!(),
                };
                if entry.path != canonical {
                    return Err(ProductError::new(format!(
                        "singleton role {} appears at noncanonical path {}",
                        entry.role.as_str(),
                        entry.path
                    )));
                }
            }
        }
    }
    require_contiguous_indices(&block_indices, "block")?;
    require_contiguous_indices(&gap_indices, "gap")?;
    for gaps in gaps_by_branch.values() {
        let normalised = wire_format::gap::normalise(gaps)
            .map_err(|error| ProductError::new(format!("normalise reopened gaps: {error:?}")))?;
        if normalised.as_slice() != gaps.as_slice() {
            return Err(ProductError::new(
                "per-gap artifacts are not in canonical branch-local normalised order",
            ));
        }
    }

    let coverage_snapshot = coverages.clone();
    let mut verified_branches = Vec::with_capacity(branches.len());
    for branch in &branches {
        let mut ranges = coverages.remove(&branch.branch).ok_or_else(|| {
            ProductError::new(format!("branch {} has no d81 coverage", branch.branch))
        })?;
        ranges.sort();
        if ranges[0].start != branch.first_tick {
            return Err(ProductError::new(format!(
                "branch {} coverage starts at {}, expected {}",
                branch.branch, ranges[0].start, branch.first_tick
            )));
        }
        let mut cursor = ranges[0].start;
        let mut gap_hours = 0u64;
        for range in ranges {
            if range.start != cursor || range.start >= range.end {
                return Err(ProductError::new(format!(
                    "branch {} has overlapping, empty, or silent d81 coverage at {}..{} after {}",
                    branch.branch, range.start, range.end, cursor
                )));
            }
            if range.is_gap {
                gap_hours = gap_hours
                    .checked_add(range.end - range.start)
                    .ok_or_else(|| ProductError::new("gap hour count overflows"))?;
            }
            cursor = range.end;
        }
        let last_tick = cursor
            .checked_sub(1)
            .ok_or_else(|| ProductError::new("branch coverage endpoint underflows"))?;
        verified_branches.push(VerifiedBranch {
            branch: branch.branch,
            first_tick: branch.first_tick,
            last_tick,
            state_observations: *state_counts.get(&branch.branch).unwrap_or(&0),
            gap_hours,
            blocks: *block_counts.get(&branch.branch).unwrap_or(&0),
        });
    }
    verified_branches.sort_by_key(|branch| branch.branch);
    for branch in &branches {
        let Some(parent) = branch.parent else {
            continue;
        };
        let covered = coverage_snapshot.get(&parent.branch).is_some_and(|ranges| {
            ranges
                .iter()
                .any(|range| range.start <= parent.through_tick && parent.through_tick < range.end)
        });
        if !covered {
            return Err(ProductError::new(format!(
                "branch {} inherits through tick {} not authoritatively covered by parent {}",
                branch.branch, parent.through_tick, parent.branch
            )));
        }
    }

    Ok(VerifiedBundle {
        ticks_per_hour_log2: metadata.ticks_per_hour_log2,
        content_manifest_sha256: metadata.content_manifest_sha256,
        definition_csv_sha256: metadata.definition_csv_sha256,
        symbol_count: symbols.len() as u64,
        schema_count: schemas.len() as u64,
        branches: verified_branches,
    })
}

fn require_entry<'a>(
    entries: &'a [BundleEntryDescriptor],
    path: &str,
    role: ArtifactRole,
) -> Result<&'a BundleEntryDescriptor, ProductError> {
    let entry = entries
        .iter()
        .find(|entry| entry.path == path)
        .ok_or_else(|| ProductError::new(format!("required artifact {path} is missing")))?;
    if entry.role != role {
        return Err(ProductError::new(format!(
            "artifact {path} has role {}, expected {}",
            entry.role.as_str(),
            role.as_str()
        )));
    }
    Ok(entry)
}

fn read_verified<F>(entry: &BundleEntryDescriptor, read: &mut F) -> Result<Vec<u8>, ProductError>
where
    F: FnMut(&str) -> Result<Vec<u8>, String>,
{
    let bytes = read(&entry.path).map_err(|error| {
        ProductError::new(format!(
            "read rebuilt-wire artifact {}: {error}",
            entry.path
        ))
    })?;
    let len = u64::try_from(bytes.len())
        .map_err(|_| ProductError::new("rebuilt-wire artifact length exceeds u64"))?;
    let digest = *blake3::hash(&bytes).as_bytes();
    if len != entry.len || digest != entry.blake3 {
        return Err(ProductError::new(format!(
            "rebuilt-wire artifact {} changed after its descriptor was authenticated",
            entry.path
        )));
    }
    Ok(bytes)
}

fn parse_indexed_path(path: &str, prefix: &str) -> Result<(u32, u64), ProductError> {
    let rest = path
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(".bin"))
        .ok_or_else(|| ProductError::new(format!("noncanonical indexed artifact path {path}")))?;
    let (branch, index) = rest.rsplit_once('-').ok_or_else(|| {
        ProductError::new(format!(
            "indexed artifact path {path} has no index separator"
        ))
    })?;
    if branch.is_empty()
        || index.len() != 8
        || !branch.bytes().all(|byte| byte.is_ascii_digit())
        || !index.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ProductError::new(format!(
            "indexed artifact path {path} is not canonical decimal"
        )));
    }
    let branch = branch
        .parse::<u32>()
        .map_err(|_| ProductError::new(format!("branch in {path} exceeds u32")))?;
    let index = index
        .parse::<u64>()
        .map_err(|_| ProductError::new(format!("index in {path} exceeds u64")))?;
    if branch.to_string() != rest.rsplit_once('-').expect("separator checked").0
        || format!("{index:08}") != rest.rsplit_once('-').expect("separator checked").1
    {
        return Err(ProductError::new(format!(
            "indexed artifact path {path} is not canonical decimal"
        )));
    }
    Ok((branch, index))
}

fn validate_province_symbol(symbol: &str) -> Result<(), ProductError> {
    let components = symbol
        .strip_prefix("province/rgb/")
        .ok_or_else(|| ProductError::new(format!("non-province base symbol {symbol:?}")))?
        .split('/')
        .collect::<Vec<_>>();
    if components.len() != 3 {
        return Err(ProductError::new(format!(
            "province symbol {symbol:?} has wrong RGB arity"
        )));
    }
    for component in components {
        let value = component.parse::<u16>().map_err(|_| {
            ProductError::new(format!("province symbol {symbol:?} has nonnumeric RGB"))
        })?;
        if value > 255 || component != value.to_string() {
            return Err(ProductError::new(format!(
                "province symbol {symbol:?} has noncanonical RGB"
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_d81_gap(gap: &GapRecord) -> Result<(), ProductError> {
    use wire_format::gap::{GapReason, GapWitness};
    match (&gap.reason, &gap.witness) {
        (
            GapReason::TornSweep,
            GapWitness::TickBracket {
                start_tick,
                end_tick,
            },
        ) if start_tick != end_tick => Ok(()),
        (
            GapReason::Backpressure,
            GapWitness::Counter {
                counter_id,
                observed,
            },
        ) if (*counter_id
            == u64::from(compact_contract::gap_record::GAP_COUNTER_CODE_QUEUE_OVERRUN_HOURS)
            || *counter_id
                == u64::from(
                    compact_contract::gap_record::GAP_COUNTER_CODE_SINK_BACKPRESSURE_HOURS,
                ))
            && gap.tick_end.checked_sub(gap.tick_start) == Some(*observed) =>
        {
            Ok(())
        }
        (
            GapReason::Skipped | GapReason::Coalesced,
            GapWitness::TickBracket {
                start_tick,
                end_tick,
            },
        ) if *start_tick == gap.tick_start && *end_tick == gap.tick_end => Ok(()),
        (GapReason::CoverageShortfall, GapWitness::Coverage { observed, expected })
            if *observed == 0 && gap.tick_end.checked_sub(gap.tick_start) == Some(*expected) =>
        {
            Ok(())
        }
        (
            reason @ (GapReason::SourceUnavailable
            | GapReason::PartialData
            | GapReason::BridgeUnavailable
            | GapReason::WorkerPanic
            | GapReason::CoverageShortfall),
            GapWitness::StatusBits {
                bit_width: 32,
                bits,
            },
        ) if bits.len() == 4 => {
            let status = u32::from_le_bytes(bits.as_slice().try_into().unwrap());
            if compact_contract::compact_status_unknown_flags_v1(status) != 0
                || !compact_contract::compact_status_is_failure(status)
            {
                return Err(ProductError::new(
                    "d81 status gap does not carry a known failure status",
                ));
            }
            let expected = if status & compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE != 0 {
                GapReason::SourceUnavailable
            } else if status & compact_contract::COMPACT_STATUS_PARTIAL_DATA != 0 {
                GapReason::PartialData
            } else if status & compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE != 0 {
                GapReason::BridgeUnavailable
            } else if status & compact_contract::COMPACT_STATUS_WORKER_PANIC != 0 {
                GapReason::WorkerPanic
            } else {
                GapReason::CoverageShortfall
            };
            if *reason != expected {
                return Err(ProductError::new(
                    "d81 status gap reason does not match its status witness",
                ));
            }
            Ok(())
        }
        _ => Err(ProductError::new(
            "d81 gap reason and witness are not a production-emitted pair",
        )),
    }
}

fn validate_d81_state(
    states: &[CollectionState],
    symbols: &SymbolManifest,
) -> Result<(), ProductError> {
    if states.len() != d81::COLLECTION_COUNT {
        return Err(ProductError::new("d81 collection-state arity is not four"));
    }
    let expected_shapes = [
        wire_format::Shape::KeyedSet,
        wire_format::Shape::KeyedSet,
        wire_format::Shape::OrderedKeyedList,
        wire_format::Shape::KeyedSet,
    ];
    for ((state, collection), expected_shape) in states
        .iter()
        .zip(d81::schema().collections)
        .zip(expected_shapes)
    {
        if collection.shape != expected_shape {
            return Err(ProductError::new("canonical d81 collection shape drifted"));
        }
        let rows: Vec<(&wire_format::body::Key, &wire_format::Record)> = match state {
            CollectionState::Keyed(rows) => rows.iter().collect(),
            CollectionState::OrderedKeyed(rows) => {
                rows.iter().map(|row| (&row.key, &row.record)).collect()
            }
            CollectionState::OrderedOccurrence(_) => {
                return Err(ProductError::new(
                    "d81 state contains an occurrence collection",
                ))
            }
        };
        for (key, record) in rows {
            for (field, component) in collection.key_fields().into_iter().zip(&key.0) {
                if field.kind == wire_format::Kind::ContentId {
                    let raw = u64::try_from(*component)
                        .map_err(|_| ProductError::new("d81 content key is outside u64"))?;
                    validate_d81_content_symbol(symbols, raw, field.field_id, "key")?;
                }
            }
            for (field, value) in collection.value_fields().zip(&record.values) {
                if field.kind == wire_format::Kind::ContentId {
                    let wire_format::Value::ContentId(raw) = value else {
                        return Err(ProductError::new(
                            "d81 ContentId record field has a non-ContentId value",
                        ));
                    };
                    validate_d81_content_symbol(symbols, *raw, field.field_id, "record value")?;
                }
            }
        }
    }
    Ok(())
}

fn validate_d81_content_symbol(
    symbols: &SymbolManifest,
    raw: u64,
    field_id: u32,
    location: &str,
) -> Result<(), ProductError> {
    let id = symbols.require_raw_id(raw).map_err(|error| {
        ProductError::new(format!(
            "d81 content {location} does not resolve in symbol manifest: {error:?}"
        ))
    })?;
    let symbol = symbols.symbol(id).ok_or_else(|| {
        ProductError::new(format!("d81 content {location} resolves to no symbol"))
    })?;
    let country = matches!(
        field_id,
        d81::field::POV_COUNTRY_ID | d81::field::PROVINCE_OWNER_COUNTRY_ID
    );
    if country {
        validate_country_symbol(symbol)
    } else {
        validate_province_symbol(symbol)
    }
}

fn validate_country_symbol(symbol: &str) -> Result<(), ProductError> {
    let tag = symbol.strip_prefix("country/tag/").ok_or_else(|| {
        ProductError::new(format!(
            "D18 country symbol is not country/tag/TAG: {symbol:?}"
        ))
    })?;
    if tag.len() != 3
        || !tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return Err(ProductError::new(format!(
            "D18 country symbol has invalid tag: {symbol:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod d81_content_symbol_tests {
    use super::*;
    use portable_content_manifest::symbol_manifest::{
        ContentIdSource, SymbolManifestBuilder, SymbolObservation,
    };
    use wire_format::body::Row;
    use wire_format::d81::FoldOutcome;

    #[test]
    fn owner_record_value_must_resolve_to_a_d18_country_symbol() {
        let mut symbols = SymbolManifestBuilder::from_base([
            "country/tag/GER",
            "country/tag/FRA",
            "province/rgb/1/2/3",
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
            ])
            .unwrap();
        let payload = compact_contract::rail_replay::encode_payload_with_hub_simulation(
            &[compact_contract::rail_replay::RailProvinceState {
                province_id: 1,
                active_terminal: true,
                anchor_variant: 0,
                anchor_bits: [1.0f32.to_bits(), 2.0f32.to_bits(), 3.0f32.to_bits()],
                neighbors: Vec::new(),
            }],
            &[compact_contract::rail_replay::RailHubSimulationState {
                pov_country_id: 7,
                province_id: 1,
                province_owner_country_id: 8,
                subtype: 0,
                route_class: 2,
                node_kind: 3,
                node_flag: 0,
                available_raw: 2,
                consumed_raw: 1,
                foreign_max_raw: 3,
                foreign_route: false,
                fallback_record: false,
            }],
            0,
            0,
            1,
            0,
        )
        .unwrap();
        let snapshot = match d81::fold_payload(&d81::schema(), &payload, 10, &symbols).unwrap() {
            FoldOutcome::Snapshot(snapshot) => snapshot,
            FoldOutcome::Unavailable { status_flags } => {
                panic!("unexpected unavailable status: {status_flags:#x}")
            }
        };
        let mut states = vec![
            CollectionState::Keyed([snapshot.frame].into_iter().collect()),
            CollectionState::Keyed(snapshot.provinces.into_iter().collect()),
            CollectionState::OrderedKeyed(
                snapshot
                    .directed_neighbors
                    .into_iter()
                    .map(|(key, record)| Row { key, record })
                    .collect(),
            ),
            CollectionState::Keyed(snapshot.hub_simulation_rows.into_iter().collect()),
        ];
        validate_d81_state(&states, symbols.manifest()).unwrap();

        let province_content_id = symbols
            .require_content_id(&ContentIdSource::new(d81::PROVINCE_CONTENT_ID_NAMESPACE, 1))
            .unwrap()
            .get();
        let owner_index = d81::schema().collections[d81::HUB_SIMULATION_INDEX]
            .value_fields()
            .position(|field| field.field_id == d81::field::PROVINCE_OWNER_COUNTRY_ID)
            .unwrap();
        let CollectionState::Keyed(rows) = &mut states[d81::HUB_SIMULATION_INDEX] else {
            panic!("hub simulation shape drifted")
        };
        rows.values_mut().next().unwrap().values[owner_index] =
            wire_format::Value::ContentId(province_content_id);
        assert!(validate_d81_state(&states, symbols.manifest())
            .unwrap_err()
            .to_string()
            .contains("not country/tag/TAG"));
    }
}

fn push_coverage(
    coverages: &mut BTreeMap<u32, Vec<Coverage>>,
    branch: u32,
    start: u64,
    end: u64,
    is_gap: bool,
) {
    coverages
        .entry(branch)
        .or_default()
        .push(Coverage { start, end, is_gap });
}

fn add_count(
    counts: &mut BTreeMap<u32, u64>,
    branch: u32,
    amount: u64,
) -> Result<(), ProductError> {
    let next = counts
        .get(&branch)
        .copied()
        .unwrap_or(0)
        .checked_add(amount)
        .ok_or_else(|| ProductError::new("verified branch count overflows"))?;
    counts.insert(branch, next);
    Ok(())
}

fn require_contiguous_indices(
    indices: &BTreeMap<u32, Vec<u64>>,
    label: &str,
) -> Result<(), ProductError> {
    for (branch, values) in indices {
        let mut ordered = values.clone();
        ordered.sort_unstable();
        for (expected, actual) in ordered.into_iter().enumerate() {
            if actual != expected as u64 {
                return Err(ProductError::new(format!(
                    "branch {branch} {label} indices are not contiguous at {actual}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod comparing_sink_tests {
    use super::*;
    use crate::build::ArtifactSink;
    use std::cell::RefCell;

    fn descriptor(path: &str, role: ArtifactRole, bytes: &[u8]) -> BundleEntryDescriptor {
        BundleEntryDescriptor {
            path: path.to_string(),
            role,
            len: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        }
    }

    #[test]
    fn comparing_sink_reads_each_body_at_push_and_retains_only_descriptors() {
        let a_path = "rebuilt_wire/branches.bin";
        let b_path = "rebuilt_wire/schemas.bin";
        let a = vec![1u8; 17];
        let b = vec![2u8; 31];
        let descriptors = vec![
            descriptor(a_path, ArtifactRole::BranchTable, &a),
            descriptor(b_path, ArtifactRole::SchemaRegistry, &b),
        ];
        let bodies = BTreeMap::from([(a_path, a.as_slice()), (b_path, b.as_slice())]);
        let reads = RefCell::new(Vec::new());
        let mut read = |path: &str| {
            reads.borrow_mut().push(path.to_string());
            bodies
                .get(path)
                .map(|bytes| bytes.to_vec())
                .ok_or_else(|| format!("missing test body {path}"))
        };
        let mut sink = ComparingSink::new(&descriptors, &mut read).unwrap();

        sink.push(b_path.to_string(), ArtifactRole::SchemaRegistry, &b)
            .unwrap();
        assert_eq!(*reads.borrow(), vec![b_path]);
        assert_eq!(sink.cursors[ArtifactRole::SchemaRegistry as usize].next, 2);

        sink.push(a_path.to_string(), ArtifactRole::BranchTable, &a)
            .unwrap();
        assert_eq!(*reads.borrow(), vec![b_path, a_path]);
        sink.finish().unwrap();
        assert_eq!(sink.descriptors(), descriptors);
    }

    #[test]
    fn comparing_sink_rejects_same_role_out_of_order_missing_and_extra() {
        let first_path = "rebuilt_wire/blocks/d81-b0-00000000.bin";
        let second_path = "rebuilt_wire/blocks/d81-b0-00000001.bin";
        let first = vec![1u8; 5];
        let second = vec![2u8; 7];
        let descriptors = vec![
            descriptor(first_path, ArtifactRole::StateBlock, &first),
            descriptor(second_path, ArtifactRole::StateBlock, &second),
        ];
        let bodies = BTreeMap::from([
            (first_path, first.as_slice()),
            (second_path, second.as_slice()),
        ]);
        let mut read = |path: &str| {
            bodies
                .get(path)
                .map(|bytes| bytes.to_vec())
                .ok_or_else(|| format!("missing test body {path}"))
        };
        let mut out_of_order = ComparingSink::new(&descriptors, &mut read).unwrap();
        assert!(out_of_order
            .push(second_path.to_string(), ArtifactRole::StateBlock, &second,)
            .unwrap_err()
            .to_string()
            .contains("differs from independently emitted"));

        let mut missing = ComparingSink::new(&descriptors, &mut read).unwrap();
        missing
            .push(first_path.to_string(), ArtifactRole::StateBlock, &first)
            .unwrap();
        assert!(missing.finish().is_err());

        let mut extra = ComparingSink::new(&descriptors[..1], &mut read).unwrap();
        extra
            .push(first_path.to_string(), ArtifactRole::StateBlock, &first)
            .unwrap();
        assert!(extra
            .push(second_path.to_string(), ArtifactRole::StateBlock, &second,)
            .unwrap_err()
            .to_string()
            .contains("unexpected artifact"));
    }
}
