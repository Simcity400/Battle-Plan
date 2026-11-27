use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::collections::BTreeMap;
use viewer_unit_reads::air_detection::{Countries, CountryModifier};

#[derive(Clone, Copy)]
pub struct Defines {
    average: i32,
    bases: i64,
    ratio: i64,
}
impl Defines {
    pub fn read(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        Ok(Self {
            average: defines.int32("NAir", "AIR_WING_AVERAGE_SIZE")?,
            bases: defines.fixed5("NCountry", "AIR_VOLUNTEER_BASES_CAPACITY_LIMIT")?,
            ratio: defines.fixed5("NCountry", "AIR_VOLUNTEER_PLANES_RATIO")?,
        })
    }
    fn allowance(
        self,
        base_capacity: i32,
        aircraft: i32,
        country: i64,
        relation: i64,
    ) -> Result<i32, String> {
        if self.average == 0
            || (self.average == -1 && (base_capacity == i32::MIN || aircraft == i32::MIN))
        {
            return Err("Air volunteer average wing size makes native division invalid".into());
        }
        let modifiers = ((country / 100_000) as i32).wrapping_add((relation / 100_000) as i32);
        let bases = ((i64::from(base_capacity / self.average)
            .wrapping_mul(100_000)
            .wrapping_mul(self.bases))
            / 10_000_000_000) as i32;
        let whole = i64::from(aircraft / self.average);
        let integral_ratio = i64::from((self.ratio / 100_000) as i32);
        let guard = integral_ratio.wrapping_mul(whole);
        let same_sign = (whole < 0) == (integral_ratio < 0);
        const PRODUCT_LIMIT: i64 = 0x36f9bfb2;
        const SATURATED_FIXED: i64 = 0x53e2d620fb40;
        let raw = if same_sign && !(0..=PRODUCT_LIMIT).contains(&guard) {
            SATURATED_FIXED
        } else if !same_sign && !(-PRODUCT_LIMIT..=0).contains(&guard) {
            -SATURATED_FIXED
        } else {
            whole.wrapping_mul(100_000).wrapping_mul(self.ratio) / 100_000
        };
        let planes = (raw.wrapping_add(if raw < 0 { -50_000 } else { 50_000 }) / 100_000) as i32;
        let capacity = modifiers.wrapping_add(bases);
        let limit = modifiers.wrapping_add(planes);
        Ok(capacity.min(limit))
    }
}

pub struct Allowances {
    values: BTreeMap<u32, Result<(i32, i32), String>>,
}
impl Allowances {
    pub fn read(
        frame: &DecodedFrame,
        ours: u32,
        defines: &Result<Defines, String>,
    ) -> Result<Self, String> {
        let links: BTreeMap<_, _> = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let hosts = frame
            .complete::<tables::VolunteerHosts>()
            .map_err(|e| e.to_string())?;
        let mut demanded = BTreeMap::new();
        let mut own: Vec<_> = hosts
            .iter()
            .filter(|host| links.get(&host.country) == Some(&ours))
            .collect();
        own.sort_by_key(|row| row.position);
        for host in own {
            let slot = if host.host == 0 {
                0
            } else {
                *links
                    .get(&host.host)
                    .ok_or("Air volunteer host country is unresolved")?
            };
            demanded.entry(slot).or_insert(host);
        }
        if demanded.is_empty() {
            return Ok(Self {
                values: BTreeMap::new(),
            });
        }
        let inputs = (|| {
            let defines = *defines.as_ref().map_err(Clone::clone)?;
            let country =
                Countries::read(frame)?.value(ours, CountryModifier::VolunteerCapacity)?;
            let stockpile = frame
                .complete::<tables::RaidCountryInputs>()
                .map_err(|e| e.to_string())?
                .iter()
                .find(|row| row.canonical_slot == ours)
                .map(|row| {
                    row.volunteer_stockpile_planes
                        .require("Air volunteer stockpile")
                        .copied()
                })
                .transpose()?
                .flatten()
                .ok_or("Air volunteer uncapped stockpile aircraft total unavailable")?;
            let mut planes = stockpile;
            for wing in frame
                .complete::<tables::AirWings>()
                .map_err(|e| e.to_string())?
            {
                if links.get(&wing.owner) == Some(&ours) {
                    planes = planes.wrapping_add(wing.planes);
                }
            }
            let bases: BTreeMap<_, _> = frame
                .complete::<tables::AirBases>()
                .map_err(|e| e.to_string())?
                .iter()
                .map(|base| (base.id, base.capacity))
                .collect();
            let mut capacity: BTreeMap<u32, i32> = demanded.keys().map(|&slot| (slot, 0)).collect();
            for site in frame
                .complete::<tables::RaidAirSites>()
                .map_err(|e| e.to_string())?
            {
                if site.list == 0
                    && let Some(total) = capacity.get_mut(&site.canonical_slot)
                {
                    *total = total.wrapping_add(
                        *bases
                            .get(&site.base)
                            .ok_or("Air volunteer host site is unresolved")?,
                    );
                }
            }
            Ok::<_, String>((defines, country, planes, capacity))
        })();
        let values = demanded
            .into_iter()
            .map(|(slot, host)| {
                let value = inputs.as_ref().map_err(Clone::clone).and_then(
                    |(defines, country, planes, capacities)| {
                        let relation = host
                            .air_capacity_modifier
                            .require("Air volunteer directed relation modifier")?
                            .ok_or("Air volunteer directed relation modifier unavailable")?;
                        let allowance =
                            defines.allowance(capacities[&slot], *planes, *country, relation)?;
                        Ok((host.deployed_divisions, allowance))
                    },
                );
                (slot, value)
            })
            .collect();
        Ok(Self { values })
    }
    pub fn host(&self, slot: u32) -> Result<Option<(i32, i32)>, String> {
        self.values
            .get(&slot)
            .cloned()
            .transpose()
            .map(|value| value.filter(|(_, allowed)| *allowed > 0))
    }
}

#[cfg(test)]
mod tests {
    use super::Defines;
    #[test]
    fn allowance_caps_only_base_term_and_truncates_modifiers_separately() {
        let defines = Defines {
            average: 80,
            bases: 200_000,
            ratio: 50_000,
        };
        assert_eq!(defines.allowance(1000, 320, 199_999, 199_999), Ok(4));
        assert_eq!(defines.allowance(100, 10000, -99_999, -99_999), Ok(2));
        assert_eq!(defines.allowance(0, 0, 200_000, 300_000), Ok(5));
    }
    #[test]
    fn allowance_retains_half_away_rounding_signed_counts_and_native_guard() {
        let defines = Defines {
            average: 80,
            bases: 100_000_000,
            ratio: 50_000,
        };
        assert_eq!(defines.allowance(1000, 80, 0, 0), Ok(1));
        assert_eq!(defines.allowance(1000, -80, 0, 0), Ok(-1));
        assert!(
            Defines {
                average: 0,
                ..defines
            }
            .allowance(1, 1, 0, 0)
            .is_err()
        );
        assert_eq!(
            Defines {
                ratio: 100_000_000_000,
                ..defines
            }
            .allowance(10 * defines.average, i32::MAX, 0, 0)
            .unwrap(),
            10000
        );
    }
}
