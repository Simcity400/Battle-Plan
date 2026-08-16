//! The leaf's own suite.
//!
//! Three kinds of test live here, and the difference matters:
//!
//! 1. **Tiling drift-killers.** Most of them are `const` assertions in
//!    `lib.rs` and `family.rs` and are proved by the BUILD, not by this file --
//!    that is stronger than a test, because a table that stops tiling cannot be
//!    compiled at all. What is left for runtime is what a `const` context cannot
//!    reach: that each family's fixture set exercises every version and every
//!    flag its table declares, so a hand-written fixture list cannot silently
//!    stop covering a new one.
//!
//! 2. **Byte goldens, written out here from the documented layout.** Every
//!    fixture in this file is assembled by [`Bytes`], which pushes little-endian
//!    words at offsets stated in the test. NONE of them is produced by
//!    `crate::encode`. That is the whole point of playbook step 8: a round-trip
//!    cannot catch a mistake both sides make, so the encoder is checked AGAINST
//!    these bytes rather than used to make them.
//!
//! 3. **Structural refusals.** One per [`ViolationKind`], each on a fixture that
//!    is legal except for the one fact under test.

use alloc::vec::Vec;
use alloc::{format, vec};

use crate::encode;
use crate::*;

/// The two budgets this tree ships, restated here ONLY because a `no_std` leaf
/// under test has no `compact_contract` to read them from. The production sites
/// pass `compact_contract::build_profile::MAX_COMPACT_STRATEGIC_RESOURCE_COUNT`
/// and `COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE`; the registry row in
/// `verify_world_fold_citations.py` records that, so this pair cannot drift
/// without the gate noticing that a site stopped naming those identifiers.
const TEST_LIMITS: Limits = Limits::new(64, 64);

// =============================================================================
// A byte builder, so every fixture is written out rather than encoded
// =============================================================================

#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn new() -> Self {
        Bytes(Vec::new())
    }
    fn at(&self) -> usize {
        self.0.len()
    }
    fn magic(&mut self, m: &[u8; 4]) -> &mut Self {
        self.0.extend_from_slice(m);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i64(&mut self, v: i64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn zeros(&mut self, n: usize) -> &mut Self {
        for _ in 0..n {
            self.0.push(0);
        }
        self
    }
    fn raw(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    fn done(&self) -> Vec<u8> {
        self.0.clone()
    }
}

/// A core payload with `row_count` rows, written out at the documented offsets.
fn core_bytes(rows: &[[u64; 3]]) -> Bytes {
    let mut b = Bytes::new();
    b.u64(7); // elapsed_wall_ms
    b.u32(rows.len() as u32); // +8  row_count
    b.u32(0); // +12 changed_rows
    b.u32(0); // +16 status_flags
    b.u32(rows.len() as u32); // +20 observed_contract_rows
    b.u32(rows.len() as u32); // +24 emitted_contract_rows
    b.u32(0); // +28 buyer_ptr_valid_rows
    b.u32(0); // +32 variant_valid_rows
    b.u32(0); // +36 amount_valid_rows
    b.u32(rows.len() as u32); // +40 partial_field_rows
    b.u32(FIXED_SCALE_RAW); // +44 scale_raw
    for row in rows {
        b.u32(row[0] as u32); // +0  contract_id
        b.u32(CORE_CONTRACT_TYPE); // +4  contract_type
        b.u32(row[1] as u32); // +8  flags
        b.u32(11); // +12 seller_tag_db_id
        b.u32(12); // +16 buyer_tag_db_id
        b.u32(0); // +20 equipment_variant_id
        b.i64(row[2] as i64); // +24 amount_raw
        b.u32(0); // +32 equipment_subbuf_status
        b.u32(0); // +36 delivery_subbuf_status
        b.u32(0); // +40 buyer_ptr_status
        b.u32(0); // +44 reserved
    }
    b
}

/// Every declared block of `body`, reached and explicitly discarded.
///
/// The only two routes past a trailer are walking every block and discarding
/// every block, so a test that cares about one family has to say which it is
/// doing for the rest. That is the property under test, exercised by the test
/// harness itself.
fn discard_body(body: TrailerBody) -> TrailerResume {
    match body {
        TrailerBody::Detail(c) => c.rows().discard_all().next,
        TrailerBody::ResourceFlow(c) => c.rows().discard_all().next,
        TrailerBody::FactoryHeader(c) => c.rows().discard_all().next,
        TrailerBody::Residual(c) => c.rows().discard_all().next,
        TrailerBody::LendLease(c) => c.rows().discard_all().next,
        TrailerBody::ResourceRow(c) => {
            c.identities()
                .discard_all()
                .next
                .rows()
                .discard_all()
                .next
        }
        TrailerBody::ClassicDeal(c) => {
            c.witnesses()
                .discard_all()
                .next
                .incomplete_owners()
                .discard_all()
                .next
                .rows()
                .discard_all()
                .next
        }
        TrailerBody::Stockpile(c) => c.countries().discard_all().next.items().discard_all().next,
        TrailerBody::Activation(c) => c.end().next,
    }
}

/// Walk every trailer of `payload`, handing each family's owned cursor to
/// `visit`, which must hand back the [`TrailerResume`] it can obtain in no way
/// other than consuming every declared block.
fn visit_trailers<F>(payload: &[u8], mut visit: F) -> Result<usize, Violation>
where
    F: FnMut(&[u8], usize, TrailerBody) -> Result<TrailerResume, Violation>,
{
    let header = read_core_header(payload)?;
    let core = core_end(payload, header.row_count())?;
    let mut cursor = TrailerCursor::new(core);
    loop {
        match cursor.next(payload)? {
            TrailerStep::End(at) => return Ok(at),
            TrailerStep::Magic { at, .. } => {
                let body = cursor.read_body(payload, TEST_LIMITS)?;
                cursor = visit(payload, at, body)?.resume();
            }
        }
    }
}

fn walk_all(payload: &[u8]) -> Result<Vec<(Trailer, usize, usize)>, Violation> {
    let mut seen = Vec::new();
    let end = visit_trailers(payload, |_, at, body| {
        let trailer = body.trailer();
        let resume = discard_body(body);
        seen.push((trailer, at, resume.end()));
        Ok(resume)
    })?;
    exact_consumption(end, payload.len())?;
    Ok(seen)
}

// =============================================================================
// 1. Coverage drift-killers
// =============================================================================

/// Every trailer this grammar declares has a fixture in this file.
///
/// A tenth family added to `Trailer::ALL` and not given a fixture fails HERE,
/// which is the failure a hand-written fixture list otherwise never produces.
#[test]
fn every_declared_trailer_has_a_handwritten_fixture() {
    let mut covered = Vec::new();
    for (payload, _) in every_fixture() {
        for (trailer, _, _) in walk_all(&payload).expect("fixture walks") {
            if !covered.contains(&trailer) {
                covered.push(trailer);
            }
        }
    }
    for trailer in Trailer::ALL {
        assert!(
            covered.contains(&trailer),
            "no handwritten fixture carries {}",
            trailer.label()
        );
    }
}

/// Every VERSION of every multi-version family has a fixture.
#[test]
fn every_declared_version_has_a_handwritten_fixture() {
    let mut flow = Vec::new();
    let mut resource = Vec::new();
    let mut deal = Vec::new();
    for (payload, _) in every_fixture() {
        visit_trailers(&payload, |_, _, body| {
            match &body {
                TrailerBody::ResourceFlow(c) => flow.push(c.header().version),
                TrailerBody::ResourceRow(c) => resource.push(c.header().version),
                TrailerBody::ClassicDeal(c) => deal.push(c.header().version),
                _ => {}
            }
            Ok(discard_body(body))
        })
        .expect("walk");
    }
    for v in FlowVersion::ALL {
        assert!(flow.contains(&v), "no fixture carries MTRF {:?}", v);
    }
    for v in ResourceRowVersion::ALL {
        assert!(resource.contains(&v), "no fixture carries MTRR {:?}", v);
    }
    for v in DealVersion::ALL {
        assert!(deal.contains(&v), "no fixture carries MTRD {:?}", v);
    }
}

/// The union of every fixture's core flag word is exactly `KNOWN_ROW_FLAGS`.
///
/// This is d68's drift-killer, applied to d43's core row: a sixth flag added to
/// `RowFlag::ALL` and never set by a fixture fails here.
#[test]
fn core_flag_fixtures_cover_exactly_the_known_mask() {
    let mut union = 0u32;
    for (payload, _) in every_fixture() {
        let header = read_core_header(&payload).expect("header");
        let mut rows = CoreRowCursor::new(header.row_count());
        while let CoreRowStep::Row { row, .. } = rows.next(&payload) {
            union |= row.flags;
        }
    }
    assert_eq!(
        union, KNOWN_ROW_FLAGS,
        "core-row fixtures cover {union:#x}, the table declares {KNOWN_ROW_FLAGS:#x}"
    );
}

/// The union of every fixture's MTD1 flag word is exactly `KNOWN_DETAIL_FLAGS`.
#[test]
fn detail_flag_fixtures_cover_exactly_the_known_mask() {
    let mut union = 0u32;
    for (payload, _) in every_fixture() {
        visit_trailers(&payload, |payload, _, body| {
            let TrailerBody::Detail(c) = body else {
                return Ok(discard_body(body));
            };
            let mut rows = c.rows();
            while let RowStep::Row { at, .. } = rows.next() {
                union |= detail_row_at(payload, at).flags;
            }
            Ok(rows.into_end()?.next)
        })
        .expect("walk");
    }
    assert_eq!(union, KNOWN_DETAIL_FLAGS);
}

/// The union of every fixture's MTRR flag word is exactly
/// `KNOWN_RESOURCE_FLAGS`, and every `NeededType` is exercised.
#[test]
fn resource_row_fixtures_cover_every_flag_and_needed_type() {
    let mut union = 0u32;
    let mut kinds = Vec::new();
    for (payload, _) in every_fixture() {
        visit_trailers(&payload, |payload, _, body| {
            let TrailerBody::ResourceRow(c) = body else {
                return Ok(discard_body(body));
            };
            let mut rows = c.identities().discard_all().next.rows();
            while let RowStep::Row { at, .. } = rows.next() {
                let row = resource_row_at(payload, at);
                union |= row.flags;
                if let Some(kind) = NeededType::from_wire(row.needed_type) {
                    if !kinds.contains(&kind) {
                        kinds.push(kind);
                    }
                }
            }
            Ok(rows.into_end()?.next)
        })
        .expect("walk");
    }
    assert_eq!(union, KNOWN_RESOURCE_FLAGS);
    for kind in NeededType::ALL {
        assert!(kinds.contains(&kind), "no fixture states {:?}", kind);
    }
}

/// Every `FlowKind`, `DealDirection`, `DealRoute` and `DealStatus` bit appears
/// in a fixture.
#[test]
fn deal_and_flow_vocabularies_are_covered() {
    let mut flow_kinds = Vec::new();
    let mut directions = Vec::new();
    let mut routes = Vec::new();
    let mut status_union = 0u32;
    for (payload, _) in every_fixture() {
        visit_trailers(&payload, |payload, _, body| match body {
            TrailerBody::ResourceFlow(c) => {
                let mut rows = c.rows();
                while let RowStep::Row { at, .. } = rows.next() {
                    if let Some(kind) = FlowKind::from_wire(flow_row_at(payload, at).flow_kind) {
                        if !flow_kinds.contains(&kind) {
                            flow_kinds.push(kind);
                        }
                    }
                }
                Ok(rows.into_end()?.next)
            }
            TrailerBody::ClassicDeal(c) => {
                let version = c.header().version;
                let mut rows = c
                    .witnesses()
                    .discard_all()
                    .next
                    .incomplete_owners()
                    .discard_all()
                    .next
                    .rows();
                while let RowStep::Row { at, .. } = rows.next() {
                    let row = deal_row_at(payload, at, version);
                    if let Some(d) = DealDirection::from_wire(row.direction) {
                        if !directions.contains(&d) {
                            directions.push(d);
                        }
                    }
                    if let Some(r) = DealRoute::from_wire(row.route_kind) {
                        if !routes.contains(&r) {
                            routes.push(r);
                        }
                    }
                    if version.carries_route_status() {
                        status_union |= row.route_status_or_reserved;
                    }
                }
                Ok(rows.into_end()?.next)
            }
            _ => Ok(discard_body(body)),
        })
        .expect("walk");
    }
    for kind in FlowKind::ALL {
        assert!(flow_kinds.contains(&kind), "no fixture states {:?}", kind);
    }
    for d in DealDirection::ALL {
        assert!(directions.contains(&d), "no fixture states {:?}", d);
    }
    for r in DealRoute::ALL {
        assert!(routes.contains(&r), "no fixture states {:?}", r);
    }
    assert_eq!(status_union, KNOWN_DEAL_STATUS);
}

// =============================================================================
// 2. The handwritten fixtures
// =============================================================================

fn mtd1_v1(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTD1");
    b.u16(1); // +4  version
    b.u16(32); // +6  header_len
    b.u16(64); // +8  row_len
    b.u16(0); // +10 reserved0
    b.u32(2); // +12 row_count
    b.u32(1); // +16 price_valid_rows
    b.u32(1); // +20 speed_valid_rows
    b.u32(1); // +24 delivery_route_valid_rows
    b.u32(1); // +28 progress_valid_rows
              // Two rows whose flag words together cover every declared MTD1 bit.
    for (contract_id, flags) in [(1u32, 0b0000011111u32), (2u32, 0b1111100000u32)] {
        b.u32(contract_id); // +0
        b.u32(flags); // +4
        b.i64(100_000); // +8  price_raw
        b.i64(200_000); // +16 amount_raw
        b.i32(3); // +24 speed_raw
        b.i32(4); // +28 completed_raw
        b.i32(5); // +32 days
        b.u32(12); // +36 delivery_country_tag_db_id
        b.i32(6); // +40 convoys_assigned
        b.i32(7); // +44 convoys_total
        b.i32(8); // +48 efficiency_raw
        b.i32(9); // +52 factory_cic_progress_raw
        b.i32(10); // +56 equipment_transfer_progress_raw
        b.u32(0); // +60 reserved
    }
    assert_eq!(b.at() - start, 32 + 2 * 64);
}

fn mtrf(b: &mut Bytes, version: FlowVersion, resource_count: u32, kinds: &[u32]) {
    let start = b.at();
    let (row_len, mask_or_count, kind_mask) = match version {
        FlowVersion::Legacy => (72usize, FLOW_RESOURCE_MASK, 0u32),
        FlowVersion::Fixed7 => (72, FLOW_RESOURCE_MASK, FLOW_KIND_MASK),
        FlowVersion::Variable => (
            16 + 8 * resource_count as usize,
            resource_count,
            FLOW_KIND_MASK,
        ),
    };
    let values = match version {
        FlowVersion::Legacy | FlowVersion::Fixed7 => 7,
        FlowVersion::Variable => resource_count as usize,
    };
    b.magic(b"MTRF");
    b.u16(version.wire()); // +4
    b.u16(32); // +6  header_len
    b.u16(row_len as u16); // +8  row_len
    b.u16(0); // +10 reserved0
    b.u32(kinds.len() as u32); // +12 row_count
    b.u32(match version {
        // v1 states the exported-row count here; v2/v3 state the country count.
        FlowVersion::Legacy => kinds.len() as u32,
        FlowVersion::Fixed7 | FlowVersion::Variable => (kinds.len() / 6) as u32,
    }); // +16
    b.u32(FIXED_SCALE_RAW); // +20 scale_raw
    b.u32(mask_or_count); // +24
    b.u32(kind_mask); // +28
    for kind in kinds {
        b.u32(21); // +0  tag_db_id
        b.u32(*kind); // +4  flow_kind
        b.u32(mask_or_count); // +8
        b.u32(0); // +12 reserved
        for slot in 0..values {
            b.i64(1_000 + slot as i64);
        }
    }
    assert_eq!(b.at() - start, 32 + kinds.len() * row_len);
}

fn mtrh_v1(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTRH");
    b.u16(1); // +4  version
    b.u16(24); // +6  header_len
    b.u16(16); // +8  row_len
    b.u16(0); // +10 reserved0
    b.u32(1); // +12 row_count
    b.u32(1); // +16 country_count
    b.u32(0); // +20 reserved1
    b.u32(21); // +0  tag_db_id
    b.i32(10); // +4  total
    b.i32(4); // +8  used
    b.i32(6); // +12 available
    assert_eq!(b.at() - start, 24 + 16);
}

/// `MTRR` at any version. `tokens` is empty for the versions with no identity
/// block; `rows` is `(resource_index, needed_type, flags)`.
fn mtrr(
    b: &mut Bytes,
    version: ResourceRowVersion,
    resource_count: u32,
    tokens: &[&str],
    countries: &[u32],
    rows: &[(u32, u32, u32)],
) {
    let start = b.at();
    let stride = TEST_LIMITS.identity_stride();
    b.magic(b"MTRR");
    b.u16(version.wire()); // +4
    b.u16(32); // +6  header_len
    b.u16(64); // +8  row_len
    b.u16(0); // +10 reserved0
    b.u32((countries.len() * rows.len()) as u32); // +12 row_count
    b.u32(countries.len() as u32); // +16 country_count
    b.u32(resource_count); // +20 resource_count
    b.u32(1); // +24 display_scale
    b.u32(if version.carries_identity() {
        stride as u32
    } else {
        0
    }); // +28
    for token in tokens {
        b.u16(token.len() as u16);
        b.raw(token.as_bytes());
        b.zeros(stride - 2 - token.len());
    }
    for country in countries {
        for (resource_index, needed_type, flags) in rows {
            b.u32(*country); // +0  tag_db_id
            b.u32(*resource_index); // +4  resource_index
            b.u32(*resource_index); // +8  visible_order
            b.u32(*needed_type); // +12 needed_type
            b.i32(5); // +16 balance
            b.i32(5); // +20 produced
            b.i32(0); // +24 imported
            b.i32(0); // +28 exported
            b.i32(0); // +32 special_project
            b.i32(0); // +36 production_line
            b.i32(5); // +40 needed
            b.i32(0); // +44 needed_current
            b.i32(0); // +48 needed_max
            b.u32(*flags); // +52 flags
            b.u32(resource_index + 1); // +56 descriptor_db_index
            b.u32(0); // +60 reserved
        }
    }
    let identity = if version.carries_identity() {
        resource_count as usize * stride
    } else {
        0
    };
    assert_eq!(
        b.at() - start,
        32 + identity + countries.len() * rows.len() * 64
    );
}

fn mtrd(b: &mut Bytes, version: DealVersion, witnesses: &[u32], incomplete: &[u32]) {
    mtrd_with_routes(
        b,
        version,
        witnesses,
        incomplete,
        [DealRoute::Sea.wire(), DealRoute::Land.wire()],
    )
}

fn mtrd_with_routes(
    b: &mut Bytes,
    version: DealVersion,
    witnesses: &[u32],
    incomplete: &[u32],
    routes: [u32; 2],
) {
    let start = b.at();
    b.magic(b"MTRD");
    b.u16(version.wire()); // +4
    b.u16(32); // +6  header_len
    b.u16(96); // +8  row_len
    b.u16(0); // +10 reserved0
    b.u32(2); // +12 row_count -- one deal, two halves
    b.u32(witnesses.len() as u32); // +16 country_count
    b.u32(FIXED_SCALE_RAW); // +20 scale_raw
    b.u32(witnesses.len() as u32); // +24 witness_count
    b.u32(incomplete.len() as u32); // +28 incomplete_count
    for w in witnesses {
        b.u32(*w);
    }
    for owner in incomplete {
        b.u32(*owner);
    }
    // Two halves of one deal: an exporter half and an importer half, with the
    // two route kinds and both status bits split across them so the vocabulary
    // drift-killer sees all of them.
    let exporter = witnesses[0];
    let importer = witnesses[1];
    for (half, (owner, counterpart, direction, route, status)) in [
        (
            exporter,
            importer,
            DealDirection::Export.wire(),
            routes[0],
            DealStatus::NoTradeInfluence.bit(),
        ),
        (
            importer,
            exporter,
            DealDirection::Import.wire(),
            routes[1],
            DealStatus::NoResources.bit(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let _ = half;
        b.u32(owner); // +0  owner_tag_db_id
        b.u32(counterpart); // +4  counterpart_tag_db_id
        b.u32(1); // +8  resource_index
        b.u32(0); // +12 visible_order
        b.u32(direction); // +16 direction
        b.u32(route); // +20 route_kind
        b.u32(DEAL_FLAG_EXACT); // +24 flags
        b.u32(if version.carries_route_status() {
            status
        } else {
            0
        }); // +28
        match version.identity_shape() {
            DealIdentityShape::Wide => {
                b.u32(exporter); // +32 identity exporter
                b.u32(importer); // +36 identity importer
            }
            DealIdentityShape::Packed => {
                let packed = ((exporter as u64) << 32)
                    | ((importer as u64) << 16)
                    | (1u64 << 8) // the resource byte the row repeats
                    | 1u64; // the eight-bit ordinal
                b.u64(packed); // +32
            }
        }
        b.i64(FIXED_SCALE_RAW as i64); // +40 requested_raw
        b.i64(0); // +48 delivered_raw
        b.i64(0); // +56 available_export_raw
        b.i64(0); // +64 factory_cost_raw
        b.i64(0); // +72 efficiency_raw
        b.i32(0); // +80 civilian_factories
        b.i32(0); // +84 convoys_assigned
        b.i32(0); // +88 convoys_required
        match version.identity_shape() {
            DealIdentityShape::Wide => b.u32(1), // +92 the one-based ordinal
            DealIdentityShape::Packed => b.u32(0), // +92 reserved zero
        };
    }
    assert_eq!(
        b.at() - start,
        32 + witnesses.len() * 4 + incomplete.len() * 4 + 2 * 96
    );
}

fn mtra_v1(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTRA");
    b.u16(1); // +4  version
    b.u16(32); // +6  header_len
    b.u32(0b11); // +8  flags: active | root_exact
    b.u32(21); // +12 country_tag_db_id
    b.u64(9); // +16 observation_sequence
    b.u32(1); // +24 root_kind
    b.u32(0x015e_c520); // +28 open_rva
    assert_eq!(b.at() - start, 32);
}

fn mts1_v1(b: &mut Bytes) {
    let start = b.at();
    let segment = 40 + 1 * 20 + 2 * 24;
    b.magic(b"MTS1");
    b.u16(1); // +4  version
    b.u16(40); // +6  header_len
    b.u16(20); // +8  country_row_len
    b.u16(24); // +10 item_row_len
    b.u32(segment as u32); // +12 segment_len
    b.u32(1); // +16 country_count
    b.u32(2); // +20 item_count
    b.u32(1); // +24 expected_country_count
    b.u32(FIXED_SCALE_RAW); // +28 scale_raw
    b.u32(STOCKPILE_GLOBAL_FLAGS); // +32 flags
    b.u32(0); // +36 reserved
    b.u32(21); // +0  tag_db_id
    b.u32(0); // +4  item_first
    b.u32(2); // +8  item_count
    b.u32(STOCKPILE_COUNTRY_COMPLETE); // +12 flags
    b.u32(0); // +16 reserved
    for ordinal in 0..2u32 {
        b.u32(100 + ordinal); // +0  equipment_variant_id
        b.u32(STOCKPILE_EQUIPMENT_TYPE); // +4  variant_type
        b.u32(ordinal); // +8  ordinal
        b.u32(0); // +12 reserved
        b.i64(5 * FIXED_SCALE_RAW as i64); // +16 amount_raw
    }
    assert_eq!(b.at() - start, segment);
}

fn mtrb_v1(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTRB");
    b.u16(1); // +4  version
    b.u16(32); // +6  header_len
    b.u16(48); // +8  row_len
    b.u16(0); // +10 reserved0
    b.u32(1); // +12 row_count
    b.u32(1); // +16 source_blocked_rows
    b.u32(0); // +20 schema_blocked_rows
    b.u32(0); // +24 owned_elsewhere_rows
    b.u32(0); // +28 shared_patch_blocked_rows
    for word in 0..12u32 {
        b.u32(word);
    }
    assert_eq!(b.at() - start, 32 + 48);
}

fn mtll_v1(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTLL");
    b.u16(1); // +4  version
    b.u16(32); // +6  header_len
    b.u16(48); // +8  row_len
    b.u16(0); // +10 reserved0
    b.u32(1); // +12 row_count
    b.u32(1); // +16 outgoing_rows
    b.u32(0); // +20 incoming_rows
    b.u32(1); // +24 amount_valid_rows
    b.u32(0); // +28 price_valid_rows
    b.u32(1); // +0  contract_id
    b.u32(11); // +4  perspective_tag_db_id
    b.u32(12); // +8  counterpart_tag_db_id
    b.u32(1); // +12 direction
    b.u32(100); // +16 equipment_variant_id
    b.u32(1); // +20 flags
    b.i64(FIXED_SCALE_RAW as i64); // +24 amount_raw
    b.i64(0); // +32 price_raw
    b.u32(0); // +40 source_record_kind
    b.u32(0); // +44 reserved
    assert_eq!(b.at() - start, 32 + 48);
}

/// Every handwritten fixture, with a name, so the coverage tests above can
/// iterate them and no family can quietly lose its only fixture.
fn every_fixture() -> Vec<(Vec<u8>, &'static str)> {
    let mut out = Vec::new();

    // A core-only payload: no trailers at all.
    out.push((core_bytes(&[[1, 0b11111, 5]]).done(), "core only"));

    // Each family alone behind the core. MTRF is deliberately FIRST in its own
    // fixture, with no MTD1 in front of it: the dispatch is by magic and does
    // not require an optional predecessor.
    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Variable, 3, &[1, 2, 3, 4, 5, 6]);
    out.push((b.done(), "MTRF v3 alone, no MTD1 predecessor"));

    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Legacy, 7, &[FlowKind::Exported.wire()]);
    out.push((b.done(), "MTRF v1 legacy fixed-seven"));

    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Fixed7, 7, &[1, 2, 3, 4, 5, 6]);
    out.push((b.done(), "MTRF v2 fixed-seven six-kind matrix"));

    let mut b = core_bytes(&[]);
    mtd1_v1(&mut b);
    out.push((b.done(), "MTD1 v1"));

    let mut b = core_bytes(&[]);
    mtrh_v1(&mut b);
    out.push((b.done(), "MTRH v1"));

    // MTRR v1: seven resources, no identity block.
    let mut b = core_bytes(&[]);
    let seven: Vec<(u32, u32, u32)> = (0..7)
        .map(|i| {
            if i == 0 {
                (i, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL)
            } else {
                (i, NeededType::Surplus.wire(), 0)
            }
        })
        .collect();
    mtrr(&mut b, ResourceRowVersion::Legacy, 7, &[], &[21], &seven);
    out.push((b.done(), "MTRR v1 fixed-seven, no identity"));

    // MTRR v2: a stated count, still no identity block.
    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::CountOnly,
        3,
        &[],
        &[21],
        &[
            (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
            (1, NeededType::Surplus.wire(), 0),
            (2, NeededType::Deficit.wire(), 0),
        ],
    );
    out.push((b.done(), "MTRR v2 count-only, no identity"));

    // MTRR v3 IDENTITY-ONLY: the shape the producer emits when it discovered
    // the resource set but its per-country rows failed. Zero rows, and legal.
    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::Identity,
        3,
        &["oil", "steel", "rubber"],
        &[],
        &[],
    );
    out.push((b.done(), "MTRR v3 identity-only, zero rows"));

    // MTRR v3 with identity AND dense rows.
    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::Identity,
        3,
        &["oil", "steel", "rubber"],
        &[21, 22],
        &[
            (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
            (1, NeededType::Surplus.wire(), 0),
            (2, NeededType::Deficit.wire(), 0),
        ],
    );
    out.push((b.done(), "MTRR v3 identity plus dense rows"));

    // MTRR v4 with a needed-unavailable oil row.
    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::NeededUnavailable,
        2,
        &["oil", "steel"],
        &[21],
        &[
            (
                0,
                NeededType::Unavailable.wire(),
                ResourceFlag::NeededUnavailable.bit(),
            ),
            (1, NeededType::Surplus.wire(), 0),
        ],
    );
    out.push((b.done(), "MTRR v4 needed-unavailable"));

    for version in DealVersion::ALL {
        let mut b = core_bytes(&[]);
        let incomplete: &[u32] = if version.carries_incomplete_owners() {
            &[22]
        } else {
            &[]
        };
        mtrd(&mut b, version, &[21, 22], incomplete);
        out.push((
            b.done(),
            match version {
                DealVersion::Legacy => "MTRD v1 packed identity",
                DealVersion::Statuses => "MTRD v2 packed identity with route status",
                DealVersion::PerOwner => "MTRD v3 witnesses plus incomplete owners",
                DealVersion::WideIdentity => "MTRD v4 wide identity",
            },
        ));
    }

    // A locked route: the third route kind, which no other fixture states.
    let mut b = core_bytes(&[]);
    mtrd_with_routes(
        &mut b,
        DealVersion::WideIdentity,
        &[21, 22],
        &[],
        [DealRoute::Locked.wire(), DealRoute::Locked.wire()],
    );
    out.push((b.done(), "MTRD v4 with a locked route"));

    let mut b = core_bytes(&[]);
    mtra_v1(&mut b);
    out.push((b.done(), "MTRA v1"));

    let mut b = core_bytes(&[]);
    mts1_v1(&mut b);
    out.push((b.done(), "MTS1 v1 with country and item rows"));

    let mut b = core_bytes(&[]);
    mtrb_v1(&mut b);
    out.push((b.done(), "MTRB v1 (lane gate only)"));

    let mut b = core_bytes(&[]);
    mtll_v1(&mut b);
    out.push((b.done(), "MTLL v1 (lane gate only)"));

    // The producer's own order, and the exact reverse of it. Both are legal:
    // the walk dispatches by magic, not by position.
    let mut b = core_bytes(&[[1, 0b11111, 5]]);
    mtd1_v1(&mut b);
    mtrf(&mut b, FlowVersion::Variable, 3, &[1, 2, 3, 4, 5, 6]);
    mtrh_v1(&mut b);
    mtra_v1(&mut b);
    mts1_v1(&mut b);
    out.push((b.done(), "producer order"));

    let mut b = core_bytes(&[[1, 0b11111, 5]]);
    mts1_v1(&mut b);
    mtra_v1(&mut b);
    mtrh_v1(&mut b);
    mtrf(&mut b, FlowVersion::Variable, 3, &[1, 2, 3, 4, 5, 6]);
    mtd1_v1(&mut b);
    out.push((b.done(), "reverse producer order"));

    out
}

/// Every handwritten fixture walks to exact consumption.
#[test]
fn every_fixture_walks_to_exact_consumption() {
    for (payload, name) in every_fixture() {
        let seen = walk_all(&payload).unwrap_or_else(|v| panic!("{name}: {v:?}"));
        if let Some((_, _, end)) = seen.last() {
            assert_eq!(*end, payload.len(), "{name} did not consume its payload");
        }
    }
}

/// The two orders carry the same five families and the same extents, so trailer
/// ORDER is not part of this grammar.
#[test]
fn trailer_order_does_not_change_what_is_delimited() {
    let fixtures = every_fixture();
    let forward = fixtures
        .iter()
        .find(|(_, n)| *n == "producer order")
        .expect("fixture");
    let reverse = fixtures
        .iter()
        .find(|(_, n)| *n == "reverse producer order")
        .expect("fixture");
    let mut a: Vec<Trailer> = walk_all(&forward.0)
        .expect("forward")
        .into_iter()
        .map(|(t, _, _)| t)
        .collect();
    let mut b: Vec<Trailer> = walk_all(&reverse.0)
        .expect("reverse")
        .into_iter()
        .map(|(t, _, _)| t)
        .collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    assert_eq!(forward.0.len(), reverse.0.len());
}

// =============================================================================
// 3. Structural refusals
// =============================================================================

#[test]
fn a_short_core_header_is_refused() {
    let err = read_core_header(&[0u8; 47]).expect_err("short header");
    assert!(matches!(
        err.kind,
        ViolationKind::Need {
            section: Section::CoreHeader,
            needed: CORE_HEADER_LEN,
            available: 47
        }
    ));
}

#[test]
fn a_core_shorter_than_its_own_row_count_is_refused() {
    let mut payload = core_bytes(&[[1, 0, 0]]).done();
    payload.truncate(CORE_HEADER_LEN + 10);
    let header = read_core_header(&payload).expect("header");
    let err = core_end(&payload, header.row_count()).expect_err("short rows");
    assert!(matches!(
        err.kind,
        ViolationKind::Need {
            section: Section::CoreRows,
            ..
        }
    ));
}

#[test]
fn an_unknown_magic_is_refused_before_any_header_is_read() {
    let mut b = core_bytes(&[]);
    b.magic(b"ZZZZ").u16(1).u16(32).u16(64).u16(0).u32(0);
    let err = walk_all(&b.done()).expect_err("unknown magic");
    assert!(matches!(
        err.kind,
        ViolationKind::UnknownTrailerMagic { .. }
    ));
}

#[test]
fn a_truncated_magic_is_refused_rather_than_treated_as_the_end() {
    let mut b = core_bytes(&[]);
    b.raw(b"MTD");
    let err = walk_all(&b.done()).expect_err("truncated magic");
    assert!(matches!(
        err.kind,
        ViolationKind::Need {
            section: Section::TrailerMagic,
            ..
        }
    ));
}

/// Every declared family refuses a duplicate.
///
/// This is the structural disagreement the conversion closes: the fold already
/// refused a duplicate of all seven families, the authority refused only three
/// (`MTD1`, `MTRA`, `MTS1`) and the lane only two. The leaf refuses all nine, so
/// no site can accept a payload another refuses.
#[test]
fn every_family_refuses_a_duplicate() {
    for trailer in Trailer::ALL {
        let mut b = core_bytes(&[]);
        let write = |b: &mut Bytes| match trailer {
            Trailer::Detail => mtd1_v1(b),
            Trailer::ResourceFlow => mtrf(b, FlowVersion::Variable, 3, &[1, 2, 3, 4, 5, 6]),
            Trailer::FactoryHeader => mtrh_v1(b),
            Trailer::ResourceRow => mtrr(
                b,
                ResourceRowVersion::Identity,
                2,
                &["oil", "steel"],
                &[21],
                &[
                    (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
                    (1, NeededType::Surplus.wire(), 0),
                ],
            ),
            Trailer::ClassicDeal => mtrd(b, DealVersion::WideIdentity, &[21, 22], &[22]),
            Trailer::Activation => mtra_v1(b),
            Trailer::Stockpile => mts1_v1(b),
            Trailer::Residual => mtrb_v1(b),
            Trailer::LendLease => mtll_v1(b),
        };
        write(&mut b);
        write(&mut b);
        let err = walk_all(&b.done())
            .unwrap_err_or_panic(&format!("{} duplicate accepted", trailer.label()));
        assert!(
            matches!(err.kind, ViolationKind::DuplicateTrailer { trailer: t, .. } if t == trailer),
            "{} produced {:?}",
            trailer.label(),
            err.kind
        );
    }
}

trait UnwrapErrOrPanic<T> {
    fn unwrap_err_or_panic(self, msg: &str) -> Violation;
}

impl<T> UnwrapErrOrPanic<T> for Result<T, Violation> {
    fn unwrap_err_or_panic(self, msg: &str) -> Violation {
        match self {
            Ok(_) => panic!("{msg}"),
            Err(v) => v,
        }
    }
}

#[test]
fn an_unknown_version_is_refused_rather_than_reinterpreted() {
    for (magic, header_len, row_len) in [
        (b"MTD1", 32u16, 64u16),
        (b"MTRF", 32, 72),
        (b"MTRH", 24, 16),
        (b"MTRR", 32, 64),
        (b"MTRD", 32, 96),
        (b"MTRA", 32, 0),
        (b"MTS1", 40, 20),
    ] {
        let mut b = core_bytes(&[]);
        b.magic(magic).u16(99).u16(header_len).u16(row_len).u16(0);
        b.zeros(header_len as usize - 12);
        let err = walk_all(&b.done()).unwrap_err_or_panic("a version 99 trailer was accepted");
        assert!(
            matches!(err.kind, ViolationKind::UnknownVersion { version: 99, .. }),
            "{:?}",
            err.kind
        );
    }
}

#[test]
fn a_declared_header_len_that_is_not_the_versions_is_refused() {
    let mut b = core_bytes(&[]);
    b.magic(b"MTD1").u16(1).u16(64).u16(64).u16(0).u32(0);
    b.zeros(16);
    let err = walk_all(&b.done()).unwrap_err_or_panic("a wrong header_len was accepted");
    assert!(matches!(
        err.kind,
        ViolationKind::HeaderLen {
            declared: 64,
            expected: 32,
            ..
        }
    ));
}

#[test]
fn a_declared_row_len_that_is_not_the_versions_is_refused() {
    let mut b = core_bytes(&[]);
    b.magic(b"MTD1").u16(1).u16(32).u16(48).u16(0).u32(0);
    b.zeros(16);
    let err = walk_all(&b.done()).unwrap_err_or_panic("a wrong row_len was accepted");
    assert!(matches!(
        err.kind,
        ViolationKind::RowLen {
            declared: 48,
            expected: 64,
            ..
        }
    ));
}

/// An `MTRF` v3 whose resource count is outside the ceiling is refused BEFORE
/// its row width is computed from it.
#[test]
fn an_mtrf_v3_resource_count_outside_the_ceiling_is_refused() {
    for count in [0u32, TEST_LIMITS.max_resource_count() + 1] {
        let mut b = core_bytes(&[]);
        b.magic(b"MTRF")
            .u16(3)
            .u16(32)
            .u16(flow_row_len(count) as u16)
            .u16(0)
            .u32(0)
            .u32(0)
            .u32(FIXED_SCALE_RAW)
            .u32(count)
            .u32(FLOW_KIND_MASK);
        let err = walk_all(&b.done()).unwrap_err_or_panic("an out-of-range count was accepted");
        assert!(
            matches!(err.kind, ViolationKind::ResourceCount { declared, .. } if declared == count),
            "{:?}",
            err.kind
        );
    }
}

/// An `MTRR` v1/v2 that states an identity stride is refused: `+28` means
/// something else in the version that has it, so sizing it either way would be
/// reading a payload under a grammar it did not declare.
#[test]
fn an_mtrr_without_an_identity_block_may_not_declare_one() {
    let mut b = core_bytes(&[]);
    b.magic(b"MTRR")
        .u16(ResourceRowVersion::CountOnly.wire())
        .u16(32)
        .u16(64)
        .u16(0)
        .u32(0)
        .u32(0)
        .u32(3)
        .u32(1)
        .u32(TEST_LIMITS.identity_stride() as u32);
    let err = walk_all(&b.done()).unwrap_err_or_panic("a v2 identity stride was accepted");
    assert!(matches!(
        err.kind,
        ViolationKind::AbsentBlockDeclared { .. }
    ));
}

/// An `MTRD` v1/v2 that states an incomplete-owner count is refused, which is
/// how a v3 payload mislabelled v2 fails instead of parsing its completeness
/// block as deal rows.
#[test]
fn an_mtrd_without_an_incomplete_block_may_not_declare_one() {
    let mut b = core_bytes(&[]);
    b.magic(b"MTRD")
        .u16(DealVersion::Statuses.wire())
        .u16(32)
        .u16(96)
        .u16(0)
        .u32(0)
        .u32(0)
        .u32(FIXED_SCALE_RAW)
        .u32(0)
        .u32(1);
    let err = walk_all(&b.done()).unwrap_err_or_panic("a v2 incomplete count was accepted");
    assert!(matches!(
        err.kind,
        ViolationKind::AbsentBlockDeclared { declared: 1, .. }
    ));
}

/// Every family refuses a payload one byte shorter than its own declared extent.
#[test]
fn every_family_refuses_a_payload_one_byte_short() {
    for (payload, name) in every_fixture() {
        if payload.len() <= CORE_HEADER_LEN {
            continue;
        }
        let mut short = payload.clone();
        short.pop();
        let err = walk_all(&short)
            .unwrap_err_or_panic(&format!("{name} accepted a payload one byte short"));
        assert!(
            matches!(
                err.kind,
                ViolationKind::Need { .. } | ViolationKind::TrailingBytes { .. }
            ),
            "{name}: {:?}",
            err.kind
        );
    }
}

/// Trailing bytes that are not a trailer are refused by exact consumption.
#[test]
fn trailing_bytes_past_the_last_trailer_are_refused() {
    let mut b = core_bytes(&[]);
    mtra_v1(&mut b);
    let mut payload = b.done();
    payload.extend_from_slice(&[0, 0, 0]);
    let err = walk_all(&payload).unwrap_err_or_panic("trailing bytes accepted");
    assert!(matches!(
        err.kind,
        ViolationKind::Need {
            section: Section::TrailerMagic,
            ..
        }
    ));
}

// =============================================================================
// 4. The encoder is checked AGAINST the handwritten bytes
// =============================================================================

#[test]
fn the_encoder_reproduces_the_handwritten_core_bytes() {
    let rows = [CoreRow {
        contract_id: 1,
        contract_type: CORE_CONTRACT_TYPE,
        flags: 0b11111,
        seller_tag_db_id: 11,
        buyer_tag_db_id: 12,
        equipment_variant_id: 0,
        amount_raw: 5,
        equipment_subbuf_status: 0,
        delivery_subbuf_status: 0,
        buyer_ptr_status: 0,
        reserved: 0,
    }];
    let encoded = encode::encode_core(
        7,
        encode::CoreCounts {
            changed_rows: 0,
            status_flags: 0,
            observed_contract_rows: 1,
            emitted_contract_rows: 1,
            buyer_ptr_valid_rows: 0,
            variant_valid_rows: 0,
            amount_valid_rows: 0,
            partial_field_rows: 1,
        },
        &rows,
    )
    .expect("encodes");
    assert_eq!(encoded, core_bytes(&[[1, 0b11111, 5]]).done());
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtra_bytes() {
    let mut b = Bytes::new();
    mtra_v1(&mut b);
    assert_eq!(
        encode::encode_activation(0b11, 21, 9, 1, 0x015e_c520),
        b.done()
    );
}

#[test]
fn the_encoder_reproduces_the_handwritten_mts1_bytes() {
    let mut b = Bytes::new();
    mts1_v1(&mut b);
    let countries = [StockpileCountryRow {
        tag_db_id: 21,
        item_first: 0,
        item_count: 2,
        flags: STOCKPILE_COUNTRY_COMPLETE,
        reserved: 0,
    }];
    let items: Vec<StockpileItemRow> = (0..2u32)
        .map(|ordinal| StockpileItemRow {
            equipment_variant_id: 100 + ordinal,
            variant_type: STOCKPILE_EQUIPMENT_TYPE,
            ordinal,
            reserved: 0,
            amount_raw: 5 * FIXED_SCALE_RAW as i64,
        })
        .collect();
    assert_eq!(
        encode::encode_stockpile(1, &countries, &items).expect("encodes"),
        b.done()
    );
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtrh_bytes() {
    let mut b = Bytes::new();
    mtrh_v1(&mut b);
    let rows = [FactoryRow {
        tag_db_id: 21,
        total: 10,
        used: 4,
        available: 6,
    }];
    assert_eq!(
        encode::encode_factory_header(&rows).expect("encodes"),
        b.done()
    );
}

/// Every MTRF version the encoder can write is byte-identical to the
/// handwritten fixture for that version.
#[test]
fn the_encoder_reproduces_the_handwritten_mtrf_bytes_at_every_version() {
    for (version, resource_count, kinds) in [
        (FlowVersion::Legacy, 7u32, vec![FlowKind::Exported.wire()]),
        (FlowVersion::Fixed7, 7, vec![1, 2, 3, 4, 5, 6]),
        (FlowVersion::Variable, 3, vec![1, 2, 3, 4, 5, 6]),
    ] {
        let mut b = Bytes::new();
        mtrf(&mut b, version, resource_count, &kinds);
        let values: Vec<i64> = (0..match version {
            FlowVersion::Legacy | FlowVersion::Fixed7 => 7,
            FlowVersion::Variable => resource_count as usize,
        })
            .map(|slot| 1_000 + slot as i64)
            .collect();
        let rows: Vec<encode::FlowRowInput<'_>> = kinds
            .iter()
            .map(|kind| encode::FlowRowInput {
                tag_db_id: 21,
                flow_kind: *kind,
                values_raw: &values,
            })
            .collect();
        let country_word = match version {
            FlowVersion::Legacy => kinds.len() as u32,
            FlowVersion::Fixed7 | FlowVersion::Variable => (kinds.len() / 6) as u32,
        };
        assert_eq!(
            encode::encode_resource_flow(version, resource_count, country_word, &rows, TEST_LIMITS)
                .expect("encodes"),
            b.done(),
            "MTRF {version:?}"
        );
    }
}

/// The encoder refuses to write an `MTRD` v1/v2 with incomplete owners, and
/// refuses an identity that does not fit the packed key.
#[test]
fn the_encoder_refuses_the_structurally_impossible_mtrd_shapes() {
    let err = encode::encode_classic_deals(DealVersion::Statuses, &[21, 22], &[22], &[])
        .expect_err("v2 cannot carry incomplete owners");
    assert!(matches!(
        err,
        encode::EncodeError::IncompleteOwnersUnsupported {
            version: 2,
            owners: 1
        }
    ));

    let row = DealRow {
        owner_tag_db_id: 21,
        counterpart_tag_db_id: 22,
        resource_index: 1,
        visible_order: 0,
        direction: DealDirection::Export.wire(),
        route_kind: DealRoute::Sea.wire(),
        flags: DEAL_FLAG_EXACT,
        route_status_or_reserved: 0,
        // Wider than the packed key's sixteen bits.
        identity_exporter_tag_db_id: 70_000,
        identity_importer_tag_db_id: 22,
        identity_ordinal: 1,
        packed_identity: None,
        reserved_ordinal_word: None,
        requested_raw: 0,
        delivered_raw: 0,
        available_export_raw: 0,
        factory_cost_raw: 0,
        efficiency_raw: 0,
        civilian_factories: 0,
        convoys_assigned: 0,
        convoys_required: 0,
    };
    let err = encode::encode_classic_deals(DealVersion::Legacy, &[21, 22], &[], &[row])
        .expect_err("a wide identity does not fit a packed key");
    assert!(matches!(
        err,
        encode::EncodeError::PackedIdentityOverflow { row: 0 }
    ));
}

/// The encoder refuses an `MTRF` row whose value count is not the trailer's, and
/// an `MTRR` identity block whose entry count is not the resource count.
#[test]
fn the_encoder_refuses_rows_that_do_not_fit_their_own_header() {
    let values = [1i64, 2];
    let rows = [encode::FlowRowInput {
        tag_db_id: 21,
        flow_kind: 1,
        values_raw: &values,
    }];
    let err = encode::encode_resource_flow(FlowVersion::Variable, 3, 1, &rows, TEST_LIMITS)
        .expect_err("a two-value row in a three-resource trailer");
    assert!(matches!(
        err,
        encode::EncodeError::FlowRowWidth {
            row: 0,
            values: 2,
            expected: 3
        }
    ));

    let err = encode::encode_resource_rows(
        ResourceRowVersion::Identity,
        3,
        0,
        &[b"oil".as_slice()],
        &[],
        TEST_LIMITS,
    )
    .expect_err("one token for three resources");
    assert!(matches!(
        err,
        encode::EncodeError::IdentityCount {
            entries: 1,
            expected: 3
        }
    ));
}

/// Everything the encoder writes walks, and consumes exactly what it wrote.
#[test]
fn everything_the_encoder_writes_walks_to_exact_consumption() {
    let core = encode::encode_core(
        7,
        encode::CoreCounts {
            changed_rows: 0,
            status_flags: 0,
            observed_contract_rows: 0,
            emitted_contract_rows: 0,
            buyer_ptr_valid_rows: 0,
            variant_valid_rows: 0,
            amount_valid_rows: 0,
            partial_field_rows: 0,
        },
        &[],
    )
    .expect("core");
    let mut payload = core.clone();
    payload.extend(
        encode::encode_resource_rows(
            ResourceRowVersion::Identity,
            2,
            0,
            &[b"oil".as_slice(), b"steel".as_slice()],
            &[],
            TEST_LIMITS,
        )
        .expect("identity-only MTRR"),
    );
    payload.extend(encode::encode_activation(0b11, 21, 9, 1, 0x015e_c520));
    let seen = walk_all(&payload).expect("walks");
    assert_eq!(
        seen.iter().map(|(t, _, _)| *t).collect::<Vec<_>>(),
        vec![Trailer::ResourceRow, Trailer::Activation]
    );
}

// =============================================================================
// 5. Version-selected reading
// =============================================================================

/// The same 96 bytes are read as two different identities under v3 and v4, and
/// each version's rule about the OTHER spelling's word still applies.
#[test]
fn the_deal_identity_is_read_at_the_width_its_version_spells() {
    let mut b = core_bytes(&[]);
    mtrd(&mut b, DealVersion::WideIdentity, &[21, 22], &[22]);
    let payload = b.done();
    let header = read_core_header(&payload).expect("header");
    let mut cursor = TrailerCursor::new(core_end(&payload, header.row_count()).expect("core"));
    cursor.next(&payload).expect("magic");
    let TrailerBody::ClassicDeal(c) = cursor.read_body(&payload, TEST_LIMITS).expect("body") else {
        panic!("expected MTRD");
    };
    let mut rows = c
        .witnesses()
        .discard_all()
        .next
        .incomplete_owners()
        .discard_all()
        .next
        .rows();
    let RowStep::Row { at, .. } = rows.next() else {
        panic!("expected a row");
    };
    let wide = deal_row_at(&payload, at, DealVersion::WideIdentity);
    assert_eq!(wide.identity_exporter_tag_db_id, 21);
    assert_eq!(wide.identity_importer_tag_db_id, 22);
    assert_eq!(wide.identity_ordinal, 1);
    assert_eq!(wide.packed_identity, None);
    assert_eq!(wide.reserved_ordinal_word, None);

    // Read the SAME bytes under the packed spelling: a different identity comes
    // out, which is exactly why an unknown version must be refused rather than
    // reinterpreted.
    let packed = deal_row_at(&payload, at, DealVersion::PerOwner);
    assert!(packed.packed_identity.is_some());
    assert_ne!(
        (
            packed.identity_exporter_tag_db_id,
            packed.identity_importer_tag_db_id
        ),
        (
            wide.identity_exporter_tag_db_id,
            wide.identity_importer_tag_db_id
        )
    );
}

/// An MTRR v3 identity-only trailer delimits its identity block and yields no
/// rows -- the shape the producer emits when the resource set was discovered but
/// the per-country rows failed.
#[test]
fn an_identity_only_mtrr_delimits_its_identities_and_no_rows() {
    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::Identity,
        3,
        &["oil", "steel", "rubber"],
        &[],
        &[],
    );
    let payload = b.done();
    let header = read_core_header(&payload).expect("header");
    let mut cursor = TrailerCursor::new(core_end(&payload, header.row_count()).expect("core"));
    cursor.next(&payload).expect("magic");
    let TrailerBody::ResourceRow(c) = cursor.read_body(&payload, TEST_LIMITS).expect("body") else {
        panic!("expected MTRR");
    };
    let t = *c.header();
    assert_eq!(t.row_count, 0);
    assert_eq!(t.identity_len, 3 * TEST_LIMITS.identity_stride());
    let mut identities = c.identities();
    let mut tokens = Vec::new();
    while let RowStep::Row { at, .. } = identities.next() {
        let entry = identity_entry_at(&payload, at, t.identity_stride)
            .expect("the cursor yielded this offset");
        tokens.push(entry.token.expect("token fits the stride").to_vec());
        assert!(entry.padding.expect("padding").iter().all(|b| *b == 0));
    }
    assert_eq!(
        tokens,
        vec![b"oil".to_vec(), b"steel".to_vec(), b"rubber".to_vec()]
    );
    assert_eq!(t.end, payload.len());
    // The zero-row block still has to be consumed to reach the parent, which is
    // what stops "this version has no rows" from being learned by NOT asking.
    let rows = identities.into_end().expect("identities").next.rows();
    assert_eq!(rows.remaining(), 0);
    assert_eq!(rows.into_end().expect("rows").next.end(), payload.len());
}

/// An `MTRF` v3 with a resource count that is NOT seven walks, which is the
/// whole reason v3 exists: a modded `common/resources` is not vanilla's seven.
#[test]
fn an_mtrf_v3_with_a_non_seven_resource_count_walks() {
    for count in [1u32, 3, 12, TEST_LIMITS.max_resource_count()] {
        let mut b = core_bytes(&[]);
        mtrf(&mut b, FlowVersion::Variable, count, &[1, 2, 3, 4, 5, 6]);
        let payload = b.done();
        let seen = walk_all(&payload).unwrap_or_else(|v| panic!("count {count}: {v:?}"));
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].2, payload.len());
    }
}

// =============================================================================
// 6. Every encoder is checked against a HANDWRITTEN fixture
// =============================================================================
//
// Codex review thread `01a00794-8e7e-7a02-b932-0eeb40336eb3`, finding 4: five
// families had no encoder-to-fixture equality, so swapping two adjacent counted
// words in an encoder would have left the suite green. One test per family, and
// the fixture is the same handwritten one the walk tests use.

#[test]
fn the_encoder_reproduces_the_handwritten_mtd1_bytes() {
    let mut b = Bytes::new();
    mtd1_v1(&mut b);
    let rows: Vec<DetailRow> = [(1u32, 0b0000011111u32), (2, 0b1111100000)]
        .into_iter()
        .map(|(contract_id, flags)| DetailRow {
            contract_id,
            flags,
            price_raw: 100_000,
            amount_raw: 200_000,
            speed_raw: 3,
            completed_raw: 4,
            days: 5,
            delivery_country_tag_db_id: 12,
            convoys_assigned: 6,
            convoys_total: 7,
            efficiency_raw: 8,
            factory_cic_progress_raw: 9,
            equipment_transfer_progress_raw: 10,
            reserved: 0,
        })
        .collect();
    assert_eq!(
        encode::encode_detail(
            &rows,
            encode::DetailCounts {
                price_valid_rows: 1,
                speed_valid_rows: 1,
                delivery_route_valid_rows: 1,
                progress_valid_rows: 1,
            },
        )
        .expect("encodes"),
        b.done()
    );
}

/// The counted words are checked POSITIONALLY, not just as a set: two of them
/// swapped must change the bytes. Without this, `encode_detail` could write
/// `speed_valid_rows` where `price_valid_rows` belongs and nothing would fail.
#[test]
fn the_mtd1_encoder_puts_each_counted_word_in_its_own_slot() {
    let base = encode::encode_detail(
        &[],
        encode::DetailCounts {
            price_valid_rows: 1,
            speed_valid_rows: 2,
            delivery_route_valid_rows: 3,
            progress_valid_rows: 4,
        },
    )
    .expect("encodes");
    assert_eq!(&base[16..20], &1u32.to_le_bytes());
    assert_eq!(&base[20..24], &2u32.to_le_bytes());
    assert_eq!(&base[24..28], &3u32.to_le_bytes());
    assert_eq!(&base[28..32], &4u32.to_le_bytes());
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtrr_bytes_at_every_version() {
    for (version, resource_count, tokens, countries, rows) in [
        (
            ResourceRowVersion::Legacy,
            7u32,
            Vec::<&str>::new(),
            vec![21u32],
            (0..7u32)
                .map(|i| {
                    if i == 0 {
                        (i, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL)
                    } else {
                        (i, NeededType::Surplus.wire(), 0)
                    }
                })
                .collect::<Vec<_>>(),
        ),
        (
            ResourceRowVersion::CountOnly,
            3,
            Vec::new(),
            vec![21],
            vec![
                (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
                (1, NeededType::Surplus.wire(), 0),
                (2, NeededType::Deficit.wire(), 0),
            ],
        ),
        (
            ResourceRowVersion::Identity,
            3,
            vec!["oil", "steel", "rubber"],
            vec![21, 22],
            vec![
                (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
                (1, NeededType::Surplus.wire(), 0),
                (2, NeededType::Deficit.wire(), 0),
            ],
        ),
        (
            ResourceRowVersion::NeededUnavailable,
            2,
            vec!["oil", "steel"],
            vec![21],
            vec![
                (
                    0,
                    NeededType::Unavailable.wire(),
                    ResourceFlag::NeededUnavailable.bit(),
                ),
                (1, NeededType::Surplus.wire(), 0),
            ],
        ),
    ] {
        let mut b = Bytes::new();
        mtrr(&mut b, version, resource_count, &tokens, &countries, &rows);
        let mut encoded_rows = Vec::new();
        for country in &countries {
            for (resource_index, needed_type, flags) in &rows {
                encoded_rows.push(ResourceRow {
                    tag_db_id: *country,
                    resource_index: *resource_index,
                    visible_order: *resource_index,
                    needed_type: *needed_type,
                    balance: 5,
                    produced: 5,
                    imported: 0,
                    exported: 0,
                    special_project: 0,
                    production_line: 0,
                    needed: 5,
                    needed_current: 0,
                    needed_max: 0,
                    flags: *flags,
                    descriptor_db_index: resource_index + 1,
                    reserved: 0,
                });
            }
        }
        let token_bytes: Vec<&[u8]> = tokens.iter().map(|t| t.as_bytes()).collect();
        assert_eq!(
            encode::encode_resource_rows(
                version,
                resource_count,
                countries.len() as u32,
                &token_bytes,
                &encoded_rows,
                TEST_LIMITS,
            )
            .expect("encodes"),
            b.done(),
            "MTRR {version:?}"
        );
    }
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtrd_bytes_at_every_version() {
    for version in DealVersion::ALL {
        let incomplete: Vec<u32> = if version.carries_incomplete_owners() {
            vec![22]
        } else {
            Vec::new()
        };
        let mut b = Bytes::new();
        mtrd(&mut b, version, &[21, 22], &incomplete);
        let mut rows = Vec::new();
        for (owner, counterpart, direction, route, status) in [
            (
                21u32,
                22u32,
                DealDirection::Export.wire(),
                DealRoute::Sea.wire(),
                DealStatus::NoTradeInfluence.bit(),
            ),
            (
                22,
                21,
                DealDirection::Import.wire(),
                DealRoute::Land.wire(),
                DealStatus::NoResources.bit(),
            ),
        ] {
            rows.push(DealRow {
                owner_tag_db_id: owner,
                counterpart_tag_db_id: counterpart,
                resource_index: 1,
                visible_order: 0,
                direction,
                route_kind: route,
                flags: DEAL_FLAG_EXACT,
                route_status_or_reserved: if version.carries_route_status() {
                    status
                } else {
                    0
                },
                identity_exporter_tag_db_id: 21,
                identity_importer_tag_db_id: 22,
                identity_ordinal: 1,
                packed_identity: None,
                reserved_ordinal_word: None,
                requested_raw: FIXED_SCALE_RAW as i64,
                delivered_raw: 0,
                available_export_raw: 0,
                factory_cost_raw: 0,
                efficiency_raw: 0,
                civilian_factories: 0,
                convoys_assigned: 0,
                convoys_required: 0,
            });
        }
        assert_eq!(
            encode::encode_classic_deals(version, &[21, 22], &incomplete, &rows).expect("encodes"),
            b.done(),
            "MTRD {version:?}"
        );
    }
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtrb_bytes() {
    let mut b = Bytes::new();
    mtrb_v1(&mut b);
    let row = ResidualRow {
        gap_id: 0,
        status: 1,
        evidence_mask: 2,
        current_support: 3,
        current_domain_id: 4,
        proposed_record_kind: 5,
        current_row_len: 6,
        source_ref_id: 7,
        owner_boundary: 8,
        root_status: 9,
        proof_status: 10,
        reserved: 11,
    };
    assert_eq!(
        encode::encode_residual(
            &[row],
            encode::ResidualCounts {
                source_blocked_rows: 1,
                schema_blocked_rows: 0,
                owned_elsewhere_rows: 0,
                shared_patch_blocked_rows: 0,
            },
        )
        .expect("encodes"),
        b.done()
    );
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtll_bytes() {
    let mut b = Bytes::new();
    mtll_v1(&mut b);
    let row = LendLeaseRow {
        contract_id: 1,
        perspective_tag_db_id: 11,
        counterpart_tag_db_id: 12,
        direction: LendLeaseDirection::Outgoing.wire(),
        equipment_variant_id: 100,
        flags: LendLeaseFlag::AmountValid.bit(),
        amount_raw: FIXED_SCALE_RAW as i64,
        price_raw: 0,
        source_record_kind: 0,
        reserved: 0,
    };
    assert_eq!(
        encode::encode_lend_lease(
            &[row],
            encode::LendLeaseCounts {
                outgoing_rows: 1,
                incoming_rows: 0,
                amount_valid_rows: 1,
                price_valid_rows: 0,
            },
        )
        .expect("encodes"),
        b.done()
    );
}

/// Every family this grammar declares has an encoder-to-handwritten-bytes test.
///
/// A list, checked against `Trailer::ALL`, so a tenth family cannot be added
/// with an encoder and no golden -- which is the gap finding 4 named.
#[test]
fn every_family_has_an_encoder_golden() {
    let with_goldens = [
        Trailer::Detail,
        Trailer::ResourceFlow,
        Trailer::FactoryHeader,
        Trailer::ResourceRow,
        Trailer::ClassicDeal,
        Trailer::Activation,
        Trailer::Stockpile,
        Trailer::Residual,
        Trailer::LendLease,
    ];
    for trailer in Trailer::ALL {
        assert!(
            with_goldens.contains(&trailer),
            "{} has no encoder-to-handwritten-bytes test",
            trailer.label()
        );
    }
}

/// The `MTRB` and `MTLL` row readers read back exactly what the handwritten
/// fixtures state -- the other half of finding 1: a family the leaf can only
/// delimit is a family each site still has to decode by hand.
#[test]
fn the_lane_only_families_have_row_readers() {
    let mut b = core_bytes(&[]);
    mtrb_v1(&mut b);
    mtll_v1(&mut b);
    let payload = b.done();
    let mut saw_residual = false;
    let mut saw_lend_lease = false;
    visit_trailers(&payload, |payload, _, body| match body {
        TrailerBody::Residual(c) => {
            let mut rows = c.rows();
            let RowStep::Row { at, .. } = rows.next() else {
                panic!("expected a row")
            };
            let row = residual_row_at(payload, at);
            assert_eq!(row.gap_id, 0);
            assert_eq!(row.proof_status, 10);
            assert_eq!(row.reserved, 11);
            saw_residual = true;
            Ok(rows.discard_all().next)
        }
        TrailerBody::LendLease(c) => {
            let mut rows = c.rows();
            let RowStep::Row { at, .. } = rows.next() else {
                panic!("expected a row")
            };
            let row = lend_lease_row_at(payload, at);
            assert_eq!(row.contract_id, 1);
            assert_eq!(row.direction, LendLeaseDirection::Outgoing.wire());
            assert_eq!(row.amount_raw, FIXED_SCALE_RAW as i64);
            assert_eq!(row.reserved, 0);
            saw_lend_lease = true;
            Ok(rows.discard_all().next)
        }
        _ => Ok(discard_body(body)),
    })
    .expect("walk");
    assert!(saw_residual && saw_lend_lease);
}

/// The two masks are the LOW bit runs, stated as literals here rather than read
/// back from the constants the encoder also uses.
///
/// Finding 3: the build-time proofs used to check only a popcount, so `0x7E`
/// and `0xFE` would have compiled, and every fixture and the encoder read the
/// same constant -- a coordinated error a round trip cannot see. The `const`
/// assertions now check the exact value; this pins the literal independently.
#[test]
fn the_flow_masks_are_the_documented_literals() {
    assert_eq!(FLOW_KIND_MASK, 0x3F);
    assert_eq!(FLOW_RESOURCE_MASK, 0x7F);
}


// =============================================================================
// 7. The owned walk: a skipped block is refused
// =============================================================================
//
// This section is lane KU's open headline finding, closed. Its shape came from
// Codex design thread `01a007a9-1e92-7a82-8263-3c5b37bcacab`.

/// Write a fixture whose every declared block of `trailer` is NON-EMPTY.
///
/// A block with no rows cannot demonstrate the property -- `remaining() == 0` is
/// exactly the legal state -- so the probe below needs fixtures that fill every
/// block, and `every_declared_block_is_probed_and_non_empty` checks it got them.
fn a_fixture_with_every_block_filled(trailer: Trailer) -> Vec<u8> {
    let mut b = core_bytes(&[]);
    match trailer {
        Trailer::Detail => mtd1_v1(&mut b),
        Trailer::ResourceFlow => mtrf(&mut b, FlowVersion::Variable, 3, &[1, 2, 3, 4, 5, 6]),
        Trailer::FactoryHeader => mtrh_v1(&mut b),
        Trailer::ResourceRow => mtrr(
            &mut b,
            ResourceRowVersion::Identity,
            2,
            &["oil", "steel"],
            &[21],
            &[
                (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
                (1, NeededType::Surplus.wire(), 0),
            ],
        ),
        // v4 so the incomplete-owner block exists AND is non-empty.
        Trailer::ClassicDeal => mtrd(&mut b, DealVersion::WideIdentity, &[21, 22], &[22]),
        Trailer::Activation => mtra_v1(&mut b),
        Trailer::Stockpile => mts1_v1(&mut b),
        Trailer::Residual => mtrb_v1(&mut b),
        Trailer::LendLease => mtll_v1(&mut b),
    }
    b.done()
}

/// Take the ONE trailer of `payload` and hand back its owned body.
fn only_body(payload: &[u8]) -> TrailerBody {
    let header = read_core_header(payload).expect("header");
    let cursor = TrailerCursor::new(core_end(payload, header.row_count()).expect("core"));
    let mut cursor = cursor;
    let TrailerStep::Magic { .. } = cursor.next(payload).expect("magic") else {
        panic!("expected a magic");
    };
    cursor.read_body(payload, TEST_LIMITS).expect("body")
}

fn assert_not_consumed(err: Violation, trailer: Trailer, block: TrailerBlock, remaining: usize) {
    assert_eq!(
        err.kind,
        ViolationKind::CallerDefect(CallerDefect::BlockNotConsumed {
            trailer,
            block,
            remaining,
        }),
        "{} {} produced {:?}",
        trailer.label(),
        block.name(),
        err.kind
    );
}

/// **THE PROPERTY.** For every declared block of every family, asking for the
/// block's end without consuming it is refused, and the refusal names the
/// family, the block and how many rows were left.
///
/// kills: `BlockCursor::into_end` returning `Ok` on a non-empty block;
/// `read_body` handing the parent back before the last block; any family cursor
/// growing a second route to `TrailerResume`; a new declared block reached
/// through a path that does not go through `BlockCursor`.
///
/// The coverage assertion at the end is what makes it a drift-killer rather
/// than thirteen hand-written cases: a tenth family, or a new arm of
/// [`TrailerBlock`], leaves the covered set short of the declared one and fails
/// HERE.
#[test]
fn every_declared_block_refuses_to_end_before_it_is_consumed() {
    let mut covered: Vec<(Trailer, TrailerBlock)> = Vec::new();
    let mut note = |trailer: Trailer, block: TrailerBlock| {
        if !covered.contains(&(trailer, block)) {
            covered.push((trailer, block));
        }
    };

    // The five one-block families.
    for trailer in [
        Trailer::Detail,
        Trailer::ResourceFlow,
        Trailer::FactoryHeader,
        Trailer::Residual,
        Trailer::LendLease,
    ] {
        let payload = a_fixture_with_every_block_filled(trailer);
        let rows = match only_body(&payload) {
            TrailerBody::Detail(c) => c.rows(),
            TrailerBody::ResourceFlow(c) => c.rows(),
            TrailerBody::FactoryHeader(c) => c.rows(),
            TrailerBody::Residual(c) => c.rows(),
            TrailerBody::LendLease(c) => c.rows(),
            _ => panic!("{} fixture produced another family", trailer.label()),
        };
        let remaining = rows.remaining();
        assert!(remaining > 0, "{} fixture has no rows", trailer.label());
        let err = rows
            .into_end()
            .unwrap_err_or_panic(&format!("{} rows skipped silently", trailer.label()));
        assert_not_consumed(err, trailer, TrailerBlock::Rows, remaining);
        note(trailer, TrailerBlock::Rows);
    }

    // MTRR: identities, then rows.
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let identities = c.identities();
    let remaining = identities.remaining();
    assert!(remaining > 0, "MTRR fixture has no identities");
    let err = identities
        .into_end()
        .unwrap_err_or_panic("MTRR identities skipped silently");
    assert_not_consumed(err, Trailer::ResourceRow, TrailerBlock::Identities, remaining);
    note(Trailer::ResourceRow, TrailerBlock::Identities);

    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let rows = c.identities().discard_all().next.rows();
    let remaining = rows.remaining();
    assert!(remaining > 0, "MTRR fixture has no rows");
    let err = rows
        .into_end()
        .unwrap_err_or_panic("MTRR rows skipped silently");
    assert_not_consumed(err, Trailer::ResourceRow, TrailerBlock::Rows, remaining);
    note(Trailer::ResourceRow, TrailerBlock::Rows);

    // MTRD: witnesses, then incomplete owners, then rows.
    let payload = a_fixture_with_every_block_filled(Trailer::ClassicDeal);
    let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let witnesses = c.witnesses();
    let remaining = witnesses.remaining();
    assert!(remaining > 0, "MTRD fixture has no witnesses");
    let err = witnesses
        .into_end()
        .unwrap_err_or_panic("MTRD witnesses skipped silently");
    assert_not_consumed(err, Trailer::ClassicDeal, TrailerBlock::Witnesses, remaining);
    note(Trailer::ClassicDeal, TrailerBlock::Witnesses);

    let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let incomplete = c.witnesses().discard_all().next.incomplete_owners();
    let remaining = incomplete.remaining();
    assert!(remaining > 0, "MTRD fixture has no incomplete owners");
    let err = incomplete
        .into_end()
        .unwrap_err_or_panic("MTRD incomplete owners skipped silently");
    assert_not_consumed(
        err,
        Trailer::ClassicDeal,
        TrailerBlock::IncompleteOwners,
        remaining,
    );
    note(Trailer::ClassicDeal, TrailerBlock::IncompleteOwners);

    let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let rows = c
        .witnesses()
        .discard_all()
        .next
        .incomplete_owners()
        .discard_all()
        .next
        .rows();
    let remaining = rows.remaining();
    assert!(remaining > 0, "MTRD fixture has no rows");
    let err = rows
        .into_end()
        .unwrap_err_or_panic("MTRD rows skipped silently");
    assert_not_consumed(err, Trailer::ClassicDeal, TrailerBlock::Rows, remaining);
    note(Trailer::ClassicDeal, TrailerBlock::Rows);

    // MTS1: countries, then items.
    let payload = a_fixture_with_every_block_filled(Trailer::Stockpile);
    let TrailerBody::Stockpile(c) = only_body(&payload) else {
        panic!("expected MTS1");
    };
    let countries = c.countries();
    let remaining = countries.remaining();
    assert!(remaining > 0, "MTS1 fixture has no countries");
    let err = countries
        .into_end()
        .unwrap_err_or_panic("MTS1 countries skipped silently");
    assert_not_consumed(err, Trailer::Stockpile, TrailerBlock::Countries, remaining);
    note(Trailer::Stockpile, TrailerBlock::Countries);

    let TrailerBody::Stockpile(c) = only_body(&payload) else {
        panic!("expected MTS1");
    };
    let items = c.countries().discard_all().next.items();
    let remaining = items.remaining();
    assert!(remaining > 0, "MTS1 fixture has no items");
    let err = items
        .into_end()
        .unwrap_err_or_panic("MTS1 items skipped silently");
    assert_not_consumed(err, Trailer::Stockpile, TrailerBlock::Items, remaining);
    note(Trailer::Stockpile, TrailerBlock::Items);

    // MTRA declares no block at all, so it has nothing to skip: its header IS
    // its end. Stated rather than omitted, so "MTRA is missing from the covered
    // set" cannot read as an oversight.
    let payload = a_fixture_with_every_block_filled(Trailer::Activation);
    let TrailerBody::Activation(c) = only_body(&payload) else {
        panic!("expected MTRA");
    };
    assert_eq!(c.end().next.end(), payload.len());

    // Drift-killer: every family with blocks is covered, and every arm of
    // `TrailerBlock` is exercised at least once.
    let mut families: Vec<Trailer> = Vec::new();
    for (trailer, _) in &covered {
        if !families.contains(trailer) {
            families.push(*trailer);
        }
    }
    for trailer in Trailer::ALL {
        if trailer == Trailer::Activation {
            continue;
        }
        assert!(
            families.contains(&trailer),
            "{} has declared blocks and none was probed",
            trailer.label()
        );
    }
    for block in TrailerBlock::ALL {
        // Exhaustive and wildcard-free ON THE ENUM, not only on `ALL`. A
        // seventh `TrailerBlock` arm is a compile error HERE even if nobody
        // adds it to `ALL` -- which is the half the build-time ordinal
        // assertion in `walk.rs` cannot reach.
        let must_be_probed = match block {
            TrailerBlock::Rows
            | TrailerBlock::Identities
            | TrailerBlock::Witnesses
            | TrailerBlock::IncompleteOwners
            | TrailerBlock::Countries
            | TrailerBlock::Items => true,
        };
        assert_eq!(
            covered.iter().any(|(_, b)| *b == block),
            must_be_probed,
            "no family's {} block was probed",
            block.name()
        );
    }
}

/// `discard_all` is the OTHER way past a block, and it lands on the block's own
/// end -- the offset the header already declared -- never past it.
///
/// kills: `discard_all` advancing by anything but `remaining * stride`, which
/// would let a site that reads a block structurally-only resume at a wrong
/// offset and still be accepted by the walk.
#[test]
fn discarding_a_block_lands_on_exactly_that_blocks_end() {
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *c.header();
    let identities = c.identities();
    let discarded = identities.discard_all();
    assert_eq!(discarded.at, header.rows_at);

    // And walking it reaches the same offset, which is what makes the two
    // routes interchangeable to the GRAMMAR and different only at the site.
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let mut identities = c.identities();
    while let RowStep::Row { .. } = identities.next() {}
    assert_eq!(identities.into_end().expect("identities").at, header.rows_at);
}

/// `read_body` without a delimited magic is a CALLER defect, not a payload one,
/// and it is reported rather than panicked.
///
/// kills: reinstating a `NotDelimited` body arm (which every site would have to
/// write an unreachable arm for), or panicking in a leaf that runs under a
/// recorder which must fail closed.
#[test]
fn reading_a_body_that_was_never_delimited_is_a_caller_defect() {
    let payload = a_fixture_with_every_block_filled(Trailer::Detail);
    let header = read_core_header(&payload).expect("header");
    let cursor = TrailerCursor::new(core_end(&payload, header.row_count()).expect("core"));
    // No `next` call, so nothing is pending.
    let err = cursor
        .read_body(&payload, TEST_LIMITS)
        .unwrap_err_or_panic("an undelimited body was read");
    assert_eq!(
        err.kind,
        ViolationKind::CallerDefect(CallerDefect::BodyNotDelimited)
    );
}

/// `peek_header` reads the family's own header words, applies no rule, and does
/// not advance -- so a site can keep checking a header VALUE before it checks
/// the header's SHAPE, which is the order three of the four sites already use.
///
/// kills: `peek_header` advancing the cursor, applying the version table, or
/// applying the declared-header-length rule; and a preview whose words are read
/// at different offsets from the body's.
#[test]
fn peeking_a_header_reads_the_bodys_words_and_moves_nothing() {
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceFlow);
    let header = read_core_header(&payload).expect("header");
    let core = core_end(&payload, header.row_count()).expect("core");
    let mut cursor = TrailerCursor::new(core);
    let TrailerStep::Magic { at, trailer } = cursor.next(&payload).expect("magic") else {
        panic!("expected a magic");
    };
    assert_eq!(trailer, Trailer::ResourceFlow);

    let HeaderPreview::ResourceFlow(words) = cursor.peek_header(&payload).expect("preview") else {
        panic!("expected an MTRF preview");
    };
    // Peeking twice is the same answer, and neither moved the cursor.
    let second = cursor.peek_header(&payload).expect("preview");
    assert_eq!(second, HeaderPreview::ResourceFlow(words));
    assert_eq!(cursor.offset(), core);
    assert_eq!(words.sized.prefix.at, at);

    let TrailerBody::ResourceFlow(c) = cursor.read_body(&payload, TEST_LIMITS).expect("body") else {
        panic!("expected MTRF");
    };
    let body = *c.header();
    assert_eq!(words.sized.prefix.version, body.version.wire());
    assert_eq!(words.sized.prefix.header_len as usize, body.header_len);
    assert_eq!(words.sized.row_len as usize, body.row_len);
    assert_eq!(words.sized.row_count as usize, body.row_count);
    assert_eq!(
        words.country_count_or_exported as usize,
        body.country_count_or_exported
    );
    assert_eq!(words.scale_raw, body.scale_raw);
    assert_eq!(words.mask_or_count, body.mask_or_count);
    assert_eq!(words.kind_mask, body.kind_mask);
}

/// A preview of a header the payload is too short for carries the SAME
/// violation `read_body` would, so looking first never changes what a truncated
/// header reports.
#[test]
fn peeking_a_truncated_header_reports_what_reading_it_would() {
    let payload = a_fixture_with_every_block_filled(Trailer::Detail);
    let header = read_core_header(&payload).expect("header");
    let core = core_end(&payload, header.row_count()).expect("core");
    let truncated = &payload[..core + 8];
    let mut cursor = TrailerCursor::new(core);
    cursor.next(truncated).expect("magic");
    let peeked = cursor
        .peek_header(truncated)
        .unwrap_err_or_panic("a truncated header previewed");
    let mut cursor = TrailerCursor::new(core);
    cursor.next(truncated).expect("magic");
    let read = cursor
        .read_body(truncated, TEST_LIMITS)
        .unwrap_err_or_panic("a truncated header read");
    assert_eq!(peeked, read);
    assert_eq!(
        read.kind,
        ViolationKind::Need {
            section: Section::TrailerHeader(Trailer::Detail),
            needed: core + Trailer::Detail.header_len(),
            available: truncated.len(),
        }
    );
}


/// `discard_all` after a PARTIAL walk still lands on the block's own end.
///
/// kills: `RowCursor::skip_all` advancing by `count * stride` instead of
/// `remaining * stride`. The zero-consumed case cannot tell those apart -- they
/// are the same number there -- which is exactly the hole Codex found in the
/// first version of this section.
#[test]
fn discarding_a_partly_walked_block_still_lands_on_its_own_end() {
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *c.header();
    let mut identities = c.identities();
    assert!(
        identities.remaining() >= 2,
        "this case needs a block with at least two rows to be able to fail"
    );
    let RowStep::Row { index, .. } = identities.next() else {
        panic!("expected a row");
    };
    assert_eq!(index, 0);
    assert_eq!(identities.remaining(), 1);
    let discarded = identities.discard_all();
    assert_eq!(discarded.at, header.rows_at);

    // Every intermediate consumed-count lands on the same end.
    for consume in 0..=2usize {
        let TrailerBody::ResourceRow(c) = only_body(&payload) else {
            panic!("expected MTRR");
        };
        let mut identities = c.identities();
        for _ in 0..consume {
            identities.next();
        }
        assert_eq!(identities.discard_all().at, header.rows_at, "after {consume}");
    }
}

/// A block a version declares NOTHING for is already at its end, and discarding
/// it moves nothing.
///
/// kills: `skip_all` on an empty cursor advancing at all, and a zero-length
/// block whose `at` is not the next block's start.
///
/// It does NOT kill a wrong stride: zero rows advance zero bytes at every
/// stride, so this case cannot observe one. `every_declared_block_discards_to_
/// its_own_end_from_every_position` is what does (Codex round 3 called the
/// earlier wording of this comment vacuous, and it was).
#[test]
fn discarding_a_version_absent_block_moves_nothing() {
    // MTRR v2 carries no identity block; MTRD v2 carries no incomplete owners.
    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::CountOnly,
        2,
        &[],
        &[21],
        &[
            (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
            (1, NeededType::Surplus.wire(), 0),
        ],
    );
    let payload = b.done();
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *c.header();
    assert_eq!(header.identity_len, 0);
    let identities = c.identities();
    assert_eq!(identities.remaining(), 0);
    let discarded = identities.discard_all();
    assert_eq!(discarded.at, header.identity_at);
    assert_eq!(discarded.at, header.rows_at);

    let mut b = core_bytes(&[]);
    mtrd(&mut b, DealVersion::Statuses, &[21, 22], &[]);
    let payload = b.done();
    let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let header = *c.header();
    assert_eq!(header.incomplete_count, 0);
    let incomplete = c
        .witnesses()
        .discard_all()
        .next
        .incomplete_owners();
    assert_eq!(incomplete.remaining(), 0);
    assert_eq!(incomplete.discard_all().at, header.rows_at);
}

/// `peek_header` applies NO rule: it reports a header the body refuses, and
/// reports it with the wrong value it actually found.
///
/// kills: `peek_header` applying the version table, the declared-header-length
/// rule, or the declared-row-length rule -- any of which would move a site's
/// pre-structural check behind a structural refusal and change which diagnostic
/// a doubly-malformed payload carries. That is the whole reason the preview
/// exists.
#[test]
fn peeking_reports_a_header_the_body_refuses() {
    // A declared header_len of 64 where MTD1's is 32, and an unknown version.
    for (version, declared_header_len, declared_row_len) in
        [(1u16, 64u16, 64u16), (9u16, 32u16, 64u16), (1u16, 32u16, 48u16)]
    {
        let mut b = core_bytes(&[]);
        b.magic(b"MTD1")
            .u16(version)
            .u16(declared_header_len)
            .u16(declared_row_len)
            .u16(0)
            .u32(0);
        b.zeros(16);
        let payload = b.done();
        let header = read_core_header(&payload).expect("header");
        let core = core_end(&payload, header.row_count()).expect("core");
        let mut cursor = TrailerCursor::new(core);
        cursor.next(&payload).expect("magic");

        let HeaderPreview::Detail(words) = cursor.peek_header(&payload).expect(
            "the preview applies no rule, so a header the body refuses still previews",
        ) else {
            panic!("expected an MTD1 preview");
        };
        assert_eq!(words.sized.prefix.version, version);
        assert_eq!(words.sized.prefix.header_len, declared_header_len);
        assert_eq!(words.sized.row_len, declared_row_len);
        assert_eq!(cursor.offset(), core);

        // And the body refuses it, which is what makes the preview's silence a
        // decision rather than an oversight.
        cursor
            .read_body(&payload, TEST_LIMITS)
            .unwrap_err_or_panic("a malformed MTD1 header was accepted");
    }
}

/// The preview is available for every one of the nine families and reads inside
/// that family's own declared header.
///
/// kills: a family whose preview arm reads a word past its header (`MTRH`'s
/// header is 24 bytes and has no `+24`), and a family added to `Trailer::ALL`
/// with no preview arm.
#[test]
fn every_family_previews_inside_its_own_header() {
    for trailer in Trailer::ALL {
        let payload = a_fixture_with_every_block_filled(trailer);
        let header = read_core_header(&payload).expect("header");
        let core = core_end(&payload, header.row_count()).expect("core");
        let mut cursor = TrailerCursor::new(core);
        cursor.next(&payload).expect("magic");
        let preview = cursor
            .peek_header(&payload)
            .unwrap_or_else(|error| panic!("{} preview: {error:?}", trailer.label()));
        let prefix = preview.prefix();
        assert_eq!(prefix.trailer, trailer, "{}", trailer.label());
        assert_eq!(prefix.at, core, "{}", trailer.label());
        assert_eq!(
            prefix.header_len as usize,
            trailer.header_len(),
            "{} previewed a declared header_len its own table disagrees with",
            trailer.label()
        );
        // Truncating the payload to one byte less than this family's header
        // makes the preview refuse, which is what "reads inside its own header"
        // means operationally.
        let short = &payload[..core + trailer.header_len() - 1];
        let mut cursor = TrailerCursor::new(core);
        cursor.next(short).expect("magic");
        cursor
            .peek_header(short)
            .unwrap_err_or_panic(&format!("{} previewed past its header", trailer.label()));
    }
}


// =============================================================================
// 8. Round-3 review: the identity stride, and the proofs that were vacuous
// =============================================================================

/// A stride an identity entry cannot fit is refused where the block is SIZED,
/// so no reader ever holds a trailer it would have to defend against.
///
/// kills: dropping the `< 2` refusal in `read_resource_row_trailer`, which is
/// what makes `identity_entry_at`'s `stride - 2` unable to underflow. Before
/// this, `Limits::new(_, 0)` plus a v3 header whose `+28` word is zero parsed
/// happily and handed a public reader a zero stride.
#[test]
fn an_identity_stride_an_entry_cannot_fit_is_refused() {
    for stride in [0usize, 1] {
        let limits = Limits::new(TEST_LIMITS.max_resource_count(), stride);
        let mut b = core_bytes(&[]);
        // A v3 header whose `+28` states exactly this stride, so nothing else
        // in the header can be the reason it is refused.
        b.magic(b"MTRR")
            .u16(ResourceRowVersion::Identity.wire())
            .u16(32)
            .u16(64)
            .u16(0)
            .u32(0);
        b.u32(0).u32(1).u32(1).u32(stride as u32);
        let payload = b.done();
        let header = read_core_header(&payload).expect("header");
        let mut cursor = TrailerCursor::new(core_end(&payload, header.row_count()).expect("core"));
        cursor.next(&payload).expect("magic");
        let err = cursor
            .read_body(&payload, limits)
            .unwrap_err_or_panic(&format!("a stride of {stride} was accepted"));
        assert_eq!(
            err.kind,
            ViolationKind::IdentityStride {
                trailer: Trailer::ResourceRow,
                stride,
            }
        );
    }
}

/// A version with no identity block yields no identity offsets, and the reader
/// refuses the zero stride such a trailer carries.
///
/// kills: giving a version-absent block a non-empty cursor, or dropping the
/// `stride < 2` guard that stops a zero stride reaching `stride - 2`.
#[test]
fn a_version_with_no_identity_block_yields_no_entries() {
    // v2: no identity block at all.
    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::CountOnly,
        2,
        &[],
        &[21],
        &[
            (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
            (1, NeededType::Surplus.wire(), 0),
        ],
    );
    let payload = b.done();
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *c.header();
    assert_eq!(header.identity_stride, 0);
    // The CURSOR is what refuses a version with no identity block: it is empty,
    // so it yields no offset to read with, and a zero stride is refused anyway.
    let identities = c.identities();
    assert_eq!(identities.remaining(), 0);
    assert!(identity_entry_at(&payload, header.identity_at, header.identity_stride).is_none());

    // v3: two slots, and the cursor yields exactly two offsets.
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *c.header();
    assert_eq!(header.resource_count, 2);
    let mut identities = c.identities();
    let mut offsets = Vec::new();
    while let RowStep::Row { at, .. } = identities.next() {
        offsets.push(at);
        assert!(identity_entry_at(&payload, at, header.identity_stride).is_some());
    }
    assert_eq!(offsets.len(), 2);
    assert_eq!(offsets[1] - offsets[0], header.identity_stride);
}

/// EVERY family's preview reports a header its own body refuses, with the wrong
/// value it actually found.
///
/// kills: any ONE family's preview arm starting to apply the version table or
/// the declared-header-length rule. The earlier version of this proof tested
/// `MTD1` alone, so eight of the nine arms were unpinned -- Codex round 3.
#[test]
fn every_familys_preview_reports_a_header_its_body_refuses() {
    for trailer in Trailer::ALL {
        let good = a_fixture_with_every_block_filled(trailer);
        let header = read_core_header(&good).expect("header");
        let core = core_end(&good, header.row_count()).expect("core");
        // Corrupt the declared header_len at `+6`: a fully sized payload whose
        // header is refused for a reason the preview must not apply.
        let mut payload = good.clone();
        let bad = (trailer.header_len() as u16) + 8;
        payload[core + TRAILER_HEADER_LEN_OFFSET] = (bad & 0xFF) as u8;
        payload[core + TRAILER_HEADER_LEN_OFFSET + 1] = (bad >> 8) as u8;

        let mut cursor = TrailerCursor::new(core);
        cursor.next(&payload).expect("magic");
        let preview = cursor.peek_header(&payload).unwrap_or_else(|error| {
            panic!(
                "{} previewed a header its body refuses: {error:?}",
                trailer.label()
            )
        });
        assert_eq!(
            preview.prefix().header_len,
            bad,
            "{} previewed a value it was not given",
            trailer.label()
        );
        assert_eq!(cursor.offset(), core, "{}", trailer.label());

        let err = cursor
            .read_body(&payload, TEST_LIMITS)
            .unwrap_err_or_panic(&format!("{} accepted a wrong header_len", trailer.label()));
        assert_eq!(
            err.kind,
            ViolationKind::HeaderLen {
                trailer,
                declared: bad as usize,
                expected: trailer.header_len(),
            },
            "{}",
            trailer.label()
        );

        // And an unknown VERSION, which is the other rule the preview must not
        // apply. `+4` is the version word in every family.
        let mut payload = good.clone();
        payload[core + TRAILER_VERSION_OFFSET] = 0xFE;
        payload[core + TRAILER_VERSION_OFFSET + 1] = 0x00;
        let mut cursor = TrailerCursor::new(core);
        cursor.next(&payload).expect("magic");
        let preview = cursor.peek_header(&payload).unwrap_or_else(|error| {
            panic!(
                "{} previewed an unknown version: {error:?}",
                trailer.label()
            )
        });
        assert_eq!(preview.prefix().version, 0x00FE, "{}", trailer.label());
        let err = cursor
            .read_body(&payload, TEST_LIMITS)
            .unwrap_err_or_panic(&format!("{} accepted an unknown version", trailer.label()));
        assert!(
            matches!(err.kind, ViolationKind::UnknownVersion { version: 0x00FE, .. }),
            "{} produced {:?}",
            trailer.label(),
            err.kind
        );
    }
}

/// EVERY declared block lands on its own end from EVERY consumed-count, not
/// only from zero.
///
/// kills: `RowCursor::skip_all` using `count * stride` instead of
/// `remaining * stride`. The zero-consumed case cannot tell those apart -- they
/// are the same number there -- and the earlier version of this proof exercised
/// a partial discard for MTRR identities alone, so eleven of the twelve blocks
/// were unpinned (Codex round 3).
#[test]
fn every_declared_block_discards_to_its_own_end_from_every_position() {
    // (family, block, the block's own end, how to reach the block)
    for trailer in [
        Trailer::Detail,
        Trailer::ResourceFlow,
        Trailer::FactoryHeader,
        Trailer::Residual,
        Trailer::LendLease,
    ] {
        let payload = a_fixture_with_every_block_filled(trailer);
        let total = match only_body(&payload) {
            TrailerBody::Detail(c) => c.rows().remaining(),
            TrailerBody::ResourceFlow(c) => c.rows().remaining(),
            TrailerBody::FactoryHeader(c) => c.rows().remaining(),
            TrailerBody::Residual(c) => c.rows().remaining(),
            TrailerBody::LendLease(c) => c.rows().remaining(),
            _ => panic!("{} fixture produced another family", trailer.label()),
        };
        assert!(total > 0, "{} has no rows", trailer.label());
        // The HEADER'S declared end, read off the grammar's own extent -- not
        // this loop's zero-consumed answer. Comparing the positions with each
        // other passes a `skip_all` that lands one byte past the end from every
        // position, which is exactly the production change this test exists to
        // catch (Codex round 4).
        let declared_end = match only_body(&payload) {
            TrailerBody::Detail(c) => c.header().sized.end,
            TrailerBody::ResourceFlow(c) => c.header().end,
            TrailerBody::FactoryHeader(c) => c.header().sized.end,
            TrailerBody::Residual(c) => c.header().sized.end,
            TrailerBody::LendLease(c) => c.header().sized.end,
            _ => panic!("{} fixture produced another family", trailer.label()),
        };
        assert_eq!(
            declared_end,
            payload.len(),
            "{} fixture is not a single trailer",
            trailer.label()
        );
        for consume in 0..=total {
            let mut rows = match only_body(&payload) {
                TrailerBody::Detail(c) => c.rows(),
                TrailerBody::ResourceFlow(c) => c.rows(),
                TrailerBody::FactoryHeader(c) => c.rows(),
                TrailerBody::Residual(c) => c.rows(),
                TrailerBody::LendLease(c) => c.rows(),
                _ => panic!("{} fixture produced another family", trailer.label()),
            };
            for _ in 0..consume {
                rows.next();
            }
            assert_eq!(
                rows.discard_all().at,
                declared_end,
                "{} rows discarded to the wrong end after {consume}",
                trailer.label()
            );
        }
    }

    // MTRR: identities, then rows.
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let mtrr_header = *c.header();
    for consume in 0..=mtrr_header.resource_count as usize {
        let TrailerBody::ResourceRow(c) = only_body(&payload) else {
            panic!("expected MTRR");
        };
        let mut identities = c.identities();
        for _ in 0..consume {
            identities.next();
        }
        assert_eq!(identities.discard_all().at, mtrr_header.rows_at, "MTRR identities after {consume}");
    }
    for consume in 0..=mtrr_header.row_count {
        let TrailerBody::ResourceRow(c) = only_body(&payload) else {
            panic!("expected MTRR");
        };
        let mut rows = c.identities().discard_all().next.rows();
        for _ in 0..consume {
            rows.next();
        }
        assert_eq!(rows.discard_all().at, mtrr_header.end, "MTRR rows after {consume}");
    }

    // MTRD: witnesses, incomplete owners, rows.
    let payload = a_fixture_with_every_block_filled(Trailer::ClassicDeal);
    let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let mtrd = *c.header();
    for consume in 0..=mtrd.witness_count {
        let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
            panic!("expected MTRD");
        };
        let mut witnesses = c.witnesses();
        for _ in 0..consume {
            witnesses.next();
        }
        assert_eq!(witnesses.discard_all().at, mtrd.incomplete_at, "MTRD witnesses after {consume}");
    }
    for consume in 0..=mtrd.incomplete_count {
        let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
            panic!("expected MTRD");
        };
        let mut incomplete = c.witnesses().discard_all().next.incomplete_owners();
        for _ in 0..consume {
            incomplete.next();
        }
        assert_eq!(incomplete.discard_all().at, mtrd.rows_at, "MTRD incomplete after {consume}");
    }
    for consume in 0..=mtrd.row_count {
        let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
            panic!("expected MTRD");
        };
        let mut rows = c
            .witnesses()
            .discard_all()
            .next
            .incomplete_owners()
            .discard_all()
            .next
            .rows();
        for _ in 0..consume {
            rows.next();
        }
        assert_eq!(rows.discard_all().at, mtrd.end, "MTRD rows after {consume}");
    }

    // MTS1: countries, items.
    let payload = a_fixture_with_every_block_filled(Trailer::Stockpile);
    let TrailerBody::Stockpile(c) = only_body(&payload) else {
        panic!("expected MTS1");
    };
    let mts1 = *c.header();
    for consume in 0..=mts1.country_count {
        let TrailerBody::Stockpile(c) = only_body(&payload) else {
            panic!("expected MTS1");
        };
        let mut countries = c.countries();
        for _ in 0..consume {
            countries.next();
        }
        assert_eq!(countries.discard_all().at, mts1.items_at, "MTS1 countries after {consume}");
    }
    for consume in 0..=mts1.item_count {
        let TrailerBody::Stockpile(c) = only_body(&payload) else {
            panic!("expected MTS1");
        };
        let mut items = c.countries().discard_all().next.items();
        for _ in 0..consume {
            items.next();
        }
        assert_eq!(items.discard_all().at, mts1.end, "MTS1 items after {consume}");
    }
}


/// `identity_entry_at` refuses rather than panics for every argument a caller
/// can invent, and it reads exactly the entry at the offset it was given.
///
/// It takes the CURSOR'S offset and the parsed stride, so it has no arithmetic
/// of its own to redirect -- which is the whole of the round-5 fix. What is left
/// is the argument checking, and all of it is RELEASE checking: a
/// `debug_assert` is compiled out of the build that ships.
///
/// kills: turning the `stride < 2` guard into a `debug_assert`, turning
/// `checked_add` back into `+`, dropping the payload bound, or making the bound
/// `>=` instead of `>`.
#[test]
fn a_forged_identity_read_is_refused_rather_than_panicking() {
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let parsed = *c.header();
    let stride = parsed.identity_stride;
    assert!(stride >= 2);
    assert!(identity_entry_at(&payload, parsed.identity_at, stride).is_some());

    // A stride the parser would have refused.
    for bad in [0usize, 1] {
        assert!(
            identity_entry_at(&payload, parsed.identity_at, bad).is_none(),
            "a stride of {bad} was read"
        );
    }

    // Arithmetic that overflows rather than landing anywhere.
    assert!(identity_entry_at(&payload, usize::MAX - 1, stride).is_none());
    assert!(identity_entry_at(&payload, parsed.identity_at, usize::MAX).is_none());

    // An offset past the payload, and a payload shorter than the parsed one.
    assert!(identity_entry_at(&payload, payload.len(), stride).is_none());
    let short = &payload[..parsed.identity_at + 1];
    assert!(identity_entry_at(short, parsed.identity_at, stride).is_none());

    // An entry that ends EXACTLY at the payload's end is still readable, so the
    // bound is `>` and not `>=`.
    let exact = &payload[..parsed.identity_at + stride];
    assert!(identity_entry_at(exact, parsed.identity_at, stride).is_some());

    // And the reader reads the entry it was given, not a neighbour: the two
    // slots of this fixture carry different tokens.
    let first = identity_entry_at(&payload, parsed.identity_at, stride).expect("slot 0");
    let second = identity_entry_at(&payload, parsed.identity_at + stride, stride).expect("slot 1");
    assert_ne!(first.token, second.token);
}
