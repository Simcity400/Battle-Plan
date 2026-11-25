use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionCensusRow {
    pub country: u32,
    pub grouped: u32,
    pub ungrouped: u32,
}
typed_codec!(DivisionCensusRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (grouped): U32, 1 => (ungrouped): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    pub const VERTEX: u32 = 1 << 31;
    pub fn is_vertex(&self) -> bool {
        self.index & Self::VERTEX != 0
    }
}
typed_codec!(SupplyNodeRow, 64, |row| (row.country as u128) << 32 | row.index as u128,
 |key| Self { country: (key >> 32) as u32, index: key as u32, ..Default::default() },
 { 0 => (province): U32, 1 => (kind): U32, 2 => (flag): U8, 3 => (available): I64, 4 => (consumed): I64, 5 => (parent): I32, 6 => (route_from): U16, 7 => (route_to): U16, 8 => (break_after): U8, 9 => (path): U32List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TrainAllocation {
    pub variant: u64,
    pub count: i32,
}
value_struct!(TrainAllocation {
    variant: u64,
    count: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SparseBytes {
    pub len: u32,
    pub runs: Vec<ByteRun>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ByteRun {
    pub start: u32,
    pub bytes: Vec<u8>,
}

impl SparseBytes {
    #[cfg(any(test, feature = "test-support"))]
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

impl replay_rows::value::Value for SparseBytes {
    fn heap_bytes(&self) -> usize {
        self.runs.capacity() * size_of::<ByteRun>()
            + self
                .runs
                .iter()
                .map(|run| run.bytes.capacity())
                .sum::<usize>()
    }
    const MINIMUM_ENCODED_BYTES: usize = 8;

    fn encoded_bytes(&self) -> io::Result<usize> {
        self.check()?;
        let mut end = 0_u32;
        self.runs.iter().try_fold(8, |size, run| {
            let gap = replay_rows::rows::varint_bytes(u128::from(run.start - end));
            let length = replay_rows::rows::varint_bytes(run.bytes.len() as u128);
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TrainInventoryRow {
    pub country: u32,
    pub node_count: u32,
    pub allocations: Vec<TrainAllocation>,
    pub controller_access: SparseBytes,
    pub allocated_trucks: Recorded<Option<i32>>,
    pub required_trucks_raw: Recorded<Option<i64>>,
    pub required_trains: Recorded<Option<i32>>,
}
typed_codec!(TrainInventoryRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (node_count): U32, 1 => (allocations): Opaque, 2 => (controller_access): Opaque, 3 => (allocated_trucks): OptionalI32, 4 => (required_trucks_raw): OptionalI64, 5 => (required_trains): OptionalI32 }, |_row| Ok(()),
 |field, format| field == 4 && format < replay_rows::rows::compatibility::VARIANT_STAT_WITNESS_FORMAT);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    pub predecessors: Vec<u32>,
    pub successors: Vec<u32>,
    pub name_prefix: Vec<u8>,
    pub name: Vec<u8>,
    pub name_first: Vec<u8>,
    pub name_second: Vec<u8>,
    pub name_unique: Vec<u8>,
    pub name_postfix: Vec<u8>,
    pub child_fronts: Recorded<Option<OrderChildFrontInputs>>,
    pub invasion_transports: Recorded<Option<i32>>,
    pub execution: Recorded<Option<OrderExecutionInputs>>,
}
typed_codec!(ArmyOrderPlanRow, 96, |row| (row.owner_group as u128) << 32 | row.instance as u128, |key| Self { owner_group: (key >> 32) as u64, instance: key as u32, ..Default::default() },
 { 0 => (front): U64, 1 => (front_valid): U8, 2 => (section): U32, 3 => (segment_start): I32, 4 => (segment_end): I32, 5 => (blitz): U8, 6 => (withdraw): U8, 7 => (fallback): U8, 8 => (virtual_order): U8, 9 => (has_child): U8, 10 => (owner_override): U64, 11 => (assigned_divisions): U64List, 12 => (predecessors): U32List, 13 => (successors): U32List, 14 => (name_prefix): Bytes, 15 => (name): Bytes, 16 => (name_first): Bytes, 17 => (name_second): Bytes, 18 => (name_unique): Bytes, 19 => (name_postfix): Bytes, 20 => (child_fronts): Opaque, 21 => (invasion_transports): OptionalI32, 22 => (execution): Opaque }, |row| {
 if let Recorded::Value(Some(inputs)) = &row.child_fronts {
  if inputs.area_province_count <= 2 && (inputs.count.is_some() || inputs.group_child_count.is_some() || inputs.sections.is_some()) { return Err(invalid("small-area child operands outside native demand")); }
  if inputs.group_child_count.is_some() && inputs.count.is_none_or(|count| count == 0) { return Err(invalid("army-group child count outside native demand")); }
  if let Some(sections) = &inputs.sections {
   let prefix = inputs.count.and_then(|count| if count == 0 { Some(0) } else { inputs.group_child_count.filter(|&group_count| group_count > 0).map(|group_count| count.min(group_count as u32) as usize) });
   if prefix != Some(sections.len()) || sections.iter().any(|section| section.group == 0 || section.group as u32 != engine_reference_values::native_values::orders_group::REFERENCE_TYPE) { return Err(invalid("invalid demanded child-front prefix or group identity")); }
   if !sections.is_empty() && inputs.group_child_count.is_none_or(|count| count <= 0) { return Err(invalid("stored child spans outside native group-count demand")); }
  }
 }
 Ok(())
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OrderExecutionInputs {
    pub route_is_ok: Option<u8>,
    pub paradrop: Option<ParadropPreparationInputs>,
}
value_struct!(OrderExecutionInputs { route_is_ok: Option<u8>, paradrop: Option<ParadropPreparationInputs> });
impl OpaqueValue for OrderExecutionInputs {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ParadropPreparationInputs {
    pub progress: i64,
    pub terms: Option<ParadropPreparationTerms>,
}
value_struct!(ParadropPreparationInputs { progress: i64, terms: Option<ParadropPreparationTerms> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ParadropPreparationTerms {
    pub target: u32,
    pub country_term: i64,
    pub relation_term: Option<i64>,
    pub leader_term: Option<i64>,
    pub days_term: i64,
}
value_struct!(ParadropPreparationTerms { target: u32, country_term: i64, relation_term: Option<i64>, leader_term: Option<i64>, days_term: i64 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OrderChildFrontInputs {
    pub area_province_count: i32,
    pub count: Option<u32>,
    pub group_child_count: Option<i32>,
    pub sections: Option<Vec<OrderChildFront>>,
}
value_struct!(OrderChildFrontInputs { area_province_count: i32, count: Option<u32>, group_child_count: Option<i32>, sections: Option<Vec<OrderChildFront>> });
impl OpaqueValue for OrderChildFrontInputs {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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


#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OrdersGroupPlanRow {
    pub group: u64,
    pub red: u32,
    pub green: u32,
    pub blue: u32,
    pub alpha: u32,
    pub primary_orders: Vec<u32>,
    pub fallback_orders: Vec<u32>,
    pub name: Vec<u8>,
}
typed_codec!(OrdersGroupPlanRow, 64, |row| row.group as u128, |key| Self { group: key as u64, ..Default::default() },
 { 0 => (red): U32, 1 => (green): U32, 2 => (blue): U32, 3 => (alpha): U32, 4 => (primary_orders): U32List, 5 => (fallback_orders): U32List, 6 => (name): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FrontRow {
    pub front: u64,
    pub section_ids: Vec<u32>,
    pub section_ends: Vec<u32>,
    pub province_pairs: Vec<u32>,
}
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionTheatreRow {
    pub faction: u64,
    pub ordinal: u32,
    pub icon: Vec<u8>,
    pub regions: Vec<u32>,
    pub country_count: u32,
    pub hidden_for: Vec<u32>,
    pub name: Recorded<Vec<u8>>,
    pub owner: Recorded<u32>,
    pub commander: Recorded<Option<u64>>,
    pub countries: Recorded<Vec<u32>>,
}
typed_codec!(FactionTheatreRow, 96, |row| (row.faction as u128) << 32 | row.ordinal as u128,
 |key| Self { faction: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
 { 0 => (icon): Bytes, 1 => (regions): U32List, 2 => (country_count): U32, 3 => (hidden_for): U32List, 4 => (name): Bytes, 5 => (owner): U32, 6 => (commander): OptionalU64, 7 => (countries): U32List }, |_row| Ok(()));
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TerrainSpeed {
    pub terrain: i32,
    pub modifier: i64,
}
value_struct!(TerrainSpeed {
    terrain: i32,
    modifier: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEtaInputsRow {
    pub division: u64,
    pub sample_hour: i32,
    pub speed_modifier: i64,
    pub speed_base: i64,
    pub speed_org: Option<i64>,
    pub speed_fuel: Option<i64>,
    pub current_speed: i64,
    pub terrain_speed: Vec<TerrainSpeed>,
    pub fort_penalty: i64,
    pub snow_penalty: i64,
    pub river_adjustment: i64,
    pub river_leader: i64,
    pub class: u32,
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    pub combat_province: u32,
    pub map_entity: u8,
    pub strategic_action: Recorded<StrategicAction>,
    pub withdrawal: Recorded<Option<u8>>,
    pub movement_priority: Recorded<Option<i32>>,
    pub order_transport: Recorded<Option<DivisionOrderTransportInputs>>,
    pub combat_references: Recorded<Option<UnitCombatReferences>>,
    pub fuel: Recorded<Option<i64>>,
    pub transport_speed: Recorded<Option<i64>>,
    pub own_base_speed: Option<Option<i64>>,
    pub queued_actions: Recorded<Option<i32>>,
}
impl DivisionMovementRow {
    pub fn base_speed(&self, panel: Option<&DivisionPanelStatsRow>) -> Option<i64> {
        match self.own_base_speed {
            Some(own) => own,
            None => Self::panel_speed(self.path_total, panel?.panel_stats),
        }
    }

    pub fn panel_speed(path_total: u32, words: [Option<i64>; 22]) -> Option<i64> {
        if (path_total as i32) <= 0 {
            return None;
        }
        words[DivisionPanelStatsRow::MAXIMUM_SPEED]
    }
}
typed_codec!(DivisionMovementRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (source): U32, 1 => (target): U32, 2 => (numerator): I64, 3 => (retreat): U8, 4 => (strategic_redeployment): U8, 5 => (naval_transfer): U8, 6 => (combat_count): U32, 7 => (path_total): U32, 8 => (path): U32List, 9 => (alternate_total): U32, 10 => (alternate_path): U32List, 11 => (combat_role): U8, 12 => (combat_province): U32, 13 => (map_entity): U8, 14 => (strategic_action): Opaque, 15 => (withdrawal): Opaque, 16 => (movement_priority): OptionalI32, 17 => (order_transport): Opaque, 18 => (combat_references): Opaque, 19 => (fuel): OptionalI64, 20 => (transport_speed): OptionalI64, 21 => (own_base_speed): Opaque, 22 => (queued_actions): OptionalI32 }, |row| {
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionOrderTransportInputs {
    pub naval_weight: Option<i64>,
    pub paradrop_eligible: Option<u8>,
    pub paradrop_modifiers: Option<ParadropTransportModifiers>,
}
value_struct!(DivisionOrderTransportInputs { naval_weight: Option<i64>, paradrop_eligible: Option<u8>, paradrop_modifiers: Option<ParadropTransportModifiers> });
impl OpaqueValue for DivisionOrderTransportInputs {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ParadropTransportModifiers {
    pub country_term: i64,
    pub commander_term: Option<i64>,
}
value_struct!(ParadropTransportModifiers { country_term: i64, commander_term: Option<i64> });


#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitSupplyDemand {
    pub count: i32,
    pub value: i64,
}
value_struct!(UnitSupplyDemand {
    count: i32,
    value @ UnitSupplyInputs::STEP: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitSupplyInputs {
    pub asked: i64,
    pub received: i64,
    pub from_state: i64,
    pub air_drops: i64,
    pub disruption: i64,
    pub division_demand: UnitSupplyDemand,
    pub airplane_demand: UnitSupplyDemand,
    pub ship_demand: UnitSupplyDemand,
    pub origins: Option<Vec<UnitSupplyOrigin>>,
}
impl UnitSupplyInputs {
    pub const STEP: replay_rows::display::Step =
        replay_rows::display::Step::fixed5(1_000, replay_rows::display::Rounding::HalfAwayFromZeroNonzero);
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
impl OpaqueValue for UnitSupplyInputs {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionSupplyFooter {
    pub asked: i64,
    pub received: i64,
    pub from_state: i64,
    pub air_drops: i64,
    pub disruption: i64,
    pub division_demand: UnitSupplyDemand,
    pub airplane_demand: UnitSupplyDemand,
    pub ship_demand: UnitSupplyDemand,
    pub origins: Option<Vec<UnitSupplyOrigin>>,
}
impl DivisionSupplyFooter {
    pub const STEP: replay_rows::display::Step = UnitSupplyInputs::STEP;
}

struct StoredDemand {
    count: i32,
    units: i64,
}
value_struct!(StoredDemand {
    count: i32,
    units: i64
});

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

struct StoredOrigins {
    origins: Option<Vec<StoredOrigin>>,
    shortfalls: bool,
}

replay_rows::checkpoint::values::wire_value!(StoredOrigins);

impl StoredOrigins {
    const SHORTFALLS: u8 = 2;
}

impl Value for StoredOrigins {
    fn heap_bytes(&self) -> usize {
        Value::heap_bytes(&self.origins)
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;

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

    fn historical(&self) -> io::Result<Option<UnitSupplyInputs>> {
        if historical_stamp().is_none() {
            return Ok(None);
        }
        Value::try_clone(self).map(|footer| Some(footer.into()))
    }
}

const _: () = assert!(
    replay_rows::display::stepped_format(replay_rows::rows::compatibility::SUPPLY_SHORTFALL_FORMAT),
    "the shortfall layout stores whole display steps"
);

impl Value for DivisionSupplyFooter {
    fn heap_bytes(&self) -> usize {
        Value::heap_bytes(&self.division_demand)
            + Value::heap_bytes(&self.airplane_demand)
            + Value::heap_bytes(&self.ship_demand)
            + Value::heap_bytes(&self.origins)
    }
    const MINIMUM_ENCODED_BYTES: usize = <UnitSupplyInputs as Value>::MINIMUM_ENCODED_BYTES;
    const HAS_STEPS: bool = true;
    #[cfg(any(test, feature = "test-support"))]
    const STRUCT_FIELDS: usize = <UnitSupplyInputs as Value>::STRUCT_FIELDS;

    fn round_display(&mut self) {
        let mut inputs = UnitSupplyInputs::from(std::mem::take(self));
        inputs.round_display();
        *self = inputs.into();
    }
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
        if input.format >= replay_rows::rows::compatibility::SUPPLY_SHORTFALL_FORMAT {
            Self::from_stored(StoredFooter::decode(input)?)
        } else {
            UnitSupplyInputs::decode(input).map(Self::from)
        }
    }
}
impl OpaqueValue for DivisionSupplyFooter {
    const RESTORED: bool = true;
}

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionStatusRow {
    pub division: u64,
    pub entrenchment: i64,
    pub entrenchment_cap: i64,
    pub supply_ratio: Option<i64>,
    pub attrition_value: i64,
    pub attrition_threshold: i64,
    pub exercising: u8,
    pub has_order: u8,
    pub invalid_orders: u8,
    pub strength_factor: i64,
    pub model_number: i32,
    pub acclimatization_maximum: Option<i64>,
    pub acclimatization: Vec<DivisionAcclimatization>,
    pub move_flags: u8,
    pub border_combat_stats: Recorded<Option<[i64; 3]>>,
    pub naming: Recorded<Option<NameOperands>>,
    pub supply_grace_maximum: Recorded<i64>,
    pub supply_grace_remaining: Recorded<i64>,
    pub country_ordinal: Recorded<Option<u32>>,
    pub invalid_orders_reason: Recorded<Option<i32>>,
    pub supply: Recorded<Option<DivisionSupplyFooter>>,
    pub dig_in_factor: Recorded<Option<i64>>,
    pub name: Recorded<Option<Vec<u8>>>,
    pub attack: Recorded<Witnessed<DivisionAttackInputs>>,
    pub camouflage: Recorded<Option<DivisionCamouflage>>,
    pub attack_stats: Recorded<Option<DivisionAttackStats>>,
    pub weather_factor: Recorded<Option<i64>>,
    pub name_template: Recorded<Witnessed<u64>>,
    pub movement_stats: Recorded<Option<DivisionMovementStats>>,
    pub out_of_supply_days: Recorded<Option<i32>>,
    pub commander_modifiers: Recorded<Option<[Option<i64>; 5]>>,
    pub supply_ratio_exact: Recorded<Option<i64>>,
    pub supply_grace_exact: Recorded<Option<i64>>,
    pub experience: Recorded<Option<i64>>,
    pub pending_template: Recorded<Option<u64>>,
    pub officer_experience: Recorded<Option<i64>>,
    pub career_counter: Recorded<Option<i32>>,
    pub panel_source: Recorded<Option<[u64; 3]>>,
    pub combat_movement: Recorded<Option<Option<i64>>>,
    pub organisation_exact: Recorded<Option<i64>>,
    pub maximum_organisation_exact: Recorded<Option<i64>>,
    pub commander_width_factor: Recorded<Option<i64>>,
}
impl DivisionStatusRow {
    pub fn officer_experience(&self) -> Option<i64> {
        self.officer_experience
            .require("DivisionStatus.officer_experience")
            .ok()
            .copied()
            .flatten()
    }
    pub fn casualties_taken(&self) -> Option<i32> {
        self.career_counter
            .require("DivisionStatus.career_counter")
            .ok()
            .copied()
            .flatten()
    }
    pub fn panel_source(&self) -> Option<[u64; 3]> {
        self.panel_source
            .require("DivisionStatus.panel_source")
            .ok()
            .copied()
            .flatten()
    }

    pub fn normalized_experience(&self, manpower: i32) -> Option<i64> {
        let experience = *self
            .experience
            .require("DivisionStatus.experience")
            .ok()?
            .as_ref()?;
        Some(if manpower < 1 {
            0
        } else if (experience as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
            0xffff_ffff
        } else {
            experience.wrapping_mul(100_000) / (i64::from(manpower) * 100_000)
        })
    }

    pub fn supply_exact(&self) -> Option<[i64; 2]> {
        match (&self.supply_ratio_exact, &self.supply_grace_exact) {
            (Recorded::Value(Some(ratio)), Recorded::Value(Some(grace))) => Some([*ratio, *grace]),
            _ => None,
        }
    }

    pub fn displayed_supply_ratio(&self) -> Option<i64> {
        let exact = match self.supply_ratio_exact {
            Recorded::Value(exact) => exact,
            _ => None,
        };
        Self::displayed_supply(self.supply_ratio, exact)
    }

    pub fn displayed_supply_grace(&self, grace: Option<i32>) -> Option<i64> {
        if let Recorded::Value(Some(exact)) = self.supply_grace_exact {
            return Some(Self::displayed_grace(exact, grace));
        }
        match self.supply_grace_remaining {
            Recorded::Value(remaining) => Some(remaining),
            Recorded::NotRecorded => None,
        }
    }

    pub fn displayed_supply(ratio: Option<i64>, exact: Option<i64>) -> Option<i64> {
        ratio.or_else(|| exact.map(|exact| Self::SUPPLY_RATIO_STEP.round(exact)))
    }

    pub fn supply_ratio_bounds(ratio: Option<i64>, exact: Option<i64>) -> Option<[i64; 2]> {
        exact
            .map(|raw| [raw, raw])
            .or_else(|| ratio.map(|shown| Self::SUPPLY_RATIO_STEP.toward_zero_bounds(shown)))
    }

    pub fn recorded_supply_ratio_bounds(&self) -> Option<[i64; 2]> {
        let exact = match self.supply_ratio_exact {
            Recorded::Value(exact) => exact,
            Recorded::NotRecorded => None,
        };
        Self::supply_ratio_bounds(self.supply_ratio, exact)
    }

    pub fn supply_grace_bounds(remaining: i64, exact: Option<i64>, grace_hours: i32) -> [i64; 2] {
        if let Some(raw) = exact {
            return [raw, raw];
        }
        let [low, mut high] = Self::SUPPLY_GRACE_STEP.toward_zero_bounds(remaining);
        let limit = i64::MAX / 100_000;
        if grace_hours > 0 && (0..=limit).contains(&remaining) {
            let step = i64::from(grace_hours) * 1_000;
            if remaining % step == 0 {
                high = high.max(remaining.saturating_add(step - 1).min(limit));
            }
        }
        [low, high]
    }

    pub fn recorded_supply_grace_bounds(&self, grace_hours: i32) -> Option<[i64; 2]> {
        if let Recorded::Value(Some(raw)) = self.supply_grace_exact {
            return Some([raw, raw]);
        }
        match self.supply_grace_remaining {
            Recorded::Value(remaining) => {
                Some(Self::supply_grace_bounds(remaining, None, grace_hours))
            }
            Recorded::NotRecorded => None,
        }
    }

    pub fn effective_supply_bounds(
        ratio: [i64; 2],
        maximum_grace: i64,
        grace: [i64; 2],
        grace_hours: i32,
    ) -> [i64; 2] {
        if maximum_grace < 1 {
            return [0, 0];
        }
        let [fraction_low, fraction_high] = Self::grace_fraction_bounds(grace, grace_hours);
        [
            ratio[0].max(fraction_low).clamp(0, 100_000),
            ratio[1].max(fraction_high).clamp(0, 100_000),
        ]
    }

    fn grace_fraction_bounds(grace: [i64; 2], grace_hours: i32) -> [i64; 2] {
        let divisor = i64::from(grace_hours) * 100_000;
        if divisor == 0 {
            return [0xffff_ffff, 0xffff_ffff];
        }
        let [low, high] = grace;
        if low == high && replay_rows::display::is_sentinel(low) {
            return [0xffff_ffff, 0xffff_ffff];
        }
        let mut has_sentinel = false;
        let a = if low <= i64::MIN + 1 {
            has_sentinel = true;
            i64::MIN + 2
        } else {
            low
        };
        let b = if high == i64::MAX {
            has_sentinel = true;
            i64::MAX - 1
        } else {
            high
        };
        if a > b {
            return [0xffff_ffff, 0xffff_ffff];
        }
        let eval = |x: i64| x.wrapping_mul(100_000) / divisor;
        let period =
            |x: i64| (i128::from(x) * 100_000 + (1_i128 << 63)).div_euclid(1_i128 << 64);
        let (pa, pb) = (period(a), period(b));
        let (fa, fb) = (eval(a), eval(b));
        let (mut min_f, mut max_f) = if pa == pb {
            (fa.min(fb), fa.max(fb))
        } else if pb == pa + 1 {
            let wrap = ((pa + 1) * (1_i128 << 64) - (1_i128 << 63) + 99_999)
                .div_euclid(100_000) as i64;
            let (before, after) = (eval(wrap - 1), eval(wrap));
            (
                fa.min(fb).min(before).min(after),
                fa.max(fb).max(before).max(after),
            )
        } else {
            let (lo, hi) = (i64::MIN / divisor, i64::MAX / divisor);
            (lo.min(hi), lo.max(hi))
        };
        if has_sentinel {
            min_f = min_f.min(0xffff_ffff);
            max_f = max_f.max(0xffff_ffff);
        }
        [min_f, max_f]
    }

    pub const COMMANDER_KEYS: [&'static str; 6] = [
        "army_speed_factor",
        "army_armor_speed_factor",
        "out_of_supply_factor",
        "combat_width_factor",
        "river_crossing_factor",
        "special_forces_out_of_supply_factor",
    ];
    pub const MAXIMUM_STEP: replay_rows::display::Step =
        replay_rows::display::Step::fixed5(10_000, replay_rows::display::Rounding::TowardZeroNonzero);
    pub const SUPPLY_RATIO_STEP: replay_rows::display::Step =
        replay_rows::display::Step::fixed5(1_000, replay_rows::display::Rounding::TowardZero);
    pub const SUPPLY_GRACE_STEP: replay_rows::display::Step =
        replay_rows::display::Step::fixed5(1_000, replay_rows::display::Rounding::TowardZero);

    pub fn displayed_grace(remaining: i64, grace: Option<i32>) -> i64 {
        match grace {
            Some(hours) if hours > 0 && (0..=i64::MAX / 100_000).contains(&remaining) => {
                remaining - remaining % (i64::from(hours) * 1_000)
            }
            _ => Self::SUPPLY_GRACE_STEP.round(remaining),
        }
    }

    pub const MOVE_EXILED: u8 = 1;
    pub const MOVE_CAN_EXFILTRATE_FROM_COAST: u8 = 2;
    pub const MOVE_NONCORE_SUPPLY_BYPASS: u8 = 4;
    pub const MOVE_NONCORE_AIR: u8 = 8;
    pub const MOVE_NONCORE_AIR_REFUSED: u8 = 64;
    pub const MOVE_LOW_ORGANISATION: u8 = 16;
    pub const MOVE_ORGANISATION_REFUSED: u8 = 32;

    pub fn has_attrition(&self) -> bool {
        Self::attrition(self.attrition_threshold, self.attrition_value)
    }

    pub fn attrition(threshold: i64, value: i64) -> bool {
        threshold < value
    }
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionPanelStatsRow {
    pub division: u64,
    pub panel_stats: [Option<i64>; 22],
    pub suppression: Recorded<Option<i64>>,
    pub status_maxima: [Option<i64>; 2],
}
impl DivisionPanelStatsRow {
    pub const PANEL_STAT_SELECTORS: [usize; 22] = [
        0x3d, 0x3c, 0, 6, 0xb, 0x41, 0x42, 9, 0xe, 4, 5, 1, 2, 0x3f, 0x40, 8, 7, 0xf, 0x43, 0x45,
        3, 0x3e,
    ];
    pub const SUPPRESSION_SELECTOR: usize = 0xc;
    pub const MAXIMUM_SPEED: usize = 21;

    pub fn maximum_hit_points(&self) -> Option<i64> {
        Self::shown_maximum(self.status_maxima[0], self.panel_stats[0])
    }

    pub fn shown_maximum(status: Option<i64>, word: Option<i64>) -> Option<i64> {
        status.or_else(|| Self::maximum(word))
    }

    pub fn maximum(word: Option<i64>) -> Option<i64> {
        word.map(|raw| DivisionStatusRow::MAXIMUM_STEP.round(raw.max(100)))
    }

    pub fn maximum_organisation(&self) -> Option<i64> {
        Self::shown_maximum(self.status_maxima[1], self.panel_stats[1])
    }

    pub fn word(&self, selector: usize) -> Option<Recorded<Option<i64>>> {
        if selector == Self::SUPPRESSION_SELECTOR {
            return Some(self.suppression);
        }
        Self::PANEL_STAT_SELECTORS
            .iter()
            .position(|&stored| stored == selector)
            .map(|at| Recorded::Value(self.panel_stats[at]))
    }
}
typed_codec!(DivisionPanelStatsRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (panel_stats[0]): OptionalI64, 1 => (panel_stats[1]): OptionalI64, 2 => (panel_stats[2]): OptionalI64, 3 => (panel_stats[3]): OptionalI64, 4 => (panel_stats[4]): OptionalI64, 5 => (panel_stats[5]): OptionalI64, 6 => (panel_stats[6]): OptionalI64, 7 => (panel_stats[7]): OptionalI64, 8 => (panel_stats[8]): OptionalI64, 9 => (panel_stats[9]): OptionalI64, 10 => (panel_stats[10]): OptionalI64, 11 => (panel_stats[11]): OptionalI64, 12 => (panel_stats[12]): OptionalI64, 13 => (panel_stats[13]): OptionalI64, 14 => (panel_stats[14]): OptionalI64, 15 => (panel_stats[15]): OptionalI64, 16 => (panel_stats[16]): OptionalI64, 17 => (panel_stats[17]): OptionalI64, 18 => (panel_stats[18]): OptionalI64, 19 => (panel_stats[19]): OptionalI64, 20 => (panel_stats[20]): OptionalI64, 21 => (panel_stats[21]): OptionalI64, 22 => (suppression): OptionalI64, 23 => (status_maxima[0]): OptionalI64 @ DivisionStatusRow::MAXIMUM_STEP, 24 => (status_maxima[1]): OptionalI64 @ DivisionStatusRow::MAXIMUM_STEP }, |_row| Ok(()));

typed_codec!(DivisionStatusRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (entrenchment): I64, 1 => (entrenchment_cap): I64, 2 => (supply_ratio): OptionalI64 @ Self::SUPPLY_RATIO_STEP, 3 => (attrition_value): I64, 4 => (attrition_threshold): I64, 5 => (exercising): U8, 6 => (has_order): U8, 7 => (invalid_orders): U8, 8 => (strength_factor): I64, 9 => (model_number): I32, 10 => (acclimatization_maximum): OptionalI64, 11 => (acclimatization): Opaque, 12 => (move_flags): U8, 13 => (border_combat_stats): Opaque, 14 => (naming): Opaque, 15 => (supply_grace_maximum): I64, 16 => (supply_grace_remaining): I64 @ Self::SUPPLY_GRACE_STEP, 17 => (country_ordinal): OptionalU32, 18 => (invalid_orders_reason): OptionalI32, 19 => (supply): Opaque, 20 => (dig_in_factor): OptionalI64, 21 => (name): Opaque, 22 => (attack): Opaque, 23 => (camouflage): Opaque, 24 => (attack_stats): Opaque, 25 => (weather_factor): OptionalI64, 26 => (name_template): Opaque, 27 => (movement_stats): Opaque, 28 => (out_of_supply_days): OptionalI32, 29 => (commander_modifiers): Opaque, 30 => (supply_ratio_exact): OptionalI64, 31 => (supply_grace_exact): OptionalI64, 32 => (experience): OptionalI64, 33 => (pending_template): OptionalU64, 34 => (officer_experience): OptionalI64, 35 => (career_counter): OptionalI32, 36 => (panel_source): Opaque, 37 => (combat_movement): Opaque, 38 => (organisation_exact): OptionalI64, 39 => (maximum_organisation_exact): OptionalI64, 40 => (commander_width_factor): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionModelInputRow {
    pub division: u64,
    pub model_number: Option<i32>,
    pub camouflage: Option<DivisionCamouflage>,
    pub exercising: Option<u8>,
}
typed_codec!(DivisionModelInputRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (model_number): OptionalI32, 1 => (camouflage): Opaque, 2 => (exercising): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionRetreatBanRow {
    pub division: u64,
    pub owner: u32,
    pub bans: u8,
}
impl DivisionRetreatBanRow {
    pub const DEFENDING: u8 = 4;
    pub const ATTACKING: u8 = 8;
}
typed_codec!(DivisionRetreatBanRow, 64, |row| row.division as u128,
 |key| Self { division: key as u64, ..Default::default() },
 { 0 => (owner): U32, 1 => (bans): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionAcclimatization {
    pub category: u32,
    pub weight: i64,
}
value_struct!(DivisionAcclimatization {
    category: u32,
    weight: i64
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionCamouflage {
    pub category: Option<u32>,
}
value_struct!(DivisionCamouflage {
    category: Option<u32>
});
impl OpaqueValue for DivisionCamouflage {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionTemplateRow {
    pub template: u64,
    pub owner: u32,
    pub name: Vec<u8>,
    pub name_key: Recorded<Vec<u8>>,
    pub unit_type: i32,
}
typed_codec!(DivisionTemplateRow, 64, |row| row.template as u128,
 |key| Self { template: key as u64, ..Default::default() },
 { 0 => (owner): U32, 1 => (name): Bytes, 2 => (unit_type): I32, 3 => (name_key): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryTemplateStateRow {
    pub garrison_priority: Recorded<Option<i32>>,
    pub country: u32,
    pub selected_staff: u64,
    pub next_hq_order: i32,
    pub lock_all: u8,
    pub garrisons: Recorded<Option<std::sync::Arc<Vec<GarrisonTemplateSelection>>>>,
}
typed_codec!(CountryTemplateStateRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (selected_staff): U64, 1 => (lock_all): U8, 2 => (garrisons): Opaque, 3 => (next_hq_order): I32, 4 => (garrison_priority): OptionalI32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GarrisonTemplateSelection {
    pub occupied: Option<u32>,
    pub state: Option<u32>,
    pub template: Option<u64>,
}
value_struct!(GarrisonTemplateSelection { occupied: Option<u32>, state: Option<u32>, template: Option<u64> });
impl OpaqueValue for GarrisonTemplateSelection {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TemplateHqContext {
    pub target: u64,
    pub deployed: u8,
    pub queued: u8,
    pub deployment: Option<HqDeployment>,
    pub equipment: Vec<HqEquipment>,
    pub required: Vec<HqRequirement>,
}
value_struct!(TemplateHqContext {target:u64, deployed:u8, queued:u8, deployment:Option<HqDeployment>, equipment:Vec<HqEquipment>, required:Vec<HqRequirement>});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct HqDeployment {
    pub country: u32,
    pub priority: i32,
    pub allow_zero: u8,
    pub assembled_manpower: i32,
    pub requested_manpower: i32,
    pub order: i32,
    pub requisitioned: u8,
}
value_struct!(HqDeployment {
    country: u32,
    priority: i32,
    allow_zero: u8,
    assembled_manpower: i32,
    requested_manpower: i32,
    order: i32,
    requisitioned: u8
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct HqEquipment {
    pub variant: u64,
    pub amount: i64,
}
value_struct!(HqEquipment {
    variant: u64,
    amount: i64
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct HqRequirement {
    pub token: u32,
    pub amount: i64,
}
value_struct!(HqRequirement {
    token: u32,
    amount: i64
});
impl OpaqueValue for TemplateHqContext {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TemplateSubunit {
    pub token: u32,
    pub local: u8,
    pub count: i32,
}
value_struct!(TemplateSubunit {
    token: u32,
    local: u8,
    count: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TemplateCell {
    pub token: u32,
    pub local: u8,
    pub occupied: bool,
}
value_struct!(TemplateCell {
    token: u32,
    local: u8,
    occupied: bool
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TemplateGrid {
    pub columns: u32,
    pub rows: u32,
    pub cells: Vec<TemplateCell>,
}
value_struct!(TemplateGrid { columns: u32, rows: u32, cells: Vec<TemplateCell> });

impl TemplateGrid {
    pub fn cell(&self, column: i16, row: i16) -> Option<&TemplateCell> {
        let index = (self.rows as i16).wrapping_mul(column).wrapping_add(row);
        self.cells.get(usize::try_from(index).ok()?)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TemplateLayout {
    pub regiments: TemplateGrid,
    pub support: TemplateGrid,
    pub regimental_support: TemplateGrid,
}
value_struct!(TemplateLayout {
    regiments: TemplateGrid,
    support: TemplateGrid,
    regimental_support: TemplateGrid
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TemplateSettings {
    pub roster_ordinal: u32,
    pub names_group: Option<Vec<u8>>,
    pub priority: i32,
    pub is_army_hq: u8,
    pub locked: u8,
    pub obsolete: u8,
    pub obsolete_change_date: i32,
    pub force_allow_recruiting: u8,
    pub allow_new_equipment: u8,
    pub allow_foreign_equipment: u8,
    pub parent: u64,
    pub creator_country: i32,
    pub origin_type: i32,
    pub fake_intel: u8,
    pub division_cap: Option<i32>,
    pub equipment_niche: u8,
    pub role: Option<u32>,
    pub ai_role: Option<u32>,
    pub ai_role_fallback: u32,
    pub total_manpower: i32,
    pub command_power_cost: i64,
    pub training_time: i32,
    pub forbidden_equipment: Option<Vec<u32>>,
}
value_struct!(TemplateSettings {
    roster_ordinal: u32,
    names_group: Option<Vec<u8>>,
    priority: i32, is_army_hq: u8, locked: u8, obsolete: u8,
    obsolete_change_date: i32, force_allow_recruiting: u8,
    allow_new_equipment: u8, allow_foreign_equipment: u8,
    parent: u64, creator_country: i32, origin_type: i32, fake_intel: u8,
    division_cap: Option<i32>, equipment_niche: u8,
    role: Option<u32>, ai_role: Option<u32>, ai_role_fallback: u32,
    total_manpower: i32, command_power_cost: i64, training_time: i32,
    forbidden_equipment: Option<Vec<u32>>
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TemplateGrids {
    pub regiments: Vec<TemplateSubunit>,
    pub support: Vec<TemplateSubunit>,
    pub regimental_support: Vec<TemplateSubunit>,
}
value_struct!(TemplateGrids {
    regiments: Vec<TemplateSubunit>,
    support: Vec<TemplateSubunit>,
    regimental_support: Vec<TemplateSubunit>
});

impl TemplateGrids {
    pub fn ambience_slots(&self) -> impl Iterator<Item = &TemplateSubunit> {
        self.regiments
            .iter()
            .chain(&self.support)
            .filter(|slot| slot.token != 0)
    }
}

#[cfg(test)]
mod template_grid_tests {
    use super::*;

    #[test]
    fn spatial_lookup_keeps_column_major_holes_and_refuses_outside_cells() {
        let grid = TemplateGrid {
            columns: 2,
            rows: 3,
            cells: (0..6)
                .map(|index| TemplateCell {
                    token: index + 10,
                    local: 0,
                    occupied: index != 2,
                })
                .collect(),
        };
        assert_eq!(grid.cell(1, 0).unwrap().token, 13);
        assert!(!grid.cell(0, 2).unwrap().occupied);
        assert!(grid.cell(2, 0).is_none());
        assert_eq!(grid.cell(0, 3).unwrap().token, 13);
        assert!(grid.cell(-1, 0).is_none());
    }

    #[test]
    fn ambience_slots_count_regiments_and_support_but_not_regimental_support() {
        let entry = |token, count| TemplateSubunit {
            token,
            local: 0,
            count,
        };
        let grids = TemplateGrids {
            regiments: vec![entry(1, 4), entry(2, 1)],
            support: vec![entry(3, 1), entry(0, 2), entry(1, 1)],
            regimental_support: vec![entry(1, 2), entry(4, 1)],
        };
        let slots: Vec<_> = grids
            .ambience_slots()
            .map(|slot| (slot.token, slot.count))
            .collect();
        assert_eq!(slots, [(1, 4), (2, 1), (3, 1), (1, 1)]);
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatProgressRow {
    pub combat: u64,
    pub attacker_progress: RetiredI64,
    pub defender_progress: RetiredI64,
    pub direction_attacker: RetiredU64,
    pub direction_province: RetiredU32,
    pub attacker_manpower_loss: Recorded<i32>,
    pub defender_manpower_loss: Recorded<i32>,
    pub attacker_equipment_losses: Recorded<Vec<LandEquipmentLoss>>,
    pub defender_equipment_losses: Recorded<Vec<LandEquipmentLoss>>,
}
typed_codec!(LandCombatProgressRow, 64, |row| row.combat as u128,
 |key| Self { combat: key as u64, ..Default::default() },
 { 0 => (attacker_progress): I64, 1 => (defender_progress): I64,
   2 => (direction_attacker): U64, 3 => (direction_province): U32,
   4 => (attacker_manpower_loss): I32, 5 => (defender_manpower_loss): I32,
   6 => (attacker_equipment_losses): Opaque, 7 => (defender_equipment_losses): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PreferredTacticRow {
    pub country: u32,
    pub tactic_id: i32,
    pub token: Vec<u8>,
}
typed_codec!(PreferredTacticRow, 32, |row| row.country as u128, |key| Self { country: key as u32, tactic_id: Default::default(), token: Default::default() },
 { 0 => (tactic_id): I32, 1 => (token): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryTacticUnlockRow {
    pub country: u32,
    pub unlocked_tactic_ids: Vec<u32>,
}
typed_codec!(CountryTacticUnlockRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (unlocked_tactic_ids): U32List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DisruptedSupplyRow {
    pub country: u32,
    pub province: u32,
    pub node_type: u32,
    pub value: i64,
}
typed_codec!(DisruptedSupplyRow, 96, |row| (row.country as u128) << 64 | (row.province as u128) << 32 | row.node_type as u128, |key| Self { country: (key >> 64) as u32, province: (key >> 32) as u32, node_type: key as u32, value: Default::default() },
 { 0 => (value): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RailCooldownRow {
    pub province: u32,
    pub cooldown_ticks: i32,
    pub building: Recorded<Option<RailwayBuildingState>>,
    pub selection: Recorded<Option<RailwaySelectionInputs>>,
    pub selection_hour: Recorded<Option<i32>>,
    pub building_bound: Recorded<Option<bool>>,
}
typed_codec!(RailCooldownRow, 32, |row| row.province as u128, |key| Self { province: key as u32, ..Default::default() },
 { 0 => (cooldown_ticks): I32, 1 => (building): Opaque, 2 => (selection): Opaque, 3 => (selection_hour): OptionalI32, 4 => (building_bound): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RailEdgeRow {
    pub from: u32,
    pub to: u32,
    pub level: i32,
}
typed_codec!(RailEdgeRow, 64, |row| (row.from as u128) << 32 | row.to as u128, |key| Self { from: (key >> 32) as u32, to: key as u32, level: Default::default() },
 { 0 => (level): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandTheatreRow {
    pub id: u64,
    pub owner: u32,
    pub ordinal: u32,
    pub volunteers_theatre: i32,
}
typed_codec!(LandTheatreRow, 64, |row| row.id as u128, |key| Self { id: key as u64, owner: Default::default(), ordinal: Default::default(), volunteers_theatre: Default::default() },
 { 0 => (owner): U32, 1 => (ordinal): U32, 2 => (volunteers_theatre): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandTheatreGroupRow {
    pub id: u64,
    pub theatre: u64,
    pub ordinal: u32,
    pub priority: i32,
    pub name: Vec<u8>,
}
typed_codec!(LandTheatreGroupRow, 64, |row| row.id as u128, |key| Self { id: key as u64, theatre: Default::default(), ordinal: Default::default(), priority: Default::default(), name: Default::default() },
 { 0 => (theatre): U64, 1 => (ordinal): U32, 2 => (priority): I32, 3 => (name): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandTheatreMemberRow {
    pub orders_group: u64,
    pub theater_group: u64,
    pub ordinal: u32,
}
typed_codec!(LandTheatreMemberRow, 128, |row| (row.theater_group as u128) << 64 | row.orders_group as u128, |key| Self { theater_group: (key >> 64) as u64, orders_group: key as u64, ordinal: Default::default() },
 { 0 => (ordinal): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SentVolunteerRow {
    pub country: u32,
    pub ordinal: u32,
    pub division: u64,
}
typed_codec!(SentVolunteerRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, division: Default::default() },
 { 0 => (division): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VolunteerHostRow {
    pub country: u32,
    pub position: u32,
    pub host: u32,
    pub deployed_divisions: i32,
    pub air_capacity_modifier: Recorded<Option<i64>>,
}
typed_codec!(VolunteerHostRow, 64, |row| (row.country as u128) << 32 | row.position as u128, |key| Self { country: (key >> 32) as u32, position: key as u32, host: Default::default(), deployed_divisions: Default::default(), air_capacity_modifier: Default::default() },
 { 0 => (host): U32, 1 => (deployed_divisions): I32, 2 => (air_capacity_modifier): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TheatreAreaRow {
    pub theatre: u64,
    pub ordinal: u32,
    pub representative: Option<u32>,
    pub provinces: Vec<u32>,
}
typed_codec!(TheatreAreaRow, 96, |row| (row.theatre as u128) << 32 | row.ordinal as u128, |key| Self { theatre: (key >> 32) as u64, ordinal: key as u32, representative: Default::default(), provinces: Default::default() },
 { 0 => (representative): OptionalU32, 1 => (provinces): U32List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StationedUnitRow {
    pub theatre: u64,
    pub ordinal: u32,
    pub unit: u64,
}
typed_codec!(StationedUnitRow, 96, |row| (row.theatre as u128) << 32 | row.ordinal as u128, |key| Self { theatre: (key >> 32) as u64, ordinal: key as u32, unit: Default::default() },
 { 0 => (unit): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OrdersGroupRow {
    pub handle: u64,
    pub country: u32,
    pub is_army_group: u8,
    pub motorization_level: u8,
    pub theatre: u64,
    pub theatre_ordinal: Option<u32>,
    pub theater_group: u64,
    pub parent: u64,
    pub leader: u64,
    pub hq_provider: u64,
    pub name: Recorded<Option<Vec<u8>>>,
}
typed_codec!(OrdersGroupRow, 64, |row| row.handle as u128, |key| Self { handle: key as u64, country: Default::default(), is_army_group: Default::default(), motorization_level: Default::default(), theatre: Default::default(), theatre_ordinal: Default::default(), theater_group: Default::default(), parent: Default::default(), leader: Default::default(), hq_provider: Default::default(), name: Default::default() },
 { 0 => (country): U32, 1 => (is_army_group): U8, 2 => (motorization_level): U8, 3 => (theatre): U64, 4 => (theatre_ordinal): OptionalU32, 5 => (theater_group): U64, 6 => (parent): U64, 7 => (leader): U64, 8 => (hq_provider): U64, 9 => (name): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OrdersGroupMemberRow {
    pub group: u64,
    pub ordinal: u32,
    pub unit: u64,
}
typed_codec!(OrdersGroupMemberRow, 96, |row| (row.group as u128) << 32 | row.ordinal as u128, |key| Self { group: (key >> 32) as u64, ordinal: key as u32, unit: Default::default() },
 { 0 => (unit): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyGroupMemberRow {
    pub group: u64,
    pub ordinal: u32,
    pub child: u64,
}
typed_codec!(ArmyGroupMemberRow, 96, |row| (row.group as u128) << 32 | row.ordinal as u128, |key| Self { group: (key >> 32) as u64, ordinal: key as u32, child: Default::default() },
 { 0 => (child): U64 }, |_row| Ok(()));

impl OpaqueValue for TerrainSpeed {}
impl OpaqueValue for TrainAllocation {}
opaque_value!(SparseBytes);
impl OpaqueValue for DivisionAcclimatization {}
impl OpaqueValue for TemplateSubunit {}
opaque_value!(TemplateGrids);

#[cfg(test)]
mod experience_tests {
    use super::*;

    #[test]
    fn division_experience_keeps_stored_xp_separate_from_manpower_normalization() {
        let mut row = DivisionStatusRow {
            experience: Recorded::Value(Some(12_345_678)),
            ..Default::default()
        };
        assert_eq!(row.normalized_experience(1000), Some(12_345));
        assert_eq!(row.normalized_experience(0), Some(0));
        assert_eq!(row.experience, Recorded::Value(Some(12_345_678)));
        row.experience = Recorded::Value(Some(i64::MAX));
        assert_eq!(row.normalized_experience(1), Some(0xffff_ffff));
        row.experience = Recorded::Value(Some(-123_456));
        assert_eq!(row.normalized_experience(10), Some(-12_345));
        row.experience = Recorded::Value(None);
        assert_eq!(row.normalized_experience(10), None);
        row.experience = Recorded::NotRecorded;
        assert_eq!(row.normalized_experience(10), None);
    }
}
opaque_value!(TemplateLayout);
impl OpaqueValue for TemplateLayout {}
opaque_value!(TemplateSettings);

impl OpaqueValue for TemplateSettings {}

replay_rows::wire_value!(
    SparseBytes,
);

replay_rows::structure!(DivisionSupplyFooter {
    asked: i64, received: i64, from_state: i64, air_drops: i64, disruption: i64,
    division_demand: UnitSupplyDemand, airplane_demand: UnitSupplyDemand,
    ship_demand: UnitSupplyDemand, origins: Option<Vec<UnitSupplyOrigin>>,
});
