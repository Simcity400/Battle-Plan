use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PatrolMissionFields {
    pub target: Option<u64>,
    pub progress: Option<Decimal>,
    pub chance: Option<Decimal>,
}

pub type PatrolMissionSave = BTreeMap<u64, PatrolMissionFields>;

pub(crate) fn patrol_inputs(
    countries: super::super::CountriesView<'_>,
    keys: [&[u8]; 3],
) -> io::Result<PatrolMissionSave> {
    if keys.iter().any(|key| key.is_empty())
        || keys
            .iter()
            .enumerate()
            .any(|(index, key)| keys[..index].contains(key))
    {
        return Err(invalid("patrol mission save keys unavailable or aliased"));
    }
    let mut result = BTreeMap::new();
    for country in countries.iter() {
        let Some(units) = field(block(&country.value)?, b"units")? else {
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
                let mission = block(
                    field(fields, b"mission")?
                        .ok_or_else(|| invalid("saved task force lacks mission"))?,
                )?;
                let target = field(mission, keys[0])?.map(typed_handle).transpose()?;
                if target == Some(0) {
                    return Err(invalid("saved patrol target explicitly null"));
                }
                let decimal = |key| -> io::Result<Option<Decimal>> {
                    let value = field(mission, key)?
                        .map(|value| Decimal::parse(scalar(value)?))
                        .transpose()?;
                    if value.as_ref().is_some_and(|value| value.raw == 0) {
                        return Err(invalid("saved patrol word explicitly zero"));
                    }
                    Ok(value)
                };
                let row = PatrolMissionFields {
                    target,
                    progress: decimal(keys[1])?,
                    chance: decimal(keys[2])?,
                };
                if result.insert(id, row).is_some() {
                    return Err(invalid(
                        "duplicate full task force in patrol mission save view",
                    ));
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::{ActualCountry, CountryEmissionIdentity, SaveDocument};
    const KEYS: [&[u8]; 3] = [b"fixture_target", b"fixture_progress", b"fixture_chance"];
    fn document(countries: &str) -> SaveDocument {
        SaveDocument::read(
            format!("HOI4txt date=1936.1.1.1 game_unique_id=c version=b countries={{{countries}}}")
                .as_bytes(),
        )
        .unwrap()
    }
    fn units(words: &str) -> String {
        [
            "units={fleet={id={id=1 type=61} task_force={id={id=2 type=61} mission={",
            words,
            "}}}}",
        ]
        .concat()
    }
    #[test]
    fn signed_words_and_full_target_generation_survive_independent_omission() {
        for (words, target, progress, chance) in [
            (
                "fixture_target={id=91 type=61} fixture_progress=-0.00001 fixture_chance=12.34567",
                Some((91u64 << 32) | 61),
                Some(-1),
                Some(1_234_567),
            ),
            (
                "fixture_target={id=92 type=61} fixture_chance=-3",
                Some((92u64 << 32) | 61),
                None,
                Some(-300_000),
            ),
            ("fixture_progress=3", None, Some(300_000), None),
            ("", None, None, None),
        ] {
            let parsed = document(&format!("X={{{}}}", units(words)))
                .mission_patrol_inputs(KEYS)
                .unwrap();
            let row = &parsed[&((2u64 << 32) | 61)];
            assert_eq!(row.target, target);
            assert_eq!(row.progress.as_ref().map(|value| value.raw), progress);
            assert_eq!(row.chance.as_ref().map(|value| value.raw), chance);
        }
    }
    #[test]
    fn malformed_or_aliased_words_are_refused_without_inventing_zero() {
        for words in [
            "fixture_progress=0",
            "fixture_chance=0",
            "fixture_progress=1 fixture_progress=2",
            "fixture_target={id=0 type=0}",
            "fixture_target={id=1}",
            "fixture_chance=bad",
        ] {
            assert!(
                document(&format!("X={{{}}}", units(words)))
                    .mission_patrol_inputs(KEYS)
                    .is_err(),
                "{words}"
            );
        }
        let doc = document("X={}");
        assert!(doc.mission_patrol_inputs([b"", b"p", b"c"]).is_err());
        assert!(doc.mission_patrol_inputs([b"t", b"p", b"t"]).is_err());
    }
    #[test]
    fn full_task_duplicates_require_authenticated_country_projection() {
        let task = units("fixture_progress=-2");
        let doc = document(&format!("X={{{task}}} X={{{task}}}"));
        assert!(doc.mission_patrol_inputs(KEYS).is_err());
        let layout = [1, 2].map(|physical_slot| CountryEmissionIdentity {
            physical_slot,
            own_id: 1,
            actual: ActualCountry::Canonical(1),
            tag: b"X".to_vec(),
        });
        assert_eq!(
            doc.project(&layout)
                .unwrap()
                .mission_patrol_inputs(KEYS)
                .unwrap()
                .len(),
            1
        );
        assert!(
            document(&format!("X={{{task}}} Y={{{task}}}"))
                .mission_patrol_inputs(KEYS)
                .is_err()
        );
    }
}
