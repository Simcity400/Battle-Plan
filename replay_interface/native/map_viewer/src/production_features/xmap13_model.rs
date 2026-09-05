use crate::Result;
use replay_core::Leaf;

pub(super) fn country_entity(
    suffix: &str,
    prefixes: [&Leaf<String>; 3],
    exists: &mut impl FnMut(&str) -> Result<bool>,
) -> Result<Option<String>> {
    for (field, prefix) in ["cosmetic_tag", "tag", "graphical_culture"]
        .into_iter()
        .zip(prefixes)
    {
        let prefix = match prefix {
            Leaf::Measured(prefix) => prefix,
            Leaf::Absent => return Err(format!("XMAP13 model selector {field} is absent").into()),
            Leaf::Unobtained => {
                return Err(format!("XMAP13 model selector {field} is unavailable").into())
            }
        };
        let candidate = format!("{prefix}_{suffix}");
        if exists(&candidate)? {
            return Ok(Some(candidate));
        }
    }
    Ok(exists(suffix)?.then(|| suffix.to_owned()))
}

pub(super) fn subunit_entity(
    sprite: &str,
    fallback_sprite: &str,
    variant: u8,
    prefixes: [&Leaf<String>; 3],
    exists: &mut impl FnMut(&str) -> Result<bool>,
) -> Result<Option<String>> {
    for suffix in [
        format!("{sprite}_{}_entity", variant as i8),
        format!("{sprite}_entity"),
        format!("{fallback_sprite}_entity"),
    ] {
        if let Some(entity) = country_entity(&suffix, prefixes, exists)? {
            return Ok(Some(entity));
        }
    }
    Ok(None)
}

pub(super) fn equipment_visual_level(rows: &[(i32, i64)]) -> Result<Option<i32>> {
    if rows.is_empty() {
        return Ok(None);
    }
    let mut amount = 0i64;
    let mut weighted = 0i64;
    for &(level, count) in rows {
        if count < 0 {
            return Err("XMAP13 equipment model amount is negative".into());
        }
        amount = amount
            .checked_add(count)
            .ok_or("XMAP13 equipment model amount overflow")?;
        weighted = weighted
            .checked_add(
                i64::from(level)
                    .checked_mul(count)
                    .ok_or("XMAP13 equipment model weighted level overflow")?,
            )
            .ok_or("XMAP13 equipment model weighted sum overflow")?;
    }
    if amount == 0 {
        return Ok(Some(0));
    }
    let fixed = weighted
        .checked_mul(100_000)
        .ok_or("XMAP13 equipment model fixed-point overflow")?
        / amount;
    let rounded = fixed
        .checked_add(if fixed < 0 { -50_000 } else { 50_000 })
        .ok_or("XMAP13 equipment model rounding overflow")?
        / 100_000;
    Ok(Some(i32::try_from(rounded)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equipment_visual_level_uses_quantities_and_fixed_point_half_away_from_zero() {
        assert_eq!(equipment_visual_level(&[(0, 99), (3, 1)]).unwrap(), Some(0));
        assert_eq!(equipment_visual_level(&[(0, 1), (3, 1)]).unwrap(), Some(2));
        assert_eq!(
            equipment_visual_level(&[(-3, 1), (0, 1)]).unwrap(),
            Some(-2)
        );
        assert_eq!(equipment_visual_level(&[]).unwrap(), None);
        assert_eq!(equipment_visual_level(&[(3, 0)]).unwrap(), Some(0));
        assert!(equipment_visual_level(&[(1, -1)]).is_err());
        assert!(equipment_visual_level(&[(i32::MAX, i64::MAX)]).is_err());
    }

    #[test]
    fn numbered_generic_precedes_unnumbered_cosmetic_model() {
        let prefixes = [
            Leaf::Measured("GER_custom".into()),
            Leaf::Measured("GER".into()),
            Leaf::Measured("western".into()),
        ];
        let mut asked = Vec::new();
        let selected = subunit_entity(
            "infantry",
            "fallback",
            2,
            prefixes.each_ref(),
            &mut |name| {
                asked.push(name.to_owned());
                Ok(matches!(
                    name,
                    "infantry_2_entity" | "GER_custom_infantry_entity"
                ))
            },
        )
        .unwrap();
        assert_eq!(selected.as_deref(), Some("infantry_2_entity"));
        assert_eq!(
            asked,
            [
                "GER_custom_infantry_2_entity",
                "GER_infantry_2_entity",
                "western_infantry_2_entity",
                "infantry_2_entity",
            ]
        );
    }

    #[test]
    fn model_prefix_reads_short_circuit_without_erasing_unknowns() {
        for unknown in [Leaf::Absent, Leaf::Unobtained] {
            let cosmetic = Leaf::Measured("custom".into());
            let prefixes = [&cosmetic, &unknown, &unknown];
            assert_eq!(
                country_entity("unit_entity", prefixes, &mut |name| Ok(
                    name == "custom_unit_entity"
                ))
                .unwrap()
                .as_deref(),
                Some("custom_unit_entity"),
            );
            let mut asked = Vec::new();
            assert!(country_entity("unit_entity", prefixes, &mut |name| {
                asked.push(name.to_owned());
                Ok(false)
            })
            .is_err());
            assert_eq!(asked, ["custom_unit_entity"]);
        }
    }

    #[test]
    fn missing_assets_exhaust_every_engine_candidate_and_preserve_signed_variant() {
        let prefixes = [
            Leaf::Measured(String::new()),
            Leaf::Measured("TAG".into()),
            Leaf::Measured("culture".into()),
        ];
        let mut asked = Vec::new();
        assert_eq!(
            subunit_entity(
                "primary",
                "fallback",
                255,
                prefixes.each_ref(),
                &mut |name| {
                    asked.push(name.to_owned());
                    Ok(false)
                }
            )
            .unwrap(),
            None
        );
        assert_eq!(
            asked,
            [
                "_primary_-1_entity",
                "TAG_primary_-1_entity",
                "culture_primary_-1_entity",
                "primary_-1_entity",
                "_primary_entity",
                "TAG_primary_entity",
                "culture_primary_entity",
                "primary_entity",
                "_fallback_entity",
                "TAG_fallback_entity",
                "culture_fallback_entity",
                "fallback_entity",
            ]
        );
    }
}
