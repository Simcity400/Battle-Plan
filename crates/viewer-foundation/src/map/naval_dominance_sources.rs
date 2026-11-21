use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RegionalDominanceSources {
    pub patrol: i64,
    pub strike_multiplier: i64,
    pub enemy_raiding: i64,
}
impl Default for RegionalDominanceSources {
    fn default() -> Self {
        Self {
            patrol: 0,
            strike_multiplier: 100_000,
            enemy_raiding: 0,
        }
    }
}

pub type RegionalDominanceSourcesMap =
    Result<BTreeMap<u32, Result<RegionalDominanceSources, String>>, String>;

pub fn region(
    map: &RegionalDominanceSourcesMap,
    region: u32,
) -> Result<RegionalDominanceSources, String> {
    map.as_ref()
        .map_err(Clone::clone)?
        .get(&region)
        .cloned()
        .unwrap_or_else(|| Ok(RegionalDominanceSources::default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_omission_is_zero_but_unavailable_coverage_never_is() {
        assert_eq!(
            region(&Ok(BTreeMap::new()), 17),
            Ok(RegionalDominanceSources::default())
        );
        assert_eq!(
            region(&Err("ledger unavailable".into()), 17),
            Err("ledger unavailable".into())
        );
        assert_eq!(
            region(&Ok([(17, Err("ship design unknown".into()))].into()), 17),
            Err("ship design unknown".into())
        );
    }
}
