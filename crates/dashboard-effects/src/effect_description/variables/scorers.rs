use super::super::*;
use crate::triggers::Scope;

#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn scored_countries(
        &self,
        item: &Item,
        scopes: Scopes,
        sorted: bool,
    ) -> Result<String, String> {
        let evaluator = self.evaluator()?;
        let result = self.scored_country_values(item, scopes, sorted);
        if let Err(why) = &result {
            evaluator.refuse_description_writer(format!("country scorer description: {why}"));
        }
        result.map(|_| String::new())
    }

    fn scored_country_values(
        &self,
        item: &Item,
        caller: Scopes,
        sorted: bool,
    ) -> Result<(), String> {
        let fields = children(item);
        let scalar = |name| {
            fields
                .iter()
                .rev()
                .find(|field| field.keyword_is(name))
                .and_then(|field| field.value.read_string())
        };
        let name = scalar("scorer").unwrap_or_default();
        let database = self
            .context
            .content
            .country_scorers
            .ok_or("country scorer content is unavailable")?;

        let Some(entries) = database.get(name)? else {
            return Ok(());
        };
        let evaluator = self.evaluator()?;
        let destination = if sorted {
            None
        } else {
            Some(
                evaluator
                    .description_temp_destination_fields(
                        fields,
                        "var",
                        b"highest_scored_country",
                        caller,
                        None,
                    )
                    .map_err(|why| why.to_string())?,
            )
        };
        let scope = evaluator
            .fresh_country_scope(None, Some(caller))
            .map_err(|why| why.to_string())?;
        let mut best = None;
        let mut best_score = -0x53e2_d620_fb40_i64;
        let mut scores = ScoreMap::default();
        for entry in entries {
            for target in evaluator
                .script_target_candidates(&entry.targets, caller)
                .map_err(|why| why.to_string())?
            {
                let scope = evaluator
                    .repoint_this(scope, Scope::Country(target))
                    .map_err(|why| why.to_string())?;
                let score = evaluator
                    .mean_time_to_happen("country scorer", &entry.score, scope)
                    .map_err(|why| why.to_string())?;
                if sorted {
                    scores.insert(target, score);
                } else if score > best_score {
                    best = Some(target);
                    best_score = score;
                }
            }
        }
        if let Some(destination) = destination {
            let value = match best {
                Some(country) => evaluator
                    .encode_description_scope(Scope::Country(country))
                    .map_err(|why| why.to_string())?,
                None => 0,
            };
            evaluator.store_description_temp(destination, value);
            return Ok(());
        }
        let values = scores.sorted();
        let count = (values.len() as i64).wrapping_mul(100_000);
        let num = content_reader_script::scoped_variable::ScopedVariable {
            parsed: true,
            num: true,
            ..Default::default()
        };
        for (name, default) in [
            ("array", b"sorted_country_list".as_slice()),
            ("scores", b"country_list_scores".as_slice()),
        ] {
            let key = evaluator
                .description_temp_destination_fields(fields, name, default, caller, Some(&num))
                .map_err(|why| why.to_string())?;
            evaluator.store_description_temp(key, count);
        }
        for (index, (country, score)) in values.into_iter().enumerate() {
            let index = content_reader_script::scoped_variable::ScopedVariable::literal(
                (index as i64).wrapping_mul(100_000),
            );
            let key = evaluator
                .description_temp_destination_fields(
                    fields,
                    "array",
                    b"sorted_country_list",
                    caller,
                    Some(&index),
                )
                .map_err(|why| why.to_string())?;
            let country = evaluator
                .encode_description_scope(Scope::Country(country))
                .map_err(|why| why.to_string())?;
            evaluator.store_description_temp(key, country);
            let key = evaluator
                .description_temp_destination_fields(
                    fields,
                    "scores",
                    b"country_list_scores",
                    caller,
                    Some(&index),
                )
                .map_err(|why| why.to_string())?;
            evaluator.store_description_temp(key, score);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Score {
    hash: u32,
    distance: usize,
    country: u32,
    value: i64,
}

#[derive(Default)]
struct ScoreMap {
    slots: Vec<Option<Score>>,
    mask: usize,
    max_probe: usize,
    size: usize,
}

impl ScoreMap {
    fn insert(&mut self, country: u32, value: i64) {
        let mixed = (country >> 16 ^ country).wrapping_mul(0x045d_9f3b);
        let mixed = (mixed >> 16 ^ mixed).wrapping_mul(0x045d_9f3b);
        let hash = mixed >> 16 ^ mixed;
        self.insert_score(Score {
            hash,
            distance: 1,
            country,
            value,
        });
    }

    fn insert_score(&mut self, mut new: Score) {
        if self.slots.is_empty() {
            self.grow();
        }
        let mut at = new.hash as usize & self.mask;
        let mut distance = 1;
        while let Some(old) = self.slots[at] {
            if old.country == new.country {
                self.slots[at].as_mut().unwrap().value = new.value;
                return;
            }
            if distance > old.distance {
                break;
            }
            at += 1;
            distance += 1;
        }
        if distance > self.max_probe || (self.size + 1) as f32 / self.mask as f32 > 0.9_f32 {
            self.grow();
            self.insert_score(new);
            return;
        }
        new.distance = distance;
        let inserted_at = at;
        loop {
            match &mut self.slots[at] {
                None => {
                    self.slots[at] = Some(new);
                    self.size += 1;
                    return;
                }
                Some(old) if new.distance > old.distance => std::mem::swap(old, &mut new),
                Some(_) => {}
            }
            at += 1;
            new.distance += 1;
            if new.distance > self.max_probe {
                std::mem::swap(self.slots[inserted_at].as_mut().unwrap(), &mut new);
                self.grow();
                self.insert_score(new);
                return;
            }
        }
    }

    fn grow(&mut self) {
        let buckets = if self.slots.is_empty() {
            8
        } else {
            (self.mask + 1) * 2
        };
        self.mask = buckets - 1;
        self.max_probe = buckets.trailing_zeros() as usize + 2;
        let old = std::mem::replace(&mut self.slots, vec![None; buckets + self.max_probe]);
        self.size = 0;
        for score in old.into_iter().flatten() {
            self.insert_score(score);
        }
    }

    fn sorted(self) -> Vec<(u32, i64)> {
        let mut values = self
            .slots
            .into_iter()
            .flatten()
            .map(|score| (score.country, score.value))
            .collect::<Vec<_>>();
        values.sort_by_key(|value| std::cmp::Reverse(value.1));
        values
    }
}
