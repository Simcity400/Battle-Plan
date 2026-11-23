use super::*;
use game_model::fuel_demand::{AirInputs, Modifier};

value_struct!(Modifier { raw: Option<i64>, present: bool, effective: i64 });
value_struct!(AirInputs { reached: bool, mission_first: u32, mission_selected: u32,
    mask: u32, veto: u8, counter: i32, limit: i32, override_reference: u64,
    override_resolved: bool, planes: i32, capacity_threshold: i64, capacity_divisor: i32,
    active_count: i32, base_fuel: i64, country_modifier: Modifier, ace: u64,
    ace_alive: u8, ace_assigned_wing: u64, ace_assigned_planes: Option<i32>,
    ace_count: i64, ace_cap: i64, ace_modifier: Modifier, ace_country_modifier: Modifier,
    air_weight: i64, mission_cost: i64, period: u8, region: Option<u32>,
    context_counts: [i32; 24], context_thresholds: [i64; 24] });

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDemandUnit {
    pub reference: u64,
    pub parent: u64,
    pub ordinal: u32,
    pub owners: Option<[u32; 3]>,
    pub rate_raw: Option<i64>,
    pub reason: Vec<u8>,
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
    pub group: Option<u64>,
    pub group_reason: Vec<u8>,
}
value_struct!(FuelDemandUnit { reference: u64, parent: u64, ordinal: u32,
    owners: Option<[u32; 3]>, rate_raw: Option<i64>, reason: Vec<u8>,
    name: Option<Vec<u8>>, name_reason: Vec<u8>, group: Option<u64>, group_reason: Vec<u8> });

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDemandGroup {
    pub reference: u64,
    pub country: u32,
    pub parent: u64,
    pub army_group: u8,
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
    pub members: Vec<u64>,
    pub children: Vec<u64>,
}
value_struct!(FuelDemandGroup { reference: u64, country: u32, parent: u64,
    army_group: u8, name: Option<Vec<u8>>, name_reason: Vec<u8>, members: Vec<u64>, children: Vec<u64> });

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDemandFleet {
    pub reference: u64,
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
}
value_struct!(FuelDemandFleet { reference: u64, name: Option<Vec<u8>>, name_reason: Vec<u8> });

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDemandWing {
    pub reference: u64,
    pub owner: Option<u32>,
    pub country_override: Option<i32>,
    pub owner_reason: Vec<u8>,
    pub pool_ordinal: u32,
    pub wing_ordinal: u32,
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
    pub inputs: Option<AirInputs>,
    pub context: Option<u32>,
    pub reason: Vec<u8>,
}
value_struct!(FuelDemandWing { reference: u64, owner: Option<u32>, country_override: Option<i32>, owner_reason: Vec<u8>, pool_ordinal: u32,
    wing_ordinal: u32, name: Option<Vec<u8>>, name_reason: Vec<u8>, inputs: Option<AirInputs>,
    context: Option<u32>, reason: Vec<u8> });

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDemandSite {
    pub list: u8,
    pub ordinal: u32,
    pub base: u64,
    pub container_owner: Option<u32>,
    pub capacity_raw: Option<i64>,
    pub demilitarized: Option<u8>,
    pub wings: Vec<FuelDemandWing>,
    pub reason: Vec<u8>,
}
value_struct!(FuelDemandSite { list: u8, ordinal: u32, base: u64, container_owner: Option<u32>,
    capacity_raw: Option<i64>, demilitarized: Option<u8>, wings: Vec<FuelDemandWing>, reason: Vec<u8> });

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDemandContext {
    pub region: u32,
    pub sums: [i64; 24],
}
value_struct!(FuelDemandContext {
    region: u32,
    sums: [i64; 24]
});

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum FuelDemandHour {
    #[default]
    Host,
    Stamped(i32),
    Historical(i32),
}

impl FuelDemandHour {
    pub fn at(self, read_hour: i32) -> i32 {
        match self {
            Self::Host => read_hour,
            Self::Stamped(hour) | Self::Historical(hour) => hour,
        }
    }
}

impl Value for FuelDemandHour {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(match (replay_rows::rows::historical_stamp(), self) {
            (Some(_), _) => size_of::<i32>(),
            (None, Self::Host) => 1,
            (None, Self::Stamped(_) | Self::Historical(_)) => 1 + size_of::<i32>(),
        })
    }
    fn recorded_bytes(&self) -> io::Result<usize> {
        Ok(match self {
            Self::Host => 1,
            Self::Stamped(_) => 1 + size_of::<i32>(),
            Self::Historical(_) => size_of::<i32>(),
        })
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match (replay_rows::rows::historical_stamp(), *self) {
            (Some(hour), Self::Host) | (Some(_), Self::Stamped(hour) | Self::Historical(hour)) => {
                output.extend_from_slice(&hour.to_le_bytes())
            }
            (None, Self::Host) => output.push(0),
            (None, Self::Stamped(hour) | Self::Historical(hour)) => {
                output.push(1);
                output.extend_from_slice(&hour.to_le_bytes());
            }
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.format < replay_rows::rows::compatibility::FUEL_DEMAND_PARTS_FORMAT {
            return input.i32().map(Self::Historical);
        }
        match input.u8()? {
            0 => Ok(Self::Host),
            1 => input.i32().map(Self::Stamped),
            _ => Err(invalid("invalid fuel demand hour tag")),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDemandRead {
    pub hour: FuelDemandHour,
    pub country_instance: Option<u32>,
    pub divisions: Vec<FuelDemandUnit>,
    pub army_reason: Vec<u8>,
    pub groups: Vec<FuelDemandGroup>,
    pub group_reason: Vec<u8>,
    pub fleets: Vec<FuelDemandFleet>,
    pub task_forces: Vec<FuelDemandUnit>,
    pub navy_reason: Vec<u8>,
    pub sites: Vec<FuelDemandSite>,
    pub air_reason: Vec<u8>,
    pub contexts: Vec<FuelDemandContext>,
    pub reason: Vec<u8>,
}
value_struct!(FuelDemandRead { hour: FuelDemandHour, country_instance: Option<u32>,
    divisions: Vec<FuelDemandUnit>, army_reason: Vec<u8>, groups: Vec<FuelDemandGroup>, group_reason: Vec<u8>,
    fleets: Vec<FuelDemandFleet>, task_forces: Vec<FuelDemandUnit>, navy_reason: Vec<u8>,
    sites: Vec<FuelDemandSite>, air_reason: Vec<u8>, contexts: Vec<FuelDemandContext>, reason: Vec<u8> });
opaque_value!(FuelDemandRead);

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum FuelDemandPart {
    Division(FuelDemandUnit),
    Group(FuelDemandGroup),
    Fleet(FuelDemandFleet),
    TaskForce(FuelDemandUnit),
    Site(FuelDemandSite),
    Context(FuelDemandContext),
}

impl Default for FuelDemandPart {
    fn default() -> Self {
        Self::Division(FuelDemandUnit::default())
    }
}

impl FuelDemandPart {
    pub const KINDS: u8 = 6;

    pub fn kind(&self) -> u8 {
        match self {
            Self::Division(_) => 0,
            Self::Group(_) => 1,
            Self::Fleet(_) => 2,
            Self::TaskForce(_) => 3,
            Self::Site(_) => 4,
            Self::Context(_) => 5,
        }
    }

    pub fn ident(&self) -> u64 {
        match self {
            Self::Division(unit) | Self::TaskForce(unit) => unit.reference,
            Self::Group(group) => group.reference,
            Self::Fleet(fleet) => fleet.reference,
            Self::Site(site) => (u64::from(site.list) << 32) | u64::from(site.ordinal),
            Self::Context(context) => u64::from(context.region),
        }
    }
}

impl Value for FuelDemandPart {
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Division(value) | Self::TaskForce(value) => Value::heap_bytes(value),
            Self::Group(value) => Value::heap_bytes(value),
            Self::Fleet(value) => Value::heap_bytes(value),
            Self::Site(value) => Value::heap_bytes(value),
            Self::Context(value) => Value::heap_bytes(value),
        }
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        engine_allocation::add(
            1,
            match self {
                Self::Division(unit) | Self::TaskForce(unit) => Value::encoded_bytes(unit)?,
                Self::Group(group) => Value::encoded_bytes(group)?,
                Self::Fleet(fleet) => Value::encoded_bytes(fleet)?,
                Self::Site(site) => Value::encoded_bytes(site)?,
                Self::Context(context) => Value::encoded_bytes(context)?,
            },
        )
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(match self {
            Self::Division(unit) => Self::Division(Value::try_clone(unit)?),
            Self::Group(group) => Self::Group(Value::try_clone(group)?),
            Self::Fleet(fleet) => Self::Fleet(Value::try_clone(fleet)?),
            Self::TaskForce(unit) => Self::TaskForce(Value::try_clone(unit)?),
            Self::Site(site) => Self::Site(Value::try_clone(site)?),
            Self::Context(context) => Self::Context(Value::try_clone(context)?),
        })
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        output.push(self.kind());
        match self {
            Self::Division(unit) | Self::TaskForce(unit) => Value::encode_reserved(unit, output),
            Self::Group(group) => Value::encode_reserved(group, output),
            Self::Fleet(fleet) => Value::encode_reserved(fleet, output),
            Self::Site(site) => Value::encode_reserved(site, output),
            Self::Context(context) => Value::encode_reserved(context, output),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(match input.u8()? {
            0 => Self::Division(Value::decode(input)?),
            1 => Self::Group(Value::decode(input)?),
            2 => Self::Fleet(Value::decode(input)?),
            3 => Self::TaskForce(Value::decode(input)?),
            4 => Self::Site(Value::decode(input)?),
            5 => Self::Context(Value::decode(input)?),
            _ => return Err(invalid("unknown fuel demand part")),
        })
    }
}
opaque_value!(FuelDemandPart);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDemandPartRow {
    pub country: u32,
    pub kind: u8,
    pub ident: u64,
    pub occurrence: u16,
    pub position: u32,
    pub part: FuelDemandPart,
}
typed_codec!(FuelDemandPartRow, 115,
 |row| (row.country as u128) << 83 | (row.kind as u128) << 80 | (row.ident as u128) << 16 | row.occurrence as u128,
 |key| Self { country: (key >> 83) as u32, kind: ((key >> 80) & 7) as u8, ident: (key >> 16) as u64, occurrence: key as u16, ..Default::default() },
 { 0 => (position): U32, 1 => (part): Opaque },
 |row| { if row.kind >= FuelDemandPart::KINDS || row.part.kind() != row.kind || row.part.ident() != row.ident { return Err(invalid("fuel demand part outside its key")); } Ok(()) });

impl FuelDemandRead {
    pub fn splittable(&self) -> bool {
        fn fits(mut idents: impl ExactSizeIterator<Item = u64>) -> bool {
            if u32::try_from(idents.len().saturating_sub(1)).is_err() {
                return false;
            }
            let mut seen: std::collections::BTreeMap<u64, u32> = std::collections::BTreeMap::new();
            idents.all(|ident| {
                let count = seen.entry(ident).or_insert(0);
                *count += 1;
                *count <= u32::from(u16::MAX) + 1
            })
        }
        fits(self.divisions.iter().map(|unit| unit.reference))
            && fits(self.groups.iter().map(|group| group.reference))
            && fits(self.fleets.iter().map(|fleet| fleet.reference))
            && fits(self.task_forces.iter().map(|unit| unit.reference))
            && fits(
                self.sites
                    .iter()
                    .map(|site| (u64::from(site.list) << 32) | u64::from(site.ordinal)),
            )
            && fits(
                self.contexts
                    .iter()
                    .map(|context| u64::from(context.region)),
            )
    }

    pub fn split(mut self, country: u32) -> io::Result<(Self, Vec<FuelDemandPartRow>)> {
        let lists: [Vec<FuelDemandPart>; FuelDemandPart::KINDS as usize] = [
            std::mem::take(&mut self.divisions)
                .into_iter()
                .map(FuelDemandPart::Division)
                .collect(),
            std::mem::take(&mut self.groups)
                .into_iter()
                .map(FuelDemandPart::Group)
                .collect(),
            std::mem::take(&mut self.fleets)
                .into_iter()
                .map(FuelDemandPart::Fleet)
                .collect(),
            std::mem::take(&mut self.task_forces)
                .into_iter()
                .map(FuelDemandPart::TaskForce)
                .collect(),
            std::mem::take(&mut self.sites)
                .into_iter()
                .map(FuelDemandPart::Site)
                .collect(),
            std::mem::take(&mut self.contexts)
                .into_iter()
                .map(FuelDemandPart::Context)
                .collect(),
        ];
        let mut rows = Vec::new();
        engine_allocation::reserve(&mut rows, lists.iter().map(Vec::len).sum())?;
        for list in lists {
            let mut seen: std::collections::BTreeMap<u64, u32> = std::collections::BTreeMap::new();
            for (position, part) in list.into_iter().enumerate() {
                let ident = part.ident();
                let next = seen.entry(ident).or_insert(0);
                let occurrence = u16::try_from(*next)
                    .map_err(|_| invalid("fuel demand part identity repeats beyond its key"))?;
                *next += 1;
                rows.push(FuelDemandPartRow {
                    country,
                    kind: part.kind(),
                    ident,
                    occurrence,
                    position: u32::try_from(position)
                        .map_err(|_| invalid("fuel demand list too long"))?,
                    part,
                });
            }
        }
        rows.sort_unstable_by_key(RowCodec::key);
        Ok((self, rows))
    }

    fn holds_lists(&self) -> bool {
        !(self.divisions.is_empty()
            && self.groups.is_empty()
            && self.fleets.is_empty()
            && self.task_forces.is_empty()
            && self.sites.is_empty()
            && self.contexts.is_empty())
    }

    fn assemble(&mut self, parts: &[FuelDemandPartRow]) -> Result<(), &'static str> {
        let mut lists: [Vec<(u32, &FuelDemandPart)>; FuelDemandPart::KINDS as usize] =
            Default::default();
        for row in parts {
            lists
                .get_mut(usize::from(row.kind))
                .ok_or("fuel demand part kind unknown")?
                .push((row.position, &row.part));
        }
        for list in &mut lists {
            list.sort_unstable_by_key(|(position, _)| *position);
            if list
                .iter()
                .enumerate()
                .any(|(index, (position, _))| *position as usize != index)
            {
                return Err("fuel demand part positions do not make one list");
            }
        }
        let [divisions, groups, fleets, task_forces, sites, contexts] = lists;
        macro_rules! fill {
            ($list:ident, $field:ident, $variant:ident) => {
                self.$field = $list
                    .into_iter()
                    .map(|(_, part)| match part {
                        FuelDemandPart::$variant(value) => Ok(value.clone()),
                        _ => Err("fuel demand part differs from its list"),
                    })
                    .collect::<Result<_, _>>()?;
            };
        }
        fill!(divisions, divisions, Division);
        fill!(groups, groups, Group);
        fill!(fleets, fleets, Fleet);
        fill!(task_forces, task_forces, TaskForce);
        fill!(sites, sites, Site);
        fill!(contexts, contexts, Context);
        Ok(())
    }

    pub fn wing_rate(&self, wing: &FuelDemandWing) -> Result<i64, &'static str> {
        let inputs = wing
            .inputs
            .as_ref()
            .ok_or("fuel current wing operands unavailable")?;
        let sums = match wing.context {
            Some(index) => {
                let context = self
                    .contexts
                    .get(index as usize)
                    .filter(|context| Some(context.region) == inputs.region)
                    .ok_or("fuel current wing context identity unavailable")?;
                Some(&context.sums)
            }
            None => None,
        };
        game_model::fuel_demand::wing_rate_with_context(inputs, sums)
    }

    pub fn native_rates(&self) -> [Result<i64, &'static str>; 3] {
        if !self.reason.is_empty() {
            return [Err("fuel current demand read identity unavailable"); 3];
        }
        let units = |rows: &[FuelDemandUnit], reason: &[u8]| {
            if !reason.is_empty() {
                return Err("fuel current cached-unit census incomplete");
            }
            rows.iter().try_fold(0_i64, |sum, row| {
                row.rate_raw
                    .map(|value| sum.wrapping_add(value))
                    .ok_or("fuel current cached-unit rate unavailable")
            })
        };
        let air = (|| {
            if !self.air_reason.is_empty() {
                return Err("fuel current air-site census incomplete");
            }
            self.sites.iter().try_fold(0_i64, |sum, site| {
                if !site.reason.is_empty() {
                    return Err("fuel current air-site closure incomplete");
                }
                site.wings.iter().try_fold(sum, |sum, wing| {
                    self.wing_rate(wing).map(|value| sum.wrapping_add(value))
                })
            })
        })();
        [
            units(&self.divisions, &self.army_reason),
            units(&self.task_forces, &self.navy_reason),
            air,
        ]
    }
}

pub fn assemble_demand_parts(
    rows: &mut [CountryFuelRow],
    parts: Result<&[FuelDemandPartRow], &'static str>,
) {
    for row in rows {
        let Recorded::Value(pack) = &mut row.demand else {
            continue;
        };
        if pack.holds_lists() {
            continue;
        }
        let assembled = parts.and_then(|parts| {
            let start = parts.partition_point(|part| part.country < row.country);
            let end = parts.partition_point(|part| part.country <= row.country);
            pack.assemble(&parts[start..end])
        });
        if let Err(reason) = assembled {
            *pack = FuelDemandRead {
                hour: pack.hour,
                country_instance: pack.country_instance,
                reason: if pack.reason.is_empty() {
                    reason.as_bytes().to_vec()
                } else {
                    std::mem::take(&mut pack.reason)
                },
                ..Default::default()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_surrender_metadata_keeps_exact_rates_and_family_refusals_independent() {
        let mut pack = FuelDemandRead {
            country_instance: None,
            divisions: vec![FuelDemandUnit {
                rate_raw: Some(17),
                ..Default::default()
            }],
            task_forces: vec![FuelDemandUnit {
                rate_raw: Some(23),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(pack.native_rates(), [Ok(17), Ok(23), Ok(0)]);
        pack.army_reason = b"synthetic army acquisition failure".to_vec();
        assert!(pack.native_rates()[0].is_err());
        assert_eq!(&pack.native_rates()[1..], &[Ok(23), Ok(0)]);
        pack.reason = b"synthetic country acquisition failure".to_vec();
        assert!(pack.native_rates().iter().all(Result::is_err));
    }
    #[test]
    fn supplying_read_roundtrip_retains_repeated_wing_occurrences_one_context_and_signed_known_values()
     {
        let input = AirInputs {
            reached: true,
            mission_first: 1,
            mission_selected: 1,
            active_count: -1,
            base_fuel: 100_000,
            air_weight: 100_000,
            mission_cost: 100_000,
            period: 1,
            region: Some(7001),
            context_counts: [2; 24],
            context_thresholds: [50_000; 24],
            ..Default::default()
        };
        let mut pack = FuelDemandRead {
            hour: FuelDemandHour::Stamped(77),
            country_instance: Some(9),
            divisions: vec![FuelDemandUnit {
                reference: 1 << 32 | 29,
                rate_raw: Some(i64::MAX),
                name: None,
                name_reason: b"synthetic retained name failure".to_vec(),
                ..Default::default()
            }],
            contexts: vec![FuelDemandContext {
                region: 7001,
                sums: [150_000; 24],
            }],
            ..Default::default()
        };
        for list in 0..2 {
            pack.sites.push(FuelDemandSite {
                list,
                ordinal: 0,
                base: 1 << 32 | 62,
                wings: (0..25)
                    .map(|ordinal| FuelDemandWing {
                        reference: 1 << 32 | 30,
                        owner: Some(7),
                        pool_ordinal: ordinal / 10,
                        wing_ordinal: ordinal % 10,
                        inputs: Some(AirInputs {
                            active_count: if list == 0 && ordinal == 0 { 0 } else { -1 },
                            ..input
                        }),
                        context: Some(0),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            });
        }
        assert_eq!(pack.native_rates(), [Ok(i64::MAX), Ok(0), Ok(-4_900_000)]);
        let row = CountryFuelRow {
            country: 7,
            country_instance: Recorded::Value(Some(9)),
            demand: Recorded::Value(pack),
            ..Default::default()
        };
        let restored = row
            .delta(None)
            .unwrap()
            .unwrap()
            .apply::<CountryFuelRow>(None)
            .unwrap();
        assert_eq!(restored, row);
        let mut changed = restored.clone();
        let Recorded::Value(pack) = &mut changed.demand else {
            panic!()
        };
        pack.sites[1].wings[24].context = Some(1);
        assert_eq!(pack.native_rates()[0], Ok(i64::MAX));
        assert!(pack.native_rates()[2].is_err());
        assert_eq!(
            changed
                .delta(Some(&restored))
                .unwrap()
                .unwrap()
                .apply(Some(restored))
                .unwrap(),
            changed
        );
    }
}

replay_rows::wire_value!(
    FuelDemandHour,
);

use replay_rows::checkpoint::values::CheckpointValue;

impl CheckpointValue for FuelDemandPart {
    const MINIMUM_BYTES: usize = 1;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        Value::encode(&self.kind(), output)?;
        match self {
            Self::Division(value) | Self::TaskForce(value) => {
                CheckpointValue::encode(value, output)
            }
            Self::Group(value) => CheckpointValue::encode(value, output),
            Self::Fleet(value) => CheckpointValue::encode(value, output),
            Self::Site(value) => CheckpointValue::encode(value, output),
            Self::Context(value) => CheckpointValue::encode(value, output),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(match input.u8()? {
            0 => Self::Division(CheckpointValue::decode(input)?),
            1 => Self::Group(CheckpointValue::decode(input)?),
            2 => Self::Fleet(CheckpointValue::decode(input)?),
            3 => Self::TaskForce(CheckpointValue::decode(input)?),
            4 => Self::Site(CheckpointValue::decode(input)?),
            5 => Self::Context(CheckpointValue::decode(input)?),
            _ => return Err(invalid("invalid checkpoint fuel demand part")),
        })
    }
}
