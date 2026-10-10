use super::*;

/// Hourly all-country division counts from the same read as Divisions.
/// Raw writer country: positive expeditionary owner, otherwise roster owner.
/// Every canonical country's native reference has a row, including zero counts;
/// a distinct raw alias chosen by a writer keeps its own counts.
/// Every complete hourly census is a read, including equal reads; unavailable
/// hours cannot carry certified coverage, and incomplete counts never certify
/// a partial country total.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionCensusRow {
    pub country: u32,
    pub grouped: u32,
    pub ungrouped: u32,
}
typed_codec!(DivisionCensusRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (grouped): U32, 1 => (ungrouped): U32 }, |_row| Ok(()));

/// Hourly simulation node. Identity is (Countries.id, native node index).
/// Parent is a province in the same companion, not an array index. Route
/// provinces retain native order and duplicates; reverse land portions only
/// in presentation. from==0 denotes native's missing-route/naval fallback.
/// Format 28 adds a row for every other vertex on a node's parent chain
/// (index `VERTEX | province`, kind/flag/supply zero), so the chain never
/// leaves the country's rows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SupplyNodeRow {
    pub country: u32,
    pub index: u32,
    pub province: u32,
    pub kind: u32,
    pub flag: u8,
    pub available: i64,
    pub consumed: i64,
    pub parent: i32,
    pub route_from: u16,
    pub route_to: u16,
    pub break_after: u8,
    pub path: Vec<u32>,
}
impl SupplyNodeRow {
    /// Index bit of a node-less parent-map vertex row.
    pub const VERTEX: u32 = 1 << 31;
    pub fn is_vertex(&self) -> bool {
        self.index & Self::VERTEX != 0
    }
}
typed_codec!(SupplyNodeRow, 64, |row| (row.country as u128) << 32 | row.index as u128,
 |key| Self { country: (key >> 32) as u32, index: key as u32, ..Default::default() },
 { 0 => (province): U32, 1 => (kind): U32, 2 => (flag): U8, 3 => (available): I64, 4 => (consumed): I64, 5 => (parent): I32, 6 => (route_from): U16, 7 => (route_to): U16, 8 => (break_after): U8, 9 => (path): U32List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TrainAllocation {
    /// Full CEquipmentVariant reference, joining VariantDefinitions.id.
    pub variant: u64,
    /// Signed native integer; neither fixed-point nor clamped to zero.
    pub count: i32,
}
value_struct!(TrainAllocation {
    variant: u64,
    count: i32
});

/// A native byte array recorded without its zero bytes.
///
/// `len` is the native element count. `runs` hold every nonzero byte as
/// maximal runs of consecutive nonzero bytes, in ascending index order. An
/// index below `len` that no run covers is a native **zero** byte; an index at
/// or beyond `len` lies outside the native array, which [`Self::get`] reports
/// as `None`. The value is canonical: every run is nonempty, holds no zero
/// byte, ends within `len` and is separated from the previous run by at least
/// one zero byte, so one native array has exactly one encoding.
///
/// Wire: `len` and the run count as little-endian u32, then per run the gap
/// from the previous run's end (from index 0 for the first run) and the run
/// length as unsigned LEB128, followed by the run's bytes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SparseBytes {
    pub len: u32,
    pub runs: Vec<ByteRun>,
}

/// Consecutive nonzero native bytes starting at native index `start`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ByteRun {
    pub start: u32,
    pub bytes: Vec<u8>,
}

impl SparseBytes {
    /// The canonical sparse form of a dense native array.
    pub fn from_dense(bytes: &[u8]) -> io::Result<Self> {
        let len = u32::try_from(bytes.len())
            .map_err(|_| invalid("sparse byte array length exceeds u32"))?;
        let count = bytes
            .iter()
            .enumerate()
            .filter(|&(index, byte)| *byte != 0 && (index == 0 || bytes[index - 1] == 0))
            .count();
        let mut runs = allocation::vec(count)?;
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == 0 {
                index += 1;
                continue;
            }
            let end = bytes[index..]
                .iter()
                .position(|&byte| byte == 0)
                .map_or(bytes.len(), |offset| index + offset);
            runs.push(ByteRun {
                start: index as u32,
                bytes: allocation::copy_vec(&bytes[index..end])?,
            });
            index = end;
        }
        Ok(Self { len, runs })
    }

    /// The native byte at `index`: zero between runs, `None` outside the array.
    pub fn get(&self, index: u32) -> Option<u8> {
        if index >= self.len {
            return None;
        }
        let after = self.runs.partition_point(|run| run.start <= index);
        Some(
            after
                .checked_sub(1)
                .and_then(|run| {
                    let run = &self.runs[run];
                    run.bytes.get((index - run.start) as usize).copied()
                })
                .unwrap_or(0),
        )
    }

    fn check(&self) -> io::Result<()> {
        let mut end = 0_u64;
        for (index, run) in self.runs.iter().enumerate() {
            let start = u64::from(run.start);
            if run.bytes.is_empty()
                || run.bytes.contains(&0)
                || (index != 0 && start <= end)
                || start + run.bytes.len() as u64 > u64::from(self.len)
            {
                return Err(invalid("sparse byte runs are not canonical"));
            }
            end = start + run.bytes.len() as u64;
        }
        Ok(())
    }
}

impl crate::value::Value for SparseBytes {
    const MINIMUM_ENCODED_BYTES: usize = 8;

    fn encoded_bytes(&self) -> io::Result<usize> {
        self.check()?;
        let mut end = 0_u32;
        self.runs.iter().try_fold(8, |size, run| {
            let gap = crate::rows::varint_bytes(u128::from(run.start - end));
            let length = crate::rows::varint_bytes(run.bytes.len() as u128);
            end = run.start + run.bytes.len() as u32;
            allocation::add(
                size,
                allocation::add(allocation::add(gap, length)?, run.bytes.len())?,
            )
        })
    }
    fn try_clone(&self) -> io::Result<Self> {
        let mut runs = allocation::vec(self.runs.len())?;
        for run in &self.runs {
            runs.push(ByteRun {
                start: run.start,
                bytes: allocation::copy_vec(&run.bytes)?,
            });
        }
        Ok(Self {
            len: self.len,
            runs,
        })
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.check()?;
        let count = u32::try_from(self.runs.len())
            .map_err(|_| invalid("sparse byte run count exceeds u32"))?;
        output.extend_from_slice(&self.len.to_le_bytes());
        output.extend_from_slice(&count.to_le_bytes());
        let mut end = 0_u32;
        for run in &self.runs {
            put_varint(output, u128::from(run.start - end));
            put_varint(output, run.bytes.len() as u128);
            output.extend_from_slice(&run.bytes);
            end = run.start + run.bytes.len() as u32;
        }
        Ok(())
    }

    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let len = input.u32()?;
        // A run takes at least a gap byte, a length byte and one value byte.
        let count = input.count(3)?;
        let mut runs = allocation::vec(count)?;
        let mut end = 0_u32;
        for index in 0..count {
            let gap = u32::try_from(input.varint()?)
                .map_err(|_| invalid("sparse byte gap exceeds u32"))?;
            let length = u32::try_from(input.varint()?)
                .map_err(|_| invalid("sparse byte run length exceeds u32"))?;
            if (index != 0 && gap == 0) || length == 0 {
                return Err(invalid("sparse byte runs are not canonical"));
            }
            let start = end
                .checked_add(gap)
                .ok_or_else(|| invalid("sparse byte run start exceeds u32"))?;
            end = start
                .checked_add(length)
                .filter(|&end| end <= len)
                .ok_or_else(|| invalid("sparse byte run ends outside its array"))?;
            let bytes = input.take(length as usize)?;
            if bytes.contains(&0) {
                return Err(invalid("sparse byte run holds a zero byte"));
            }
            runs.push(ByteRun {
                start,
                bytes: allocation::copy_vec(bytes)?,
            });
        }
        Ok(Self { len, runs })
    }
}

/// Hourly country supply operands, including empty inventories/censuses.
/// Allocation order and duplicate/null references survive. The access array
/// is shared by the country's SupplyNodes; index via CountryLinks canonical
/// slot. `controller_access` records the companion's byte array with its
/// count and every nonzero byte (see [`SparseBytes`]): an in-array index
/// not covered by a run is zero, which denies access like any zero byte,
/// and an out-of-array index is denied.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TrainInventoryRow {
    pub country: u32,
    pub node_count: u32,
    pub allocations: Vec<TrainAllocation>,
    pub controller_access: SparseBytes,
    /// Stored supply-system operands. NotRecorded in older formats; None is a
    /// counted capture refusal.
    pub allocated_trucks: Recorded<Option<i32>>,
    pub required_trucks_raw: Recorded<Option<i64>>,
    pub required_trains: Recorded<Option<i32>>,
}
typed_codec!(TrainInventoryRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (node_count): U32, 1 => (allocations): Opaque, 2 => (controller_access): Opaque, 3 => (allocated_trucks): OptionalI32, 4 => (required_trucks_raw): OptionalI64, 5 => (required_trains): OptionalI32 }, |_row| Ok(()));

/// Plan operands. Keys join ArmyOrders; bytes and reference bits are raw.
/// `has_child` is the native manage_child_sections byte, not a pointer.
/// `front_valid` records inequality to the engine's invalid-reference sentinel.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArmyOrderPlanRow {
    pub owner_group: u64,
    pub instance: u32,
    pub front: u64,
    pub front_valid: u8,
    pub section: u32,
    pub segment_start: i32,
    pub segment_end: i32,
    pub blitz: u8,
    pub withdraw: u8,
    pub fallback: u8,
    pub virtual_order: u8,
    pub has_child: u8,
    pub owner_override: u64,
    pub assigned_divisions: Vec<u64>,
    /// Native ordered +0x1E0 predecessor and +0x1F8 successor order
    /// instances, scoped to `owner_group`: the plan graph the map draws.
    pub predecessors: Vec<u32>,
    pub successors: Vec<u32>,
    /// Order-name strings, recorded only for move, invasion and paradrop
    /// orders (the arrows they name): localisation keys, localised in the
    /// viewer's language, except `name_unique`, which the game stores already
    /// localised at creation and every client draws as stored.
    pub name_prefix: Vec<u8>,
    pub name: Vec<u8>,
    pub name_first: Vec<u8>,
    pub name_second: Vec<u8>,
    pub name_unique: Vec<u8>,
    pub name_postfix: Vec<u8>,
    /// First-path controller-area count and stored child fronts. None is
    /// outside bound/nonempty-path demand or a field refusal; historical
    /// formats use NotRecorded. Earlier operands survive a later child-header
    /// or payload refusal.
    pub child_fronts: Recorded<Option<OrderChildFrontInputs>>,
    /// The embedded convoy subscriber's supplied convoy count, signed and
    /// read only for authenticated naval-invasion orders.
    pub invasion_transports: Recorded<Option<i32>>,
    /// Raw simulation route_is_ok and type4 preparation operands. Historical
    /// absence differs from a current reached field refusal; readiness is
    /// calculated by the viewer, never recorded.
    pub execution: Recorded<Option<OrderExecutionInputs>>,
}
typed_codec!(ArmyOrderPlanRow, 96, |row| (row.owner_group as u128) << 32 | row.instance as u128, |key| Self { owner_group: (key >> 32) as u64, instance: key as u32, ..Default::default() },
 { 0 => (front): U64, 1 => (front_valid): U8, 2 => (section): U32, 3 => (segment_start): I32, 4 => (segment_end): I32, 5 => (blitz): U8, 6 => (withdraw): U8, 7 => (fallback): U8, 8 => (virtual_order): U8, 9 => (has_child): U8, 10 => (owner_override): U64, 11 => (assigned_divisions): U64List, 12 => (predecessors): U32List, 13 => (successors): U32List, 14 => (name_prefix): Bytes, 15 => (name): Bytes, 16 => (name_first): Bytes, 17 => (name_second): Bytes, 18 => (name_unique): Bytes, 19 => (name_postfix): Bytes, 20 => (child_fronts): Opaque, 21 => (invasion_transports): OptionalI32, 22 => (execution): Opaque }, |row| {
 if let Recorded::Value(Some(inputs)) = &row.child_fronts {
  if inputs.area_province_count <= 2 && (inputs.count.is_some() || inputs.group_child_count.is_some() || inputs.sections.is_some()) { return Err(invalid("small-area child operands outside native demand")); }
  if inputs.group_child_count.is_some() && inputs.count.is_none_or(|count| count == 0) { return Err(invalid("army-group child count outside native demand")); }
  if let Some(sections) = &inputs.sections {
   let prefix = inputs.count.and_then(|count| if count == 0 { Some(0) } else { inputs.group_child_count.filter(|&group_count| group_count > 0).map(|group_count| count.min(group_count as u32) as usize) });
   if prefix != Some(sections.len()) || sections.iter().any(|section| section.group == 0 || section.group as u32 != engine::corders_group::REFERENCE_TYPE) { return Err(invalid("invalid demanded child-front prefix or group identity")); }
   if !sections.is_empty() && inputs.group_child_count.is_none_or(|count| count <= 0) { return Err(invalid("stored child spans outside native group-count demand")); }
  }
 }
 Ok(())
});

/// Stored order route-validity byte (saved route_is_ok, token3937), independent from active
/// and can_execute. Only type3/4 have a pack; unreadable route bytes remain
/// None. Type4 reaches preparation only for a known nonzero route byte.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OrderExecutionInputs {
    pub route_is_ok: Option<u8>,
    pub paradrop: Option<ParadropPreparationInputs>,
}
value_struct!(OrderExecutionInputs { route_is_ok: Option<u8>, paradrop: Option<ParadropPreparationInputs> });
opaque_value!(Option<OrderExecutionInputs>);

/// Type4 accumulated preparation+ D8. A later modifier/target refusal keeps
/// this saved progress while withholding only the complete term group.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ParadropPreparationInputs {
    pub progress: i64,
    pub terms: Option<ParadropPreparationTerms>,
}
value_struct!(ParadropPreparationInputs { progress: i64, terms: Option<ParadropPreparationTerms> });

/// Raw type4 preparation operands. Defines come from the recorded content
/// stack. An absent relation/leader term means a known skip, whereas
/// failure withholds the whole terms group.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ParadropPreparationTerms {
    pub target: u32,
    pub country_term: i64,
    pub relation_term: Option<i64>,
    pub leader_term: Option<i64>,
    pub days_term: i64,
}
value_struct!(ParadropPreparationTerms { target: u32, country_term: i64, relation_term: Option<i64>, leader_term: Option<i64>, days_term: i64 });

/// First path controller-area raw signed count. At <=2 the renderer
/// reaches neither child header nor payload. Above2, count None is a header
/// refusal. A positive AG/type2 count reaches group_child_count, the signed
/// Army-group bound from this same acquisition; zero reaches no spans.
/// Positive group_child_count reaches exactly min(count, group_child_count)
/// ordered stored entries; sections None is a demanded-prefix refusal.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OrderChildFrontInputs {
    pub area_province_count: i32,
    pub count: Option<u32>,
    pub group_child_count: Option<i32>,
    pub sections: Option<Vec<OrderChildFront>>,
}
value_struct!(OrderChildFrontInputs { area_province_count: i32, count: Option<u32>, group_child_count: Option<i32>, sections: Option<Vec<OrderChildFront>> });
opaque_value!(Option<OrderChildFrontInputs>);

/// Stored child-front operands. Order and duplicate groups are retained;
/// signed spans are relative to the parent order's segment start.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OrderChildFront {
    pub group: u64,
    pub segment_start: i32,
    pub segment_end: i32,
}
value_struct!(OrderChildFront {
    group: u64,
    segment_start: i32,
    segment_end: i32
});

#[cfg(test)]
#[path = "movement/order_children_tests.rs"]
mod order_children_tests;

/// A group's raw colour and ordered primary and fallback root orders,
/// as instance ids scoped to this group.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OrdersGroupPlanRow {
    pub group: u64,
    pub red: u32,
    pub green: u32,
    pub blue: u32,
    pub alpha: u32,
    pub primary_orders: Vec<u32>,
    pub fallback_orders: Vec<u32>,
    /// The stored group name drawn by plan labels.
    pub name: Vec<u8>,
}
typed_codec!(OrdersGroupPlanRow, 64, |row| row.group as u128, |key| Self { group: key as u64, ..Default::default() },
 { 0 => (red): U32, 1 => (green): U32, 2 => (blue): U32, 3 => (alpha): U32, 4 => (primary_orders): U32List, 5 => (fallback_orders): U32List, 6 => (name): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FrontRow {
    pub front: u64,
    pub section_ids: Vec<u32>,
    pub section_ends: Vec<u32>,
    pub province_pairs: Vec<u32>,
}
// Each section_ends entry is the exclusive u32-word offset into province_pairs.
// Section IDs and pairs retain native array order, including empty sections/fronts.
typed_codec!(FrontRow, 64, |row| row.front as u128, |key| Self { front: key as u64, ..Default::default() },
{ 0 => (section_ids): U32List, 1 => (section_ends): U32List, 2 => (province_pairs): U32List }, |row| {
 if row.section_ids.len() != row.section_ends.len() || !row.province_pairs.len().is_multiple_of(2) { return Err(invalid("invalid front section arrays")); }
 let mut end = 0;
 let mut ids = allocation::copy_vec(&row.section_ids)?;
 ids.sort_unstable();
 if ids.windows(2).any(|pair| pair[0] == pair[1]) { return Err(invalid("invalid front section range or duplicate id")); }
 for &next in &row.section_ends {
  if next < end || !next.is_multiple_of(2) || next as usize > row.province_pairs.len() { return Err(invalid("invalid front section range or duplicate id")); }
  end = next;
 }
 if end as usize != row.province_pairs.len() { return Err(invalid("unclaimed front pairs")); }
 Ok(())
});

/// FactionTheatres: one row per faction theatre in stored manager order
/// (0xA8-byte entries), saved as `pings.regions`. The political map draws
/// one `faction_ping_map_icon` per theatre on its first region for every
/// faction member. All fields are stored.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FactionTheatreRow {
    /// Full native CFaction reference, as in Factions.
    pub faction: u64,
    /// Native index in the theatre array.
    pub ordinal: u32,
    /// The faction's sprite key (save `icon`); the pin draws `<icon>_miniature`.
    pub icon: Vec<u8>,
    /// Theatre +0x48 strategic region ids in native order; the pin sits over
    /// the first region and its anchor averages every listed region.
    pub regions: Vec<u32>,
    /// Count of the theatre's +0x30 country list, the number under the pin.
    pub country_count: u32,
    /// Raw country links whose theatre pin is hidden, in stored order.
    pub hidden_for: Vec<u32>,
}
typed_codec!(FactionTheatreRow, 96, |row| (row.faction as u128) << 32 | row.ordinal as u128,
 |key| Self { faction: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
 { 0 => (icon): Bytes, 1 => (regions): U32List, 2 => (country_count): U32, 3 => (hidden_for): U32List }, |_row| Ok(()));
/// Hourly ETA operands, keyed by the full Divisions handle, emitted only for a
/// division with a nonempty route. Every value is already composed the way the
/// engine composes it, so a replayer reproduces the native ETA bit for bit
/// without reading memory: the per-division speed graph collapses to the five
/// prepared terms, and everything that varies per route leg is a parallel
/// vector in route order. `leg_river_combined`, `leg_transport_speed` and
/// `leg_rail_speed` are empty when no leg uses that branch; otherwise they are
/// leg-aligned and zero on legs that do not. The vectors cover the legs the
/// native walk evaluates, so they end at the first zero-speed leg, not at the
/// end of the route.
/// One terrain the route touches: its one-based native index (`CTerrainType
/// +0x88`, an i32 the content's category count bounds, not a byte) and the
/// composed sub-unit plus leader modifier.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TerrainSpeed {
    pub terrain: i32,
    pub modifier: i64,
}
value_struct!(TerrainSpeed {
    terrain: i32,
    modifier: i64
});

/// A complete moving-division table is published every hour, including carried
/// rows. Consumers call valid_at before deriving. A seek needs no earlier
/// witness state. The owner-approved five-hour operand cadence is an exception
/// inside this hourly table.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionEtaInputsRow {
    pub division: u64,
    /// Latest full operand read; unchanged rows are carried between stagger slots.
    pub sample_hour: i32,
    /// `NormalSpeedOperands::prepare()`: the whole per-division modifier graph.
    pub speed_modifier: i64,
    pub speed_base: i64,
    pub speed_org: Option<i64>,
    pub speed_fuel: Option<i64>,
    /// The (location -> next) edge speed, evaluated once per division. Recorded
    /// because that call alone may pass a null destination.
    pub current_speed: i64,
    /// Each terrain the route touches, once, in the order the scan first
    /// reaches it. A terrain not listed was not read.
    pub terrain_speed: Vec<TerrainSpeed>,
    pub fort_penalty: i64,
    pub snow_penalty: i64,
    pub river_adjustment: i64,
    pub river_leader: i64,
    pub class: u32,
    /// Effective modifier operands of the strategic-rail gate: country,
    /// current state and friendly current state. Absent when that getter
    /// branch was not queried.
    pub rail_country_modifier: Option<i64>,
    pub rail_state_modifier: Option<i64>,
    pub rail_friendly_state_modifier: Option<i64>,
    pub leg_river_combined: Vec<i64>,
    pub leg_transport_speed: Vec<i64>,
    pub leg_rail_speed: Vec<i64>,
}
typed_codec!(DivisionEtaInputsRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (speed_modifier): I64, 1 => (speed_base): I64, 2 => (speed_org): OptionalI64, 3 => (speed_fuel): OptionalI64, 4 => (current_speed): I64, 5 => (terrain_speed): Opaque, 6 => (fort_penalty): I64, 7 => (snow_penalty): I64, 8 => (river_adjustment): I64, 9 => (river_leader): I64, 10 => (class): U32, 11 => (rail_country_modifier): OptionalI64, 12 => (rail_state_modifier): OptionalI64, 13 => (rail_friendly_state_modifier): OptionalI64, 14 => (leg_river_combined): Opaque, 15 => (leg_transport_speed): Opaque, 16 => (leg_rail_speed): Opaque, 17 => (sample_hour): I32 }, |row| {
 let mut lengths = [&row.leg_river_combined, &row.leg_transport_speed, &row.leg_rail_speed]
  .into_iter()
  .map(Vec::len)
  .filter(|&len| len != 0);
 if let Some(first) = lengths.next() && lengths.any(|len| len != first) { return Err(invalid("ETA input leg branch vectors differ in length")); }
 for (at, entry) in row.terrain_speed.iter().enumerate() {
  if entry.terrain <= 0 { return Err(invalid("ETA input terrain index is not a database slot")); }
  if row.terrain_speed[..at].iter().any(|seen| seen.terrain == entry.terrain) { return Err(invalid("ETA input terrain listed twice")); }
 }
 Ok(())
});

impl DivisionEtaInputsRow {
    /// Approved operand age; the game's counter TTL starts at five hours and
    /// decays hourly. This is not cache-history parity.
    pub fn valid_at(&self, hour: i32) -> bool {
        (0..5).contains(&(i64::from(hour) - i64::from(self.sample_hour)))
    }
}

/// Shared native operand/status types; replay owns their compact wire encoding.
pub use engine::carmy::{StrategicAction, StrategicActionFailure, StrategicRedeploymentInputs};

value_struct!(StrategicRedeploymentInputs {
    country_modifier: i64,
    state_modifier: Option<i64>,
    friendly_state_modifier: Option<i64>,
});

impl crate::value::Value for StrategicAction {
    const MINIMUM_ENCODED_BYTES: usize = 1;

    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(match self {
            Self::Unreached => 1,
            Self::Available(inputs) => 1 + crate::value::Value::encoded_bytes(inputs)?,
            Self::Withheld(_) => 2,
        })
    }

    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }

    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Unreached => output.push(0),
            Self::Available(inputs) => {
                output.push(1);
                crate::value::Value::encode_reserved(inputs, output)?;
            }
            Self::Withheld(reason) => {
                output.push(2);
                output.push(match reason {
                    StrategicActionFailure::Owner => 0,
                    StrategicActionFailure::CountryModifier => 1,
                    StrategicActionFailure::CurrentState => 2,
                    StrategicActionFailure::StateModifier => 3,
                    StrategicActionFailure::Friendly => 4,
                    StrategicActionFailure::FriendlyStateModifier => 5,
                });
            }
        }
        Ok(())
    }

    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(match input.u8()? {
            0 => Self::Unreached,
            1 => Self::Available(crate::value::Value::decode(input)?),
            2 => Self::Withheld(match input.u8()? {
                0 => StrategicActionFailure::Owner,
                1 => StrategicActionFailure::CountryModifier,
                2 => StrategicActionFailure::CurrentState,
                3 => StrategicActionFailure::StateModifier,
                4 => StrategicActionFailure::Friendly,
                5 => StrategicActionFailure::FriendlyStateModifier,
                _ => return Err(invalid("strategic action refusal code invalid")),
            }),
            _ => return Err(invalid("strategic action status invalid")),
        })
    }
}
opaque_value!(StrategicAction);

/// Hourly, keyed by the full Divisions handle; native scalar widths are preserved.
/// Same-hour raw native transport operands for every authenticated class0/13
/// unit, including idle, unassigned and transferred units. Each failed sibling
/// stays absent independently; an absent transfer is explicit native null.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UnitTransportInputs {
    pub sample_hour: i32,
    pub weight: Option<i64>,
    pub transfer: Option<UnitTransferInputs>,
    /// Only army transport reads the actual template's grouped-member predicate.
    pub coastal_template: Option<bool>,
}
value_struct!(UnitTransportInputs {
    sample_hour: i32, weight: Option<i64>, transfer: Option<UnitTransferInputs>,
    coastal_template: Option<bool>
});
opaque_value!(Option<UnitTransportInputs>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitTransferInputs {
    Absent,
    Present {
        transfer: u64,
        status: Option<i32>,
        source: Option<u32>,
    },
}
impl crate::value::Value for UnitTransferInputs {
    const MINIMUM_ENCODED_BYTES: usize = 1;

    fn encoded_bytes(&self) -> io::Result<usize> {
        use crate::value::Value;
        Ok(match self {
            Self::Absent => 1,
            Self::Present {
                transfer,
                status,
                source,
            } => {
                1 + Value::encoded_bytes(transfer)?
                    + Value::encoded_bytes(status)?
                    + Value::encoded_bytes(source)?
            }
        })
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        use crate::value::Value;
        match self {
            Self::Absent => output.push(0),
            Self::Present {
                transfer,
                status,
                source,
            } => {
                output.push(1);
                Value::encode_reserved(transfer, output)?;
                Value::encode_reserved(status, output)?;
                Value::encode_reserved(source, output)?;
            }
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match input.u8()? {
            0 => Ok(Self::Absent),
            1 => Ok(Self::Present {
                transfer: crate::value::Value::decode(input)?,
                status: crate::value::Value::decode(input)?,
                source: crate::value::Value::decode(input)?,
            }),
            _ => Err(invalid("unit transport transfer discriminant invalid")),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionMovementRow {
    pub division: u64,
    pub source: u32,
    pub target: u32,
    pub numerator: i64,
    pub retreat: u8,
    pub strategic_redeployment: u8,
    pub naval_transfer: u8,
    pub combat_count: u32,
    pub path_total: u32,
    pub path: Vec<u32>,
    pub alternate_total: u32,
    pub alternate_path: Vec<u32>,
    pub combat_role: u8,
    /// Province of the first referenced combat; zero when there is none.
    pub combat_province: u32,
    /// 1 when the unit's map entity is set.
    pub map_entity: u8,
    /// Present only for class-0 strategic action 6/11: signed path count > 0,
    /// null target, no retreat, strategic byte nonzero. A withheld status
    /// affects only this glyph; operands never carry ETA age.
    pub strategic_action: Recorded<StrategicAction>,
    /// Raw saved `withdraw` byte behind the withdrawal arrow. None is
    /// an isolated guarded-read refusal; historical files use NotRecorded.
    pub withdrawal: Recorded<Option<u8>>,
    /// Raw signed move priority, saved as `move_priority`; movement arrows
    /// use <1 only for class0 follow-orders style.
    pub movement_priority: Recorded<Option<i32>>,
    /// Same-pass class0 assigned-unit transport operands. Independent sibling
    /// fields preserve partial availability. No unit selection or derived
    /// demand/capacity/readiness is stored.
    pub order_transport: Recorded<Option<DivisionOrderTransportInputs>>,
    /// Current-hour domains for a freshly searched counter route, independent
    /// of the legacy stored-route operands and their carried sample hour.
    /// None is a counted acquisition refusal; older formats use NotRecorded.
    pub fresh: Recorded<Option<FreshCounterEta>>,
    /// Complete ordered unit combat references from this callback hour.
    /// Repeats and runtime-minted full handles are retained. None is an
    /// independent acquisition refusal; historical absence is NotRecorded.
    /// Entry count equals combat_count; no selection or derived query is stored.
    pub combat_references: Recorded<Option<UnitCombatReferences>>,
    pub selected: Recorded<Option<SelectedUnitInputs>>,
    pub transport: Recorded<Option<UnitTransportInputs>>,
}
typed_codec!(DivisionMovementRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (source): U32, 1 => (target): U32, 2 => (numerator): I64, 3 => (retreat): U8, 4 => (strategic_redeployment): U8, 5 => (naval_transfer): U8, 6 => (combat_count): U32, 7 => (path_total): U32, 8 => (path): U32List, 9 => (alternate_total): U32, 10 => (alternate_path): U32List, 11 => (combat_role): U8, 12 => (combat_province): U32, 13 => (map_entity): U8, 14 => (strategic_action): Opaque, 15 => (withdrawal): Opaque, 16 => (movement_priority): OptionalI32, 17 => (order_transport): Opaque, 18 => (fresh): Opaque, 19 => (combat_references): Opaque, 20 => (selected): Opaque, 21 => (transport): Opaque }, |row| {
 check_unit_combat_references(&row.combat_references, row.combat_count)?;
 let reached = (row.path_total as i32) > 0 && row.target == 0
     && row.retreat == 0 && row.strategic_redeployment != 0;
 let Recorded::Value(action) = &row.strategic_action else { return Ok(()); };
 match (reached, action) {
     (true, StrategicAction::Available(_) | StrategicAction::Withheld(_)) => Ok(()),
     (true, StrategicAction::Unreached) => Err(invalid("strategic redeployment action operands missing")),
     (false, StrategicAction::Available(_) | StrategicAction::Withheld(_)) => Err(invalid("strategic redeployment action branch not reached")),
     (false, StrategicAction::Unreached) => Ok(()),
 }
});

/// Naval stat+2B8 and paradrop stat+5B5 are independent raw observations.
/// Para modifiers are present only for a unit contributing at its exact
/// owned primary root and current source province. Missing reached operands
/// are refusals; a viewer must not infer zero from None.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DivisionOrderTransportInputs {
    pub naval_weight: Option<i64>,
    pub paradrop_eligible: Option<u8>,
    pub paradrop_modifiers: Option<ParadropTransportModifiers>,
}
value_struct!(DivisionOrderTransportInputs { naval_weight: Option<i64>, paradrop_eligible: Option<u8>, paradrop_modifiers: Option<ParadropTransportModifiers> });
opaque_value!(Option<DivisionOrderTransportInputs>);

/// Key171 terms: unit roster-owner country plus the selected per-unit
/// commander accumulator. None is a known absent commander; a failed
/// selector withholds the complete modifier pack.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ParadropTransportModifiers {
    pub country_term: i64,
    pub commander_term: Option<i64>,
}
value_struct!(ParadropTransportModifiers { country_term: i64, commander_term: Option<i64> });

#[cfg(test)]
#[path = "movement/order_execution_tests.rs"]
mod order_execution_tests;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnitSupplyDemand {
    pub count: i32,
    pub value: i64,
}
value_struct!(UnitSupplyDemand {
    count: i32,
    value: i64
});

/// Every native distributed-supply row, in original order. Nonpositive values
/// and duplicate provinces affect totals and must survive recording.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnitSupplyOrigin {
    pub province: i32,
    pub requested: i64,
    pub received: i64,
}
value_struct!(UnitSupplyOrigin {
    province: i32,
    requested: i64,
    received: i64
});

/// Stored supply-consumer operands for aggregation and its footer. The
/// disruption operand is overwritten by the last aggregate member. These
/// amounts cannot be recovered from the stored supply ratio or hub table.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnitSupplyInputs {
    pub asked: i64,
    pub received: i64,
    pub from_state: i64,
    pub air_drops: i64,
    pub disruption: i64,
    pub division_demand: UnitSupplyDemand,
    pub airplane_demand: UnitSupplyDemand,
    pub ship_demand: UnitSupplyDemand,
    /// None is a counted node-fragment read/capacity failure. The independent
    /// scalar footer lines remain available; Some(empty) records zero origins.
    pub origins: Option<Vec<UnitSupplyOrigin>>,
}
value_struct!(UnitSupplyInputs { asked: i64, received: i64, from_state: i64,
    air_drops: i64, disruption: i64, division_demand: UnitSupplyDemand,
    airplane_demand: UnitSupplyDemand, ship_demand: UnitSupplyDemand,
    origins: Option<Vec<UnitSupplyOrigin>> });
opaque_value!(Option<UnitSupplyInputs>);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AttritionBorder {
    pub state: Option<u32>,
    pub country: Option<u32>,
    pub factor: Option<i64>,
}
value_struct!(AttritionBorder { state: Option<u32>, country: Option<u32>, factor: Option<i64> });
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AttritionSupply {
    pub penalty: Option<i64>,
    pub attrition_factor: Option<i64>,
}
value_struct!(AttritionSupply { penalty: Option<i64>, attrition_factor: Option<i64> });
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AttritionThermal {
    pub base: i64,
    pub country_factor: Option<i64>,
    pub leader_factor: Option<Option<i64>>,
}
value_struct!(AttritionThermal { base: i64, country_factor: Option<i64>, leader_factor: Option<Option<i64>> });
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AttritionWeather {
    pub acclimatization: Option<i64>,
    pub attrition: Option<i64>,
    pub heat: Option<Option<AttritionThermal>>,
    pub winter: Option<Option<AttritionThermal>>,
}
value_struct!(AttritionWeather { acclimatization: Option<i64>, attrition: Option<i64>,
    heat: Option<Option<AttritionThermal>>, winter: Option<Option<AttritionThermal>> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttritionTerrain {
    /// Terrain localisation key, resolved in the viewer's language. A failed
    /// key copy does not erase independent numeric terms.
    pub name: Option<Vec<u8>>,
    pub attrition: Option<i64>,
    pub equipment: Option<i64>,
}
value_struct!(AttritionTerrain { name: Option<Vec<u8>>, attrition: Option<i64>, equipment: Option<i64> });

/// Runtime accumulator identity joins the narrow hourly breakdown table. Native
/// formatter depth and scaling are per-use, independent of tree deduplication.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AttritionModifierSource {
    pub accumulator: u64,
    pub key: u32,
    pub depth: i32,
    pub scale: i64,
}
value_struct!(AttritionModifierSource {
    accumulator: u64,
    key: u32,
    depth: i32,
    scale: i64
});

/// Fresh attrition operands, distinct from the earlier counter cache.
/// None is a counted independent read failure; Some(None) is a skipped
/// optional branch. Source/label failures never participate in numeric totals.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArmyAttritionInputs {
    pub province: Option<Option<u32>>,
    pub upkeep: Option<i64>,
    pub terrain: Option<Option<AttritionTerrain>>,
    pub state: Option<Option<i64>>,
    pub supply: Option<AttritionSupply>,
    pub country_upkeep_modifier: Option<i64>,
    pub moving_state: Option<Option<i64>>,
    pub weather: Option<Option<AttritionWeather>>,
    pub training: Option<bool>,
    pub training_attrition: Option<Option<i64>>,
    pub border: Option<Option<AttritionBorder>>,
    pub country_attrition_modifier: Option<i64>,
    pub sources: [Option<Option<AttritionModifierSource>>; engine::carmy_attrition::SOURCE_COUNT],
}
value_struct!(ArmyAttritionInputs { province: Option<Option<u32>>, upkeep: Option<i64>,
    terrain: Option<Option<AttritionTerrain>>, state: Option<Option<i64>>,
    supply: Option<AttritionSupply>, country_upkeep_modifier: Option<i64>,
    moving_state: Option<Option<i64>>, weather: Option<Option<AttritionWeather>>,
    training: Option<bool>, training_attrition: Option<Option<i64>>,
    border: Option<Option<AttritionBorder>>, country_attrition_modifier: Option<i64>,
    sources: [Option<Option<AttritionModifierSource>>; engine::carmy_attrition::SOURCE_COUNT] });
opaque_value!(ArmyAttritionInputs);

impl ArmyAttritionInputs {
    /// Playback uses the engine's same ordered wrapping fixed5 arithmetic.
    pub fn evaluate(&self, border_ratio: Option<i64>) -> Result<i64, &'static str> {
        let missing = "recorded attrition numeric operand unavailable";
        engine::carmy_attrition::Inputs {
            province: self.province.ok_or(missing),
            upkeep: self.upkeep.ok_or(missing),
            terrain: self.terrain.as_ref().ok_or(missing).map(|value| {
                value
                    .as_ref()
                    .map(|value| engine::carmy_attrition::Terrain {
                        name: Ok(0),
                        attrition: value.attrition.ok_or(missing),
                        equipment: value.equipment.ok_or(missing),
                    })
            }),
            state: self.state.ok_or(missing),
            supply: self
                .supply
                .ok_or(missing)
                .map(|supply| engine::carmy_attrition::Supply {
                    penalty: supply.penalty.ok_or(missing),
                    attrition_factor: supply.attrition_factor.ok_or(missing),
                }),
            country_upkeep_modifier: self.country_upkeep_modifier.ok_or(missing),
            moving_state: self.moving_state.ok_or(missing),
            weather: self.weather.ok_or(missing).map(|weather| {
                weather.map(|weather| {
                    let thermal = |value: Option<Option<AttritionThermal>>| {
                        value.ok_or(missing).map(|value| {
                            value.map(|value| engine::carmy_attrition::Thermal {
                                base: value.base,
                                country_factor: value.country_factor.ok_or(missing),
                                leader_factor: value.leader_factor.ok_or(missing),
                            })
                        })
                    };
                    engine::carmy_attrition::Weather {
                        acclimatization: weather.acclimatization.ok_or(missing),
                        attrition: weather.attrition.ok_or(missing),
                        heat: thermal(weather.heat),
                        winter: thermal(weather.winter),
                    }
                })
            }),
            training: self.training.ok_or(missing),
            training_attrition: self.training_attrition.ok_or(missing),
            border: self.border.ok_or(missing).map(|border| {
                border.map(|border| engine::carmy_attrition::Border {
                    state: border.state.ok_or(missing),
                    country: border.country.ok_or(missing),
                    factor: border.factor.ok_or(missing),
                })
            }),
            country_attrition_modifier: self.country_attrition_modifier.ok_or(missing),
            sources: [Ok(None); engine::carmy_attrition::SOURCE_COUNT],
        }
        .evaluate(border_ratio)
    }
}

/// Hourly, keyed by the full Divisions handle; native scalar widths are preserved.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionStatusRow {
    pub division: u64,
    pub maximum_hit_points: i64,
    pub maximum_organisation: i64,
    pub entrenchment: i64,
    pub entrenchment_cap: i64,
    pub supply_ratio: i64,
    pub attrition_value: i64,
    pub attrition_threshold: i64,
    pub attrition: u8,
    pub exercising: u8,
    /// Whether an order of the unit's group or its parent holds it; false for
    /// an ungrouped unit.
    pub has_order: u8,
    /// The byte that gates the yellow `invalid_orders` glyph.
    pub invalid_orders: u8,
    /// Stored fixed5 strength factor, multiplied by the plain strength ratio
    /// to produce the counter's strength-bar value.
    pub strength_factor: i64,
    /// The stored land model number, rewritten once a day per division and on
    /// template change, deploy and unit creation; the entity suffix is its
    /// sign-extended low byte.
    pub model_number: i32,
    /// `NAcclimatization::CData` maximum, raw; `None` when this division's
    /// acclimatization could not be read this hour, and then the entries are
    /// empty. The rest of the row stands.
    pub acclimatization_maximum: Option<i64>,
    /// Every acclimatization entry in native order, repeats kept. The
    /// camouflage winner is the viewer's to pick.
    pub acclimatization: Vec<DivisionAcclimatization>,
    /// Raid army-reach inputs, one bit each (the `MOVE_*` constants). The
    /// commander's retreat bans are [`DivisionRetreatBanRow`]'s, read daily.
    pub move_flags: u8,
    /// Cached defense, soft attack and hard attack, signed fixed5. Present
    /// only for class0 armies in states whose StateFlags.border_conflict_active
    /// is nonzero. None is outside demand; a failed demanded read withholds
    /// the table.
    pub border_combat_stats: Recorded<Option<[i64; 3]>>,
    /// Member-name generator inputs. None on guarded failure or arena shortage
    /// (counted by capture); older formats use NotRecorded.
    pub naming: Recorded<Option<NameOperands>>,
    /// Stored fixed5 grace operands. Older formats use NotRecorded; zero is
    /// a recorded value.
    pub supply_grace_maximum: Recorded<i64>,
    pub supply_grace_remaining: Recorded<i64>,
    /// Stored division roster position. None is a counted capture refusal;
    /// roster swap-removal changes this.
    pub country_ordinal: Recorded<Option<u32>>,
    /// Signed invalid-order reason, read only when invalid_orders is nonzero.
    /// None is outside that demand or a counted field-local capture failure;
    /// older formats use NotRecorded.
    pub invalid_orders_reason: Recorded<Option<i32>>,
    /// Native low-supply footer amounts. None is a counted scalar-pack failure;
    /// older recordings use NotRecorded without affecting the status prefix.
    pub supply: Recorded<Option<UnitSupplyInputs>>,
    pub attrition_inputs: Recorded<ArmyAttritionInputs>,
    /// Fresh entrenchment factor: ONE plus the selected commander's
    /// combat_entrenchment modifier, with wrapping addition. None is an
    /// independent read refusal; this is not a derived duration.
    pub dig_in_factor: Recorded<Option<i64>>,
    /// Exact cached name, including empty bytes. None is a counted isolated
    /// read/capacity refusal; older formats use NotRecorded.
    pub name: Recorded<Option<Vec<u8>>>,
    /// Current accepted-pass cached attack operands, with independent leaf refusals.
    pub attack: Recorded<DivisionAttackInputs>,
}
impl DivisionStatusRow {
    /// A nonzero exile byte substitutes for province move access and allows
    /// crossing controller areas within one state owner.
    pub const MOVE_EXILED: u8 = 1;
    /// The cached `can_exfiltrate_from_coast` operand, passed to the path request.
    pub const MOVE_CAN_EXFILTRATE_FROM_COAST: u8 = 2;
}
typed_codec!(DivisionStatusRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (maximum_hit_points): I64, 1 => (maximum_organisation): I64, 2 => (entrenchment): I64, 3 => (entrenchment_cap): I64, 4 => (supply_ratio): I64, 5 => (attrition_value): I64, 6 => (attrition_threshold): I64, 7 => (attrition): U8, 8 => (exercising): U8, 9 => (has_order): U8, 10 => (invalid_orders): U8, 11 => (strength_factor): I64, 12 => (model_number): I32, 13 => (acclimatization_maximum): OptionalI64, 14 => (acclimatization): Opaque, 15 => (move_flags): U8, 16 => (border_combat_stats): Opaque, 17 => (naming): Opaque, 18 => (supply_grace_maximum): I64, 19 => (supply_grace_remaining): I64, 20 => (country_ordinal): OptionalU32, 21 => (invalid_orders_reason): OptionalI32, 22 => (supply): Opaque, 23 => (attrition_inputs): Opaque, 24 => (dig_in_factor): OptionalI64, 25 => (name): Opaque, 26 => (attack): Opaque }, |row| {
    if let Recorded::Value(packet) = &row.attack { packet.check()?; }
    Ok(())
});

/// DivisionRetreatBans: key division (64 bits). A division whose commander
/// bans retreat: bits of the leader modifier block selected for it,
/// whatever its combats; a division without a ban has no row. The game
/// applies them only when the division stands on the defending or attacking
/// side of one of its combats (LandCombats defenders/attackers), which
/// the reader checks at its own hour.
///
/// Sliced, daily (family `retreat_bans`, owner: the canonical slot of the
/// division's roster owner): each owner's divisions are re-read in its daily
/// 1/24 slice, with no hook or change fingerprint (owner rule: no harsher
/// than the game, which re-checks raid offers in a rolling sweep); an owner
/// new to the census, or whose read was not published, is read at the next
/// hour. `SliceReads` holds each owner's read hour, rows or none; a division
/// created after its owner's read has no row until the next one. A division
/// whose roster owner cannot be resolved could belong to any read owner, so
/// that hour's read returns no rows and is truncated: every earlier ban
/// stands, and the owners are read again at the next hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionRetreatBanRow {
    pub division: u64,
    /// Canonical slot of the roster owner the division was read under.
    pub owner: u32,
    /// [`Self::DEFENDING`] and [`Self::ATTACKING`].
    pub bans: u8,
}
impl DivisionRetreatBanRow {
    /// The selected block has `cannot_retreat_while_defending` (0x297) > 0.
    pub const DEFENDING: u8 = 4;
    /// The selected block has `cannot_retreat_while_attacking` (0x298) > 0.
    pub const ATTACKING: u8 = 8;
}
typed_codec!(DivisionRetreatBanRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (owner): U32, 1 => (bans): U8 }, |_row| Ok(()));

/// One acclimatization entry: the category's lexer token and its raw weight.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionAcclimatization {
    pub category: u32,
    pub weight: i64,
}
value_struct!(DivisionAcclimatization {
    category: u32,
    weight: i64
});

/// Daily template dictionary: names and the native `unit_type` category, not a
/// proved sprite frame. What a template edit changes for the counter and the
/// 3D model is hourly in [`DivisionTemplateIconRow`]. Effective maxima are
/// per-division.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionTemplateRow {
    pub template: u64,
    pub owner: u32,
    pub name: Vec<u8>,
    /// The composition's display-name localisation fallback.
    pub name_key: Recorded<Vec<u8>>,
    pub unit_type: i32,
}
typed_codec!(DivisionTemplateRow, 64, |row| row.template as u128,
 |key| Self { template: key as u64, ..Default::default() },
 { 0 => (owner): U32, 1 => (name): Bytes, 2 => (unit_type): I32, 3 => (name_key): Bytes }, |_row| Ok(()));

/// One grouped template entry. Token zero is a null-object definition.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TemplateSubunit {
    pub token: u32,
    /// Nonzero for a country-local clone of the token's definition.
    pub local: u8,
    /// Native signed count.
    pub count: i32,
}
value_struct!(TemplateSubunit {
    token: u32,
    local: u8,
    count: i32
});

/// Hourly companion of LandCombats, keyed by its full native handle. Fixed
/// progress values have denominator 100,000; the defender value is the engine's
/// complemented attacker fraction, not a second independently rounded ratio.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatProgressRow {
    pub combat: u64,
    pub attacker_progress: i64,
    pub defender_progress: i64,
    pub direction_attacker: u64,
    pub direction_province: u32,
    /// Exact signed native loss counters, dated by this table's hourly read.
    pub attacker_manpower_loss: Recorded<i32>,
    pub defender_manpower_loss: Recorded<i32>,
    /// Complete native pools; independent delta fields avoid rewriting an
    /// unchanged pool when only manpower/progress or the other side changes.
    pub attacker_equipment_losses: Recorded<Vec<LandEquipmentLoss>>,
    pub defender_equipment_losses: Recorded<Vec<LandEquipmentLoss>>,
}
typed_codec!(LandCombatProgressRow, 64, |row| row.combat as u128,
 |key| Self { combat: key as u64, ..Default::default() },
 { 0 => (attacker_progress): I64, 1 => (defender_progress): I64,
   2 => (direction_attacker): U64, 3 => (direction_province): U32,
   4 => (attacker_manpower_loss): I32, 5 => (defender_manpower_loss): I32,
   6 => (attacker_equipment_losses): Opaque, 7 => (defender_equipment_losses): Opaque }, |_row| Ok(()));

/// PreferredTactics: key country (32 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PreferredTacticRow {
    pub country: u32,
    pub tactic_id: i32,
    pub token: Vec<u8>,
}
typed_codec!(PreferredTacticRow, 32, |row| row.country as u128, |key| Self { country: key as u32, tactic_id: Default::default(), token: Default::default() },
 { 0 => (tactic_id): I32, 1 => (token): Bytes }, |_row| Ok(()));

/// CountryTacticUnlocks: key country (32 bits), one payload mask bit. Hourly.
///
/// Every country's doctrine-unlocked tactics, as database IDs saved under
/// `enable_tactic`, in stored vector order. An empty list means no unlocks.
/// The unlock token compared by triggers belongs to the tactic definition
/// rather than the country; playback derives it from content.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryTacticUnlockRow {
    /// Raw country reference.
    pub country: u32,
    pub unlocked_tactic_ids: Vec<u32>,
}
typed_codec!(CountryTacticUnlockRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (unlocked_tactic_ids): U32List }, |_row| Ok(()));

/// ProvinceSupply: key province (32 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProvinceSupplyRow {
    pub province: u32,
    pub controller_index: u16,
    pub capital_country_index: u16,
    pub supply_node_level: u8,
    pub naval_base_level: u8,
    pub flags: u8,
}
typed_codec!(ProvinceSupplyRow, 32, |row| row.province as u128, |key| Self { province: key as u32, controller_index: Default::default(), capital_country_index: Default::default(), supply_node_level: Default::default(), naval_base_level: Default::default(), flags: Default::default() },
 { 0 => (controller_index): U16, 1 => (capital_country_index): U16, 2 => (supply_node_level): U8, 3 => (naval_base_level): U8, 4 => (flags): U8 }, |_row| Ok(()));

/// HubMotorization: key owner:province:node_type:actor (128 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HubMotorizationRow {
    pub owner: u32,
    pub actor: u32,
    pub node_index: u32,
    pub map_entry_index: u32,
    pub province: i32,
    pub node_type: i32,
    pub level: u8,
}
typed_codec!(HubMotorizationRow, 128, |row| (row.owner as u128) << 96 | (row.province as u32 as u128) << 64 | (row.node_type as u32 as u128) << 32 | row.actor as u128, |key| Self { owner: (key >> 96) as u32, province: (key >> 64) as u32 as i32, node_type: (key >> 32) as u32 as i32, actor: key as u32, node_index: Default::default(), map_entry_index: Default::default(), level: Default::default() },
 { 0 => (node_index): U32, 1 => (map_entry_index): U32, 2 => (level): U8 }, |_row| Ok(()));

/// DisruptedSupply: key country:province:node_type (96 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DisruptedSupplyRow {
    pub country: u32,
    pub province: u32,
    pub node_type: u32,
    pub value: i64,
}
typed_codec!(DisruptedSupplyRow, 96, |row| (row.country as u128) << 64 | (row.province as u128) << 32 | row.node_type as u128, |key| Self { country: (key >> 64) as u32, province: (key >> 32) as u32, node_type: key as u32, value: Default::default() },
 { 0 => (value): I64 }, |_row| Ok(()));

/// ForeignHomebases: key country:ordinal (64 bits), 10 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ForeignHomebaseRow {
    pub country: u32,
    pub ordinal: u32,
    pub province: u32,
    pub supply: i64,
    pub start: i64,
    pub penalty: i64,
    pub added_penalty: i64,
    pub duration: i64,
    pub hours: i64,
    pub decay: i64,
    pub base: u8,
    pub source: Vec<u8>,
}
typed_codec!(ForeignHomebaseRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, province: Default::default(), supply: Default::default(), start: Default::default(), penalty: Default::default(), added_penalty: Default::default(), duration: Default::default(), hours: Default::default(), decay: Default::default(), base: Default::default(), source: Default::default() },
 { 0 => (province): U32, 1 => (supply): I64, 2 => (start): I64, 3 => (penalty): I64, 4 => (added_penalty): I64, 5 => (duration): I64, 6 => (hours): I64, 7 => (decay): I64, 8 => (base): U8, 9 => (source): Bytes }, |_row| Ok(()));

/// Source rail-info cooldown and selected building, keyed by province.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RailCooldownRow {
    pub province: u32,
    pub cooldown_ticks: i32,
    /// None is outside the reached health demand or a counted read refusal;
    /// a present state with a null building preserves native absence.
    pub building: Recorded<Option<RailwayBuildingState>>,
    /// Native selected-movement domains from this province's original info walk.
    pub selection: Recorded<Option<RailwaySelectionInputs>>,
    /// Kept outside the packet so unchanged native vectors remain unchanged
    /// when this hourly observation advances; carried packets refuse selection.
    pub selection_hour: Recorded<Option<i32>>,
}
typed_codec!(RailCooldownRow, 32, |row| row.province as u128, |key| Self { province: key as u32, ..Default::default() },
 { 0 => (cooldown_ticks): I32, 1 => (building): Opaque, 2 => (selection): Opaque, 3 => (selection_hour): OptionalI32 }, |_row| Ok(()));

/// RailEdges: key from:to (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RailEdgeRow {
    pub from: u32,
    pub to: u32,
    pub level: i32,
}
typed_codec!(RailEdgeRow, 64, |row| (row.from as u128) << 32 | row.to as u128, |key| Self { from: (key >> 32) as u32, to: key as u32, level: Default::default() },
 { 0 => (level): I32 }, |_row| Ok(()));

/// LandTheatres: key id (64 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandTheatreRow {
    pub id: u64,
    pub owner: u32,
    pub ordinal: u32,
    /// Stored signed theatre value, independent of the roster owner.
    pub volunteers_theatre: i32,
}
typed_codec!(LandTheatreRow, 64, |row| row.id as u128, |key| Self { id: key as u64, owner: Default::default(), ordinal: Default::default(), volunteers_theatre: Default::default() },
 { 0 => (owner): U32, 1 => (ordinal): U32, 2 => (volunteers_theatre): I32 }, |_row| Ok(()));

/// LandTheatreGroups: key id (64 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandTheatreGroupRow {
    pub id: u64,
    pub theatre: u64,
    pub ordinal: u32,
    pub priority: i32,
    pub name: Vec<u8>,
}
typed_codec!(LandTheatreGroupRow, 64, |row| row.id as u128, |key| Self { id: key as u64, theatre: Default::default(), ordinal: Default::default(), priority: Default::default(), name: Default::default() },
 { 0 => (theatre): U64, 1 => (ordinal): U32, 2 => (priority): I32, 3 => (name): Bytes }, |_row| Ok(()));

/// LandTheatreMembers: key theater_group:orders_group (128 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandTheatreMemberRow {
    pub orders_group: u64,
    pub theater_group: u64,
    pub ordinal: u32,
}
typed_codec!(LandTheatreMemberRow, 128, |row| (row.theater_group as u128) << 64 | row.orders_group as u128, |key| Self { theater_group: (key >> 64) as u64, orders_group: key as u64, ordinal: Default::default() },
 { 0 => (ordinal): U32 }, |_row| Ok(()));

/// InvasionPreparations: key owner_group:instance (96 bits), as ArmyOrders.
/// Hourly.
///
/// Stored required-preparation operands for each naval invasion order
/// (type 3), and its accumulated preparation (`progress`). The game
/// computes required hours on demand and stores none; the viewer computes
/// `hours = trunc(A * B / 1e10) * 24` with
/// `A = 100000 + country_term + relation_term.unwrap_or(0) + leader_term
/// (only when leader_applies) + relation_bonus.unwrap_or(0)` and
/// `B = NAVAL_INVASION_PREPARE_DAYS * 100000 + days_term`, the define from content.
///
/// `C` is the owning group's resolved country. `target` (`T`) is the
/// controller of the state of the order's first path province, zero without
/// a path or state. Each term is an effective modifier: `country_term` and
/// `days_term` are country keys 0x1A and 0x184; `relation_term` is key 0x1A
/// on `C`'s diplomacy row at `T`'s canonical slot, present only when
/// `0 < T`, `T` is neither `C`'s id nor the same country, and the row is
/// non-null. `relation_bonus` is key 0x1CE on that row, present whenever
/// the row is non-null (slot zero when `T` is zero). `leader_term` is key
/// 0x1A on the group leader's accumulator, present when the group also has
/// an HQ provider, and counted only when `leader_applies`: the provider's
/// root order, computed from the group's then its parent's primary orders'
/// assigned members, is this order.
///
/// The row also holds the sea route's path-cost operands for the unit used
/// to search the invasion route: `unit_owner` is that unit's raw owner
/// (zero without a unit); the `zero_*` bytes are its owner's slot-0 war
/// presence, retired-war byte (zero without a war), and co-belligerent
/// byte. `co_belligerents` holds ascending target slots whose stored byte
/// is nonzero (only 0 or 1 is written). The game serializes none of them.
///
/// The save writes only progress among the preparation operands, which the
/// oracle compares. In a complete row, a slot absent from co_belligerents has a
/// zero byte; unavailable never means zero. Unit selection and candidate-set
/// evidence live in engine::invasion_preparations; the path-cost operands have
/// no save comparator.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InvasionPreparationRow {
    pub owner_group: u64,
    pub instance: u32,
    pub progress: i64,
    pub target: u32,
    pub country_term: i64,
    pub relation_term: Option<i64>,
    pub relation_bonus: Option<i64>,
    pub leader_term: Option<i64>,
    pub leader_applies: u8,
    pub days_term: i64,
    pub unit_owner: u32,
    pub zero_war_present: u8,
    pub zero_war_retired: u8,
    pub zero_co_belligerent: u8,
    pub co_belligerents: Vec<u32>,
}
typed_codec!(InvasionPreparationRow, 96, |row| (row.owner_group as u128) << 32 | row.instance as u128, |key| Self { owner_group: (key >> 32) as u64, instance: key as u32, ..Default::default() },
 { 0 => (progress): I64, 1 => (target): U32, 2 => (country_term): I64, 3 => (relation_term): OptionalI64, 4 => (relation_bonus): OptionalI64, 5 => (leader_term): OptionalI64, 6 => (leader_applies): U8, 7 => (days_term): I64, 8 => (unit_owner): U32, 9 => (zero_war_present): U8, 10 => (zero_war_retired): U8, 11 => (zero_co_belligerent): U8, 12 => (co_belligerents): U32List }, |_row| Ok(()));

/// SentVolunteers: key country:ordinal (64 bits), 1 payload mask bit.
///
/// One position of the stored `volunteers_sent` list: the raw division
/// reference in stored order. Null and unresolved references remain
/// stored; the counter tint skips them.
///
/// An empty complete table means no sent references; unavailable is distinct.
/// Consumers resolve only live handles through the complete Divisions census.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SentVolunteerRow {
    pub country: u32,
    pub ordinal: u32,
    pub division: u64,
}
typed_codec!(SentVolunteerRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, division: Default::default() },
 { 0 => (division): U64 }, |_row| Ok(()));

/// VolunteerHosts: key country:position (64 bits), 2 payload mask bits.
///
/// One position of the volunteer-host vector and its paired tally. The
/// position is part of the key because the vector can hold a repeated host.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VolunteerHostRow {
    /// Raw country reference of the country that owns the vector.
    pub country: u32,
    pub position: u32,
    /// Raw country reference exactly as the native vector stores it.
    pub host: u32,
    /// Paired native word; the engine re-derives it from the sender's armies.
    pub deployed_divisions: i32,
}
typed_codec!(VolunteerHostRow, 64, |row| (row.country as u128) << 32 | row.position as u128, |key| Self { country: (key >> 32) as u32, position: key as u32, host: Default::default(), deployed_divisions: Default::default() },
 { 0 => (host): U32, 1 => (deployed_divisions): I32 }, |_row| Ok(()));

/// TheatreAreas: key theatre:ordinal (96 bits), 2 payload mask bits.
///
/// One CControllerArea listed by a volunteer CTheatre at `+0x18`. `representative`
/// is the province behind the area's `+0x28`, which is the only member the save
/// writer emits; `provinces` is the area's whole `+0x30` membership.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TheatreAreaRow {
    /// Full native CTheatre handle; join it to LandTheatres.
    pub theatre: u64,
    pub ordinal: u32,
    /// Representative province id; absent when the native pointer is null.
    pub representative: Option<u32>,
    /// Member province ids in native array order, repeats included.
    pub provinces: Vec<u32>,
}
typed_codec!(TheatreAreaRow, 96, |row| (row.theatre as u128) << 32 | row.ordinal as u128, |key| Self { theatre: (key >> 32) as u64, ordinal: key as u32, representative: Default::default(), provinces: Default::default() },
 { 0 => (representative): OptionalU32, 1 => (provinces): U32List }, |_row| Ok(()));

/// StationedUnits: key theatre:ordinal (96 bits), 1 payload mask bit.
///
/// One position of a volunteer CTheatre's `+0x68` stationed-unit vector.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StationedUnitRow {
    /// Full native CTheatre handle; join it to LandTheatres.
    pub theatre: u64,
    pub ordinal: u32,
    /// Full native CUnit handle; join it to Divisions or TaskForces.
    pub unit: u64,
}
typed_codec!(StationedUnitRow, 96, |row| (row.theatre as u128) << 32 | row.ordinal as u128, |key| Self { theatre: (key >> 32) as u64, ordinal: key as u32, unit: Default::default() },
 { 0 => (unit): U64 }, |_row| Ok(()));

/// OrdersGroups: key handle (64 bits), 8 payload mask bits.
///
/// One row per authenticated type-53 COrdersGroup, army and army group alike.
/// Every reference field is the pointee's full native handle; zero preserves
/// the engine's own null pointer and is never an invented identity.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OrdersGroupRow {
    pub handle: u64,
    /// Raw country reference at +0x3c, kept as read like every other raw alias.
    pub country: u32,
    /// Raw +0x39 discriminator byte, the source of the save's field_marshal_group.
    pub is_army_group: u8,
    /// Raw stored byte, written unconditionally to the save.
    pub motorization_level: u8,
    pub theatre: u64,
    /// Native position in the theatre's regular or army-group roster, chosen
    /// by is_army_group. None means absent, not position zero.
    pub theatre_ordinal: Option<u32>,
    pub theater_group: u64,
    /// Parent CArmyGroup handle, zero when this group has no parent.
    pub parent: u64,
    /// CUnitLeader handle, zero when no leader is assigned.
    pub leader: u64,
    /// CUnit handle of the HQ-providing unit, zero when absent.
    pub hq_provider: u64,
    /// Stored orders-group name bytes; None is a counted capture refusal.
    pub name: Recorded<Option<Vec<u8>>>,
}
typed_codec!(OrdersGroupRow, 64, |row| row.handle as u128, |key| Self { handle: key as u64, country: Default::default(), is_army_group: Default::default(), motorization_level: Default::default(), theatre: Default::default(), theatre_ordinal: Default::default(), theater_group: Default::default(), parent: Default::default(), leader: Default::default(), hq_provider: Default::default(), name: Default::default() },
 { 0 => (country): U32, 1 => (is_army_group): U8, 2 => (motorization_level): U8, 3 => (theatre): U64, 4 => (theatre_ordinal): OptionalU32, 5 => (theater_group): U64, 6 => (parent): U64, 7 => (leader): U64, 8 => (hq_provider): U64, 9 => (name): Opaque }, |_row| Ok(()));

/// OrdersGroupMembers: key group:ordinal (96 bits), 1 payload mask bit.
///
/// The ordinal is the native member-vector index, which is engine state and the
/// order the save serializes; it is not a generated key.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OrdersGroupMemberRow {
    pub group: u64,
    pub ordinal: u32,
    /// Full CUnit handle of the assigned division.
    pub unit: u64,
}
typed_codec!(OrdersGroupMemberRow, 96, |row| (row.group as u128) << 32 | row.ordinal as u128, |key| Self { group: (key >> 32) as u64, ordinal: key as u32, unit: Default::default() },
 { 0 => (unit): U64 }, |_row| Ok(()));

/// ArmyGroupMembers: key group:ordinal (96 bits), 1 payload mask bit.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArmyGroupMemberRow {
    pub group: u64,
    pub ordinal: u32,
    /// Full child COrdersGroup handle.
    pub child: u64,
}
typed_codec!(ArmyGroupMemberRow, 96, |row| (row.group as u128) << 32 | row.ordinal as u128, |key| Self { group: (key >> 32) as u64, ordinal: key as u32, child: Default::default() },
 { 0 => (child): U64 }, |_row| Ok(()));

opaque_value!(Vec<TerrainSpeed>);
opaque_value!(Vec<TrainAllocation>);
opaque_value!(SparseBytes);
opaque_value!(Vec<DivisionAcclimatization>);
opaque_value!(Vec<TemplateSubunit>);

#[cfg(test)]
#[path = "unit_transport_inputs_tests.rs"]
mod unit_transport_inputs_tests;
