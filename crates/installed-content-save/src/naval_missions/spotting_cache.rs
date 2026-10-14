//! Mission spotting caches are emitted only when positive.
//! The caller binds their keys through the recorded LexicalTokens, and
//! supplies the same full country view used by the ordinary save parser.

use super::*;

pub type MissionSpottingCaches = BTreeMap<u64, [Option<Decimal>; 2]>;

pub(crate) fn spotting_caches(
    countries: super::super::CountriesView<'_>,
    keys: [&[u8]; 2],
) -> io::Result<MissionSpottingCaches> {
    if keys.iter().any(|key| key.is_empty()) || keys[0] == keys[1] {
        return Err(invalid(
            "mission spotting cache keys unavailable or aliased",
        ));
    }
    let mut result = BTreeMap::new();
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
                let task_fields = block(&task.value)?;
                let id = object_id(task_fields)?;
                let mission = field(task_fields, b"mission")?
                    .ok_or_else(|| invalid("saved task force lacks mission"))?;
                let mission = block(mission)?;
                let mut values = [None, None];
                for (key, value) in keys.into_iter().zip(&mut values) {
                    *value = field(mission, key)?
                        .map(|saved| Decimal::parse(scalar(saved)?))
                        .transpose()?;
                    if value.as_ref().is_some_and(|value| value.raw <= 0) {
                        return Err(invalid("mission spotting cache save field is not positive"));
                    }
                }
                if result.insert(id, values).is_some() {
                    return Err(invalid(
                        "duplicate full task force in spotting cache save view",
                    ));
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "spotting_cache_tests.rs"]
mod tests;
