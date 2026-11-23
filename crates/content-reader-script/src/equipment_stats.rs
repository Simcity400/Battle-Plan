pub fn category_mask(name: impl AsRef<[u8]>) -> Option<u64> {
    let name = name.as_ref();
    engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.category_names))
        .iter()
        .find(|(key, _)| name.eq_ignore_ascii_case(key.as_bytes()))
        .map(|(_, mask)| *mask)
}

pub fn category_names(mask: u64) -> Result<Vec<&'static str>, u64> {
    let mut unknown = mask;
    let mut names = Vec::new();
    for &(name, bit) in engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.category_names)) {
        if bit != 0 && mask & bit != 0 {
            names.push(name);
            unknown &= !bit;
        }
    }
    if unknown == 0 {
        Ok(names)
    } else {
        Err(unknown)
    }
}

pub const STAT_COUNT: usize = 78;

pub fn stat_metadata() -> &'static [(&'static str, &'static str, bool)] {
    use engine_executable::executable_tables::{auxiliary_part, required};
    let stat_metadata = required(auxiliary_part(|tables| &tables.stat_metadata));
    stat_metadata
}

pub fn stat_index(keyword: &[u8]) -> Option<usize> {
    stat_metadata()
        .iter()
        .position(|(name, _, _)| name.as_bytes() == keyword)
}


pub const RELIABILITY_STAT: usize = 65;
pub const RELIABILITY_FACTOR_STAT: usize = 66;
pub const CARRIER_SIZE_STAT: usize = 72;
pub const BUILD_COST_IC_STAT: usize = 77;
pub const FORBIDDEN_RELIABILITY_SCALED_STATS: [usize; 4] = [
    RELIABILITY_STAT,
    RELIABILITY_FACTOR_STAT,
    BUILD_COST_IC_STAT,
    CARRIER_SIZE_STAT,
];
