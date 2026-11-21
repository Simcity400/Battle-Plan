use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidingGridFields {
    pub spotting: Option<Decimal>,
    pub region: Option<u32>,
}
pub type RaidingGridSave = BTreeMap<u64, RaidingGridFields>;

pub(crate) fn raiding_grid(
    countries: super::super::CountriesView<'_>,
    keys: [&[u8]; 2],
) -> io::Result<RaidingGridSave> {
    if keys.iter().any(|key| key.is_empty()) || keys[0] == keys[1] {
        return Err(invalid("raiding grid save keys unavailable or aliased"));
    }
    let mut rows = BTreeMap::new();
    for country in countries.iter() {
        let country_fields = block(&country.value)?;
        let Some(units) = field(country_fields, b"units")? else {
            continue;
        };
        for fleet in block(units)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"fleet"))
        {
            for task in block(&fleet.value)?
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"task_force"))
            {
                let fields = block(&task.value)?;
                let id = object_id(fields)?;
                let mission = field(fields, b"mission")?
                    .ok_or_else(|| invalid("saved task force lacks mission"))?;
                let mission = block(mission)?;
                let spotting = field(mission, keys[0])?
                    .map(|value| Decimal::parse(scalar(value)?))
                    .transpose()?;
                if spotting.as_ref().is_some_and(|value| value.raw == 0) {
                    return Err(invalid("saved retained raiding spotting explicitly zero"));
                }
                let region = field(mission, keys[1])?
                    .map(|value| integer(scalar(value)?))
                    .transpose()?;
                if rows
                    .insert(id, RaidingGridFields { spotting, region })
                    .is_some()
                {
                    return Err(invalid(
                        "duplicate full task force in raiding grid save view",
                    ));
                }
            }
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use crate::SaveDocument;
    fn read(fields: &str) -> io::Result<super::RaidingGridSave> {
        SaveDocument::read(format!("HOI4txt date=1936.1.1.1 game_unique_id=c version=b countries={{X={{units={{fleet={{id={{id=1 type=61}} task_force={{id={{id=2 type=61}} mission={{{fields}}} }} }} }} }} }}").as_bytes()).unwrap()
            .mission_raiding_grid([b"fixture_spotting", b"fixture_region"])
    }
    use std::io;
    #[test]
    fn raiding_grid_preserves_nonzero_signed_spotting_and_independent_region_omission() {
        let id = (2_u64 << 32) | 61;
        for (fields, spotting, region) in [
            ("", None, None),
            (
                "fixture_spotting=-0.125 fixture_region=17",
                Some(-12500),
                Some(17),
            ),
            ("fixture_spotting=0.00001", Some(1), None),
            ("fixture_region=0", None, Some(0)),
        ] {
            let rows = read(fields).unwrap();
            assert_eq!(rows[&id].spotting.as_ref().map(|value| value.raw), spotting);
            assert_eq!(rows[&id].region, region);
        }
        assert!(read("fixture_spotting=0").is_err());
        assert!(read("fixture_region=1 fixture_region=2").is_err());
    }
    #[test]
    fn retained_grid_uses_the_authenticated_whole_country_projection() {
        let units = "units={fleet={id={id=1 type=61} task_force={id={id=2 type=61} mission={fixture_spotting=-2 fixture_region=7}}}}";
        let document = SaveDocument::read(format!("HOI4txt date=1936.1.1.1 game_unique_id=c version=b countries={{X={{{units}}} X={{{units}}}}}").as_bytes()).unwrap();
        let keys = [b"fixture_spotting".as_slice(), b"fixture_region".as_slice()];
        assert!(document.mission_raiding_grid(keys).is_err());
        let layout = [1, 2].map(|physical_slot| crate::CountryEmissionIdentity {
            physical_slot,
            own_id: 1,
            actual: crate::ActualCountry::Canonical(1),
            tag: b"X".to_vec(),
        });
        let rows = document
            .project(&layout)
            .unwrap()
            .mission_raiding_grid(keys)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[&((2_u64 << 32) | 61)].spotting.as_ref().unwrap().raw,
            -200_000
        );
    }
}
