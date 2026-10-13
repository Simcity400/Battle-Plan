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

/// Simulation node, read once a game day at its country's owner slice with the country's
/// TrainInventory row; SliceReads holds the read hour. Identity is (Countries.id, native
/// node index). Parent is a province in the same companion, not an array index. Route
/// provinces retain native order and duplicates; reverse land portions only in
/// presentation. from==0 denotes native's missing-route/naval fallback. Format 28 adds a
/// row for every other vertex on a node's parent chain (index `VERTEX | province`,
/// kind/flag/supply zero), so the chain never leaves the country's rows. A node, parent
/// or vertex province can be an id the game's province getter resolves as null (zero,
/// outside the census or an empty slot), recorded as stored.
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
    /// Full equipment variant reference, joining VariantDefinitions.id.
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

/// Country supply operands, including empty inventories/censuses, read once a
/// game day at the country's owner slice with its SupplyNodes rows.
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
    /// Full native faction reference, as in Factions.
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
/// One terrain a retired [`DivisionEtaInputsRow`] route touched.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TerrainSpeed {
    pub terrain: i32,
    pub modifier: i64,
}
value_struct!(TerrainSpeed {
    terrain: i32,
    modifier: i64
});

/// Retired per-division speed graph and route-leg operands
/// (`RETIRED_INPUTS_FORMAT`). The row keeps its field framing so older
/// recordings discard the table by name; no current writer emits it.
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
    /// Retired current-hour speed graph of the fresh counter route.
    pub fresh: Recorded<Retired>,
    /// Complete ordered unit+1A8/+1B4 references from this callback hour. Repeats and
    /// runtime-minted full handles are retained. None is an independent acquisition
    /// refusal; historical absence is NotRecorded. Entry count equals combat_count; no
    /// selection or derived query is stored.
    pub combat_references: Recorded<Option<UnitCombatReferences>>,
    /// Retired selected-route commander, retreat and exile operands.
    pub selected: Recorded<Retired>,
    /// Retired selected transport weight, transfer and template operands.
    pub transport: Recorded<Retired>,
    /// Raw signed fixed5 maximum speed of the division's stat block
    /// (`[division+0x138]+0x290`, a `stat block`), read for a division with a nonempty
    /// route. Movement-speed calculation starts from this value, scales it by the edge
    /// modifier, organisation and fuel, floors it at 1 km/h and multiplies by the
    /// movement scale; the hourly step (vslot 0xD8) adds that product to `numerator`, and
    /// no field caches it. Playback estimates arrival from this speed alone. None is
    /// outside demand or a counted guarded refusal; older formats use NotRecorded.
    pub base_speed: Recorded<Option<i64>>,
}
typed_codec!(DivisionMovementRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (source): U32, 1 => (target): U32, 2 => (numerator): I64, 3 => (retreat): U8, 4 => (strategic_redeployment): U8, 5 => (naval_transfer): U8, 6 => (combat_count): U32, 7 => (path_total): U32, 8 => (path): U32List, 9 => (alternate_total): U32, 10 => (alternate_path): U32List, 11 => (combat_role): U8, 12 => (combat_province): U32, 13 => (map_entity): U8, 14 => (strategic_action): Opaque, 15 => (withdrawal): Opaque, 16 => (movement_priority): OptionalI32, 17 => (order_transport): Opaque, 18 => (fresh): Opaque, 19 => (combat_references): Opaque, 20 => (selected): Opaque, 21 => (transport): Opaque, 22 => (base_speed): OptionalI64 }, |row| {
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

/// A supply footer demand pair: the native signed count, exact, and its
/// amount at the footer's display step ([`UnitSupplyInputs::STEP`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnitSupplyDemand {
    pub count: i32,
    pub value: i64,
}
value_struct!(UnitSupplyDemand {
    count: i32,
    value @ UnitSupplyInputs::STEP: i64
});

/// Every native distributed-supply row, in original order: the province
/// exact, both amounts at the footer's display step
/// ([`UnitSupplyInputs::STEP`]). Nonpositive amounts and duplicate provinces
/// affect totals and survive.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnitSupplyOrigin {
    pub province: i32,
    pub requested: i64,
    pub received: i64,
}
value_struct!(UnitSupplyOrigin {
    province: i32,
    requested @ UnitSupplyInputs::STEP: i64,
    received @ UnitSupplyInputs::STEP: i64
});

/// Stored supply-consumer operands of aggregation and footer formatting
/// for a task force or railway gun, at the precision the footer shows them
/// ([`crate::display`]). Every unit counter's supply child reaches the same
/// footer through its tooltip builder and supply aggregation, as port mission
/// items do. The footer passes every printed amount through the two-decimal
/// formatter, which rounds
/// the fixed5 word half away from zero to 0.01 (1000 raw) before
/// localisation: the needed and received supply, the three demand values,
/// the state, supply-node, per-hub and air-drop receipts and the disruption,
/// every installed mod the same. Each amount is stored rounded that way, so a
/// single unit's footer prints the game's text. The footer tests `0 <` on the
/// state and air-drop receipts, the demands, the disruption and the hub
/// receipts before it prints a line, so a nonzero amount below half a step
/// keeps one step with its sign (it then shows 0.01 where the game shows
/// 0.00).
///
/// The footer sums its members before rounding; by owner decision 2026-10-11 a footer
/// over several units sums the stored amounts and can differ from the game in its last
/// digit at boundaries, and so can the received and flow-reduction percentages, a hub
/// list's order between hubs whose receipts round equal, the hub cut-off (`< 0.05`) for a
/// sum from 0.045 up to 0.05, and the flow-reduction line when the requested and received
/// totals round equal. The disruption operand is overwritten by the last native aggregate
/// member. These amounts cannot be recovered from the stored supply ratio or hub table.
/// Format 101 and earlier recorded them exact and decode them unscaled.
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
impl UnitSupplyInputs {
    /// Every footer amount: 0.01, half away from zero, nonzero kept by the two-decimal
    /// formatter.
    pub const STEP: crate::display::Step =
        crate::display::Step::fixed5(1_000, crate::display::Rounding::HalfAwayFromZeroNonzero);
}
value_struct!(UnitSupplyInputs {
    asked @ Self::STEP: i64,
    received @ Self::STEP: i64,
    from_state @ Self::STEP: i64,
    air_drops @ Self::STEP: i64,
    disruption @ Self::STEP: i64,
    division_demand: UnitSupplyDemand,
    airplane_demand: UnitSupplyDemand,
    ship_demand: UnitSupplyDemand,
    origins: Option<Vec<UnitSupplyOrigin>>
});
opaque_value!(Option<UnitSupplyInputs>);

/// A division's supply footer: the [`UnitSupplyInputs`] amounts, with the
/// same widths and display steps, its received amounts stored as shortfalls
/// ([`StoredFooter`]). DivisionStatus.supply was a provisional tombstone,
/// whose empty payload reads as not recorded (`opaque_value!(T, restored)`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionSupplyFooter {
    pub asked: i64,
    pub received: i64,
    pub from_state: i64,
    pub air_drops: i64,
    /// Overwritten, not summed, by each member of the native aggregate.
    pub disruption: i64,
    pub division_demand: UnitSupplyDemand,
    pub airplane_demand: UnitSupplyDemand,
    pub ship_demand: UnitSupplyDemand,
    /// None is a counted node-fragment read/capacity failure. The independent
    /// scalar footer lines remain available; Some(empty) records zero origins.
    pub origins: Option<Vec<UnitSupplyOrigin>>,
}
impl DivisionSupplyFooter {
    pub const STEP: crate::display::Step = UnitSupplyInputs::STEP;
}

/// A footer demand pair as [`DivisionSupplyFooter`] stores it: the count
/// exact, the amount in whole display steps.
struct StoredDemand {
    count: i32,
    units: i64,
}
value_struct!(StoredDemand {
    count: i32,
    units: i64
});

/// A distributed-supply row as [`DivisionSupplyFooter`] stores it: the
/// requested amount in whole display steps, then the received word, the
/// steps it fell short of the request by where [`StoredOrigins`] marks
/// shortfalls and the received steps themselves where it does not.
struct StoredOrigin {
    province: i32,
    requested: i64,
    received: i64,
}
value_struct!(StoredOrigin {
    province: i32,
    requested: i64,
    received: i64
});

/// The footer's distributed-supply rows, and whether the footer's received
/// words are shortfalls. The [`UnitSupplyInputs`] layout tags the list `0`
/// (absent) or `1` (present); a footer storing shortfalls tags it `2` or `3`.
/// From [`crate::rows::compatibility::SUPPLY_SHORTFALL_FORMAT`] the reader
/// tells format-102 captures written before the shortfalls, which hold the
/// received steps, from current ones and decodes both exactly. A reader older
/// than the shortfalls refuses tags `2` and `3` as invalid option tags.
struct StoredOrigins {
    origins: Option<Vec<StoredOrigin>>,
    shortfalls: bool,
}

impl StoredOrigins {
    const SHORTFALLS: u8 = 2;
}

impl Value for StoredOrigins {
    const MINIMUM_ENCODED_BYTES: usize = 1;

    // The tag keeps its width.
    fn encoded_bytes(&self) -> io::Result<usize> {
        Value::encoded_bytes(&self.origins)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            origins: Value::try_clone(&self.origins)?,
            shortfalls: self.shortfalls,
        })
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let marker = if self.shortfalls { Self::SHORTFALLS } else { 0 };
        match &self.origins {
            None => output.push(marker),
            Some(origins) => {
                output.push(marker | 1);
                origins.encode_reserved(output)?;
            }
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let tag = input.u8()?;
        if tag > (Self::SHORTFALLS | 1) {
            return Err(invalid("invalid supply origins tag"));
        }
        Ok(Self {
            origins: if tag & 1 == 1 {
                Some(Value::decode(input)?)
            } else {
                None
            },
            shortfalls: tag & Self::SHORTFALLS != 0,
        })
    }
}

/// [`DivisionSupplyFooter`]'s wire layout from
/// [`crate::rows::compatibility::SUPPLY_SHORTFALL_FORMAT`]: the
/// [`UnitSupplyInputs`] layout, every amount in whole display steps. Current
/// writers store the received amount as the steps it fell short of the asked
/// amount by, and each origin's as the steps it fell short of its request by
/// (wrapping, so every pair of amounts is stored exactly), and say so in the
/// origins' tag ([`StoredOrigins`]). Supply aggregation sums the consumers'
/// asked and received amounts together and the footer prints both; a
/// division supplied in full receives what it asks for, so its stored
/// shortfall stays zero while its demand drifts.
struct StoredFooter {
    asked: i64,
    received: i64,
    from_state: i64,
    air_drops: i64,
    disruption: i64,
    division_demand: StoredDemand,
    airplane_demand: StoredDemand,
    ship_demand: StoredDemand,
    origins: StoredOrigins,
}
value_struct!(StoredFooter {
    asked: i64,
    received: i64,
    from_state: i64,
    air_drops: i64,
    disruption: i64,
    division_demand: StoredDemand,
    airplane_demand: StoredDemand,
    ship_demand: StoredDemand,
    origins: StoredOrigins
});

impl StoredDemand {
    fn new(demand: &UnitSupplyDemand) -> Self {
        Self {
            count: demand.count,
            units: DivisionSupplyFooter::STEP.units(demand.value),
        }
    }
    fn demand(&self) -> io::Result<UnitSupplyDemand> {
        Ok(UnitSupplyDemand {
            count: self.count,
            value: DivisionSupplyFooter::STEP.value(self.units)?,
        })
    }
}

impl DivisionSupplyFooter {
    /// Write this footer in the [`StoredFooter`] layout, received words as
    /// shortfalls, without building it.
    fn encode_shortfalls(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let units = |value| Self::STEP.units(value);
        let asked = units(self.asked);
        for word in [
            asked,
            asked.wrapping_sub(units(self.received)),
            units(self.from_state),
            units(self.air_drops),
            units(self.disruption),
        ] {
            word.encode_reserved(output)?;
        }
        for demand in [
            &self.division_demand,
            &self.airplane_demand,
            &self.ship_demand,
        ] {
            StoredDemand::new(demand).encode_reserved(output)?;
        }
        let Some(origins) = &self.origins else {
            output.push(StoredOrigins::SHORTFALLS);
            return Ok(());
        };
        output.push(StoredOrigins::SHORTFALLS | 1);
        let count = u32::try_from(origins.len())
            .map_err(|_| invalid("nested collection count exceeds u32"))?;
        count.encode_reserved(output)?;
        for origin in origins {
            let requested = units(origin.requested);
            StoredOrigin {
                province: origin.province,
                requested,
                received: requested.wrapping_sub(units(origin.received)),
            }
            .encode_reserved(output)?;
        }
        Ok(())
    }

    /// The footer a [`StoredFooter`] holds, every amount at its step: a
    /// received word is a shortfall where the origins' tag says so
    /// ([`Self::encode_shortfalls`] inverted), and the received steps where it does not.
    fn from_stored(stored: StoredFooter) -> io::Result<Self> {
        let value = |units| Self::STEP.value(units);
        let shortfalls = stored.origins.shortfalls;
        let received = |from: i64, word: i64| {
            value(if shortfalls {
                from.wrapping_sub(word)
            } else {
                word
            })
        };
        let origins = match stored.origins.origins {
            Some(origins) => {
                let mut restored = allocation::vec(origins.len())?;
                for origin in origins {
                    restored.push(UnitSupplyOrigin {
                        province: origin.province,
                        requested: value(origin.requested)?,
                        received: received(origin.requested, origin.received)?,
                    });
                }
                Some(restored)
            }
            None => None,
        };
        Ok(Self {
            asked: value(stored.asked)?,
            received: received(stored.asked, stored.received)?,
            from_state: value(stored.from_state)?,
            air_drops: value(stored.air_drops)?,
            disruption: value(stored.disruption)?,
            division_demand: stored.division_demand.demand()?,
            airplane_demand: stored.airplane_demand.demand()?,
            ship_demand: stored.ship_demand.demand()?,
            origins,
        })
    }

    /// The test-only historical writer stands in for format 101, which
    /// stored the [`UnitSupplyInputs`] layout.
    fn historical(&self) -> io::Result<Option<UnitSupplyInputs>> {
        if historical_stamp().is_none() {
            return Ok(None);
        }
        Value::try_clone(self).map(|footer| Some(footer.into()))
    }
}

const _: () = assert!(
    crate::display::stepped_format(crate::rows::compatibility::SUPPLY_SHORTFALL_FORMAT),
    "the shortfall layout stores whole display steps"
);

impl Value for DivisionSupplyFooter {
    const MINIMUM_ENCODED_BYTES: usize = <UnitSupplyInputs as Value>::MINIMUM_ENCODED_BYTES;
    const HAS_STEPS: bool = true;
    #[cfg(test)]
    const STRUCT_FIELDS: usize = <UnitSupplyInputs as Value>::STRUCT_FIELDS;

    fn round_display(&mut self) {
        let mut inputs = UnitSupplyInputs::from(std::mem::take(self));
        inputs.round_display();
        *self = inputs.into();
    }
    // Every layout has the UnitSupplyInputs widths.
    fn encoded_bytes(&self) -> io::Result<usize> {
        let scalars = [
            self.asked,
            self.received,
            self.from_state,
            self.air_drops,
            self.disruption,
        ];
        let size = <i64 as Value>::encoded_many_bytes(&scalars)?;
        let size = [
            &self.division_demand,
            &self.airplane_demand,
            &self.ship_demand,
        ]
        .into_iter()
        .try_fold(size, |size, demand| {
            allocation::add(size, Value::encoded_bytes(demand)?)
        })?;
        allocation::add(size, Value::encoded_bytes(&self.origins)?)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            origins: Value::try_clone(&self.origins)?,
            division_demand: self.division_demand.clone(),
            airplane_demand: self.airplane_demand.clone(),
            ship_demand: self.ship_demand.clone(),
            ..*self
        })
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match self.historical()? {
            Some(inputs) => inputs.encode_reserved(output),
            None => self.encode_shortfalls(output),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.format >= crate::rows::compatibility::SUPPLY_SHORTFALL_FORMAT {
            Self::from_stored(StoredFooter::decode(input)?)
        } else {
            UnitSupplyInputs::decode(input).map(Self::from)
        }
    }
}
opaque_value!(Option<DivisionSupplyFooter>, restored);

impl From<UnitSupplyInputs> for DivisionSupplyFooter {
    fn from(inputs: UnitSupplyInputs) -> Self {
        Self {
            asked: inputs.asked,
            received: inputs.received,
            from_state: inputs.from_state,
            air_drops: inputs.air_drops,
            disruption: inputs.disruption,
            division_demand: inputs.division_demand,
            airplane_demand: inputs.airplane_demand,
            ship_demand: inputs.ship_demand,
            origins: inputs.origins,
        }
    }
}

impl From<DivisionSupplyFooter> for UnitSupplyInputs {
    fn from(footer: DivisionSupplyFooter) -> Self {
        Self {
            asked: footer.asked,
            received: footer.received,
            from_state: footer.from_state,
            air_drops: footer.air_drops,
            disruption: footer.disruption,
            division_demand: footer.division_demand,
            airplane_demand: footer.airplane_demand,
            ship_demand: footer.ship_demand,
            origins: footer.origins,
        }
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
    /// Retired glyph byte: stored attrition exceeds its threshold, which playback
    /// computes from this row ([`DivisionStatusRow::has_attrition`]).
    pub attrition: RetiredU8,
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
    /// `acclimatization data` maximum, raw, as recordings without [`Self::camouflage`]
    /// captured it: `None` when this division's acclimatization could not be read that
    /// hour, and then the entries are empty. Current writers store `None` and no entries.
    pub acclimatization_maximum: Option<i64>,
    /// Every acclimatization entry in native order, repeats kept, as
    /// recordings without [`Self::camouflage`] captured it; their consumers
    /// pick the camouflage winner from it.
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
    /// Division +0x600/+0x610 fixed5 supply grace. Older formats use NotRecorded; zero is
    /// a recorded value. With `supply_ratio` (division+0x40), the division's only record
    /// of these supply operands: DivisionLandInputs no longer copies them.
    pub supply_grace_maximum: Recorded<i64>,
    pub supply_grace_remaining: Recorded<i64>,
    /// Stored division roster position. None is a counted capture refusal;
    /// roster swap-removal changes this.
    pub country_ordinal: Recorded<Option<u32>>,
    /// Signed invalid-order reason, read only when invalid_orders is nonzero.
    /// None is outside that demand or a counted field-local capture failure;
    /// older formats use NotRecorded.
    pub invalid_orders_reason: Recorded<Option<i32>>,
    /// The supply child's per-source footer, recorded for every division at display
    /// precision ([`DivisionSupplyFooter`]). None is a counted scalar-pack failure.
    /// Formats before 99 use NotRecorded, as do format-102 recordings written while the
    /// field was a provisional tombstone, whose empty payload decodes as not recorded.
    pub supply: Recorded<Option<DivisionSupplyFooter>>,
    /// Retired fresh attrition operands of the per-modifier breakdown; the
    /// counter keeps `attrition_value`, `attrition_threshold` and `attrition`.
    pub attrition_inputs: Recorded<Retired>,
    /// Fresh commander entrenchment factor: ONE plus the selected commander's
    /// combat_entrenchment modifier, with wrapping addition. None is an independent read
    /// refusal; this is not a derived duration.
    pub dig_in_factor: Recorded<Option<i64>>,
    /// Exact cached name, including empty bytes. None is a counted isolated
    /// read/capacity refusal; older formats use NotRecorded.
    pub name: Recorded<Option<Vec<u8>>>,
    /// The selected-attack operands recordings through format 101 captured every hour
    /// ([`DivisionAttackInputs`], each per-terrain value the sum of the stat-block and
    /// terrain adjuster attacks). Current writers leave it empty ([`Witnessed`]) except
    /// for the divisions the collector's off-by-default `attack_witness` diagnostic
    /// samples, read fresh at the row's hour.
    pub attack: Recorded<Witnessed<DivisionAttackInputs>>,
    /// The category selected from the division's acclimatization to camouflage its land
    /// model. None is this hour's read refusal; the rest of the row stands. Older formats
    /// use NotRecorded and keep [`Self::acclimatization`] and its maximum instead.
    pub camouflage: Recorded<Option<DivisionCamouflage>>,
    /// The stat block's attack operands ([`DivisionAttackStats`]). The collector carries
    /// a division's value while no hooked writer (the active-stat refresh) touched it
    /// since the read it came from, so every hour holds what the stat block holds. None
    /// is a read refusal; older formats use NotRecorded.
    pub attack_stats: Recorded<Option<DivisionAttackStats>>,
    /// The fresh acclimatization factor for division `+0x4B8`, which scales the region
    /// weather term of the attack list. None is a read refusal; older formats use
    /// NotRecorded.
    pub weather_factor: Recorded<Option<i64>>,
}
// Display steps ([`crate::display`]). Entrenchment and its cap (the spade glyph's `> 0`
// and `<` tests), the attrition value and threshold (the attrition glyph's comparison),
// supply_grace_maximum (the `< 1` gate), and strength_factor, dig_in_factor and
// border_combat_stats, which only feed derived products, stay exact.
impl DivisionStatusRow {
    /// `maximum_hit_points` and `maximum_organisation` at the game's display precision
    /// ([`crate::display`]): the maximum-strength and maximum-organisation lines print
    /// one highlighted decimal, the combat window's organisation maximum uses one
    /// decimal, and the counter hover uses whole units in a fixed colour. Every installed
    /// mod uses the same finest precision: one decimal, truncated by the fixed5
    /// formatter. The getters keep them at least 100 raw, so a nonzero maximum below a
    /// tenth keeps one tenth and the `maximum > 0` tests (`binding.rs`, the advance
    /// warning) read the same. By owner decision 2026-10-11 the strength and organisation
    /// ratios drawn from them can differ from the game in their last digit at boundaries.
    pub const MAXIMUM_STEP: crate::display::Step =
        crate::display::Step::fixed5(10_000, crate::display::Rounding::TowardZeroNonzero);
    /// `supply_ratio` (`division+0x40`) at the game's display precision: the counter
    /// hover and supply descriptions print the effective value as a whole percentage (in
    /// a fixed colour for the hover), every installed mod the same: 0.01 truncated by the
    /// fixed5 formatter. Truncation commutes with the effective value's `max` against the
    /// grace share, and every supply crate threshold and LOW_SUPPLY gate in vanilla and
    /// the installed mods is a multiple of 0.01, so the frames and the low-supply gate
    /// are unchanged; a mod threshold off that grid could flip at its boundary.
    pub const SUPPLY_RATIO_STEP: crate::display::Step =
        crate::display::Step::fixed5(1_000, crate::display::Rounding::TowardZero);
    /// The wire step of `supply_grace_remaining`: 0.01 hour, which every
    /// value [`Self::displayed_grace`] stores is a multiple of.
    pub const SUPPLY_GRACE_STEP: crate::display::Step =
        crate::display::Step::fixed5(1_000, crate::display::Rounding::TowardZero);

    /// `supply_grace_remaining` (`division+0x610`, fixed5) as capture stores it. The game
    /// shows it only as a fixed5 share of `G`, the live `NMilitary.SUPPLY_GRACE` hours,
    /// scaling both operands before division, at whole percent, truncated: on its own as
    /// stored supply in a fixed colour, and as the larger of it and the received ratio in
    /// the unit supply text, low-supply gate, counter supply icon and availability.
    /// Truncating toward zero to a multiple of `1000 G` keeps that percent for every
    /// whole positive `G`, and a share on the percent grid keeps every comparison with
    /// the received ratio (a multiple of 0.01) and with a threshold on that grid. The
    /// hourly update moves the value by at most 0.15 when it regains and by 0.75 times
    /// the missing supply share when it spends (vanilla), so it changes the percent only
    /// every few hours. An unknown or nonpositive `G`, a negative value and one whose
    /// share product would wrap keep [`Self::SUPPLY_GRACE_STEP`]; a sentinel word stays
    /// exact.
    pub fn displayed_grace(remaining: i64, grace: Option<i32>) -> i64 {
        match grace {
            Some(hours) if hours > 0 && (0..=i64::MAX / 100_000).contains(&remaining) => {
                remaining - remaining % (i64::from(hours) * 1_000)
            }
            _ => Self::SUPPLY_GRACE_STEP.round(remaining),
        }
    }

    /// Division `+0x2AE` is nonzero: the exile byte lets edge admission bypass province
    /// move access, allowing a move across controller areas within one state owner.
    pub const MOVE_EXILED: u8 = 1;
    /// The cached `can_exfiltrate_from_coast` operand, passed to the path request.
    pub const MOVE_CAN_EXFILTRATE_FROM_COAST: u8 = 2;

    /// The attrition glyph test: the division's stored attrition (`division+0x438`)
    /// exceeds the threshold the row records, a signed fixed-point comparison of the same
    /// read.
    pub fn has_attrition(&self) -> bool {
        self.attrition_threshold < self.attrition_value
    }
}
typed_codec!(DivisionStatusRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (maximum_hit_points): I64 @ Self::MAXIMUM_STEP, 1 => (maximum_organisation): I64 @ Self::MAXIMUM_STEP, 2 => (entrenchment): I64, 3 => (entrenchment_cap): I64, 4 => (supply_ratio): I64 @ Self::SUPPLY_RATIO_STEP, 5 => (attrition_value): I64, 6 => (attrition_threshold): I64, 7 => (attrition): U8, 8 => (exercising): U8, 9 => (has_order): U8, 10 => (invalid_orders): U8, 11 => (strength_factor): I64, 12 => (model_number): I32, 13 => (acclimatization_maximum): OptionalI64, 14 => (acclimatization): Opaque, 15 => (move_flags): U8, 16 => (border_combat_stats): Opaque, 17 => (naming): Opaque, 18 => (supply_grace_maximum): I64, 19 => (supply_grace_remaining): I64 @ Self::SUPPLY_GRACE_STEP, 20 => (country_ordinal): OptionalU32, 21 => (invalid_orders_reason): OptionalI32, 22 => (supply): Opaque, 23 => (attrition_inputs): Opaque, 24 => (dig_in_factor): OptionalI64, 25 => (name): Opaque, 26 => (attack): Opaque, 27 => (camouflage): Opaque, 28 => (attack_stats): Opaque, 29 => (weather_factor): OptionalI64 }, |_row| Ok(()));

/// DivisionRetreatBans: key division (64 bits). A division whose commander
/// bans retreat: bits of the leader modifier block selected for it,
/// whatever its combats; a division without a ban has no row. The game
/// applies them only when the division stands on the defending or attacking
/// side of one of its combats (LandCombats defenders/attackers), which
/// the reader checks at its own hour.
///
/// Hourly: every division each hour, with each commander's two blocks read
/// again only after a writer of them ran (the collector's
/// retreat_ban_changes). A division whose roster owner cannot be resolved
/// makes the hour's read return no rows, truncated. Before format 102 the
/// table held each owner's daily read, and those recordings refuse it.
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

/// The acclimatization category used to camouflage a division's land model. Division
/// `+0x4B8` holds the acclimatization data; selection takes the entry with the first
/// strictly greatest weight above zero, and keeps its category when that category is not
/// null and the weight reaches the maximum (+0x38) times the live
/// `NGraphics.ACCLIMATIZATION_CAMO_SHOW_AT`, a wrapping signed product truncated toward
/// zero by 100,000. The capture applies exactly that test, so a division's camouflage
/// changes only when the winner or its crossing of the threshold does. The province and
/// state conditions that follow (`forbid_camo_when`, `change_camo_when`) depend on where
/// the model stands and stay with the consumer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DivisionCamouflage {
    /// The category's lexer token; None when no category reaches the
    /// threshold, the weights are all zero or negative, or the winning
    /// category pointer is null.
    pub category: Option<u32>,
}
value_struct!(DivisionCamouflage {
    category: Option<u32>
});
opaque_value!(Option<DivisionCamouflage>);

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

/// Hourly companion of LandCombats, keyed by its full native handle: each
/// side's loss counters.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatProgressRow {
    pub combat: u64,
    /// Retired: playback computes both sides' progress from the ordered LandCombats
    /// rosters and their Divisions organisation, and the marker's direction from the
    /// first attacker with a map entity.
    pub attacker_progress: RetiredI64,
    pub defender_progress: RetiredI64,
    pub direction_attacker: RetiredU64,
    pub direction_province: RetiredU32,
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
/// Stored required-preparation operands for each naval invasion order (type 3), and its
/// accumulated preparation (`progress`). The game computes required hours on demand and
/// stores none; the viewer computes the fixed5 product of two terms, truncated to whole
/// days and multiplied by 24. The first is one plus `country_term`, present relation
/// terms and `leader_term` only when `leader_applies`. The second is the content define
/// `NAVAL_INVASION_PREPARE_DAYS` scaled to fixed5 plus `days_term`.
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
/// `unit_owner` is the raw `unit+0x1E0` of the unit used by the naval-invasion route
/// search (zero without a unit); the sea route's path-cost operands for that owner are
/// the slot-0 relation bytes and co-belligerents of its CountryCounterInputs row, so the
/// retired `zero_*` and `co_belligerents` fields record nothing.
///
/// The save writes only progress among the preparation operands, which the
/// oracle compares. Unit selection lives in engine::invasion_preparations.
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
    pub zero_war_present: RetiredU8,
    pub zero_war_retired: RetiredU8,
    pub zero_co_belligerent: RetiredU8,
    pub co_belligerents: RetiredU32List,
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
/// One controller area listed by a volunteer theatre at `+0x18`. `representative` is the
/// province behind the area's `+0x28`, which is the only member the save writer emits;
/// `provinces` is the area's whole `+0x30` membership.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TheatreAreaRow {
    /// Full native theatre handle; join it to LandTheatres.
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
/// One position of a volunteer theatre's `+0x68` stationed-unit vector.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StationedUnitRow {
    /// Full native theatre handle; join it to LandTheatres.
    pub theatre: u64,
    pub ordinal: u32,
    /// Full native unit handle; join it to Divisions or TaskForces.
    pub unit: u64,
}
typed_codec!(StationedUnitRow, 96, |row| (row.theatre as u128) << 32 | row.ordinal as u128, |key| Self { theatre: (key >> 32) as u64, ordinal: key as u32, unit: Default::default() },
 { 0 => (unit): U64 }, |_row| Ok(()));

/// OrdersGroups: key handle (64 bits), 8 payload mask bits.
///
/// One row per authenticated type-53 orders group, army and army group alike. Every
/// reference field is the pointee's full native handle; zero preserves the engine's own
/// null pointer and is never an invented identity.
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
    /// Parent army group handle, zero when this group has no parent.
    pub parent: u64,
    /// Unit leader handle, zero when no leader is assigned.
    pub leader: u64,
    /// Unit handle of the HQ-providing unit, zero when absent.
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
    /// Full unit handle of the assigned division.
    pub unit: u64,
}
typed_codec!(OrdersGroupMemberRow, 96, |row| (row.group as u128) << 32 | row.ordinal as u128, |key| Self { group: (key >> 32) as u64, ordinal: key as u32, unit: Default::default() },
 { 0 => (unit): U64 }, |_row| Ok(()));

/// ArmyGroupMembers: key group:ordinal (96 bits), 1 payload mask bit.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArmyGroupMemberRow {
    pub group: u64,
    pub ordinal: u32,
    /// Full child orders group handle.
    pub child: u64,
}
typed_codec!(ArmyGroupMemberRow, 96, |row| (row.group as u128) << 32 | row.ordinal as u128, |key| Self { group: (key >> 32) as u64, ordinal: key as u32, child: Default::default() },
 { 0 => (child): U64 }, |_row| Ok(()));

opaque_value!(Vec<TerrainSpeed>);
opaque_value!(Vec<TrainAllocation>);
opaque_value!(SparseBytes);
opaque_value!(Vec<DivisionAcclimatization>);
opaque_value!(Vec<TemplateSubunit>);
