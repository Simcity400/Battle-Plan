use crate::catalog::AuthenticatedProvinceCatalog;
use crate::codec::{
    encode_branches, encode_bundle_metadata_entry_descriptors, encode_schemas, SchemaDefinition,
};
use crate::model::{
    ArtifactRole, BranchSpec, BundleBuildConfig, BundleEntryDescriptor, D81LeadingGap,
    D81LeadingGapKind, D81SessionInspection, H4crSelection, H4crSession, H4crSource,
    PreparedBundle, ProductError, BRANCHES_PATH, BUNDLE_PATH, PROVINCES_PATH, SCHEMAS_PATH,
    SYMBOLS_PATH,
};
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode, StreamValidationError,
};
use compact_contract::{
    parse_compact_record_header, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF,
};
use portable_content_manifest::symbol_manifest::{SymbolManifestBuilder, SymbolObservation};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::path::Path;
use wire_format::body::{
    diff_shaped, encode_shaped_baseline, encode_shaped_delta, CollectionState, Row,
};
use wire_format::d81::{self, DecodedPayload, FoldOutcome};
use wire_format::frame::{BlockHeader, BlockWriter, Frame, Kind};
use wire_format::gap::{encode_record as encode_gap, GapPart, GapReason, GapRecord, GapWitness};

struct ScannedBranch {
    branch: u32,
    decoded_sha256: [u8; 32],
    first_coverage_tick: u64,
    last_closed_tick: u64,
    data_records: u64,
    publication_authoritative_records: u64,
    failure_records: u64,
    /// DATA records whose `PSD1` envelope refused reconstruction. Each one's
    /// hour publishes as a coverage-shortfall gap instead of aborting the build; the
    /// count is the named exclusion the session inspection carries.
    unreconstructed_records: u64,
    first_data_tick: Option<u64>,
    country_bindings: BTreeMap<u32, String>,
}

struct D18CountryCarrier {
    status: u32,
    rows: Option<Vec<(u32, String)>>,
}

enum ScannedD81Item {
    Gap(GapRecord),
    Closed { tick: u64, payload: Vec<u8> },
}

struct PendingBlock {
    branch: u32,
    index: u64,
    first_tick: u64,
    frames: Vec<Frame>,
    prior: Vec<CollectionState>,
}

struct StreamingGapWriter {
    branch: u32,
    next_index: u64,
    pending: Option<GapRecord>,
}

impl StreamingGapWriter {
    fn push(&mut self, gap: GapRecord, sink: &mut dyn ArtifactSink) -> Result<(), ProductError> {
        gap.validate()
            .map_err(|error| ProductError::new(format!("validate d81 gap: {error:?}")))?;
        if gap.branch != self.branch {
            return Err(ProductError::new(format!(
                "d81 gap branch {} differs from stream branch {}",
                gap.branch, self.branch
            )));
        }
        if let Some(pending) = self.pending.as_mut() {
            if gap.tick_start < pending.tick_end {
                return Err(ProductError::new(
                    "d81 streaming gaps overlap or arrive out of tick order",
                ));
            }
            if pending.tick_end == gap.tick_start
                && pending.reason == gap.reason
                && pending.part == gap.part
                && pending.domains == gap.domains
                && pending.witness == gap.witness
            {
                pending.tick_end = gap.tick_end;
                return Ok(());
            }
        }
        self.flush(sink)?;
        self.pending = Some(gap);
        Ok(())
    }

    fn finish(mut self, sink: &mut dyn ArtifactSink) -> Result<(), ProductError> {
        self.flush(sink)
    }

    fn flush(&mut self, sink: &mut dyn ArtifactSink) -> Result<(), ProductError> {
        let Some(mut gap) = self.pending.take() else {
            return Ok(());
        };
        if let GapWitness::Coverage { observed, expected } = &mut gap.witness {
            *observed = 0;
            *expected = gap
                .tick_end
                .checked_sub(gap.tick_start)
                .ok_or_else(|| ProductError::new("d81 gap interval underflows"))?;
        }
        let bytes = encode_gap(&gap)
            .map_err(|error| ProductError::new(format!("encode d81 gap: {error:?}")))?;
        sink.push(
            format!(
                "rebuilt_wire/gaps/d81-b{}-{:08}.bin",
                self.branch, self.next_index
            ),
            ArtifactRole::GapRecord,
            &bytes,
        )?;
        self.next_index = self
            .next_index
            .checked_add(1)
            .ok_or_else(|| ProductError::new("d81 gap artifact index overflows"))?;
        Ok(())
    }
}

pub(crate) trait ArtifactSink {
    fn push(&mut self, path: String, role: ArtifactRole, bytes: &[u8]) -> Result<(), ProductError>;
    fn descriptors(&self) -> Vec<BundleEntryDescriptor>;
}

impl ArtifactSink for PreparedBundle {
    fn push(&mut self, path: String, role: ArtifactRole, bytes: &[u8]) -> Result<(), ProductError> {
        self.push_bytes(path, role, bytes)
    }

    fn descriptors(&self) -> Vec<BundleEntryDescriptor> {
        self.entries
            .iter()
            .map(BundleEntryDescriptor::from)
            .collect()
    }
}

struct BranchStreamEncoder<'a> {
    config: BundleBuildConfig,
    schema: &'a wire_format::Schema,
    symbols: &'a SymbolManifestBuilder,
    branch: u32,
    declared_first_tick: u64,
    declared_gaps: Vec<D81LeadingGap>,
    sink: &'a mut dyn ArtifactSink,
    started: bool,
    block: Option<PendingBlock>,
    next_block_index: u64,
    gaps: StreamingGapWriter,
}

impl BranchStreamEncoder<'_> {
    fn observe(&mut self, tick: u64, payload: Vec<u8>) -> Result<(), ProductError> {
        if !self.started {
            self.start(tick)?;
        }
        let outcome = match d81::fold_payload(self.schema, &payload, tick, self.symbols) {
            Ok(outcome) => outcome,
            Err(error) => {
                return Err(ProductError::new(format!(
                    "branch {} d81 payload at tick {tick} rejected during streaming fold: {error}",
                    self.branch
                )))
            }
        };
        match outcome {
            FoldOutcome::Unavailable { status_flags } => {
                close_block(&mut self.block, self.schema, self.sink)?;
                self.gaps
                    .push(status_gap(self.branch, tick, status_flags)?, self.sink)?;
            }
            FoldOutcome::Snapshot(payload) => {
                if self.config.require_complete_hub_simulation
                    && !payload.header.hub_simulation_complete
                {
                    close_block(&mut self.block, self.schema, self.sink)?;
                    self.gaps.push(
                        coverage_gap(
                            self.branch,
                            tick,
                            tick.checked_add(1).ok_or_else(|| {
                                ProductError::new("d81 incomplete-simulation gap tick overflows")
                            })?,
                        ),
                        self.sink,
                    )?;
                } else {
                    let state = payload_states(payload);
                    let starts_new = self.block.as_ref().is_none_or(|pending| {
                        tick.saturating_sub(pending.first_tick) >= self.config.block_hours
                    });
                    if starts_new {
                        close_block(&mut self.block, self.schema, self.sink)?;
                        self.block = Some(start_block(
                            self.schema,
                            self.branch,
                            self.next_block_index,
                            tick,
                            state,
                        )?);
                        self.next_block_index = self
                            .next_block_index
                            .checked_add(1)
                            .ok_or_else(|| ProductError::new("d81 block index overflows"))?;
                    } else {
                        append_state(
                            self.block
                                .as_mut()
                                .expect("block exists after starts_new false"),
                            self.schema,
                            tick,
                            state,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn observe_gap(&mut self, gap: GapRecord) -> Result<(), ProductError> {
        if !self.started {
            self.start(gap.tick_start)?;
        }
        crate::verify::validate_d81_gap(&gap)?;
        if gap.branch != self.branch {
            return Err(ProductError::new(format!(
                "authenticated d81 gap branch {} differs from encoder branch {}",
                gap.branch, self.branch
            )));
        }
        close_block(&mut self.block, self.schema, self.sink)?;
        self.gaps.push(gap, self.sink)
    }

    fn start(&mut self, first_closed: u64) -> Result<(), ProductError> {
        if self.declared_first_tick > first_closed {
            return Err(ProductError::new(
                "branch first tick is later than its selected H4CR coverage",
            ));
        }
        if self.declared_first_tick < first_closed && self.declared_gaps.is_empty() {
            self.gaps.push(
                coverage_gap(self.branch, self.declared_first_tick, first_closed),
                self.sink,
            )?;
        } else if self.declared_first_tick < first_closed {
            self.declared_gaps
                .sort_by_key(|gap| (gap.tick_start(), gap.tick_end(), gap.kind()));
            let mut cursor = self.declared_first_tick;
            for declared in self.declared_gaps.drain(..) {
                if declared.branch() != self.branch
                    || declared.tick_start() != cursor
                    || declared.tick_end() > first_closed
                {
                    return Err(ProductError::new(format!(
                        "branch {} declared leading gaps do not exactly cover {}..{}",
                        self.branch, self.declared_first_tick, first_closed
                    )));
                }
                let duration = declared.tick_end() - declared.tick_start();
                let (reason, witness) = match declared.kind() {
                    D81LeadingGapKind::CaptureFailed => (
                        GapReason::CoverageShortfall,
                        GapWitness::Coverage {
                            observed: 0,
                            expected: duration,
                        },
                    ),
                    D81LeadingGapKind::Coalesced => (
                        GapReason::Coalesced,
                        GapWitness::TickBracket {
                            start_tick: declared.tick_start(),
                            end_tick: declared.tick_end(),
                        },
                    ),
                    D81LeadingGapKind::ExplicitGap => (
                        GapReason::Skipped,
                        GapWitness::TickBracket {
                            start_tick: declared.tick_start(),
                            end_tick: declared.tick_end(),
                        },
                    ),
                };
                self.gaps.push(
                    GapRecord {
                        tick_start: declared.tick_start(),
                        tick_end: declared.tick_end(),
                        branch: self.branch,
                        reason,
                        part: GapPart::State,
                        domains: vec![d81::DOMAIN],
                        witness,
                    },
                    self.sink,
                )?;
                cursor = declared.tick_end();
            }
            if cursor != first_closed {
                return Err(ProductError::new(format!(
                    "branch {} declared leading gaps end at {}, expected {}",
                    self.branch, cursor, first_closed
                )));
            }
        } else if !self.declared_gaps.is_empty() {
            return Err(ProductError::new(format!(
                "branch {} declares leading gaps but selected H4CR begins at branch origin",
                self.branch
            )));
        }
        self.started = true;
        Ok(())
    }

    fn finish(mut self) -> Result<(), ProductError> {
        if !self.started {
            return Err(ProductError::new("d81 branch has no closed hours"));
        }
        close_block(&mut self.block, self.schema, self.sink)?;
        self.gaps.finish(self.sink)
    }
}

fn coverage_gap(branch: u32, start: u64, end: u64) -> GapRecord {
    GapRecord {
        tick_start: start,
        tick_end: end,
        branch,
        reason: GapReason::CoverageShortfall,
        part: GapPart::State,
        domains: vec![d81::DOMAIN],
        witness: GapWitness::Coverage {
            observed: 0,
            expected: end - start,
        },
    }
}

pub fn build_d81_bundle(
    config: BundleBuildConfig,
    branches: &[BranchSpec],
    catalog: &AuthenticatedProvinceCatalog,
    sessions: &[H4crSession<'_>],
) -> Result<PreparedBundle, ProductError> {
    build_d81_bundle_with_leading_gaps(config, branches, catalog, sessions, &[])
}

pub fn build_d81_bundle_with_leading_gaps(
    config: BundleBuildConfig,
    branches: &[BranchSpec],
    catalog: &AuthenticatedProvinceCatalog,
    sessions: &[H4crSession<'_>],
    leading_gaps: &[D81LeadingGap],
) -> Result<PreparedBundle, ProductError> {
    let mut prepared = PreparedBundle::create_spool(std::env::temp_dir().as_path())?;
    build_d81_into(
        config,
        branches,
        catalog,
        sessions,
        leading_gaps,
        &mut prepared,
    )?;
    prepared
        .entries
        .sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    crate::verify_prepared_bundle(&prepared)?;
    Ok(prepared)
}

pub(crate) fn build_d81_into(
    config: BundleBuildConfig,
    branches: &[BranchSpec],
    catalog: &AuthenticatedProvinceCatalog,
    sessions: &[H4crSession<'_>],
    leading_gaps: &[D81LeadingGap],
    sink: &mut dyn ArtifactSink,
) -> Result<(), ProductError> {
    if config.block_hours == 0 || config.block_hours > 4096 {
        return Err(ProductError::new(
            "block_hours must be within the production bound 1..=4096",
        ));
    }
    crate::codec::validate_branches(branches)?;
    if sessions.len() != branches.len() {
        return Err(ProductError::new(
            "exactly one sealed H4CR session is required for each branch",
        ));
    }

    let mut country_bindings = BTreeMap::<u32, String>::new();
    for session in sessions {
        let (reader, selection): (Box<dyn Read + '_>, Option<H4crSelection>) = match session.source
        {
            H4crSource::Bytes(bytes) => (Box::new(Cursor::new(bytes)), None),
            H4crSource::File(path) => (
                Box::new(compact_stream::open(path).map_err(|error| {
                    ProductError::new(format!("open compact stream {}: {error}", path.display()))
                })?),
                None,
            ),
            H4crSource::RetainedFile(file) => (crate::retained_stream::open(file)?, None),
            H4crSource::BytesSelection(bytes, selection) => {
                (Box::new(Cursor::new(bytes)), Some(selection))
            }
            H4crSource::FileSelection(path, selection) => (
                Box::new(compact_stream::open(path).map_err(|error| {
                    ProductError::new(format!("open compact stream {}: {error}", path.display()))
                })?),
                Some(selection),
            ),
            H4crSource::RetainedFileSelection(file, selection) => {
                (crate::retained_stream::open(file)?, Some(selection))
            }
        };
        let scan = scan_h4cr(
            session.branch,
            session.identity_mode,
            selection,
            Some(catalog),
            reader,
            |_| Ok(()),
        )?;
        let branch = branches
            .iter()
            .find(|branch| branch.branch == session.branch)
            .ok_or_else(|| {
                ProductError::new(format!(
                    "H4CR session names unknown branch {}",
                    session.branch
                ))
            })?;
        if scan.decoded_sha256 != branch.session_fingerprint {
            return Err(ProductError::new(format!(
                "branch {} decoded H4CR SHA-256 does not match authenticated provenance",
                scan.branch
            )));
        }
        for (source_id, symbol) in scan.country_bindings {
            if let Some(previous) = country_bindings.insert(source_id, symbol.clone()) {
                if previous != symbol {
                    return Err(ProductError::new(format!(
                        "D18 country source {source_id} is ambiguously bound to {previous} and {symbol}"
                    )));
                }
            }
        }
    }
    let mut source_by_country_symbol = BTreeMap::<String, u32>::new();
    for (source_id, symbol) in &country_bindings {
        if let Some(previous) = source_by_country_symbol.insert(symbol.clone(), *source_id) {
            if previous != *source_id {
                return Err(ProductError::new(format!(
                    "D18 country symbol {symbol} aliases source ids {previous} and {source_id}"
                )));
            }
        }
    }
    let symbol_builder = build_symbol_manifest(catalog, &country_bindings)?;
    let schema = d81::schema();
    schema
        .validate(0)
        .map_err(|error| ProductError::new(format!("canonical d81 schema invalid: {error:?}")))?;
    let schema_registry = encode_schemas(&[SchemaDefinition {
        domain: d81::DOMAIN,
        schema_hash: schema.schema_hash(),
        canonical_bytes: schema.canonical_bytes(),
    }])?;
    let branch_table = encode_branches(branches)?;
    let symbol_manifest = symbol_builder
        .manifest()
        .encode_json()
        .map_err(|error| ProductError::new(format!("encode symbol manifest: {error:?}")))?;
    sink.push(
        BRANCHES_PATH.to_string(),
        ArtifactRole::BranchTable,
        &branch_table,
    )?;
    sink.push(
        SCHEMAS_PATH.to_string(),
        ArtifactRole::SchemaRegistry,
        &schema_registry,
    )?;
    sink.push(
        SYMBOLS_PATH.to_string(),
        ArtifactRole::SymbolManifest,
        &symbol_manifest,
    )?;
    sink.push(
        PROVINCES_PATH.to_string(),
        ArtifactRole::ProvinceCatalog,
        &catalog.encode_artifact()?,
    )?;

    let branch_by_id = branches
        .iter()
        .map(|branch| (branch.branch, branch))
        .collect::<BTreeMap<_, _>>();
    let mut gaps_by_branch: BTreeMap<u32, Vec<D81LeadingGap>> = BTreeMap::new();
    for gap in leading_gaps {
        if !branch_by_id.contains_key(&gap.branch()) {
            return Err(ProductError::new(format!(
                "declared d81 leading gap names unknown branch {}",
                gap.branch()
            )));
        }
        gaps_by_branch.entry(gap.branch()).or_default().push(*gap);
    }
    let mut seen_sessions = BTreeSet::new();
    let mut ordered_sessions = sessions
        .iter()
        .map(|session| (format!("d81-b{}", session.branch), session))
        .collect::<Vec<_>>();
    ordered_sessions.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    for (_, session) in ordered_sessions {
        if !seen_sessions.insert(session.branch) {
            return Err(ProductError::new(format!(
                "branch {} has more than one H4CR session",
                session.branch
            )));
        }
        let (reader, selection): (Box<dyn Read + '_>, Option<H4crSelection>) = match session.source
        {
            H4crSource::Bytes(bytes) => (Box::new(Cursor::new(bytes)), None),
            H4crSource::File(path) => (
                Box::new(compact_stream::open(path).map_err(|error| {
                    ProductError::new(format!("open compact stream {}: {error}", path.display()))
                })?),
                None,
            ),
            H4crSource::RetainedFile(file) => (crate::retained_stream::open(file)?, None),
            H4crSource::BytesSelection(bytes, selection) => {
                (Box::new(Cursor::new(bytes)), Some(selection))
            }
            H4crSource::FileSelection(path, selection) => (
                Box::new(compact_stream::open(path).map_err(|error| {
                    ProductError::new(format!("open compact stream {}: {error}", path.display()))
                })?),
                Some(selection),
            ),
            H4crSource::RetainedFileSelection(file, selection) => {
                (crate::retained_stream::open(file)?, Some(selection))
            }
        };
        let branch = branch_by_id.get(&session.branch).ok_or_else(|| {
            ProductError::new(format!(
                "H4CR session names unknown branch {}",
                session.branch
            ))
        })?;
        let mut encoder = BranchStreamEncoder {
            config,
            schema: &schema,
            symbols: &symbol_builder,
            branch: session.branch,
            declared_first_tick: branch.first_tick,
            declared_gaps: gaps_by_branch.remove(&session.branch).unwrap_or_default(),
            sink,
            started: false,
            block: None,
            next_block_index: 0,
            gaps: StreamingGapWriter {
                branch: session.branch,
                next_index: 0,
                pending: None,
            },
        };
        let scan = scan_h4cr(
            session.branch,
            session.identity_mode,
            selection,
            Some(catalog),
            reader,
            |item| match item {
                ScannedD81Item::Gap(gap) => encoder.observe_gap(gap),
                ScannedD81Item::Closed { tick, payload } => encoder.observe(tick, payload),
            },
        )?;
        if scan.decoded_sha256 != branch.session_fingerprint {
            return Err(ProductError::new(format!(
                "branch {} decoded H4CR SHA-256 does not match authenticated provenance",
                scan.branch
            )));
        }
        let first_tick = scan.first_coverage_tick;
        if first_tick < branch.first_tick {
            return Err(ProductError::new(format!(
                "branch {} selected H4CR starts at {} before branch table {}",
                scan.branch, first_tick, branch.first_tick
            )));
        }
        // A branch whose snapshots were ALL excluded BY NAME still publishes:
        // every lost hour rides as a coverage-shortfall gap in this same bundle and
        // the count is on the session manifest, so refusing here would be the
        // exact "one lost record destroys the publication" defect class this
        // lane closed everywhere else. The refusal stays for a branch that has
        // NEITHER authoritative snapshots NOR named exclusions -- nothing was
        // lost, there is simply nothing certified, which is a stream-shape
        // error and not a degradation.
        if scan.publication_authoritative_records == 0 && scan.unreconstructed_records == 0 {
            return Err(ProductError::new(format!(
                "branch {} contains no publication-authoritative d81 snapshot",
                scan.branch
            )));
        }
        encoder.finish()?;
    }
    if seen_sessions.len() != branch_by_id.len() {
        return Err(ProductError::new(
            "one or more declared branches has no H4CR session",
        ));
    }

    if !gaps_by_branch.is_empty() {
        return Err(ProductError::new(
            "declared d81 leading gaps remain without a branch session",
        ));
    }
    let mut descriptors = sink.descriptors();
    descriptors.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let bundle_metadata = encode_bundle_metadata_entry_descriptors(
        catalog.content_manifest_sha256,
        catalog.definition_csv_sha256,
        &descriptors,
    )?;
    sink.push(
        BUNDLE_PATH.to_string(),
        ArtifactRole::BundleMetadata,
        &bundle_metadata,
    )?;
    Ok(())
}

pub fn build_d81_bundle_from_file(
    config: BundleBuildConfig,
    branch: BranchSpec,
    catalog: &AuthenticatedProvinceCatalog,
    path: &Path,
    identity_mode: StreamIdentityMode,
    materialization_parent: &Path,
) -> Result<crate::model::MaterializedBundle, ProductError> {
    let session = H4crSession {
        branch: branch.branch,
        identity_mode,
        source: H4crSource::File(path),
    };
    build_d81_bundle(config, &[branch], catalog, &[session])?.materialize(materialization_parent)
}

pub fn inspect_d81_session(session: H4crSession<'_>) -> Result<D81SessionInspection, ProductError> {
    inspect_d81_session_inner(session, None)
}

/// Catalog-aware inspection: the province half of the identity-portability
/// oracle runs against the authenticated catalog, so
/// `unreconstructed_records` matches exactly what a bundle build over the
/// same stream will degrade into coverage gaps (Codex review round 28,
/// finding 1). Publication paths MUST use this variant when they copy the
/// count into `d81_unreconstructed_records`.
pub fn inspect_d81_session_with_catalog(
    session: H4crSession<'_>,
    catalog: &AuthenticatedProvinceCatalog,
) -> Result<D81SessionInspection, ProductError> {
    inspect_d81_session_inner(session, Some(catalog))
}

fn inspect_d81_session_inner(
    session: H4crSession<'_>,
    catalog: Option<&AuthenticatedProvinceCatalog>,
) -> Result<D81SessionInspection, ProductError> {
    let (reader, selection): (Box<dyn Read + '_>, Option<H4crSelection>) = match session.source {
        H4crSource::Bytes(bytes) => (Box::new(Cursor::new(bytes)), None),
        H4crSource::File(path) => (
            Box::new(compact_stream::open(path).map_err(|error| {
                ProductError::new(format!("open compact stream {}: {error}", path.display()))
            })?),
            None,
        ),
        H4crSource::RetainedFile(file) => (crate::retained_stream::open(file)?, None),
        H4crSource::BytesSelection(bytes, selection) => {
            (Box::new(Cursor::new(bytes)), Some(selection))
        }
        H4crSource::FileSelection(path, selection) => (
            Box::new(compact_stream::open(path).map_err(|error| {
                ProductError::new(format!("open compact stream {}: {error}", path.display()))
            })?),
            Some(selection),
        ),
        H4crSource::RetainedFileSelection(file, selection) => {
            (crate::retained_stream::open(file)?, Some(selection))
        }
    };
    let scan = scan_h4cr(
        session.branch,
        session.identity_mode,
        selection,
        catalog,
        reader,
        |_| Ok(()),
    )?;
    Ok(D81SessionInspection {
        first_tick: scan.first_coverage_tick,
        first_d81_tick: scan.first_data_tick,
        last_tick: scan.last_closed_tick,
        decoded_sha256: scan.decoded_sha256,
        data_records: scan.data_records,
        publication_authoritative_records: scan.publication_authoritative_records,
        failure_records: scan.failure_records,
        unreconstructed_records: scan.unreconstructed_records,
    })
}

fn build_symbol_manifest(
    catalog: &AuthenticatedProvinceCatalog,
    country_bindings: &BTreeMap<u32, String>,
) -> Result<SymbolManifestBuilder, ProductError> {
    if catalog.bindings.is_empty() {
        return Err(ProductError::new(
            "authenticated province catalog has no bindings",
        ));
    }
    let symbols = catalog
        .bindings
        .iter()
        .map(|binding| binding.symbol.clone())
        .chain(country_bindings.values().cloned())
        .collect::<Vec<_>>();
    let mut builder = SymbolManifestBuilder::from_base(symbols)
        .map_err(|error| ProductError::new(format!("build symbol manifest: {error:?}")))?;
    builder
        .bind_existing_sources(catalog.bindings.iter().map(|binding| {
            SymbolObservation::content_id(
                d81::PROVINCE_CONTENT_ID_NAMESPACE,
                u64::from(binding.province_id),
                binding.symbol.clone(),
            )
        }))
        .map_err(|error| {
            ProductError::new(format!("bind authenticated province symbols: {error:?}"))
        })?;
    builder
        .bind_existing_sources(country_bindings.iter().map(|(source_id, symbol)| {
            SymbolObservation::content_id(
                d81::COUNTRY_CONTENT_ID_NAMESPACE,
                u64::from(*source_id),
                symbol.clone(),
            )
        }))
        .map_err(|error| {
            ProductError::new(format!("bind authenticated D18 country symbols: {error:?}"))
        })?;
    Ok(builder)
}

fn scan_h4cr<F>(
    branch: u32,
    mode: StreamIdentityMode,
    selection: Option<H4crSelection>,
    catalog: Option<&AuthenticatedProvinceCatalog>,
    mut reader: Box<dyn Read + '_>,
    mut on_closed: F,
) -> Result<ScannedBranch, ProductError>
where
    F: FnMut(ScannedD81Item) -> Result<(), ProductError>,
{
    // The AUTHENTICATED province catalog is the only authority for province
    // identities; payload occurrence proves nothing (Codex review round 27,
    // finding 2). A scan WITHOUT a catalog (session inspection) skips the
    // province half of the oracle and keeps its counts comparable.
    let catalog_province_ids: BTreeSet<u32> = catalog
        .map(|catalog| catalog.bindings.iter().map(|b| b.province_id).collect())
        .unwrap_or_default();
    let mut validator = match mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };
    let mut sha256 = Sha256::new();
    let mut header_bytes = [0u8; COMPACT_RECORD_HEADER_LEN];
    let mut first_coverage_tick = None;
    let mut last_closed_tick: Option<u64> = None;
    let mut decoded_offset = 0u64;
    let mut saw_selection_start = selection.is_none();
    let mut saw_selection_end = selection.is_none();
    let mut saw_first_sequence = selection.is_none();
    let mut saw_last_sequence = selection.is_none();
    let mut publication_authoritative_records = 0u64;
    let mut failure_records = 0u64;
    let mut data_records = 0u64;
    let mut unreconstructed_records = 0u64;
    // The retained consumer baselines this scan reconstructs `PSD1` envelopes
    // against, and the reason the CURRENT frame's d81 DATA is missing when its
    // envelope refused — taken at FRAME_END, where the hour degrades to a gap.
    let mut span_delta = compact_contract::payload_span_delta::PayloadSpanDeltaConsumer::new();
    let mut pending_data_lost: Option<String> = None;
    let mut first_data_tick = None;
    let mut pending_frame_tick = None;
    let mut pending_perf_domains = BTreeMap::new();
    let mut pending_terminal_domains = BTreeSet::new();
    let mut pending_perf_status = None;
    let mut pending_data: Option<(u32, Vec<u8>)> = None;
    let mut pending_d18_data: Option<D18CountryCarrier> = None;
    let mut pending_d18_lost: Option<String> = None;
    let mut pending_publication_authoritative = false;
    // DOMAIN_DATA_REPEAT reconstruction baselines for the two domains this
    // scanner decodes payloads of. A repeat's payload is a volatile prefix
    // plus a digest of the previous published payload's tail; verifying that
    // digest against the stored baseline and reconstructing lets this scan
    // see exactly the bytes the original full record carried.
    let mut repeat_baselines: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
    // Country roster entries decoded from full D18 records OUTSIDE the
    // selection: true wire assertions that fill identity holes for selected
    // hours whose own d18 record predates the selection (Codex review round
    // 25, finding 1). Selected frames' bindings take precedence.
    let mut primed_country_bindings: BTreeMap<u32, String> = BTreeMap::new();
    // Every country id any walked D18 roster vouched for, in wire order —
    // the scan-side portability oracle for d81 payloads (round 26). Province
    // identities are answered by the authenticated catalog instead (round
    // 27).
    let mut bound_country_ids: BTreeSet<u32> = BTreeSet::new();
    let mut pending_d81_gaps = Vec::new();
    let mut next_gap_ordinal_by_lane = BTreeMap::new();
    let mut last_gap_end_by_lane = BTreeMap::new();
    let mut country_bindings = BTreeMap::<u32, String>::new();
    let mut country_source_by_symbol = BTreeMap::<String, u32>::new();
    if selection.is_some_and(|bounds| {
        bounds.first_sequence > bounds.last_sequence || bounds.start_byte >= bounds.end_byte
    }) {
        return Err(ProductError::new(
            "H4CR selection bounds are empty or reversed",
        ));
    }
    loop {
        let record_start = decoded_offset;
        let Some(()) = read_header(&mut reader, &mut header_bytes)? else {
            break;
        };
        sha256.update(header_bytes);
        let header = parse_compact_record_header(&header_bytes).map_err(|error| {
            ProductError::new(format!("H4CR framing rejected on branch {branch}: {error}"))
        })?;
        let mut payload = vec![0u8; header.payload_len];
        reader.read_exact(&mut payload).map_err(|error| {
            ProductError::new(format!(
                "H4CR payload is truncated on branch {branch}: {error}"
            ))
        })?;
        decoded_offset = record_start
            .checked_add(COMPACT_RECORD_HEADER_LEN as u64)
            .and_then(|value| value.checked_add(header.payload_len as u64))
            .ok_or_else(|| ProductError::new("decoded H4CR byte position overflows"))?;
        sha256.update(&payload);
        let mut repeat_validation_refused = false;
        let accepted = match validator.validate_record(&header, &payload) {
            Ok(accepted) => accepted,
            // A SEMANTICALLY MALFORMED DOMAIN_DATA_REPEAT IS A LOST RECORD,
            // NOT A BROKEN SCAN (Codex review round 14, finding 1): the
            // framing layer parsed it, so walk it as ordinary and let the
            // baseline-maintenance block below refuse it BY NAME; a selected
            // instance then degrades to the excluded-and-gap outcomes instead
            // of aborting the product.
            Err(
                _error @ (StreamValidationError::InvalidRecordPayload(_)
                | StreamValidationError::PayloadLengthMismatch { .. }
                | StreamValidationError::ReservedDomainZero),
            ) if header.record_type
                == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT =>
            {
                // Only SEMANTIC failures of a type-14 record are tolerable
                // losses; the refusal itself is NOT discarded — baseline
                // maintenance below refuses reconstruction by name, or a
                // valid-digest repeat with illegal flags would pass as
                // authoritative (Codex review rounds 15/17). SEQUENCE-level
                // errors say the stream reordered or lost bytes and stay
                // fatal: tolerating them while the validator still expects
                // N would admit physically reordered streams (Codex review
                // round 18, finding 2).
                validator.tolerate_failed_record(&header);
                repeat_validation_refused = true;
                AcceptedRecordKind::Ordinary
            }
            Err(error) => {
                return Err(ProductError::new(format!(
                    "H4CR stream validation failed on branch {branch}: {error}"
                )))
            }
        };
        let decoded_domain_gap = if accepted == AcceptedRecordKind::Ordinary
            && header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP
        {
            let carrier =
                compact_contract::DomainGapRecordV1::decode(&payload).map_err(|error| {
                    ProductError::new(format!(
                        "branch {branch} DOMAIN_GAP at sequence {} rejected: {error}",
                        header.sequence
                    ))
                })?;
            if carrier.target_hour_raw != header.game_hour_raw {
                return Err(ProductError::new(format!(
                    "branch {branch} DOMAIN_GAP envelope hour {} differs from target {}",
                    header.game_hour_raw, carrier.target_hour_raw
                )));
            }
            let lane = (header.domain_id, carrier.part.code());
            let expected = next_gap_ordinal_by_lane.entry(lane).or_insert(0u64);
            if carrier.outcome_ordinal != *expected {
                return Err(ProductError::new(format!(
                    "branch {branch} DOMAIN_GAP domain {} part {} outcome ordinal {} is not expected {}",
                    header.domain_id,
                    carrier.part.code(),
                    carrier.outcome_ordinal,
                    *expected
                )));
            }
            *expected = expected
                .checked_add(1)
                .ok_or_else(|| ProductError::new("DOMAIN_GAP outcome ordinal overflows"))?;
            if let Some(prior_end) = last_gap_end_by_lane.get(&lane) {
                if carrier.range_start_hour_raw <= *prior_end {
                    return Err(ProductError::new(format!(
                        "branch {branch} DOMAIN_GAP domain {} part {} range {}..={} overlaps or regresses after {}",
                        header.domain_id,
                        carrier.part.code(),
                        carrier.range_start_hour_raw,
                        carrier.range_end_hour_raw,
                        prior_end
                    )));
                }
            }
            last_gap_end_by_lane.insert(lane, carrier.range_end_hour_raw);
            Some(carrier)
        } else {
            None
        };
        // PRIME THE PSD1 CONSUMER THROUGH EVERY ENVELOPE IN THE STREAM,
        // SELECTED OR NOT. A stitched member's selection may begin mid-chain;
        // the retained source it cuts from still carries every earlier record,
        // and this whole scan digests all of them against the branch
        // fingerprint, so the baseline a selected delta names is available and
        // trustworthy even when the record carrying it is outside the product.
        // Priming here keeps a mid-chain selection reconstructing exactly what
        // the producer meant instead of falsely refusing until the next
        // keyframe; only records INSIDE the selection are folded or counted.
        let mut scanned_d81_envelope: Option<
            Result<Option<Vec<u8>>, String>,
        > = None;
        if accepted == AcceptedRecordKind::Ordinary
            && header.domain_id == d81::DOMAIN as u16
            && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
        {
            scanned_d81_envelope = Some(
                compact_contract::payload_span_delta::reconstruct_if_enveloped(
                    &mut span_delta,
                    header.record_flags,
                    compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
                        domain_id: header.domain_id,
                        source_sequence: header.sequence,
                        game_hour_raw: header.game_hour_raw,
                    },
                    &payload,
                ),
            );
        }
        let selected = match selection {
            None => true,
            Some(bounds) => {
                if record_start < bounds.start_byte && bounds.start_byte < decoded_offset {
                    return Err(ProductError::new(
                        "H4CR selection start byte falls inside a record",
                    ));
                }
                if record_start < bounds.end_byte && bounds.end_byte < decoded_offset {
                    return Err(ProductError::new(
                        "H4CR selection end byte falls inside a record",
                    ));
                }
                if record_start == bounds.start_byte && header.sequence != bounds.first_sequence {
                    return Err(ProductError::new(
                        "H4CR selection byte start disagrees with first sequence",
                    ));
                }
                if decoded_offset == bounds.end_byte && header.sequence != bounds.last_sequence {
                    return Err(ProductError::new(
                        "H4CR selection byte end disagrees with last sequence",
                    ));
                }
                saw_selection_start |= record_start == bounds.start_byte;
                saw_selection_end |= decoded_offset == bounds.end_byte;
                saw_first_sequence |= header.sequence == bounds.first_sequence;
                saw_last_sequence |= header.sequence == bounds.last_sequence;
                header.sequence >= bounds.first_sequence
                    && header.sequence <= bounds.last_sequence
                    && record_start >= bounds.start_byte
                    && decoded_offset <= bounds.end_byte
            }
        };
        // Baseline maintenance runs for EVERY d18/d81 data record on the
        // stream, selected or not: a retained selection may BEGIN on a
        // repeat whose full record precedes bounds.start_byte, and that
        // repeat is only verifiable against a baseline the scan kept while
        // walking up to the selection. `repeat_reconstruction` carries the
        // verified original payload of a SELECTED repeat into the decode
        // paths below.
        let mut repeat_reconstruction: Option<Vec<u8>> = None;
        let mut repeat_failure: Option<String> = None;
        if accepted == AcceptedRecordKind::Ordinary
            && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            && (header.domain_id == 18 || header.domain_id == d81::DOMAIN as u16)
        {
            repeat_baselines.insert(header.domain_id, payload.clone());
            if header.domain_id == 18 {
                // PRIME THE COUNTRY ROSTER FROM EVERY FULL D18 RECORD,
                // SELECTED OR NOT (Codex review round 25, finding 1): a
                // selection may begin after the roster entry an inside-
                // selected d81 hour references was published. These are true
                // wire assertions; filling holes with them keeps identity
                // portability checkable without inventing state. Selected
                // frames still overwrite with their own (later) roster.
                if let Ok(carrier) = decode_complete_d18_country_rows(&payload) {
                    if let Some(rows) = carrier.rows {
                        for (source_id, symbol) in rows {
                            primed_country_bindings.entry(source_id).or_insert(symbol);
                            // Every rostered id is vouched for by this very
                            // record, whatever map wins later.
                            bound_country_ids.insert(source_id);
                        }
                    }
                }
            }
        } else if accepted == AcceptedRecordKind::Ordinary
            && header.record_type
                == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
            && (header.domain_id == 18 || header.domain_id == d81::DOMAIN as u16)
        {
            // FAIL-CLOSED, NOT FAIL-DEAD (Codex review round 12, finding 3):
            // a refused repeat is a lost record, not a broken build. The
            // stale baseline is kept so a later full record resynchronises,
            // the refusal is named, and a SELECTED repeat degrades to the
            // excluded-and-gap outcome in the decode paths below.
            // A VALIDATOR-refused repeat never even reaches reconstruction:
            // its rejected flags are part of the verdict, and digest-plus-
            // length checks alone would accept it as authoritative (Codex
            // review round 17, finding 2).
            if repeat_validation_refused {
                repeat_failure = Some(
                    "DOMAIN_DATA_REPEAT failed stream validation".to_string(),
                );
            } else {
                match reconstruct_repeat_payload(
                    &mut repeat_baselines,
                    header.domain_id,
                    header.sequence,
                    header.game_hour_raw,
                    branch,
                    &payload,
                ) {
                    Ok(reconstructed) => {
                        repeat_baselines.insert(header.domain_id, reconstructed.clone());
                        repeat_reconstruction = Some(reconstructed);
                    }
                    Err(error) => repeat_failure = Some(error.to_string()),
                }
            }
        }
        if !selected {
            continue;
        }
        if accepted == AcceptedRecordKind::Ordinary
            && header.domain_id == 18
            && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
        {
            let carrier = decode_complete_d18_country_rows(&payload).map_err(|error| {
                ProductError::new(format!(
                    "branch {branch} D18 country identity at sequence {} rejected: {error}",
                    header.sequence
                ))
            })?;
            if pending_d18_data.replace(carrier).is_some() {
                return Err(ProductError::new(format!(
                    "branch {branch} repeats D18 DATA in frame {}",
                    header.game_hour_raw
                )));
            }
            repeat_baselines.insert(18, payload.clone());
        }
        // A d18 REPEAT vouches for a tail byte-identical to the previous
        // published d18 payload: the reconstruction above was already
        // verified against this scanner's own baseline; decode it exactly
        // like a full record.
        if accepted == AcceptedRecordKind::Ordinary
            && header.domain_id == 18
            && header.record_type
                == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
        {
            let reconstructed = match repeat_reconstruction.as_deref() {
                Some(reconstructed) => reconstructed,
                // THE SCAN REFUSED THIS REPEAT: a lost record, accounted as
                // an exclusion — never a build abort (Codex review round 12,
                // finding 3). The hour's PERF/DATA binding below closes as a
                // NAMED d18 gap instead of a seal mismatch (round 13,
                // finding 1); the next full d18 record resynchronises.
                None => {
                    unreconstructed_records += 1;
                    pending_d18_lost = Some(repeat_failure.clone().expect(
                        "a selected repeat without its reconstruction carries its refusal",
                    ));
                    // THE HOUR'S IDENTITY ROSTER IS UNVERIFIED, SO ITS D81
                    // STATE CANNOT BE CERTIFIED EITHER: country identities
                    // the manifest cannot vouch for would fail portability
                    // during the streaming fold and abort the build (Codex
                    // review round 14, finding 3). Close the whole hour as
                    // the named lost-data outcome instead — a repeat carries
                    // no new roster, so no other hour loses an identity.
                    if pending_data_lost.is_none() {
                        pending_data_lost = Some(format!(
                            "d18 DOMAIN_DATA_REPEAT refused verification: {}",
                            repeat_failure.clone().expect("refusal reason checked above")
                        ));
                    }
                    continue;
                }
            };
            let carrier = match decode_complete_d18_country_rows(reconstructed) {
                Ok(carrier) => carrier,
                Err(error) => {
                    // DIGEST-VALID RESOLUTION OF MALFORMED D18 BYTES IS STILL
                    // A LOST HOUR, NOT A BROKEN BUILD (Codex review round 22):
                    // the malformed full record may even lie outside the
                    // selected extent, where nothing ever decoded it. Same
                    // named gap as a verification refusal.
                    unreconstructed_records += 1;
                    let reason =
                        format!("D18 repeated payload failed domain validation: {error}");
                    pending_d18_lost = Some(reason.clone());
                    if pending_data_lost.is_none() {
                        pending_data_lost = Some(reason);
                    }
                    continue;
                }
            };
            if pending_d18_data.replace(carrier).is_some() {
                return Err(ProductError::new(format!(
                    "branch {branch} repeats D18 DATA in frame {}",
                    header.game_hour_raw
                )));
            }
        }
        if accepted == AcceptedRecordKind::Ordinary {
            let record_tick = u64::try_from(header.game_hour_raw).map_err(|_| {
                ProductError::new(format!(
                    "branch {branch} ordinary record uses negative raw game hour {}",
                    header.game_hour_raw
                ))
            })?;
            if header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP {
                match pending_frame_tick {
                    None => pending_frame_tick = Some(record_tick),
                    Some(open) if open == record_tick => {}
                    Some(open) => {
                        return Err(ProductError::new(format!(
                            "branch {branch} DOMAIN_GAP advanced from frame {open} to {record_tick} without FRAME_END"
                        )))
                    }
                }
                let carrier = decoded_domain_gap.ok_or_else(|| {
                    ProductError::new("selected ordinary DOMAIN_GAP bypassed full-source decoding")
                })?;
                if !pending_terminal_domains.insert(header.domain_id) {
                    return Err(ProductError::new(format!(
                        "branch {branch} repeats terminal outcome for domain {} in tick {record_tick}",
                        header.domain_id
                    )));
                }
                if header.domain_id == d81::DOMAIN as u16 {
                    pending_d81_gaps.push(map_d81_domain_gap(branch, carrier)?);
                    failure_records = failure_records
                        .checked_add(1)
                        .ok_or_else(|| ProductError::new("d81 failure count overflows"))?;
                }
                continue;
            }
            if header.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_END {
                if pending_frame_tick != Some(record_tick) {
                    return Err(ProductError::new(format!(
                        "branch {branch} FRAME_END at {record_tick} has no matching open frame"
                    )));
                }
                let emitted_domains =
                    u32::from_le_bytes(payload[16..20].try_into().expect("FRAME_END shape"));
                let scheduled_domains =
                    u32::from_le_bytes(payload[20..24].try_into().expect("FRAME_END shape"));
                let frame_status =
                    u32::from_le_bytes(payload[24..28].try_into().expect("FRAME_END shape"));
                let reserved =
                    u32::from_le_bytes(payload[28..32].try_into().expect("FRAME_END shape"));
                let observed_domains =
                    u32::try_from(pending_terminal_domains.len()).map_err(|_| {
                        ProductError::new("FRAME_END terminal domain count exceeds u32")
                    })?;
                if !pending_terminal_domains.contains(&(d81::DOMAIN as u16)) {
                    return Err(ProductError::new(format!(
                        "branch {branch} frame {record_tick} has no exact D81 terminal PERF or H4DG outcome"
                    )));
                }
                let expected_status =
                    pending_perf_domains
                        .iter()
                        .fold(0u32, |status_or, (domain, status)| {
                            status_or
                                | compact_contract::compact_status_for_global_interpretation(
                                    *domain, *status,
                                )
                        });
                let unknown_status =
                    compact_contract::compact_status_unknown_flags_v1(frame_status);
                if reserved != 0
                    || emitted_domains != scheduled_domains
                    || emitted_domains != observed_domains
                    || frame_status != expected_status
                    || unknown_status != 0
                {
                    return Err(ProductError::new(format!(
                        "branch {branch} FRAME_END seal mismatch at {record_tick}: emitted={emitted_domains}, scheduled={scheduled_domains}, terminal={observed_domains}, status={frame_status:#x}, expected_status={expected_status:#x}, unknown={unknown_status:#x}, reserved={reserved:#x}"
                    )));
                }
                match (
                    pending_perf_domains.get(&18).copied(),
                    pending_d18_data.take(),
                ) {
                    (None, None) => {}
                    (Some(perf), Some(carrier)) if perf == carrier.status => {                        if let Some(rows) = carrier.rows {
                            for (source_id, symbol) in rows {
                                if let Some(previous) =
                                    country_bindings.insert(source_id, symbol.clone())
                                {
                                    if previous != symbol {
                                        return Err(ProductError::new(format!(
                                            "branch {branch} D18 country source {source_id} changes from {previous} to {symbol}"
                                        )));
                                    }
                                }
                                if let Some(previous) =
                                    country_source_by_symbol.insert(symbol.clone(), source_id)
                                {
                                    if previous != source_id {
                                        return Err(ProductError::new(format!(
                                            "branch {branch} D18 country symbol {symbol} aliases source ids {previous} and {source_id}"
                                        )));
                                    }
                                }
                            }
                        }
                    }
                    // THE HOUR WHOSE D18 REPEAT REFUSED VERIFICATION CLOSES
                    // AS A NAMED GAP, NOT A SEAL MISMATCH: same lost-record
                    // class as the d81 status-bound gap below — the record
                    // arrived but its bytes could not be recovered, which is
                    // never a corrupt product.
                    (Some(_perf), None) if pending_d18_lost.take().is_some() => {}
                    (perf, data) => {
                        return Err(ProductError::new(format!(
                            "branch {branch} D18 PERF/DATA binding differs in closed frame {record_tick}: PERF={perf:?}, DATA={:?}",
                            data.as_ref().map(|carrier| carrier.status)
                        )))
                    }
                }
                let lost_data = pending_data_lost.take();
                let mut unreconstructable_perf = None;
                let payload = match (pending_perf_status.take(), pending_data.take()) {
                    (None, None) => None,
                    (Some(perf), Some((data, payload))) if perf == data && lost_data.is_none() => {
                        Some(payload)
                    }
                    // THE HOUR WHOSE IDENTITY ROSTER WAS LOST CLOSES AS A GAP
                    // EVEN WHEN ITS D81 BYTES WERE HEALTHY: an unverifiable
                    // country roster cannot be certified portable downstream,
                    // so the whole hour degrades together (Codex review round
                    // 15, finding 2).
                    (Some(perf), Some(_)) if lost_data.is_some() => {
                        unreconstructable_perf = Some(perf);
                        None
                    }
                    // THE HOUR WHOSE DATA ENVELOPE REFUSED RECONSTRUCTION
                    // CLOSES AS A STATUS-BOUND GAP, NOT A BUILD FAILURE.
                    //
                    // The record arrived but its bytes could not be recovered,
                    // which is the ordinary consequence of a dropped or damaged
                    // record rather than a corrupt product; refusing
                    // publication for it would be the exact defect class lane
                    // PS's Codex rounds 7-9 closed for every other walker ("one
                    // lost record destroyed a whole capture's publication"). It
                    // is named instead: the exclusion count rides the session
                    // inspection, and this tick publishes with the same
                    // status-witness gap any unavailable hour carries.
                    (Some(perf), None) if lost_data.is_some() => {
                        unreconstructable_perf = Some(perf);
                        None
                    }
                    (perf, data) => {
                        return Err(ProductError::new(format!(
                            "branch {branch} d81 PERF/DATA binding differs in closed frame {record_tick}: PERF={perf:?}, DATA={:?}, unreconstructable={lost_data:?}",
                            data.as_ref().map(|(status, _)| status)
                        )));
                    }
                };
                let coverage_start = if let Some(prior) = last_closed_tick {
                    if record_tick <= prior {
                        return Err(ProductError::new(format!(
                            "branch {branch} selected H4CR FRAME_END regresses or duplicates at {prior}..{record_tick}"
                        )));
                    }
                    prior.checked_add(1).ok_or_else(|| {
                        ProductError::new("selected H4CR FRAME_END successor overflows u64")
                    })?
                } else {
                    pending_d81_gaps
                        .first()
                        .map(|gap| gap.tick_start)
                        .unwrap_or(record_tick)
                        .min(record_tick)
                };
                first_coverage_tick.get_or_insert(coverage_start);
                let coverage_end = record_tick.checked_add(1).ok_or_else(|| {
                    ProductError::new("selected H4CR FRAME_END coverage overflows u64")
                })?;
                if pending_d81_gaps.len() > 1 {
                    return Err(ProductError::new(format!(
                        "branch {branch} frame {record_tick} has more than one D81 H4DG terminal"
                    )));
                }
                match (pending_d81_gaps.pop(), payload) {
                    (Some(gap), None) => {
                        if gap.tick_start != coverage_start || gap.tick_end != coverage_end {
                            return Err(ProductError::new(format!(
                                "branch {branch} D81 H4DG {}..{} does not exactly authenticate closed-frame coverage {coverage_start}..{coverage_end}",
                                gap.tick_start, gap.tick_end
                            )));
                        }
                        on_closed(ScannedD81Item::Gap(gap))?;
                    }
                    (None, Some(payload)) => {
                        if coverage_start != record_tick {
                            return Err(ProductError::new(format!(
                                "branch {branch} D81 DATA at {record_tick} follows silent coverage start {coverage_start}; an exact H4DG range is required"
                            )));
                        }
                        on_closed(ScannedD81Item::Closed {
                            tick: record_tick,
                            payload,
                        })?;
                        // THE HOUR ACTUALLY PUBLISHES: only now may its
                        // snapshot count as publication-authoritative — an
                        // hour gapped by lost d18 identity never reaches this
                        // arm (Codex review round 23, finding 2).
                        if pending_publication_authoritative {
                            publication_authoritative_records = publication_authoritative_records
                                .checked_add(1)
                                .ok_or_else(|| {
                                    ProductError::new("d81 success count overflows")
                                })?;
                            pending_publication_authoritative = false;
                        }
                    }
                    (Some(_), Some(_)) => {
                        return Err(ProductError::new(format!(
                            "branch {branch} d81 frame {record_tick} carries both DATA and authenticated state-gap outcomes"
                        )));
                    }
                    (None, None) => match unreconstructable_perf.take() {
                        // THE LOST-DATA HOUR: the perf terminal exists, the
                        // payload does not. Publish a coverage-shortfall gap
                        // and keep building — see the lost-data arm above.
                        // A COVERAGE witness, deliberately not the PERF
                        // status: that status describes the read, which
                        // succeeded, and a StatusBits witness without failure
                        // bits is rejected at verification ("d81 status gap
                        // does not carry a known failure status"); what is
                        // missing here is this hour's STATE, which is exactly
                        // what CoverageShortfall names. The precise cause —
                        // an envelope whose digest refused — is the named
                        // exclusion count on the session inspection.
                        Some(_) => {
                            let tick_end = record_tick.checked_add(1).ok_or_else(|| {
                                ProductError::new("d81 lost-hour gap tick overflows")
                            })?;
                            on_closed(ScannedD81Item::Gap(coverage_gap(
                                branch, record_tick, tick_end,
                            )))?;
                        }
                        None => {
                            return Err(ProductError::new(format!(
                                "branch {branch} d81 frame {record_tick} has neither status-bound DATA nor H4DG"
                            )));
                        }
                    },
                }
                last_closed_tick = Some(record_tick);
                pending_frame_tick = None;
                pending_perf_domains.clear();
                pending_terminal_domains.clear();
                pending_publication_authoritative = false;
                // The d18 loss marker is PER-FRAME: taking it here (win or
                // lose) can never leak into a later frame and excuse an
                // unrelated missing D18 DATA record (Codex review round 14,
                // finding 2).
                pending_d18_lost = None;
            } else {
                match pending_frame_tick {
                    None => pending_frame_tick = Some(record_tick),
                    Some(open) if open == record_tick => {}
                    Some(open) => {
                        return Err(ProductError::new(format!(
                            "branch {branch} advanced from frame {open} to {record_tick} without FRAME_END"
                        )))
                    }
                }
            }
        }
        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF {
            let status_flags = u32::from_le_bytes(
                payload
                    .get(32..36)
                    .ok_or_else(|| ProductError::new("DOMAIN_PERF status word is truncated"))?
                    .try_into()
                    .map_err(|_| ProductError::new("DOMAIN_PERF status word is malformed"))?,
            );
            if pending_perf_domains
                .insert(header.domain_id, status_flags)
                .is_some()
            {
                let label = if header.domain_id == d81::DOMAIN as u16 {
                    "d81 PERF".to_string()
                } else {
                    format!("PERF domain {}", header.domain_id)
                };
                return Err(ProductError::new(format!(
                    "branch {branch} repeats {label} in tick {}",
                    header.game_hour_raw
                )));
            }
            if !pending_terminal_domains.insert(header.domain_id) {
                return Err(ProductError::new(format!(
                    "branch {branch} repeats terminal outcome for domain {} in tick {}",
                    header.domain_id, header.game_hour_raw
                )));
            }
        }
        if header.domain_id == d81::DOMAIN as u16 {
            let tick = u64::try_from(header.game_hour_raw).map_err(|_| {
                ProductError::new(format!(
                    "branch {branch} d81 uses negative raw game hour {}",
                    header.game_hour_raw
                ))
            })?;
            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF {
                let status_flags = u32::from_le_bytes(
                    payload
                        .get(32..36)
                        .ok_or_else(|| ProductError::new("d81 PERF status word is truncated"))?
                        .try_into()
                        .map_err(|_| ProductError::new("d81 PERF status word is malformed"))?,
                );
                if pending_frame_tick != Some(tick) {
                    return Err(ProductError::new(format!(
                        "branch {branch} d81 PERF at {tick} is outside its open frame"
                    )));
                }
                if pending_perf_status.replace(status_flags).is_some() {
                    return Err(ProductError::new(format!(
                        "branch {branch} repeats d81 PERF at tick {tick}"
                    )));
                }
                continue;
            }
            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
                || header.record_type
                    == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
            {
                // fall through to the d81 payload handling below; a REPEAT is
                // first reconstructed against this scanner's own baseline.
            } else {
                continue;
            }
            if pending_frame_tick != Some(tick) {
                return Err(ProductError::new(format!(
                    "branch {branch} d81 DATA at {tick} is outside its open frame"
                )));
            }
            // UNWRAP BEFORE ANYTHING LOOKS AT THE PAYLOAD. A `PSD1` envelope
            // carries no RRL1 grammar; the fold downstream of this scan reads
            // the payload the domain encoded, exactly as every other walker.
            // The reconstruction itself already happened above, selected or
            // not (see the priming block); here it is only harvested.
            let was_repeat = header.record_type
                == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT;
            let data_bytes = if was_repeat {
                let resolved = match repeat_reconstruction.as_deref() {
                    Some(resolved) => resolved,
                    // THE SCAN REFUSED THIS REPEAT (missing baseline, wrong
                    // length, tail-digest mismatch): the same lost-record
                    // class as a refused envelope, so the SAME
                    // excluded-and-gap outcome — never a build abort.
                    None => {
                        unreconstructed_records += 1;
                        pending_data_lost = Some(repeat_failure.clone().expect(
                            "a selected repeat without its reconstruction carries its refusal",
                        ));
                        continue;
                    }
                };
                if header.record_flags
                    & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
                    != 0
                {
                    // THE SPAN-DELTA CONSUMER MUST SEE REPEAT-DERIVED PAYLOADS.
                    // A flagged repeat resolves to the repeated hour's ON-WIRE
                    // bytes, which are a PSD1 envelope exactly when that hour rode
                    // enveloped. `repeat_basis` authenticates those bytes against
                    // this scan's retained baseline (digest match) and returns the
                    // full original payload; a refusal is excluded and named like
                    // any other lost record — never `?`-aborted.
                    match compact_contract::payload_span_delta::repeat_basis(
                        &span_delta,
                        header.domain_id,
                        resolved,
                    ) {
                        Ok(basis) => basis,
                        Err(reason) => {
                            unreconstructed_records += 1;
                            pending_data_lost =
                                Some(format!("repeated envelope refused composition: {reason}"));
                            continue;
                        }
                    }
                } else {
                    // An unflagged repeat resolves to plain grammar bytes.
                    resolved.to_vec()
                }
            } else {
                match scanned_d81_envelope.take().expect(
                    "an enveloped d81 DATA record was primed during the scan pass",
                ) {
                    Ok(Some(reconstructed)) => reconstructed,
                    // The raw on-wire bytes are already stored as this
                    // domain's repeat baseline by the maintenance block above.
                    Ok(None) => payload.clone(),
                    Err(reason) => {
                        // EXCLUDED AND NAMED, NEVER A PUBLICATION ABORT. A
                        // record that cannot be reconstructed is the ordinary
                        // consequence of a lost or damaged record — the next
                        // keyframe resynchronises this consumer at the 64-hour
                        // cadence — and the hour degrades to a coverage-shortfall gap
                        // when its frame closes (see the lost-data arms at
                        // FRAME_END). The record counts only in
                        // `unreconstructed_records`: it is not usable DATA and
                        // must not claim coverage or authority.
                        unreconstructed_records += 1;
                        pending_data_lost = Some(reason);
                        continue;
                    }
                }
            };
            let was_enveloped =
                header.record_flags & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA != 0;
            let (payload_header, _) = match d81::split_payload(&data_bytes) {
                Ok(split) => split,
                // RECONSTRUCTED, YET NOT GRAMMAR. The codec's digest excludes
                // the declared volatile prefix — for d53/d43 a wall clock, but
                // for d81 the payload's own RRL1 magic/version/length — so
                // damage confined to those eight bytes reconstructs faithfully
                // and fails HERE instead. For an enveloped record that is the
                // same lost-record outcome as any other: excluded and named,
                // hour degrades to a gap at FRAME_END. A RAW record with an
                // invalid grammar keeps its pre-delta hard refusal — this lane
                // did not change how unenveloped corruption behaves.
                Err(error) if was_enveloped => {
                    unreconstructed_records += 1;
                    pending_data_lost =
                        Some(format!("reconstructed envelope is not valid d81 grammar: {error}"));
                    continue;
                }
                // DIGEST-VALID RESOLUTION OF MALFORMED D81 BYTES IS STILL A
                // LOST HOUR: the malformed full record may lie outside the
                // selected extent where nothing ever decoded it, and its
                // repeat carries those bytes inside (Codex review round 22).
                Err(error) if was_repeat => {
                    unreconstructed_records += 1;
                    pending_data_lost = Some(format!(
                        "repeated payload is not valid d81 grammar: {error}"
                    ));
                    continue;
                }
                Err(error) => {
                    return Err(ProductError::new(format!(
                        "branch {branch} d81 payload at tick {tick} rejected: {error}"
                    )));
                }
            };
            let status_flags = payload_header.status_flags;
            let payload_version = payload_header.payload_version;
            let hub_simulation_complete = payload_header.hub_simulation_complete;
            // SCAN-SIDE IDENTITY PORTABILITY ORACLE (Codex review rounds
            // 23-27): decode the resolved bytes for their referenced ids and
            // require countries to be vouched by walked D18 rosters and
            // provinces by the AUTHENTICATED catalog — payload occurrence
            // proves nothing, and a missing check here would let the
            // second-pass fold die build-wide. An unvouched identity taints
            // ONLY this hour; raw full-record corruption keeps its
            // pre-delta hard refusal.
            match compact_contract::rail_replay::decode_payload(&data_bytes) {
                Ok(decoded) => {
                    let mut unknown_country: Option<u32> = None;
                    let mut unknown_province: Option<u32> = None;
                    'provinces: for province in &decoded.provinces {
                        if !catalog_province_ids.is_empty()
                            && !catalog_province_ids.contains(&province.province_id)
                        {
                            unknown_province = Some(province.province_id);
                            break 'provinces;
                        }
                        for neighbor in &province.neighbors {
                            if !catalog_province_ids.is_empty()
                                && !catalog_province_ids.contains(&neighbor.province_id)
                            {
                                unknown_province = Some(neighbor.province_id);
                                break 'provinces;
                            }
                        }
                    }
                    if unknown_province.is_none() {
                        'hub_rows: for row in &decoded.hub_simulation_rows {
                            for country_id in
                                [row.province_owner_country_id, row.pov_country_id]
                            {
                                if country_id != 0 && !bound_country_ids.contains(&country_id) {
                                    unknown_country = Some(country_id);
                                    break 'hub_rows;
                                }
                            }
                            if !catalog_province_ids.is_empty()
                                && !catalog_province_ids.contains(&row.province_id)
                            {
                                unknown_province = Some(row.province_id);
                                break 'hub_rows;
                            }
                        }
                    }
                    let identity_loss = unknown_country
                        .map(|id| {
                            format!(
                                "d81 state references country id {id} that no walked D18 roster vouches for"
                            )
                        })
                        .or_else(|| {
                            unknown_province.map(|id| {
                                format!(
                                    "d81 state references province id {id} absent from the authenticated province catalog"
                                )
                            })
                        });
                    if let Some(reason) = identity_loss {
                        if was_repeat || was_enveloped {
                            unreconstructed_records += 1;
                            pending_data_lost = Some(reason);
                            continue;
                        }
                        return Err(ProductError::new(format!(
                            "branch {branch} d81 payload at tick {tick} rejected: {reason}"
                        )));
                    }
                }
                Err(error) => {
                    // split_payload above already proved this grammar with the
                    // same decoder family; a divergent failure here is a bug,
                    // not a stream condition.
                    return Err(ProductError::new(format!(
                        "branch {branch} d81 payload at tick {tick} failed identity pre-read: {error}"
                    )));
                }
            }
            let unknown = compact_contract::compact_status_unknown_flags_v1(status_flags);
            if unknown != 0 {
                if was_repeat {
                    unreconstructed_records += 1;
                    pending_data_lost = Some(format!(
                        "repeated d81 payload has unknown status {unknown:#x}"
                    ));
                    continue;
                }
                return Err(ProductError::new(format!(
                    "branch {branch} d81 payload at tick {tick} has unknown status {unknown:#x}"
                )));
            }
            if pending_data.replace((status_flags, data_bytes)).is_some() {
                return Err(ProductError::new(format!(
                    "branch {branch} repeats d81 DATA at tick {tick}"
                )));
            }
            // The repeat baseline maintenance above already stored this
            // record's ON-WIRE bytes (raw DATA payload, or a verified repeat's
            // resolved bytes) before the selection filter; storing again here
            // from the post-unwrap side would put BASIS bytes where the gate's
            // digest expects WIRE bytes.
            data_records = data_records
                .checked_add(1)
                .ok_or_else(|| ProductError::new("d81 data count overflows"))?;
            first_data_tick.get_or_insert(tick);
            let failure = compact_contract::compact_status_is_failure(status_flags);
            if failure {
                failure_records = failure_records
                    .checked_add(1)
                    .ok_or_else(|| ProductError::new("d81 failure count overflows"))?;
            } else if payload_version == d81::PAYLOAD_VERSION_V3 && hub_simulation_complete {
                // DEFERRED to the frame's actual close: an hour whose d18
                // identity was lost closes as a named gap and NEVER publishes
                // this snapshot, so counting it here would report an
                // authoritative record that does not exist (Codex review
                // round 23, finding 2).
                pending_publication_authoritative = true;
            }
        }
    }
    if !(saw_selection_start && saw_selection_end && saw_first_sequence && saw_last_sequence) {
        return Err(ProductError::new(format!(
            "branch {branch} selected H4CR extent does not align to exact record/byte boundaries"
        )));
    }
    if let Some(tick) = pending_frame_tick {
        return Err(ProductError::new(format!(
            "branch {branch} selected H4CR ends with unclosed frame {tick}"
        )));
    }
    let first_coverage_tick = first_coverage_tick
        .ok_or_else(|| ProductError::new("selected H4CR contains no state coverage"))?;
    let last_closed_tick = last_closed_tick
        .ok_or_else(|| ProductError::new("selected H4CR contains no closed hours"))?;
    Ok(ScannedBranch {
        branch,
        decoded_sha256: sha256.finalize().into(),
        first_coverage_tick,
        last_closed_tick,
        data_records,
        publication_authoritative_records,
        failure_records,
        unreconstructed_records,
        first_data_tick,
        country_bindings: {
            let mut merged = country_bindings;
            for (source_id, symbol) in primed_country_bindings {
                merged.entry(source_id).or_insert(symbol);
            }
            merged
        },
    })
}

/// Verify one DOMAIN_DATA_REPEAT record against this scanner's stored
/// baseline for `domain`, reconstruct the original payload (fresh volatile
/// prefix over the verified tail), and hand it back for the caller to refresh
/// the baseline with. Fail-closed by name: a missing baseline, a wrong
/// length, or any digest mismatch is returned as the named error — the CALLER
/// keeps its stale baseline and, when the repeat is selected, degrades to
/// `unreconstructed_records` plus the coverage gap instead of aborting the
/// build (Codex review round 12, finding 3).
fn reconstruct_repeat_payload(
    repeat_baselines: &mut BTreeMap<u16, Vec<u8>>,
    domain_id: u16,
    sequence: u64,
    game_hour_raw: i32,
    branch: u32,
    payload: &[u8],
) -> Result<Vec<u8>, ProductError> {
    let span = compact_contract::domain_declared_volatile_prefix_len(domain_id);
    let expected_len = compact_contract::domain_data_repeat_payload_len(domain_id);
    if payload.len() != expected_len {
        return Err(ProductError::new(format!(
            "branch {branch} DOMAIN_DATA_REPEAT for domain {domain_id} at sequence {sequence} is {} byte(s), not {expected_len}",
            payload.len()
        )));
    }
    let Some(baseline) = repeat_baselines.get(&domain_id) else {
        return Err(ProductError::new(format!(
            "branch {branch} DOMAIN_DATA_REPEAT for domain {domain_id} at sequence {sequence} has no stored baseline"
        )));
    };
    if baseline.len() <= span {
        return Err(ProductError::new(format!(
            "branch {branch} DOMAIN_DATA_REPEAT baseline for domain {domain_id} carries no tail beyond its declared prefix"
        )));
    }
    let digest = Sha256::digest(&baseline[span..]);
    if digest.as_slice() != &payload[span..] {
        return Err(ProductError::new(format!(
            "branch {branch} DOMAIN_DATA_REPEAT for domain {domain_id} at sequence {sequence} fails its baseline digest; the recorded tail does not match this scan's stored payload"
        )));
    }
    let mut reconstructed = payload[..span].to_vec();
    reconstructed.extend_from_slice(&baseline[span..]);
    repeat_baselines.insert(domain_id, reconstructed.clone());
    let _ = game_hour_raw;
    Ok(reconstructed)
}

fn decode_complete_d18_country_rows(payload: &[u8]) -> Result<D18CountryCarrier, String> {
    if payload.len() < 20 {
        return Err("payload is shorter than the D18 header".into());
    }
    let row_count = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
    let changed = u32::from_le_bytes(payload[12..16].try_into().unwrap()) as usize;
    let status = u32::from_le_bytes(payload[16..20].try_into().unwrap());
    let unknown = compact_contract::compact_status_unknown_flags_v1(status);
    if unknown != 0 {
        return Err(format!("status has unknown bits {unknown:#x}"));
    }
    if compact_contract::compact_status_is_failure(status) {
        return Ok(D18CountryCarrier { status, rows: None });
    }
    if row_count == 0 || changed != row_count {
        return Err("successful D18 payload is not a complete nonempty roster".into());
    }
    let mut cursor = 20usize;
    let mut rows = BTreeMap::<u32, String>::new();
    let mut source_by_symbol = BTreeMap::<String, u32>::new();
    for index in 0..row_count {
        let source_id = d18_u32(payload, &mut cursor, "country id")?;
        let tag = d18_string(payload, &mut cursor, "country tag")?;
        if source_id == 0
            || tag.len() != 3
            || !tag
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Err(format!("row {index} has invalid country identity"));
        }
        let marker = *payload
            .get(cursor)
            .ok_or_else(|| "cosmetic marker is truncated".to_string())?;
        cursor += 1;
        match marker {
            0 | 1 => {}
            2 => {
                let _ = d18_string(payload, &mut cursor, "cosmetic country tag")?;
            }
            _ => return Err(format!("row {index} has invalid cosmetic marker {marker}")),
        }
        let symbol = format!("country/tag/{tag}");
        if rows.insert(source_id, symbol.clone()).is_some() {
            return Err(format!("row {index} duplicates country source {source_id}"));
        }
        if let Some(previous) = source_by_symbol.insert(symbol.clone(), source_id) {
            return Err(format!(
                "country symbol {symbol} aliases source ids {previous} and {source_id}"
            ));
        }
    }
    validate_d18_extension(payload, cursor)?;
    Ok(D18CountryCarrier {
        status,
        rows: Some(rows.into_iter().collect()),
    })
}

fn d18_u32(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<u32, String> {
    let end = cursor
        .checked_add(4)
        .ok_or_else(|| format!("{label} offset overflows"))?;
    let value = u32::from_le_bytes(
        bytes
            .get(*cursor..end)
            .ok_or_else(|| format!("{label} is truncated"))?
            .try_into()
            .unwrap(),
    );
    *cursor = end;
    Ok(value)
}

fn d18_string(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<String, String> {
    let len_end = cursor
        .checked_add(2)
        .ok_or_else(|| format!("{label} length offset overflows"))?;
    let len = u16::from_le_bytes(
        bytes
            .get(*cursor..len_end)
            .ok_or_else(|| format!("{label} length is truncated"))?
            .try_into()
            .unwrap(),
    ) as usize;
    let end = len_end
        .checked_add(len)
        .ok_or_else(|| format!("{label} extent overflows"))?;
    let value = std::str::from_utf8(
        bytes
            .get(len_end..end)
            .ok_or_else(|| format!("{label} is truncated"))?,
    )
    .map_err(|_| format!("{label} is not UTF-8"))?
    .to_string();
    *cursor = end;
    Ok(value)
}

fn validate_d18_extension(payload: &[u8], cursor: usize) -> Result<(), String> {
    let end = cursor
        .checked_add(40)
        .ok_or_else(|| "CIM1 header extent overflows".to_string())?;
    let header = payload
        .get(cursor..end)
        .ok_or_else(|| "CIM1 extension is truncated".to_string())?;
    if &header[..4] != b"CIM1"
        || u16::from_le_bytes(header[4..6].try_into().unwrap()) != 1
        || u16::from_le_bytes(header[6..8].try_into().unwrap()) != 40
        || header[36..40] != [0; 4]
    {
        return Err("CIM1 header is invalid".into());
    }
    let state_count = u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
    let expected_claims = u32::from_le_bytes(header[24..28].try_into().unwrap()) as usize;
    let expected_cores = u32::from_le_bytes(header[28..32].try_into().unwrap()) as usize;
    let major_count = u32::from_le_bytes(header[32..36].try_into().unwrap()) as usize;
    let mut at = end;
    let mut claims = 0usize;
    let mut cores = 0usize;
    for _ in 0..state_count {
        let row_end = at
            .checked_add(12)
            .ok_or_else(|| "CIM1 state header extent overflows".to_string())?;
        let row = payload
            .get(at..row_end)
            .ok_or_else(|| "CIM1 state row is truncated".to_string())?;
        let row_claims = u32::from_le_bytes(row[4..8].try_into().unwrap()) as usize;
        let row_cores = u32::from_le_bytes(row[8..12].try_into().unwrap()) as usize;
        claims = claims
            .checked_add(row_claims)
            .ok_or_else(|| "CIM1 claim count overflows".to_string())?;
        cores = cores
            .checked_add(row_cores)
            .ok_or_else(|| "CIM1 core count overflows".to_string())?;
        let memberships = row_claims
            .checked_add(row_cores)
            .and_then(|value| value.checked_mul(4))
            .ok_or_else(|| "CIM1 membership extent overflows".to_string())?;
        at = row_end
            .checked_add(memberships)
            .ok_or_else(|| "CIM1 state extent overflows".to_string())?;
        if at > payload.len() {
            return Err("CIM1 state memberships are truncated".into());
        }
    }
    if claims != expected_claims || cores != expected_cores {
        return Err("CIM1 membership totals disagree".into());
    }
    at = at
        .checked_add(
            major_count
                .checked_mul(8)
                .ok_or_else(|| "CIM1 major extent overflows".to_string())?,
        )
        .ok_or_else(|| "CIM1 total extent overflows".to_string())?;
    if at != payload.len() {
        return Err("CIM1 extension does not exactly cover payload".into());
    }
    Ok(())
}

fn map_d81_domain_gap(
    branch: u32,
    carrier: compact_contract::DomainGapRecordV1,
) -> Result<GapRecord, ProductError> {
    use compact_contract::{DomainGapPart, DomainGapReason};

    if carrier.part != DomainGapPart::State {
        return Err(ProductError::new(format!(
            "branch {branch} d81 DOMAIN_GAP part {:?} is not state",
            carrier.part
        )));
    }
    let tick_start = u64::try_from(carrier.range_start_hour_raw).map_err(|_| {
        ProductError::new(format!(
            "branch {branch} d81 DOMAIN_GAP starts at negative hour {}",
            carrier.range_start_hour_raw
        ))
    })?;
    let tick_end = u64::try_from(carrier.range_end_hour_raw)
        .ok()
        .and_then(|tick| tick.checked_add(1))
        .ok_or_else(|| ProductError::new("d81 DOMAIN_GAP half-open range overflows"))?;
    let width = tick_end
        .checked_sub(tick_start)
        .ok_or_else(|| ProductError::new("d81 DOMAIN_GAP range is reversed"))?;
    let (reason, witness) = match carrier.reason {
        DomainGapReason::Torn => (
            GapReason::TornSweep,
            GapWitness::TickBracket {
                start_tick: carrier.tick_before,
                end_tick: carrier.tick_after,
            },
        ),
        DomainGapReason::Overrun => (
            GapReason::Backpressure,
            GapWitness::Counter {
                counter_id: u64::from(
                    compact_contract::gap_record::GAP_COUNTER_CODE_QUEUE_OVERRUN_HOURS,
                ),
                observed: width,
            },
        ),
        DomainGapReason::Backpressure => (
            GapReason::Backpressure,
            GapWitness::Counter {
                counter_id: u64::from(
                    compact_contract::gap_record::GAP_COUNTER_CODE_SINK_BACKPRESSURE_HOURS,
                ),
                observed: width,
            },
        ),
        DomainGapReason::ClockUnreadable => (
            GapReason::SourceUnavailable,
            GapWitness::StatusBits {
                bit_width: 32,
                bits: compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE
                    .to_le_bytes()
                    .to_vec(),
            },
        ),
        DomainGapReason::WorkerPanic => (
            GapReason::WorkerPanic,
            GapWitness::StatusBits {
                bit_width: 32,
                bits: compact_contract::COMPACT_STATUS_WORKER_PANIC
                    .to_le_bytes()
                    .to_vec(),
            },
        ),
        DomainGapReason::SourceUnavailable => (
            GapReason::SourceUnavailable,
            GapWitness::StatusBits {
                bit_width: 32,
                bits: compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE
                    .to_le_bytes()
                    .to_vec(),
            },
        ),
        DomainGapReason::ReaderSilent => (
            GapReason::CoverageShortfall,
            GapWitness::Coverage {
                observed: 0,
                expected: width,
            },
        ),
        DomainGapReason::HourLabelMismatch => (
            GapReason::CoverageShortfall,
            GapWitness::Coverage {
                observed: 0,
                expected: width,
            },
        ),
    };
    let gap = GapRecord {
        tick_start,
        tick_end,
        branch,
        reason,
        part: GapPart::State,
        domains: vec![d81::DOMAIN],
        witness,
    };
    crate::verify::validate_d81_gap(&gap)?;
    Ok(gap)
}

fn read_header(
    reader: &mut dyn Read,
    header: &mut [u8; COMPACT_RECORD_HEADER_LEN],
) -> Result<Option<()>, ProductError> {
    let mut filled = 0;
    while filled < header.len() {
        let read = reader
            .read(&mut header[filled..])
            .map_err(|error| ProductError::new(format!("read H4CR header: {error}")))?;
        if read == 0 {
            if filled == 0 {
                return Ok(None);
            }
            return Err(ProductError::new(format!(
                "H4CR stream ends mid-header ({filled}/{})",
                header.len()
            )));
        }
        filled += read;
    }
    Ok(Some(()))
}

fn payload_states(payload: DecodedPayload) -> Vec<CollectionState> {
    let keyed = |rows: Vec<(wire_format::body::Key, wire_format::Record)>| {
        CollectionState::Keyed(rows.into_iter().collect())
    };
    vec![
        keyed(vec![payload.frame]),
        keyed(payload.provinces),
        CollectionState::OrderedKeyed(
            payload
                .directed_neighbors
                .into_iter()
                .map(|(key, record)| Row { key, record })
                .collect(),
        ),
        keyed(payload.hub_simulation_rows),
    ]
}

fn start_block(
    schema: &wire_format::Schema,
    branch: u32,
    index: u64,
    tick: u64,
    state: Vec<CollectionState>,
) -> Result<PendingBlock, ProductError> {
    let mut body = Vec::new();
    encode_shaped_baseline(schema, &state, &mut body)
        .map_err(|error| ProductError::new(format!("encode d81 baseline: {error:?}")))?;
    Ok(PendingBlock {
        branch,
        index,
        first_tick: tick,
        frames: vec![Frame::state(Kind::Baseline, tick, body)],
        prior: state,
    })
}

fn append_state(
    block: &mut PendingBlock,
    schema: &wire_format::Schema,
    tick: u64,
    next: Vec<CollectionState>,
) -> Result<(), ProductError> {
    let deltas = schema
        .collections
        .iter()
        .zip(block.prior.iter().zip(&next))
        .map(|(collection, (prior, next))| {
            diff_shaped(collection, prior, next)
                .map_err(|error| ProductError::new(format!("diff d81 state: {error:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if deltas.iter().all(Option::is_none) {
        if let Some(last) = block.frames.last_mut() {
            if last.kind == Kind::Unchanged {
                let expected = last
                    .last_covered_tick_checked(1)
                    .and_then(|last_tick| last_tick.checked_add(1));
                if expected == Some(tick) {
                    last.run_len = last
                        .run_len
                        .checked_add(1)
                        .ok_or_else(|| ProductError::new("UNCHANGED run length overflows"))?;
                    block.prior = next;
                    return Ok(());
                }
            }
        }
        block.frames.push(Frame::unchanged(tick, 1));
    } else {
        let mut body = Vec::new();
        encode_shaped_delta(schema, &block.prior, &deltas, &mut body)
            .map_err(|error| ProductError::new(format!("encode minimal d81 delta: {error:?}")))?;
        block.frames.push(Frame::state(Kind::Delta, tick, body));
    }
    block.prior = next;
    Ok(())
}

fn close_block(
    block: &mut Option<PendingBlock>,
    schema: &wire_format::Schema,
    sink: &mut dyn ArtifactSink,
) -> Result<(), ProductError> {
    let Some(pending) = block.take() else {
        return Ok(());
    };
    let mut writer = BlockWriter::new(
        BlockHeader {
            domain: d81::DOMAIN,
            branch: pending.branch,
            schema_hash: schema.schema_hash(),
            first_tick: pending.first_tick,
        },
        1,
    );
    for frame in pending.frames {
        writer
            .push(frame)
            .map_err(|error| ProductError::new(format!("d81 block invariant failed: {error:?}")))?;
    }
    let bytes = writer
        .finish()
        .map_err(|error| ProductError::new(format!("finish d81 block: {error:?}")))?;
    sink.push(
        format!(
            "rebuilt_wire/blocks/d81-b{}-{:08}.bin",
            pending.branch, pending.index
        ),
        ArtifactRole::StateBlock,
        &bytes,
    )?;
    Ok(())
}

fn status_gap(branch: u32, tick: u64, status_flags: u32) -> Result<GapRecord, ProductError> {
    let reason = if status_flags & compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE != 0 {
        GapReason::SourceUnavailable
    } else if status_flags & compact_contract::COMPACT_STATUS_PARTIAL_DATA != 0 {
        GapReason::PartialData
    } else if status_flags & compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE != 0 {
        GapReason::BridgeUnavailable
    } else if status_flags & compact_contract::COMPACT_STATUS_WORKER_PANIC != 0 {
        GapReason::WorkerPanic
    } else {
        GapReason::CoverageShortfall
    };
    Ok(GapRecord {
        tick_start: tick,
        tick_end: tick
            .checked_add(1)
            .ok_or_else(|| ProductError::new("d81 status gap tick overflows"))?,
        branch,
        reason,
        part: GapPart::State,
        domains: vec![d81::DOMAIN],
        witness: GapWitness::StatusBits {
            bit_width: 32,
            bits: status_flags.to_le_bytes().to_vec(),
        },
    })
}

#[cfg(test)]
mod streaming_gap_tests {
    use super::*;
    use crate::model::BundleEntry;
    use wire_format::gap::decode_record as decode_gap;

    #[derive(Default)]
    struct TestSink {
        descriptors: Vec<BundleEntryDescriptor>,
        bodies: Vec<Vec<u8>>,
    }

    impl ArtifactSink for TestSink {
        fn push(
            &mut self,
            path: String,
            role: ArtifactRole,
            bytes: &[u8],
        ) -> Result<(), ProductError> {
            let entry = BundleEntry::from_bytes(path, role, bytes.to_vec())?;
            self.descriptors.push(BundleEntryDescriptor::from(&entry));
            self.bodies.push(entry.bytes);
            Ok(())
        }

        fn descriptors(&self) -> Vec<BundleEntryDescriptor> {
            self.descriptors.clone()
        }
    }

    fn writer(branch: u32) -> StreamingGapWriter {
        StreamingGapWriter {
            branch,
            next_index: 0,
            pending: None,
        }
    }

    fn gap(branch: u32, start: u64, end: u64, reason: GapReason, witness: GapWitness) -> GapRecord {
        GapRecord {
            tick_start: start,
            tick_end: end,
            branch,
            reason,
            part: GapPart::State,
            domains: vec![d81::DOMAIN],
            witness,
        }
    }

    #[test]
    fn adjacent_identical_coverage_gaps_coalesce_and_final_flush_is_exact() {
        let mut sink = TestSink::default();
        let mut gaps = writer(7);
        gaps.push(coverage_gap(7, 40, 41), &mut sink).unwrap();
        gaps.push(coverage_gap(7, 41, 42), &mut sink).unwrap();
        gaps.push(coverage_gap(7, 42, 43), &mut sink).unwrap();
        assert!(
            sink.descriptors.is_empty(),
            "pending run must remain bounded and unflushed"
        );

        gaps.finish(&mut sink).unwrap();
        assert_eq!(sink.descriptors.len(), 1);
        assert_eq!(
            sink.descriptors[0].path,
            "rebuilt_wire/gaps/d81-b7-00000000.bin"
        );
        assert_eq!(sink.descriptors[0].role, ArtifactRole::GapRecord);
        let decoded = decode_gap(&sink.bodies[0]).unwrap();
        assert_eq!(decoded.tick_start, 40);
        assert_eq!(decoded.tick_end, 43);
        assert_eq!(
            decoded.witness,
            GapWitness::Coverage {
                observed: 0,
                expected: 3,
            }
        );
        let rebuilt = BundleEntry::from_bytes(
            sink.descriptors[0].path.clone(),
            sink.descriptors[0].role,
            sink.bodies[0].clone(),
        )
        .unwrap();
        assert_eq!(sink.descriptors[0], BundleEntryDescriptor::from(&rebuilt));
    }

    #[test]
    fn changed_reason_or_witness_kind_preserves_canonical_ordinals() {
        let mut sink = TestSink::default();
        let mut gaps = writer(3);
        gaps.push(coverage_gap(3, 10, 11), &mut sink).unwrap();
        gaps.push(
            gap(
                3,
                11,
                12,
                GapReason::PartialData,
                GapWitness::StatusBits {
                    bit_width: 8,
                    bits: vec![1],
                },
            ),
            &mut sink,
        )
        .unwrap();
        gaps.push(
            gap(
                3,
                12,
                13,
                GapReason::PartialData,
                GapWitness::TickBracket {
                    start_tick: 12,
                    end_tick: 13,
                },
            ),
            &mut sink,
        )
        .unwrap();
        gaps.finish(&mut sink).unwrap();

        assert_eq!(
            sink.descriptors
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "rebuilt_wire/gaps/d81-b3-00000000.bin",
                "rebuilt_wire/gaps/d81-b3-00000001.bin",
                "rebuilt_wire/gaps/d81-b3-00000002.bin",
            ]
        );
        let decoded = sink
            .bodies
            .iter()
            .map(|bytes| decode_gap(bytes).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            decoded.iter().map(|gap| gap.reason).collect::<Vec<_>>(),
            vec![
                GapReason::CoverageShortfall,
                GapReason::PartialData,
                GapReason::PartialData,
            ]
        );
        assert_eq!(decoded[1].witness.kind(), 1);
        assert_eq!(decoded[2].witness.kind(), 3);
        for (descriptor, body) in sink.descriptors.iter().zip(&sink.bodies) {
            let rebuilt =
                BundleEntry::from_bytes(descriptor.path.clone(), descriptor.role, body.clone())
                    .unwrap();
            assert_eq!(*descriptor, BundleEntryDescriptor::from(&rebuilt));
        }
    }

    #[test]
    fn overlap_or_out_of_order_gap_is_rejected_without_emission() {
        let mut sink = TestSink::default();
        let mut gaps = writer(0);
        gaps.push(coverage_gap(0, 20, 22), &mut sink).unwrap();
        let error = gaps.push(coverage_gap(0, 21, 23), &mut sink).unwrap_err();
        assert!(error
            .to_string()
            .contains("overlap or arrive out of tick order"));
        assert!(sink.descriptors.is_empty());
    }

    #[test]
    fn authenticated_state_gaps_close_block_and_force_next_snapshot_baseline() {
        let schema = d81::schema();
        let symbols = SymbolManifestBuilder::from_base(["province/rgb/1/2/3"]).unwrap();
        let payload =
            compact_contract::rail_replay::encode_payload_with_hub_simulation(&[], &[], 0, 0, 0, 0)
                .unwrap();
        let gaps = [
            gap(
                0,
                11,
                12,
                GapReason::TornSweep,
                GapWitness::TickBracket {
                    start_tick: 11,
                    end_tick: 12,
                },
            ),
            gap(
                0,
                11,
                12,
                GapReason::Backpressure,
                GapWitness::Counter {
                    counter_id: u64::from(
                        compact_contract::gap_record::GAP_COUNTER_CODE_QUEUE_OVERRUN_HOURS,
                    ),
                    observed: 1,
                },
            ),
            gap(
                0,
                11,
                12,
                GapReason::Backpressure,
                GapWitness::Counter {
                    counter_id: u64::from(
                        compact_contract::gap_record::GAP_COUNTER_CODE_SINK_BACKPRESSURE_HOURS,
                    ),
                    observed: 1,
                },
            ),
        ];
        for gap in gaps {
            let mut sink = TestSink::default();
            let mut encoder = BranchStreamEncoder {
                config: BundleBuildConfig::default(),
                schema: &schema,
                symbols: &symbols,
                branch: 0,
                declared_first_tick: 10,
                declared_gaps: Vec::new(),
                sink: &mut sink,
                started: false,
                block: None,
                next_block_index: 0,
                gaps: writer(0),
            };
            encoder.observe(10, payload.clone()).unwrap();
            encoder.observe_gap(gap).unwrap();
            encoder.observe(12, payload.clone()).unwrap();
            encoder.finish().unwrap();

            let blocks = sink
                .descriptors
                .iter()
                .zip(&sink.bodies)
                .filter(|(descriptor, _)| descriptor.role == ArtifactRole::StateBlock)
                .map(|(_, bytes)| wire_format::frame::read_block(bytes, 1).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(blocks.len(), 2);
            assert_eq!(blocks[0].1[0].kind, Kind::Baseline);
            assert_eq!(blocks[0].1[0].tick, 10);
            assert_eq!(blocks[1].1[0].kind, Kind::Baseline);
            assert_eq!(blocks[1].1[0].tick, 12);
            assert_eq!(
                sink.descriptors
                    .iter()
                    .filter(|entry| entry.role == ArtifactRole::GapRecord)
                    .count(),
                1
            );
        }
    }
}
