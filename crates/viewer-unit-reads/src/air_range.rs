
use replay_data::rows::{
    AceRangeInputRow, AirWingRow, CountryCommandPowerRow, RaidCountryInputRow,
};
use std::{cell::OnceCell, collections::BTreeMap};

#[derive(Clone, Copy)]
pub struct Defines {
    pub map_scale: i64,
    pub ace_wing_size: i64,
    pub ace_wing_size_max_bonus: i64,
}

pub trait Sources {
    fn slot(&self, raw: u32) -> Option<u32>;
    fn country(&self, slot: u32) -> Result<&RaidCountryInputRow, String>;
    fn command_power(&self, slot: u32) -> Result<&CountryCommandPowerRow, String>;
    fn wing(&self, handle: u64) -> Result<Option<&AirWingRow>, String>;
    fn ace(&self, handle: u64) -> Result<Option<&AceRangeInputRow>, String>;
}

pub struct FrameSources<'a> {
    frame: &'a replay_frame::frame::DecodedFrame,
    countries: OnceCell<Result<BTreeMap<u32, &'a RaidCountryInputRow>, String>>,
    command_power: OnceCell<Result<BTreeMap<u32, &'a CountryCommandPowerRow>, String>>,
    #[expect(clippy::type_complexity, reason = "each ace's own read or refusal, cached once and refused as a whole when unread")]
    aces: OnceCell<Result<BTreeMap<u64, Result<&'a AceRangeInputRow, String>>, String>>,
}

impl<'a> FrameSources<'a> {
    pub fn new(frame: &'a replay_frame::frame::DecodedFrame) -> Self {
        Self {
            frame,
            countries: OnceCell::new(),
            command_power: OnceCell::new(),
            aces: OnceCell::new(),
        }
    }
}

impl Sources for FrameSources<'_> {
    fn slot(&self, raw: u32) -> Option<u32> {
        let rows = self
            .frame
            .complete::<replay_data::rows::tables::CountryLinks>()
            .ok()?;
        rows.binary_search_by_key(&raw, |row| row.id)
            .ok()
            .map(|at| rows[at].canonical_slot)
    }
    fn country(&self, slot: u32) -> Result<&RaidCountryInputRow, String> {
        self.countries
            .get_or_init(|| {
                self.frame
                    .complete::<replay_data::rows::tables::RaidCountryInputs>()
                    .map(|rows| rows.iter().map(|row| (row.canonical_slot, row)).collect())
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)?
            .get(&slot)
            .copied()
            .ok_or_else(|| format!("country slot {slot} has no RaidCountryInputs row"))
    }
    fn command_power(&self, slot: u32) -> Result<&CountryCommandPowerRow, String> {
        self.command_power
            .get_or_init(|| {
                self.frame
                    .complete::<replay_data::rows::tables::CountryCommandPower>()
                    .map(|rows| rows.iter().map(|row| (row.canonical_slot, row)).collect())
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)?
            .get(&slot)
            .copied()
            .ok_or_else(|| format!("country slot {slot} has no CountryCommandPower row"))
    }
    fn wing(&self, handle: u64) -> Result<Option<&AirWingRow>, String> {
        let rows = self
            .frame
            .complete::<replay_data::rows::tables::AirWings>()
            .map_err(|e| e.to_string())?;
        Ok(rows
            .binary_search_by_key(&handle, |row| row.id)
            .ok()
            .map(|at| &rows[at]))
    }
    fn ace(&self, handle: u64) -> Result<Option<&AceRangeInputRow>, String> {
        let rows = self
            .aces
            .get_or_init(|| {
                let rows = self
                    .frame
                    .complete::<replay_data::rows::tables::AceRangeInputs>()
                    .map_err(|error| error.to_string())?;
                let mut aces = BTreeMap::<u64, Result<&AceRangeInputRow, String>>::new();
                for row in rows {
                    match aces.entry(row.ace) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(Ok(row));
                        }
                        std::collections::btree_map::Entry::Occupied(mut entry) => {
                            if entry.get().as_ref().is_ok_and(|previous| *previous != row) {
                                let _ = entry.insert(Err(format!(
                                    "ace handle {:#x} has conflicting recorded owners or operands",
                                    row.ace
                                )));
                            }
                        }
                    }
                }
                Ok(aces)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        rows.get(&handle)
            .map(|row| row.as_ref().copied().map_err(Clone::clone))
            .transpose()
    }
}

pub fn wing_range(
    wing: &AirWingRow,
    defines: Defines,
    sources: &impl Sources,
) -> Result<i64, String> {
    let base = to_map(wing.base_range_raw, defines.map_scale);
    let raw = if wing.country_override > 0 {
        wing.country_override as u32
    } else {
        wing.owner
    };
    let slot = sources
        .slot(raw)
        .ok_or("a wing country reference does not resolve")?;
    let country = sources.country(slot)?;
    let mut factor = 100_000_i64;
    if wing.ace != 0 {
        let ace = sources
            .ace(wing.ace)?
            .ok_or("a wing's ace has no AceRangeInputs row")?;
        if ace.alive != 0 {
            let owner = sources.command_power(ace.canonical_slot)?;
            let scaled = ace.range.wrapping_mul(ace_scale(ace, defines, sources)?) / 100_000;
            let scaled = registered(owner.ace_effectiveness_factor, "ace_effectiveness_factor")?
                .wrapping_add(100_000)
                .wrapping_mul(scaled)
                / 100_000;
            factor = scaled.wrapping_add(100_000);
            if ace.type_range != 0 {
                factor = factor.wrapping_add(registered(
                    sources.command_power(slot)?.air_ace_bonuses_factor,
                    "air_ace_bonuses_factor",
                )?);
            }
        }
    }
    let air = registered(country.air_range_factor, "air_range_factor")?;
    Ok(base.wrapping_mul(factor.wrapping_add(air)) / 100_000)
}

pub fn ace_scale(
    ace: &AceRangeInputRow,
    defines: Defines,
    sources: &impl Sources,
) -> Result<i64, String> {
    if ace.wing == 0 {
        return Ok(100_000);
    }
    let Some(wing) = sources.wing(ace.wing)? else {
        return Ok(100_000);
    };
    let planes = i64::from(wing.planes);
    if planes == 0 {
        return Ok(100_000);
    }
    Ok(if finite(defines.ace_wing_size) {
        (defines.ace_wing_size.wrapping_mul(100_000) / planes.wrapping_mul(100_000))
            .min(defines.ace_wing_size_max_bonus)
    } else {
        defines.ace_wing_size_max_bonus.min(0xffff_ffff)
    })
}

pub fn finite(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

pub fn to_map(value: i64, scale: i64) -> i64 {
    if scale == 0 || !finite(value) {
        0xffff_ffff
    } else {
        value.wrapping_mul(100_000) / scale
    }
}

pub fn registered(value: Option<i64>, name: &str) -> Result<i64, String> {
    value.ok_or_else(|| format!("{name} is not a registered modifier"))
}

#[cfg(test)]
mod tests;
