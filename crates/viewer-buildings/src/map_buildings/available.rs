use super::*;
use viewer_scene_base::projection_fold::{RowFold, fields};
use map_display_data::{retained::Retained, value_index::ValueIndex};
use replay_frame::frame::ViewIdentity;

type Key = (bool, u32, usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildingInput {
    pub location: u32,
    pub building_type: u32,
    pub country: u32,
    pub status_flags: u32,
    pub level: i16,
    pub healthy_level: i16,
    pub partial_health: i64,
}
impl From<&BuildingRow> for BuildingInput {
    fn from(row: &BuildingRow) -> Self {
        Self {
            location: row.location,
            building_type: row.building_type,
            country: row.country,
            status_flags: row.status_flags,
            level: row.level,
            healthy_level: row.healthy_level,
            partial_health: row.partial_health,
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Name {
    reason: Retained<String>,
    invalid: bool,
}
#[derive(Clone, Default, PartialEq, Eq)]
struct Count {
    index: Option<usize>,
    refused: Option<Retained<String>>,
    unnamed: bool,
    built: bool,
    missing: bool,
    destroyed: bool,
    damaged: bool,
    uncertain: bool,
}
#[derive(Clone, Default)]
struct Census {
    built: usize,
    missing: usize,
    destroyed: usize,
    damaged: usize,
    uncertain: usize,
    unnamed: usize,
    reasons: ValueIndex<Retained<String>, usize>,
    missing_types: ValueIndex<usize, usize>,
}
impl Census {
    fn change(&mut self, value: &Count, add: bool) -> io::Result<()> {
        fn adjust(count: &mut usize, value: bool, add: bool) {
            if value {
                if add {
                    *count += 1;
                } else {
                    *count -= 1;
                }
            }
        }
        for (count, value) in [
            (&mut self.built, value.built),
            (&mut self.missing, value.missing),
            (&mut self.destroyed, value.destroyed),
            (&mut self.damaged, value.damaged),
            (&mut self.uncertain, value.uncertain),
            (&mut self.unnamed, value.unnamed),
        ] {
            adjust(count, value, add);
        }
        if let Some(reason) = &value.refused {
            change_count(&mut self.reasons, reason.clone(), add)?;
        }
        if value.missing {
            change_count(
                &mut self.missing_types,
                value.index.expect("missing building type"),
                add,
            )?;
        }
        Ok(())
    }
    fn apply(&self, assets: &MapBuildingAssets, summary: &mut MapBuildingSummary) {
        summary.built = self.built;
        summary.missing_slots = self.missing;
        summary.destroyed = self.destroyed;
        summary.damaged = self.damaged;
        summary.uncertain_damage = self.uncertain;
        summary.unnamed_tokens = self.unnamed;
        summary.refused_tokens = self
            .reasons
            .iter()
            .map(|(key, count)| (key.to_string(), *count))
            .collect();
        summary.missing_by_type.clear();
        for (index, count) in self.missing_types.iter() {
            *summary
                .missing_by_type
                .entry(assets.facts.ordered[*index].name.clone())
                .or_default() += count;
        }
    }
}
fn change_count<K: Ord + Clone>(
    index: &mut ValueIndex<K, usize>,
    key: K,
    add: bool,
) -> io::Result<()> {
    let count = index.get(&key).copied().unwrap_or(0);
    *index = if add {
        index.inserted(key, count + 1)?
    } else if count == 1 {
        index.removed(&key)?
    } else {
        index.inserted(key, count.checked_sub(1).expect("census predecessor"))?
    };
    Ok(())
}
fn referenced_names(
    input: replay_frame::frame::HourInput<'_>,
    rows: &[replay_data::rows::LexicalTokenRow],
    complete: bool,
    by_token_row: &ValueIndex<(u32, bool, u128), ()>,
    mut dirty: ValueIndex<u32, ()>,
    previous: Option<&RowFold<Name>>,
) -> io::Result<RowFold<Name>> {
    use replay_frame::frame::TableChanges;
    let table = TableId::LexicalTokens;
    let mask = fields(table, &["literal"]);
    match input.changes().since(table, previous.map(|old| &old.identity), mask) {
        TableChanges::Unchanged => {}
        TableChanges::Rows(facts) => {
            for key in facts.deleted().iter().copied().chain(
                facts.changed().iter()
                    .filter(|row| row.inserted || row.fields & mask != 0)
                    .map(|row| row.key),
            ) {
                let token = u32::try_from(key)
                    .map_err(|_| invalid("lexical token key exceeds u32"))?;
                if by_token_row.iter_from(&(token, false, 0))
                    .next().is_some_and(|(key, _)| key.0 == token)
                    || previous.is_some_and(|old| old.values.contains_key(&key))
                {
                    dirty = dirty.inserted(token, ())?;
                }
            }
        }
        TableChanges::Discontinuity(_) => {
            for &(token, _, _) in by_token_row.keys() {
                dirty = dirty.inserted(token, ())?;
            }
            if let Some(old) = previous {
                for &key in old.values.keys() {
                    dirty = dirty.inserted(key as u32, ())?;
                }
            }
        }
    }
    let count = dirty.len();
    let mut patches = Vec::with_capacity(count);
    let mut edits = Vec::with_capacity(count);
    let mut values = previous.map_or_else(ValueIndex::default, |old| old.values.clone());
    for &token in dirty.keys() {
        let referenced = by_token_row
            .iter_from(&(token, false, 0))
            .next()
            .is_some_and(|(key, _)| key.0 == token);
        let row = referenced
            .then(|| rows.binary_search_by_key(&token, |row| row.token).ok())
            .flatten()
            .map(|at| &rows[at]);
        let next = row
            .map(|row| {
                let names = TokenNames::from_rows(std::slice::from_ref(row), complete);
                let result = names.name(token);
                Ok::<_, io::Error>(Name {
                    invalid: result.is_err(),
                    reason: Retained::new(match result {
                        Ok(spelling) => format!(
                            "token {} ({spelling}) names no authored building type",
                            token
                        ),
                        Err(error) => error.to_string(),
                    }),
                })
            })
            .transpose()?;
        let key = u128::from(token);
        if values.get(&key) != next.as_ref() {
            edits.push(key);
            patches.push((key, next));
        }
    }
    values = values.patched(&patches)?;
    Ok(RowFold {
        identity: input.frame().tables[table.index()].identity(),
        values,
        edits: Retained::new(edits),
    })
}

pub(super) struct AvailableRows {
    branch: u64,
    campaign: ViewIdentity,
    rows: [RowFold<BuildingInput>; 2],
    names: RowFold<Name>,
    lexical_complete: bool,
    types: TokenTypes,
    by_token_row: ValueIndex<(u32, bool, u128), ()>,
    absent_tokens: ValueIndex<u32, usize>,
    by_candidate: ValueIndex<(Key, u128), BuildingInput>,
    pub(super) values: ValueIndex<Key, BuildingInput>,
    by_country_key: ValueIndex<(u32, Key), ()>,
    counts: ValueIndex<(bool, u128), Count>,
    invalid_names: ValueIndex<(bool, u128), Retained<String>>,
    census: Census,
    selected: Retained<Result<Vec<usize>, String>>,
    city: Option<City>,
}
struct City {
    provinces: RowFold<u32>,
    by_state_province: ValueIndex<(u32, u32), ()>,
    available: ValueIndex<Key, BuildingInput>,
    health: ValueIndex<u32, Option<i64>>,
}
impl AvailableRows {
    pub(super) fn country_keys(&self, raw: u32) -> impl Iterator<Item = Key> + '_ {
        self.by_country_key
            .iter_from(&(raw, (false, 0, 0)))
            .take_while(move |(key, _)| key.0 == raw)
            .map(|(key, _)| key.1)
    }
    pub(super) fn get(&self, key: &Key) -> Option<&BuildingInput> {
        self.values.get(key)
    }
    #[cfg(test)]
    pub(super) fn read(
        assets: &MapBuildingAssets,
        frame: &DecodedFrame,
        tokens: &TokenTypes,
        old: Option<&Self>,
        summary: &mut MapBuildingSummary,
    ) -> io::Result<Self> {
        Self::read_input(
            assets,
            replay_frame::frame::HourInput::new(frame),
            tokens,
            old,
            summary,
        )
    }

    pub(super) fn read_input(
        assets: &MapBuildingAssets,
        input: replay_frame::frame::HourInput<'_>,
        tokens: &TokenTypes,
        old: Option<&Self>,
        summary: &mut MapBuildingSummary,
    ) -> io::Result<Self> {
        let frame = input.frame();
        let old = old.filter(|old| {
            old.branch == frame.branch && old.campaign.is(&frame.tables[TableId::Campaign.index()])
        });
        let state = rows::<tables::StateBuildings>(frame)?;
        let province = rows::<tables::ProvinceBuildings>(frame)?;
        let state = RowFold::read_input(
            input,
            TableId::StateBuildings,
            state.rows(),
            fields(
                TableId::StateBuildings,
                &[
                    "status_flags",
                    "country",
                    "level",
                    "healthy_level",
                    "partial_health",
                ],
            ),
            old.map(|old| &old.rows[0]),
            |row| Ok(BuildingInput::from(row)),
        )?;
        let province = RowFold::read_input(
            input,
            TableId::ProvinceBuildings,
            province.rows(),
            fields(
                TableId::ProvinceBuildings,
                &[
                    "status_flags",
                    "country",
                    "level",
                    "healthy_level",
                    "partial_health",
                ],
            ),
            old.map(|old| &old.rows[1]),
            |row| Ok(BuildingInput::from(row)),
        )?;
        let current = [state, province];
        let mut by_token_row = old.map_or_else(ValueIndex::default, |old| old.by_token_row.clone());
        let mut name_keys = ValueIndex::default();
        for (at, rows) in current.iter().enumerate() {
            let provincial = at != 0;
            for &key in rows.edits.iter() {
                if let Some(row) = old.and_then(|old| old.rows[at].values.get(&key)) {
                    by_token_row = by_token_row.removed(&(row.building_type, provincial, key))?;
                    name_keys = name_keys.inserted(row.building_type, ())?;
                }
                if let Some(row) = rows.values.get(&key) {
                    by_token_row = by_token_row.inserted((row.building_type, provincial, key), ())?;
                    name_keys = name_keys.inserted(row.building_type, ())?;
                }
            }
        }
        let lexical = rows::<tables::LexicalTokens>(frame)?;
        let lexical_complete = lexical.is_complete();
        let names = referenced_names(
            input, lexical.rows(), lexical_complete, &by_token_row, name_keys,
            old.map(|old| &old.names),
        )?;
        let mut absent_tokens =
            old.map_or_else(ValueIndex::default, |old| old.absent_tokens.clone());
        let mut by_candidate = old.map_or_else(ValueIndex::default, |old| old.by_candidate.clone());
        let mut values = old.map_or_else(ValueIndex::default, |old| old.values.clone());
        let mut counts = old.map_or_else(ValueIndex::default, |old| old.counts.clone());
        let mut invalid_names =
            old.map_or_else(ValueIndex::default, |old| old.invalid_names.clone());
        let mut census = old.map_or_else(Census::default, |old| old.census.clone());
        let mut dirty = ValueIndex::default();
        for (at, rows) in current.iter().enumerate() {
            let provincial = at != 0;
            for &key in rows.edits.iter() {
                if let Some(row) = old.and_then(|old| old.rows[at].values.get(&key)) {
                    if old.is_some_and(|old| {
                        old.names.values.get(&(row.building_type as u128)).is_none()
                    }) {
                        change_count(&mut absent_tokens, row.building_type, false)?;
                    }
                }
                if let Some(row) = rows.values.get(&key) {
                    if names.values.get(&(row.building_type as u128)).is_none() {
                        change_count(&mut absent_tokens, row.building_type, true)?;
                    }
                }
                dirty = dirty.inserted((provincial, key), ())?;
            }
        }
        let mut retyped = ValueIndex::default();
        let mut error = None;
        tokens.0.visit_changes(
            &old.map_or_else(ValueIndex::default, |old| old.types.0.clone()),
            |token, _, _| {
                if error.is_none() {
                    match retyped.inserted(*token, ()) {
                        Ok(next) => retyped = next,
                        Err(why) => error = Some(why),
                    }
                }
            },
        );
        if let Some(error) = error {
            return Err(error);
        }
        for &token in names.edits.iter() {
            retyped = retyped.inserted(token as u32, ())?;
        }
        if old.is_some_and(|old| old.lexical_complete != lexical_complete) {
            for &token in absent_tokens.keys() {
                retyped = retyped.inserted(token, ())?;
            }
        }
        for &token in retyped.keys() {
            for (&(_, provincial, key), _) in by_token_row
                .iter_from(&(token, false, 0))
                .take_while(|(key, _)| key.0 == token)
            {
                dirty = dirty.inserted((provincial, key), ())?;
            }
        }
        let mut changed_keys = ValueIndex::default();
        for &(provincial, key) in dirty.keys() {
            let at = usize::from(provincial);
            if let Some(before) = counts.get(&(provincial, key)).cloned() {
                census.change(&before, false)?;
                if let Some(index) = before.index {
                    let location = (key >> 32) as u32;
                    let target = (provincial, location, index);
                    by_candidate = by_candidate.removed(&(target, key))?;
                    changed_keys = changed_keys.inserted(target, ())?;
                }
            }
            invalid_names = invalid_names.removed(&(provincial, key))?;
            let Some(row) = current[at]
                .values
                .get(&key)
                .filter(|row| row.building_type != 0)
            else {
                counts = counts.removed(&(provincial, key))?;
                continue;
            };
            let name = names.values.get(&(row.building_type as u128));
            let invalid = name.is_none_or(|name| name.invalid);
            let reason = match name {
                Some(name) => name.reason.clone(),
                None => {
                    use dashboard_text::table_access::TokenError;
                    Retained::new(
                        if row.building_type == 0 {
                            TokenError::Rejected
                        } else if lexical_complete {
                            TokenError::Absent(row.building_type)
                        } else {
                            TokenError::Missing(row.building_type)
                        }
                        .to_string(),
                    )
                }
            };
            if invalid {
                invalid_names = invalid_names.inserted((provincial, key), reason.clone())?;
            }
            let mut count = Count::default();
            match tokens.get(&row.building_type) {
                Some(Ok(index)) => {
                    let index = *index;
                    count.index = Some(index);
                    let target = (provincial, row.location, index);
                    by_candidate = by_candidate.inserted((target, key), *row)?;
                    changed_keys = changed_keys.inserted(target, ())?;
                    let building = &assets.facts.ordered[index];
                    if slots_for_level(building, row.level, assets.defines.max_levels) != 0 {
                        count.built = true;
                        count.missing = !assets
                            .slots
                            .contains_key(&(row.location, building.slot_name().to_owned()));
                        (count.destroyed, count.damaged, count.uncertain) = health_state(
                            row.level,
                            row.healthy_level,
                            row.partial_health,
                            &assets.defines,
                        );
                    }
                }
                Some(Err(why)) => {
                    count.refused = Some(Retained::new(why.clone()));
                }
                None => {
                    count.refused = Some(reason);
                    count.unnamed = invalid;
                }
            }
            census.change(&count, true)?;
            counts = counts.inserted((provincial, key), count)?;
        }
        for &token in names.edits.iter() {
            let token = token as u32;
            let was_absent =
                old.is_some_and(|old| old.names.values.get(&(token as u128)).is_none());
            let absent = names.values.get(&(token as u128)).is_none();
            if was_absent == absent {
                continue;
            }
            let count = by_token_row
                .iter_from(&(token, false, 0))
                .take_while(|(key, _)| key.0 == token)
                .count();
            absent_tokens = if absent && count != 0 {
                absent_tokens.inserted(token, count)?
            } else {
                absent_tokens.removed(&token)?
            };
        }
        let mut by_country_key =
            old.map_or_else(ValueIndex::default, |old| old.by_country_key.clone());
        for &key in changed_keys.keys() {
            if let Some(row) = values.get(&key) {
                by_country_key = by_country_key.removed(&(row.country, key))?;
            }
            values = match by_candidate
                .last_at_or_before(&(key, u128::MAX))
                .filter(|(candidate, _)| candidate.0 == key)
            {
                Some((_, row)) => values.inserted(key, *row)?,
                None => values.removed(&key)?,
            };
            if let Some(row) = values.get(&key) {
                by_country_key = by_country_key.inserted((row.country, key), ())?;
            }
        }
        census.apply(assets, summary);
        let selected = match old {
            Some(old) => old.selected.clone(),
            None => Retained::new(assets.city_templates()),
        };
        let city = old.and_then(|old| old.city.as_ref()).map(|old| City {
            provinces: old.provinces.clone(),
            by_state_province: old.by_state_province.clone(),
            available: old.available.clone(),
            health: old.health.clone(),
        });
        Ok(Self {
            branch: frame.branch,
            campaign: frame.tables[TableId::Campaign.index()].identity(),
            rows: current,
            names,
            lexical_complete,
            types: tokens.clone(),
            by_token_row,
            absent_tokens,
            by_candidate,
            values,
            by_country_key,
            counts,
            invalid_names,
            census,
            selected,
            city,
        })
    }
    pub(super) fn city_damage(
        &mut self,
        frame: &DecodedFrame,
        assets: &MapBuildingAssets,
    ) -> io::Result<super::super::sprawl::Damage> {
        let validate = (|| {
            frame.complete::<tables::StateBuildings>()?;
            frame.complete::<tables::ProvinceBuildings>()?;
            frame.complete::<tables::LexicalTokens>()?;
            if let Some((_, reason)) = self.invalid_names.iter().next() {
                return Err(invalid(reason.to_string()));
            }
            core::<tables::Provinces>(frame)
        })();
        let provinces = match validate {
            Ok(rows) => rows,
            Err(error) => {
                self.city = None;
                return Ok(super::super::sprawl::Damage::from_fold(
                    ValueIndex::default(),
                    Some(error.to_string()),
                ));
            }
        };
        let selected = match &*self.selected {
            Ok(selected) => selected,
            Err(reason) => {
                self.city = None;
                return Ok(super::super::sprawl::Damage::from_fold(
                    ValueIndex::default(),
                    Some(reason.clone()),
                ));
            }
        };
        let old = self.city.as_ref();
        let provinces = RowFold::read(
            frame,
            TableId::Provinces,
            provinces,
            fields(TableId::Provinces, &["state"]),
            old.map(|old| &old.provinces),
            |row| Ok(row.state),
        )?;
        let mut by_state_province =
            old.map_or_else(ValueIndex::default, |old| old.by_state_province.clone());
        let mut dirty = ValueIndex::default();
        for &province in provinces.edits.iter() {
            if let Some(&state) = old.and_then(|old| old.provinces.values.get(&province)) {
                by_state_province = by_state_province.removed(&(state, province as u32))?;
            }
            if let Some(&state) = provinces.values.get(&province) {
                by_state_province = by_state_province.inserted((state, province as u32), ())?;
            }
            dirty = dirty.inserted(province as u32, ())?;
        }
        let mut changed_buildings = ValueIndex::default();
        let mut error = None;
        self.values.visit_changes(
            &old.map_or_else(ValueIndex::default, |old| old.available.clone()),
            |key, before, after| {
                if selected.contains(&key.2)
                    && before.map(|row| (row.level, row.healthy_level, row.partial_health))
                        != after.map(|row| (row.level, row.healthy_level, row.partial_health))
                    && error.is_none()
                {
                    match changed_buildings.inserted(*key, ()) {
                        Ok(next) => changed_buildings = next,
                        Err(why) => error = Some(why),
                    }
                }
            },
        );
        if let Some(error) = error {
            return Err(error);
        }
        for &(provincial, location, _) in changed_buildings.keys() {
            if provincial {
                dirty = dirty.inserted(location, ())?;
            } else {
                for (&(_, province), _) in by_state_province
                    .iter_from(&(location, 0))
                    .take_while(|(key, _)| key.0 == location)
                {
                    dirty = dirty.inserted(province, ())?;
                }
            }
        }
        let mut health = old.map_or_else(ValueIndex::default, |old| old.health.clone());
        for &province in dirty.keys() {
            let Some(&state) = provinces.values.get(&(province as u128)) else {
                health = health.removed(&province)?;
                continue;
            };
            let mut minimum = 100_000;
            if state != 0 {
                for &index in selected {
                    let provincial = assets.facts.ordered[index].provincial();
                    let location = if provincial { province } else { state };
                    if let Some(row) = self.values.get(&(provincial, location, index))
                        && row.level > 0
                    {
                        minimum = minimum.min(super::super::sprawl::average_health(
                            row.level,
                            row.healthy_level,
                            row.partial_health,
                        ));
                    }
                }
            }
            health = health.inserted(province, (state != 0).then_some(minimum))?;
        }
        self.city = Some(City {
            provinces,
            by_state_province,
            available: self.values.clone(),
            health: health.clone(),
        });
        Ok(super::super::sprawl::Damage::from_fold(health, None))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{LexicalTokenRow, ProvinceRow, TableRows};
    use replay_frame::frame::TableView;
    use viewer_foundation::test_support::{test_frame, test_put as put};

    fn assets() -> MapBuildingAssets {
        let types = ["infrastructure", "industrial_complex"].map(|name| BuildingType {
            name: name.to_owned(),
            show_on_map: Some(6),
            ..Default::default()
        });
        let types = Buildings {
            declaration_order: types.iter().map(|kind| kind.name.clone()).collect(),
            types: types
                .into_iter()
                .map(|kind| (kind.name.clone(), kind))
                .collect(),
            ..Default::default()
        };
        MapBuildingAssets::assemble(
            ModelLibrary::default(),
            &types,
            Defines {
                shrink_distance: 180.0,
                destroyed: 6_000_000,
                recovered: 8_000_000,
                damaged: 80_000,
                max_levels: 15,
                popup_speed: 5.0,
            },
            slot_groups(std::iter::empty(), &BTreeMap::new()),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
    }
    #[test]
    fn token_reasons_follow_only_referenced_keys_and_release_unused_literals() {
        use replay_frame::frame::{FrameDecoder, FrameEncoder, HourInput};
        let mut source = test_frame(1);
        let mut lexical: Vec<_> = (1..=1501).map(|token| LexicalTokenRow {
            token, literal: format!("unused_{token}").into_bytes(),
        }).collect();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(&[TableId::LexicalTokens]).with_change_facts();
        let mut references = ValueIndex::default().inserted((700, false, 1), ()).unwrap();
        let mut previous = None;
        for step in 0..8 {
            source.hour = step + 1;
            let mut dirty = ValueIndex::default();
            match step {
                1 => lexical[1200].literal = b"irrelevant replacement".to_vec(),
                2 => {
                    references = references.inserted((1201, true, 2), ()).unwrap();
                    dirty = dirty.inserted(1201, ()).unwrap();
                }
                3 => lexical[699].literal = b"invalid\0literal".to_vec(),
                4 => {
                    references = references.removed(&(700, false, 1)).unwrap();
                    dirty = dirty.inserted(700, ()).unwrap();
                }
                5 => { lexical.remove(1200); }
                6 => lexical.push(LexicalTokenRow {
                    token: 1502, literal: b"late token".to_vec(),
                }),
                7 => {
                    references = references.inserted((1502, false, 3), ()).unwrap();
                    dirty = dirty.inserted(1502, ()).unwrap();
                }
                _ => {}
            }
            put(&mut source, TableRows::LexicalTokens(lexical.clone()));
            let update = encoder.encode(source.clone(), step == 0 || step == 6).unwrap();
            let frame = decoder.advance(&update).unwrap();
            let rows = rows::<tables::LexicalTokens>(frame).unwrap();
            let next = referenced_names(
                HourInput::new(frame), rows.rows(), rows.is_complete(), &references,
                dirty, previous.as_ref(),
            ).unwrap();
            let oracle = TokenNames::from_rows(&lexical, true);
            let expected: BTreeSet<_> = references.keys().filter_map(|&(token, _, _)| {
                lexical.binary_search_by_key(&token, |row| row.token).ok().map(|_| token)
            }).collect();
            assert_eq!(next.values.len(), expected.len());
            for token in expected {
                let actual = next.values.get(&u128::from(token)).unwrap();
                let expected = oracle.name(token);
                assert_eq!(actual.invalid, expected.is_err());
                let reason = match expected {
                    Ok(name) => format!("token {token} ({name}) names no authored building type"),
                    Err(error) => error.to_string(),
                };
                assert_eq!(&*actual.reason, &reason);
            }
            if step == 1 {
                assert!(next.edits.is_empty());
                assert!(next.values.ptr_eq(&previous.as_ref().unwrap().values));
            }
            if step >= 4 { assert!(next.values.get(&700).is_none()); }
            previous = Some(next);
        }
    }

    #[test]
    fn census_and_city_health_match_independent_rows_through_dictionary_and_source_gaps() {
        let assets = assets();
        let mut frame = test_frame(0);
        put(
            &mut frame,
            TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 1,
                    literal: b"infrastructure".to_vec(),
                },
                LexicalTokenRow {
                    token: 2,
                    literal: b"industrial_complex".to_vec(),
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 1,
                    state: 7,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 2,
                    state: 7,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 3,
                    state: 0,
                    ..Default::default()
                },
            ]),
        );
        put(&mut frame, TableRows::ProvinceBuildings(Vec::new()));
        let mut previous = None;
        for step in 0..7i16 {
            frame.hour += 1;
            put(
                &mut frame,
                TableRows::StateBuildings(vec![
                    BuildingRow {
                        location: 7,
                        building_type: 1,
                        level: 5,
                        healthy_level: step,
                        partial_health: if step == 3 { i64::MIN } else { 123 },
                        ..Default::default()
                    },
                    BuildingRow {
                        location: 7,
                        building_type: 2,
                        level: 3,
                        healthy_level: 2,
                        ..Default::default()
                    },
                    BuildingRow {
                        location: 8,
                        building_type: 9,
                        level: 3,
                        healthy_level: 1,
                        ..Default::default()
                    },
                ]),
            );
            if step == 2 {
                put(
                    &mut frame,
                    TableRows::LexicalTokens(vec![
                        LexicalTokenRow {
                            token: 1,
                            literal: b"INDUSTRIAL_COMPLEX".to_vec(),
                        },
                        LexicalTokenRow {
                            token: 2,
                            literal: b"infrastructure".to_vec(),
                        },
                        LexicalTokenRow {
                            token: 9,
                            literal: b"unknown".to_vec(),
                        },
                    ]),
                );
            }
            if step == 4 {
                frame.tables[TableId::StateBuildings.index()] = TableView::Truncated {
                    rows: TableRows::StateBuildings(
                        frame.complete::<tables::StateBuildings>().unwrap().to_vec(),
                    ),
                    missing: 1,
                }
                .into();
            }
            let tokens = assets.token_map(&frame).unwrap();
            let mut summary = MapBuildingSummary::default();
            let mut next =
                AvailableRows::read(&assets, &frame, &tokens, previous.as_ref(), &mut summary)
                    .unwrap();
            let mut available = BTreeMap::new();
            let mut expected = MapBuildingSummary::default();
            let names = TokenNames::new(&frame).unwrap();
            for (provincial, rows) in [
                (
                    false,
                    rows::<tables::StateBuildings>(&frame).unwrap().rows(),
                ),
                (
                    true,
                    rows::<tables::ProvinceBuildings>(&frame).unwrap().rows(),
                ),
            ] {
                for row in rows.iter().filter(|row| row.building_type != 0) {
                    match tokens.get(&row.building_type) {
                        Some(Ok(index)) => {
                            let kind = &assets.facts.ordered[*index];
                            available.insert((provincial, row.location, *index), row);
                            if slots_for_level(kind, row.level, assets.defines.max_levels) != 0 {
                                expected.built += 1;
                                expected.missing_slots += 1;
                                *expected
                                    .missing_by_type
                                    .entry(kind.name.clone())
                                    .or_default() += 1;
                                let (destroyed, damaged, uncertain) = health_state(
                                    row.level,
                                    row.healthy_level,
                                    row.partial_health,
                                    &assets.defines,
                                );
                                expected.destroyed += usize::from(destroyed);
                                expected.damaged += usize::from(damaged);
                                expected.uncertain_damage += usize::from(uncertain);
                            }
                        }
                        Some(Err(reason)) => {
                            *expected.refused_tokens.entry(reason.clone()).or_default() += 1;
                        }
                        None => {
                            let reason = match names.name(row.building_type) {
                                Ok(spelling) => format!(
                                    "token {} ({spelling}) names no authored building type",
                                    row.building_type
                                ),
                                Err(error) => {
                                    expected.unnamed_tokens += 1;
                                    error.to_string()
                                }
                            };
                            *expected.refused_tokens.entry(reason).or_default() += 1;
                        }
                    }
                }
            }
            assert_eq!(summary, expected);
            let actual = next.city_damage(&frame, &assets).unwrap();
            let oracle = super::super::super::sprawl::Damage::read(
                &frame,
                &assets.facts.ordered,
                &available,
            );
            assert_eq!(actual, oracle);
            let quiet = AvailableRows::read(
                &assets,
                &frame,
                &tokens,
                Some(&next),
                &mut MapBuildingSummary::default(),
            )
            .unwrap();
            assert!(quiet.values.ptr_eq(&next.values));
            previous = Some(next);
        }
    }
}
