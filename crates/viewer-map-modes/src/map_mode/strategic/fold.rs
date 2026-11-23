use super::*;
use map_display_data::landscape::Style;
use viewer_scene_base::projection_fold::{RowFold, fields};
use content_reader_map::map::MapData;
use map_display_data::{landscape::ProvinceField, retained::Retained, value_index::ValueIndex};
use replay_data::rows::{RowCodec, TableId, TableSchema};
use replay_frame::frame::ViewIdentity;
use std::sync::Weak;

#[derive(Default)]
pub(super) struct Fold {
    pub(super) bank: Option<Weak<MapData>>,
    geometry: Option<Geometry>,
    air: Option<Air>,
}
struct Geometry {
    bank: Weak<MapData>,
    planes: [ProvinceField; 2],
    keys: ValueIndex<u32, ()>,
    invalid: usize,
    navy: ValueIndex<u32, Style>,
}
impl Geometry {
    fn build(content: &Content, inputs: &Inputs, bank: Weak<MapData>) -> io::Result<Self> {
        let len = inputs
            .map
            .definitions
            .last_key_value()
            .map_or(1, |(&id, _)| id as usize + 1);
        let mut keys = ValueIndex::default();
        let mut air = ValueIndex::default();
        let mut navy = ValueIndex::default();
        let mut invalid = 0;
        for (&id, region) in &content.regions {
            let Some(key) = id.checked_add(1) else {
                invalid += 1;
                continue;
            };
            keys = keys.inserted(key, ())?;
            for province in &region.provinces {
                if let Some(definition) = inputs.map.definitions.get(province) {
                    air = air.inserted(*province as usize, key)?;
                    if definition.kind == ProvinceKind::Sea {
                        navy = navy.inserted(*province as usize, key)?;
                    }
                }
            }
        }
        let mut navy_styles = ValueIndex::default();
        for &key in keys.keys() {
            navy_styles = navy_styles.inserted(key, style(content.navy))?;
        }
        Ok(Self {
            bank,
            navy: navy_styles,
            planes: [
                ProvinceField::filled(len)?
                    .patched_sorted(air.iter().map(|(key, value)| (*key, *value)))?,
                ProvinceField::filled(len)?
                    .patched_sorted(navy.iter().map(|(key, value)| (*key, *value)))?,
            ],
            keys,
            invalid,
        })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Score {
    region: u32,
    friendly: i64,
    enemy: i64,
    friendly_missions: u16,
    enemy_missions: u16,
}
impl Score {
    fn needs_counts(self) -> bool {
        self.friendly == 0 && self.enemy == 0 && self.friendly_missions != 0
    }
    fn row(self) -> RaidAirScoreRow {
        RaidAirScoreRow {
            region: self.region,
            friendly: self.friendly,
            enemy: self.enemy,
            friendly_missions: self.friendly_missions,
            enemy_missions: self.enemy_missions,
            ..Default::default()
        }
    }
}
struct Owned<V> {
    rows: RowFold<(u32, V)>,
    by_raw_row: ValueIndex<(u32, u128), ()>,
    active: ValueIndex<u128, V>,
    links: ValueIndex<u128, u32>,
    selected: u32,
}
impl<V: Clone + PartialEq> Owned<V> {
    fn read<T: TableSchema>(
        inputs: &Inputs,
        links: &RowFold<u32>,
        by_slot_raw: &ValueIndex<(u32, u32), ()>,
        selected: u32,
        old: Option<&Self>,
        rows: &[T::Row],
        mask: u64,
        extract: impl FnMut(&T::Row) -> io::Result<(u32, V)>,
    ) -> io::Result<Self> {
        let rows = RowFold::read(
            inputs.frame,
            T::ID,
            rows,
            mask,
            old.map(|old| &old.rows),
            extract,
        )?;
        let mut by_raw_row = old.map_or_else(ValueIndex::default, |old| old.by_raw_row.clone());
        let mut dirty = ValueIndex::default();
        for &key in rows.edits.iter() {
            if let Some((raw, _)) = old.and_then(|old| old.rows.values.get(&key)) {
                by_raw_row = by_raw_row.removed(&(*raw, key))?;
            }
            if let Some((raw, _)) = rows.values.get(&key) {
                by_raw_row = by_raw_row.inserted((*raw, key), ())?;
            }
            dirty = dirty.inserted(key, ())?;
        }
        let mut raw_changes = ValueIndex::default();
        let mut error = None;
        links.values.visit_changes(
            &old.map_or_else(ValueIndex::default, |old| old.links.clone()),
            |raw, _, _| {
                if error.is_none() {
                    match raw_changes.inserted(*raw as u32, ()) {
                        Ok(next) => raw_changes = next,
                        Err(why) => error = Some(why),
                    }
                }
            },
        );
        if let Some(error) = error {
            return Err(error);
        }
        if old.is_none_or(|old| old.selected != selected) {
            for (&(_, raw), _) in by_slot_raw
                .iter_from(&(selected, 0))
                .take_while(|(key, _)| key.0 == selected)
            {
                raw_changes = raw_changes.inserted(raw, ())?;
            }
            if let Some(old) = old {
                for &key in old.active.keys() {
                    dirty = dirty.inserted(key, ())?;
                }
            }
        }
        for &raw in raw_changes.keys() {
            for (&(_, key), _) in by_raw_row
                .iter_from(&(raw, 0))
                .take_while(|(key, _)| key.0 == raw)
            {
                dirty = dirty.inserted(key, ())?;
            }
        }
        let mut active = old.map_or_else(ValueIndex::default, |old| old.active.clone());
        for &key in dirty.keys() {
            active = match rows
                .values
                .get(&key)
                .filter(|(raw, _)| links.values.get(&(*raw as u128)) == Some(&selected))
            {
                Some((_, value)) => active.inserted(key, value.clone())?,
                None => active.removed(&key)?,
            };
        }
        Ok(Self {
            rows,
            by_raw_row,
            active,
            links: links.values.clone(),
            selected,
        })
    }
}
struct Air {
    branch: u64,
    campaign: ViewIdentity,
    links: RowFold<u32>,
    by_slot_raw: ValueIndex<(u32, u32), ()>,
    countries: RowFold<u32>,
    by_slot_country: ValueIndex<(u32, u32), ()>,
    scores: Owned<Score>,
    by_region_row: ValueIndex<(u32, u128), Score>,
    fallback_rows: ValueIndex<u128, ()>,
    counters: Option<Owned<i32>>,
    wars: Option<Wars>,
    counter_view: ViewIdentity,
    war_view: ViewIdentity,
    styles: ValueIndex<u32, Style>,
    outcomes: ValueIndex<u32, Retained<Result<Style, String>>>,
    refused: ValueIndex<Retained<String>, usize>,
    selected: u32,
}
struct Wars {
    rows: RowFold<(u32, u32)>,
    by_raw_row: ValueIndex<(u32, u128), ()>,
    links: ValueIndex<u128, u32>,
    selected: u32,
    active: ValueIndex<u128, Retained<Result<(), String>>>,
    errors: ValueIndex<u128, Retained<String>>,
    link_view: ViewIdentity,
}
impl Wars {
    fn read(
        inputs: &Inputs,
        links: &RowFold<u32>,
        by_slot_raw: &ValueIndex<(u32, u32), ()>,
        selected: u32,
        old: Option<&Self>,
    ) -> io::Result<Result<Self, String>> {
        let wars = match inputs.frame.complete::<tables::WarRelations>() {
            Ok(rows) => rows,
            Err(error) => return Ok(Err(error.to_string())),
        };
        let link_rows = inputs.frame.complete::<tables::CountryLinks>()?;
        let link_view = &inputs.frame.tables[TableId::CountryLinks.index()];
        let ordered = if old.is_some_and(|old| old.link_view.is(link_view)) {
            true
        } else if let Some(facts) = old.and_then(|old| {
            link_view.changes_for(&old.link_view, TableId::CountryLinks.full_mask())
        }) {
            facts.changed().iter().all(|edit| {
                link_rows
                    .binary_search_by_key(&edit.key, RowCodec::key)
                    .is_ok_and(|at| {
                        (at == 0 || link_rows[at - 1].id < link_rows[at].id)
                            && (at + 1 == link_rows.len()
                                || link_rows[at].id < link_rows[at + 1].id)
                    })
            })
        } else {
            link_rows.windows(2).all(|pair| pair[0].id < pair[1].id)
        };
        if !ordered {
            return Ok(Err(
                "CountryLinks rows are not in ascending raw-id order".into()
            ));
        }
        let rows = RowFold::read(
            inputs.frame,
            TableId::WarRelations,
            wars,
            0,
            old.map(|old| &old.rows),
            |row| Ok((row.first_country, row.second_country)),
        )?;
        let mut by_raw_row = old.map_or_else(ValueIndex::default, |old| old.by_raw_row.clone());
        let mut dirty = ValueIndex::default();
        for &key in rows.edits.iter() {
            if let Some(&(a, b)) = old.and_then(|old| old.rows.values.get(&key)) {
                by_raw_row = by_raw_row.removed(&(a, key))?.removed(&(b, key))?;
            }
            if let Some(&(a, b)) = rows.values.get(&key) {
                by_raw_row = by_raw_row.inserted((a, key), ())?.inserted((b, key), ())?;
            }
            dirty = dirty.inserted(key, ())?;
        }
        let mut raws = ValueIndex::default();
        let mut error = None;
        links.values.visit_changes(
            &old.map_or_else(ValueIndex::default, |old| old.links.clone()),
            |key, _, _| {
                if error.is_none() {
                    match raws.inserted(*key as u32, ()) {
                        Ok(next) => raws = next,
                        Err(why) => error = Some(why),
                    }
                }
            },
        );
        if let Some(error) = error {
            return Err(error);
        }
        if old.is_none_or(|old| old.selected != selected) {
            for (&(_, raw), _) in by_slot_raw
                .iter_from(&(selected, 0))
                .take_while(|(key, _)| key.0 == selected)
            {
                raws = raws.inserted(raw, ())?;
            }
            if let Some(old) = old {
                for &key in old.active.keys() {
                    dirty = dirty.inserted(key, ())?;
                }
            }
        }
        for &raw in raws.keys() {
            for (&(_, key), _) in by_raw_row
                .iter_from(&(raw, 0))
                .take_while(|(key, _)| key.0 == raw)
            {
                dirty = dirty.inserted(key, ())?;
            }
        }
        let mut active = old.map_or_else(ValueIndex::default, |old| old.active.clone());
        let mut errors = old.map_or_else(ValueIndex::default, |old| old.errors.clone());
        for &key in dirty.keys() {
            active = active.removed(&key)?;
            errors = errors.removed(&key)?;
            let Some(&(first, second)) = rows.values.get(&key) else {
                continue;
            };
            let first_slot = links.values.get(&(first as u128));
            let second_slot = links.values.get(&(second as u128));
            let other = match (
                first_slot == Some(&selected),
                second_slot == Some(&selected),
            ) {
                (false, false) => continue,
                (true, true) => Err(format!(
                    "WarRelations joins country slot {selected} to itself"
                )),
                (true, false) => Ok(second),
                (false, true) => Ok(first),
            };
            let result = other.and_then(|raw| match links.values.get(&(raw as u128)) {
                None => Err(format!(
                    "WarRelations names country {raw}, which CountryLinks lacks"
                )),
                Some(0) => Err(format!(
                    "WarRelations links country {raw} to the reserved slot"
                )),
                Some(_) => Ok(()),
            });
            if let Err(why) = &result {
                errors = errors.inserted(key, Retained::new(why.clone()))?;
            }
            active = active.inserted(key, Retained::new(result))?;
        }
        Ok(Ok(Self {
            rows,
            by_raw_row,
            links: links.values.clone(),
            selected,
            active,
            errors,
            link_view: link_view.identity(),
        }))
    }
    fn any(&self) -> Result<bool, String> {
        match self.errors.iter().next() {
            Some((_, error)) => Err(error.to_string()),
            None => Ok(!self.active.is_empty()),
        }
    }
}
fn adjust(
    index: &mut ValueIndex<Retained<String>, usize>,
    reason: &Retained<String>,
    add: bool,
) -> io::Result<()> {
    let n = index.get(reason).copied().unwrap_or(0);
    *index = if add {
        index.inserted(reason.clone(), n + 1)?
    } else if n == 1 {
        index.removed(reason)?
    } else {
        index.inserted(reason.clone(), n - 1)?
    };
    Ok(())
}
impl Fold {
    pub(super) fn read(
        &mut self,
        content: &Content,
        mode: Stock,
        inputs: &Inputs,
        warnings: &mut Vec<String>,
    ) -> io::Result<Layer> {
        let bank = self.bank.clone().unwrap_or_default();
        let bound = self.geometry.as_ref().is_some_and(|geometry| {
            geometry.bank.ptr_eq(&bank)
                && bank.strong_count() != 0
                && bank.as_ptr() == inputs.map as *const MapData
        });
        if !bound {
            self.geometry = Some(Geometry::build(content, inputs, bank)?);
            self.air = None;
        }
        let geometry = self.geometry.as_ref().expect("strategic geometry");
        let air = mode == Stock::StrategicAir;
        let mut refused = ValueIndex::default();
        if geometry.invalid != 0 {
            refused = refused.inserted(
                Retained::new("region identity cannot be represented".to_owned()),
                geometry.invalid,
            )?;
        }
        let styles = if air {
            let old = self.air.as_ref().filter(|old| {
                old.branch == inputs.frame.branch
                    && old
                        .campaign
                        .is(&inputs.frame.tables[TableId::Campaign.index()])
            });
            match Air::read(content, geometry, inputs, old)? {
                Ok(next) => {
                    for (reason, n) in next.refused.iter() {
                        let count = refused.get(reason).copied().unwrap_or(0) + n;
                        refused = refused.inserted(reason.clone(), count)?;
                    }
                    let styles = next.styles.clone();
                    self.air = Some(next);
                    styles
                }
                Err(reason) => {
                    self.air = None;
                    let count = geometry.keys.len();
                    if count != 0 {
                        let reason = Retained::new(reason);
                        let count = count + refused.get(&reason).copied().unwrap_or(0);
                        refused = refused.inserted(reason, count)?;
                    }
                    ValueIndex::default()
                }
            }
        } else {
            geometry.navy.clone()
        };
        for (reason, count) in refused.iter() {
            warnings.push(format!(
                "{} map mode: {count} strategic-region colours withheld: {reason}",
                if air { "Air" } else { "Navy" }
            ));
        }
        Ok(Layer {
            region_of: geometry.planes[usize::from(!air)].clone(),
            styles,
            naval_fx: !air,
        })
    }
}
impl Air {
    fn read(
        content: &Content,
        geometry: &Geometry,
        inputs: &Inputs,
        old: Option<&Self>,
    ) -> io::Result<Result<Self, String>> {
        let Some(selected) = inputs.selected else {
            return Ok(Err("viewing country is absent".into()));
        };
        let frame = inputs.frame;
        macro_rules! source {
            ($value:expr) => {
                match $value {
                    Ok(rows) => rows,
                    Err(error) => return Ok(Err(error.to_string())),
                }
            };
        }
        let links = RowFold::read(
            frame,
            TableId::CountryLinks,
            source!(super::super::core::<tables::CountryLinks>(frame)),
            fields(TableId::CountryLinks, &["canonical_slot"]),
            old.map(|old| &old.links),
            |row| Ok(row.canonical_slot),
        )?;
        let countries = RowFold::read(
            frame,
            TableId::Countries,
            source!(super::super::core::<tables::Countries>(frame)),
            fields(TableId::Countries, &["canonical_slot"]),
            old.map(|old| &old.countries),
            |row| Ok(row.canonical_slot),
        )?;
        let mut by_slot_raw = old.map_or_else(ValueIndex::default, |old| old.by_slot_raw.clone());
        let mut by_slot_country =
            old.map_or_else(ValueIndex::default, |old| old.by_slot_country.clone());
        for (new, before, index) in [
            (&links, old.map(|old| &old.links), &mut by_slot_raw),
            (
                &countries,
                old.map(|old| &old.countries),
                &mut by_slot_country,
            ),
        ] {
            for &key in new.edits.iter() {
                if let Some(&slot) = before.and_then(|old| old.values.get(&key)) {
                    *index = index.removed(&(slot, key as u32))?;
                }
                if let Some(&slot) = new.values.get(&key) {
                    *index = index.inserted((slot, key as u32), ())?;
                }
            }
        }
        if by_slot_country
            .iter_from(&(selected, 0))
            .next()
            .is_none_or(|(key, _)| key.0 != selected)
        {
            return Ok(Err("viewing country has no Countries row".into()));
        }
        let scores = Owned::read::<tables::RaidAirScores>(
            inputs,
            &links,
            &by_slot_raw,
            selected,
            old.map(|old| &old.scores),
            source!(frame.complete::<tables::RaidAirScores>()),
            fields(
                TableId::RaidAirScores,
                &["friendly", "enemy", "friendly_missions", "enemy_missions"],
            ),
            |row| {
                Ok((
                    row.country,
                    Score {
                        region: row.region,
                        friendly: row.friendly,
                        enemy: row.enemy,
                        friendly_missions: row.friendly_missions,
                        enemy_missions: row.enemy_missions,
                    },
                ))
            },
        )?;
        let mut dirty_rows = ValueIndex::default();
        let mut error = None;
        scores.active.visit_changes(
            &old.map_or_else(ValueIndex::default, |old| old.scores.active.clone()),
            |key, _, _| {
                if error.is_none() {
                    match dirty_rows.inserted(*key, ()) {
                        Ok(next) => dirty_rows = next,
                        Err(why) => error = Some(why),
                    }
                }
            },
        );
        if let Some(error) = error {
            return Err(error);
        }
        let mut by_region_row =
            old.map_or_else(ValueIndex::default, |old| old.by_region_row.clone());
        let mut fallback_rows =
            old.map_or_else(ValueIndex::default, |old| old.fallback_rows.clone());
        let mut dirty = ValueIndex::default();
        for &key in dirty_rows.keys() {
            if let Some(score) = old.and_then(|old| old.scores.active.get(&key)) {
                by_region_row = by_region_row.removed(&(score.region, key))?;
                dirty = dirty.inserted(score.region, ())?;
            }
            fallback_rows = fallback_rows.removed(&key)?;
            if let Some(score) = scores.active.get(&key) {
                by_region_row = by_region_row.inserted((score.region, key), *score)?;
                dirty = dirty.inserted(score.region, ())?;
                if score.needs_counts() {
                    fallback_rows = fallback_rows.inserted(key, ())?;
                }
            }
        }
        let counter_view = frame.tables[TableId::CountryCounterInputs.index()].identity();
        let war_view = frame.tables[TableId::WarRelations.index()].identity();
        let counts_changed = old.is_none_or(|old| {
            old.selected != selected
                || !old.links.values.ptr_eq(&links.values)
                || !old
                    .counter_view
                    .is(&frame.tables[TableId::CountryCounterInputs.index()])
                || !old
                    .war_view
                    .fields_unchanged(&frame.tables[TableId::WarRelations.index()], 0)
        });
        if counts_changed {
            for &key in fallback_rows.keys() {
                if let Some(score) = scores.active.get(&key) {
                    dirty = dirty.inserted(score.region, ())?;
                }
            }
        }
        let mut counters = None;
        let mut wars = None;
        let mut allowed = None;
        let mut styles = old.map_or_else(ValueIndex::default, |old| old.styles.clone());
        let mut outcomes = old.map_or_else(ValueIndex::default, |old| old.outcomes.clone());
        let mut refused = old.map_or_else(ValueIndex::default, |old| old.refused.clone());
        if old.is_none() {
            for &key in geometry.keys.keys() {
                styles = styles.inserted(key, style(content.air[3]))?;
            }
        }
        for &region in dirty.keys() {
            let Some(key) = region
                .checked_add(1)
                .filter(|key| geometry.keys.contains_key(key))
            else {
                continue;
            };
            if let Some(before) = outcomes.get(&region)
                && let Err(reason) = &**before
            {
                let reason = Retained::new(reason.clone());
                adjust(&mut refused, &reason, false)?;
            }
            let winner = by_region_row
                .last_at_or_before(&(region, u128::MAX))
                .filter(|(key, _)| key.0 == region)
                .map(|(_, value)| *value);
            let color = if let Some(score) = winner {
                if score.needs_counts() && allowed.is_none() {
                    let value = match frame.complete::<tables::CountryCounterInputs>() {
                        Err(error) => Err(error.to_string()),
                        Ok(rows) => {
                            let next = Owned::read::<tables::CountryCounterInputs>(
                                inputs,
                                &links,
                                &by_slot_raw,
                                selected,
                                old.and_then(|old| old.counters.as_ref()),
                                rows,
                                fields(TableId::CountryCounterInputs, &["owned_state_count"]),
                                |row| Ok((row.country, row.owned_state_count)),
                            )?;
                            let value = match next.active.iter().next() {
                                None => Err("viewing country's owned-state count is absent".into()),
                                Some((_, n)) if *n < 1 => Ok(false),
                                Some(_) if selected == 0 => {
                                    Err("Country0's enemy roster is not recorded".into())
                                }
                                Some(_) => {
                                    match Wars::read(
                                        inputs,
                                        &links,
                                        &by_slot_raw,
                                        selected,
                                        old.and_then(|old| old.wars.as_ref()),
                                    )? {
                                        Err(reason) => Err(reason),
                                        Ok(next) => {
                                            let value = next.any();
                                            wars = Some(next);
                                            value
                                        }
                                    }
                                }
                            };
                            counters = Some(next);
                            value
                        }
                    };
                    allowed = Some(value);
                }
                let counts = if score.needs_counts() {
                    allowed.as_ref().expect("lazy mission counts").clone()
                } else {
                    Ok(false)
                };
                counts
                    .and_then(|counts| {
                        air_color(
                            &score.row(),
                            if counts { score.friendly_missions } else { 0 },
                        )
                    })
                    .map(|index| style(content.air[index]))
            } else {
                Ok(style(content.air[3]))
            };
            if let Err(reason) = &color {
                let reason = Retained::new(reason.clone());
                adjust(&mut refused, &reason, true)?;
            }
            styles = match &color {
                Ok(style) => styles.inserted(key, *style)?,
                Err(_) => styles.removed(&key)?,
            };
            outcomes = outcomes.inserted(region, Retained::new(color))?;
        }
        if counters.is_none() {
            if let Some(previous) = old.and_then(|old| old.counters.as_ref()) {
                counters = Some(previous.copy());
            }
        }
        if wars.is_none() {
            if let Some(previous) = old.and_then(|old| old.wars.as_ref()) {
                wars = Some(previous.copy());
            }
        }
        Ok(Ok(Self {
            branch: frame.branch,
            campaign: frame.tables[TableId::Campaign.index()].identity(),
            links,
            by_slot_raw,
            countries,
            by_slot_country,
            scores,
            by_region_row,
            fallback_rows,
            counters,
            wars,
            counter_view,
            war_view,
            styles,
            outcomes,
            refused,
            selected,
        }))
    }
}
impl<V: Clone> Owned<V> {
    fn copy(&self) -> Self {
        Self {
            rows: self.rows.clone(),
            by_raw_row: self.by_raw_row.clone(),
            active: self.active.clone(),
            links: self.links.clone(),
            selected: self.selected,
        }
    }
}
impl Wars {
    fn copy(&self) -> Self {
        Self {
            rows: self.rows.clone(),
            by_raw_row: self.by_raw_row.clone(),
            links: self.links.clone(),
            selected: self.selected,
            active: self.active.clone(),
            errors: self.errors.clone(),
            link_view: self.link_view.clone(),
        }
    }
}
