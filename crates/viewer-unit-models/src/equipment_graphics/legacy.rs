
use content_reader_equipment_graphics::equipment_graphics::{Candidate, insert_legacy};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug)]
pub struct Equipment<'a> {
    pub archetype: &'a str,
    pub archetype_ordinal: usize,
    pub sprite: &'a str,
    pub visual_level: i32,
    pub model_priority: i32,
    pub parent_depth: i32,
    pub has_enabled_module_slots: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Variant<'a> {
    pub handle: u64,
    pub id: u32,
    pub creator_reference: i32,
    pub creator_slot: i32,
    pub upgrade_sum: i32,
    pub equipment: Equipment<'a>,
}

#[derive(Clone, Copy, Debug)]
pub struct Country<'a> {
    pub tag: &'a str,
    pub cosmetic: &'a str,
    pub graphical_culture: &'a str,
    pub reference: i32,
    pub canonical_slot: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Subunit<'a> {
    pub key: &'a str,
    pub sprite: &'a str,
    pub categories: u64,
    pub first_archetype: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    pub country: Country<'a>,
    pub subunit: Subunit<'a>,
    pub selected: Option<&'a Variant<'a>>,
    pub available: &'a [Variant<'a>],
    pub best_candidates: &'a [Variant<'a>],
}

pub fn append_candidates(
    context: &Context<'_>,
    candidates: &mut Vec<Candidate>,
    entity_exists: impl FnMut(&str) -> bool,
) -> Result<(), String> {
    append_candidates_with_priority(context, candidates, 0, entity_exists)
}

pub fn append_candidates_with_priority(
    context: &Context<'_>,
    candidates: &mut Vec<Candidate>,
    base_priority: i32,
    entity_exists: impl FnMut(&str) -> bool,
) -> Result<(), String> {
    append_candidates_with_sources(context, candidates, base_priority, entity_exists).map(|_| ())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Variant(u64),
    Best(u64),
    Subunit,
}

pub fn append_candidates_with_sources(
    context: &Context<'_>,
    candidates: &mut Vec<Candidate>,
    base_priority: i32,
    mut entity_exists: impl FnMut(&str) -> bool,
) -> Result<Vec<(String, Source)>, String> {
    let Some(archetype) = context.subunit.first_archetype else {
        return Ok(Vec::new());
    };
    let mut variants: Vec<_> = context
        .available
        .iter()
        .filter(|variant| variant.equipment.archetype == archetype)
        .collect();
    variants.sort_by(|left, right| compare(context.country, left, right));
    let mut best = context
        .best_candidates
        .iter()
        .filter(|variant| variant.equipment.archetype == archetype)
        .min_by(|left, right| compare(context.country, left, right));
    if let Some(selected) = context.selected {
        if !variants
            .iter()
            .any(|variant| variant.handle == selected.handle)
        {
            variants.push(selected);
        }
        best = best.or(Some(selected));
    }
    let mut producer = Producer {
        context,
        candidates,
        entity_exists: &mut entity_exists,
        source: Source::Subunit,
        sources: Vec::new(),
    };
    if let Some(selected) = context.selected
        && !selected.equipment.sprite.is_empty()
    {
        producer.stem(
            selected.equipment.sprite,
            base_priority.wrapping_add(1000),
            &variants,
            best,
        )?;
    }
    producer.stem(context.subunit.sprite, base_priority, &variants, best)?;
    if context.subunit.key != context.subunit.sprite {
        producer.stem(context.subunit.key, base_priority, &variants, best)?;
    }
    Ok(producer.sources)
}

pub(crate) fn compare(owner: Country<'_>, left: &Variant<'_>, right: &Variant<'_>) -> Ordering {
    let order = |variant: &Variant<'_>| dashboard_text::variant_names::VariantOrder {
        archetype_ordinal: variant.equipment.archetype_ordinal,
        parent_depth: variant.equipment.parent_depth,
        upgrade_sum: if variant.equipment.has_enabled_module_slots {
            0
        } else {
            variant.upgrade_sum
        },
        creator_reference: variant.creator_reference,
        creator_slot: variant.creator_slot,
        id: variant.id,
    };
    dashboard_text::variant_names::compare_variants(
        owner.reference,
        owner.canonical_slot,
        order(left),
        order(right),
    )
}

fn visual_level(subunit: Subunit<'_>, variant: &Variant<'_>) -> i32 {
    if subunit.categories & 0x80_0040_03c1 == 0 {
        variant.equipment.visual_level
    } else {
        variant.equipment.model_priority
    }
}

struct Producer<'a, 'b, F> {
    context: &'b Context<'a>,
    candidates: &'b mut Vec<Candidate>,
    entity_exists: &'b mut F,
    source: Source,
    sources: Vec<(String, Source)>,
}

impl<F: FnMut(&str) -> bool> Producer<'_, '_, F> {
    fn selected(&self, variant: &Variant<'_>) -> bool {
        self.context
            .selected
            .is_some_and(|selected| selected.handle == variant.handle)
    }

    fn visual_level(&self, variant: &Variant<'_>) -> i32 {
        visual_level(self.context.subunit, variant)
    }

    fn stem(
        &mut self,
        stem: &str,
        base: i32,
        variants: &[&Variant<'_>],
        best: Option<&Variant<'_>>,
    ) -> Result<(), String> {
        let mut best_has_model = false;
        for variant in variants {
            let visual = self.visual_level(variant);
            let is_best = best.is_some_and(|best| best.handle == variant.handle);
            let rank = base
                .wrapping_add(visual)
                .wrapping_add(if self.selected(variant) { 30 } else { 0 })
                .wrapping_add(if is_best { 20 } else { 0 })
                .wrapping_add(5);
            let numbered = format!("{stem}_{visual}");
            self.source = Source::Variant(variant.handle);
            let found = self.prefixes(&format!("{numbered}_entity"), rank)?;
            let alternatives = self.alternatives(&numbered, rank)?;
            best_has_model |= is_best && (found || alternatives);
        }
        if !best_has_model && let Some(best) = best {
            let rank = base
                .wrapping_add(self.visual_level(best))
                .wrapping_add(if self.selected(best) { 30 } else { 0 })
                .wrapping_add(28);
            self.source = Source::Best(best.handle);
            self.prefixes(&format!("{stem}_entity"), rank)?;
            self.alternatives(&format!("{}_{stem}", self.context.country.tag), rank)?;
        }
        self.source = Source::Subunit;
        self.prefixes(&format!("{stem}_entity"), base)?;
        self.alternatives(stem, base)?;
        Ok(())
    }

    fn alternatives(&mut self, stem: &str, rank: i32) -> Result<bool, String> {
        let mut found = 0_i32;
        for alternative in 0..4 {
            if self.prefixes(
                &format!("{stem}_alt_{alternative}_entity"),
                rank.wrapping_sub(1).wrapping_sub(found),
            )? {
                found += 1;
            }
        }
        Ok(found != 0)
    }

    fn prefixes(&mut self, suffix: &str, rank: i32) -> Result<bool, String> {
        let country = self.context.country;
        let mut found = false;
        if !country.cosmetic.is_empty() {
            found |= self.series(
                &format!("{}_{suffix}", country.cosmetic),
                rank.wrapping_add(4),
            )?;
        }
        found |= self.series(&format!("{}_{suffix}", country.tag), rank.wrapping_add(3))?;
        if !country.graphical_culture.is_empty() {
            found |= self.series(
                &format!("{}_{suffix}", country.graphical_culture),
                rank.wrapping_add(2),
            )?;
        }
        found |= self.series(suffix, rank.wrapping_add(1))?;
        Ok(found)
    }

    fn series(&mut self, first: &str, rank: i32) -> Result<bool, String> {
        let mut sequence = 1_i32;
        let mut found = false;
        loop {
            let entity = if sequence == 1 {
                first.to_owned()
            } else {
                format!("{first}_{sequence}")
            };
            if !(self.entity_exists)(&entity) {
                return Ok(found);
            }
            let weight = i64::from(rank.wrapping_mul(10).wrapping_sub(sequence)) * 100_000;
            if insert_legacy(
                self.candidates,
                Candidate {
                    entity: entity.clone(),
                    weight,
                },
            ) {
                self.sources.push((entity, self.source));
                found = true;
            }
            sequence = sequence.checked_add(1).ok_or_else(|| {
                "equipment model numbered-entity sequence exceeds the native signed index"
                    .to_owned()
            })?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(id: u32, visual_level: i32) -> Variant<'static> {
        Variant {
            handle: u64::from(id),
            id,
            creator_reference: 1,
            creator_slot: 1,
            upgrade_sum: 0,
            equipment: Equipment {
                archetype: "infantry_equipment",
                archetype_ordinal: 0,
                sprite: "",
                visual_level,
                model_priority: visual_level + 1,
                parent_depth: visual_level,
                has_enabled_module_slots: false,
            },
        }
    }

    fn context<'a>(available: &'a [Variant<'a>]) -> Context<'a> {
        Context {
            country: Country {
                tag: "GER",
                cosmetic: "COS",
                graphical_culture: "western",
                reference: 1,
                canonical_slot: 1,
            },
            subunit: Subunit {
                key: "infantry",
                sprite: "infantry",
                categories: 0x20,
                first_archetype: Some("infantry_equipment"),
            },
            selected: available.first(),
            available,
            best_candidates: available,
        }
    }

    #[test]
    fn selected_variant_bonus_can_beat_newer_available_equipment() {
        let available = [variant(1, 1), variant(2, 3)];
        let mut candidates = Vec::new();
        append_candidates(&context(&available), &mut candidates, |entity| {
            matches!(entity, "GER_infantry_1_entity" | "GER_infantry_3_entity")
        })
        .unwrap();
        assert_eq!(candidates[0].entity, "GER_infantry_1_entity");
        assert_eq!(candidates[0].weight, (39 * 10 - 1) * 100_000);
    }

    #[test]
    fn missing_suffix_stops_the_series_and_cosmetic_wins_prefix_rank() {
        let available = [variant(1, 1)];
        let mut candidates = Vec::new();
        append_candidates(&context(&available), &mut candidates, |entity| {
            matches!(
                entity,
                "COS_infantry_1_entity" | "GER_infantry_1_entity" | "GER_infantry_1_entity_3"
            )
        })
        .unwrap();
        assert_eq!(candidates[0].entity, "COS_infantry_1_entity");
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn the_recorded_best_candidate_decides_between_numbered_models() {
        let available = [variant(1, 1), variant(2, 3)];
        let exists =
            |entity: &str| matches!(entity, "GER_infantry_1_entity" | "GER_infantry_3_entity");
        for (best, winner) in [
            (&available[..1], "GER_infantry_1_entity"),
            (&available[1..], "GER_infantry_3_entity"),
        ] {
            let context = Context {
                selected: None,
                best_candidates: best,
                ..context(&available)
            };
            let mut candidates = Vec::new();
            append_candidates(&context, &mut candidates, exists).unwrap();
            assert_eq!(candidates[0].entity, winner);
        }
    }

    #[test]
    fn modular_equipment_does_not_use_upgrade_sum_for_ordering() {
        let mut older = variant(1, 1);
        let mut newer = variant(2, 1);
        older.upgrade_sum = 100;
        let owner = context(&[]).country;
        assert_eq!(compare(owner, &older, &newer), Ordering::Less);
        older.equipment.has_enabled_module_slots = true;
        newer.equipment.has_enabled_module_slots = true;
        assert_eq!(compare(owner, &older, &newer), Ordering::Greater);
    }

    #[test]
    fn combined_production_roster_sorts_archetype_order_before_depth() {
        let mut first = variant(1, 1);
        let mut second = variant(2, 99);
        first.equipment.archetype_ordinal = 4;
        second.equipment.archetype_ordinal = 5;
        assert_eq!(
            compare(context(&[]).country, &first, &second),
            Ordering::Less
        );
    }

    #[test]
    fn null_reference_does_not_alias_a_nonzero_reference_in_slot_zero() {
        let mut null_creator = variant(1, 1);
        let mut aliased_creator = variant(2, 1);
        null_creator.creator_reference = 0;
        null_creator.creator_slot = 0;
        aliased_creator.creator_reference = 7;
        aliased_creator.creator_slot = 0;
        let mut owner = context(&[]).country;
        owner.reference = 0;
        owner.canonical_slot = 0;
        assert_eq!(
            compare(owner, &null_creator, &aliased_creator),
            Ordering::Less
        );
    }
}
