use super::*;
use engine::fuel_demand::{AirInputs, Modifier};

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
pub struct FuelDemandUnit {
    pub reference: u64,
    pub parent: u64,
    pub ordinal: u32,
    /// Native roster, expeditionary and logical owner words, independently of
    /// the country whose tooltip traversed this occurrence.
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
pub struct FuelDemandFleet {
    pub reference: u64,
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
}
value_struct!(FuelDemandFleet { reference: u64, name: Option<Vec<u8>>, name_reason: Vec<u8> });

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FuelDemandWing {
    pub reference: u64,
    pub owner: Option<u32>,
    /// 140F4A700 selects a signed-positive override, otherwise the pool owner.
    pub country_override: Option<i32>,
    pub owner_reason: Vec<u8>,
    pub pool_ordinal: u32,
    pub wing_ordinal: u32,
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
    pub inputs: Option<AirInputs>,
    /// Index of an owned same-read context, not a later region lookup.
    pub context: Option<u32>,
    pub reason: Vec<u8>,
}
value_struct!(FuelDemandWing { reference: u64, owner: Option<u32>, country_override: Option<i32>, owner_reason: Vec<u8>, pool_ordinal: u32,
    wing_ordinal: u32, name: Option<Vec<u8>>, name_reason: Vec<u8>, inputs: Option<AirInputs>,
    context: Option<u32>, reason: Vec<u8> });

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
pub struct FuelDemandContext {
    pub region: u32,
    pub sums: [i64; 24],
}
value_struct!(FuelDemandContext {
    region: u32,
    sums: [i64; 24]
});

/// Supplying-read current required-use rates at fixed5. These are independent
/// of FuelAwardWindow's completed updates. Names are stored game-language
/// bytes from this boundary; consumers retain the shared language guard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FuelDemandRead {
    pub hour: i32,
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
value_struct!(FuelDemandRead { hour: i32, country_instance: Option<u32>,
    divisions: Vec<FuelDemandUnit>, army_reason: Vec<u8>, groups: Vec<FuelDemandGroup>, group_reason: Vec<u8>,
    fleets: Vec<FuelDemandFleet>, task_forces: Vec<FuelDemandUnit>, navy_reason: Vec<u8>,
    sites: Vec<FuelDemandSite>, air_reason: Vec<u8>, contexts: Vec<FuelDemandContext>, reason: Vec<u8> });
opaque_value!(FuelDemandRead);

impl FuelDemandRead {
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
        engine::fuel_demand::wing_rate_with_context(inputs, sums)
    }

    /// Native cached army/naval sums and recomputed air sum. A family's failure
    /// leaves other families and known contributor rows available. Do not use
    /// this wrapping aggregate as an exact widened reporting sum.
    pub fn native_rates(&self) -> [Result<i64, &'static str>; 3] {
        if !self.reason.is_empty() || self.country_instance.is_none() {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supplying_read_roundtrip_retains_5000_repeated_wing_occurrences_one_context_and_signed_known_values()
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
            hour: 77,
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
                wings: (0..2500)
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
        assert_eq!(pack.native_rates(), [Ok(i64::MAX), Ok(0), Ok(-499_900_000)]);
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
        pack.sites[1].wings[2499].context = Some(1);
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
