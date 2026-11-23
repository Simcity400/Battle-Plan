
#[cfg(test)]
pub(super) mod tests;

use viewer_layer_core::layers::{Carried, Settling, Dirty, Inputs, Layer, Num, Text, View};
use viewer_layer_core::playhead_host::{mirror::{Mirror, Mirrors}};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};
use viewer_runtime::resource_origins::{
    self,
    Definition,
    Folds,
    INFRASTRUCTURE_TOKEN,
    Input,
    Origin,
    ResourceOperands,
    Values,
    building_grants_from,
    grant_lists,
    grant_of,
    resource_operands,
};
use viewer_map_icons::resource_icons::{HourIcons, Laid, laid_icon};

const WHOLE: [TableId; 13] = [
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::ResourceOrigins,
    TableId::CountryOilFactors,
    TableId::StateBuildings,
    TableId::TradeRoutes,
    TableId::States,
    TableId::ResourceRights,
    TableId::StateTemporaryResources,
    TableId::StateResourceInputs,
    TableId::StateResources,
    TableId::StateBaseResources,
];

const BY_STATE: [TableId; 7] = [
    TableId::ResourceOrigins,
    TableId::ResourceRights,
    TableId::StateTemporaryResources,
    TableId::StateResources,
    TableId::StateBaseResources,
    TableId::StateBuildings,
    TableId::StateResourceInputs,
];

#[derive(Clone, Copy)]
struct Columns {
    link_slot: Num<u32>,
    literal: Text,
    definition_token: Num<u32>,
    definition_active: Num<u8>,
    controller: Num<u32>,
    origin_list: Num<u8>,
    origin_efficiency: Num<i64>,
    origin_route: Num<i64>,
    origin_local: Num<i64>,
    right_holder: Num<u32>,
    oil_factor: Num<i64>,
    temporary: Num<i64>,
    input_gate: Num<u8>,
    input_global: Num<i64>,
    input_origin: Num<u8>,
    input_level: Num<i32>,
    input_modifier: Num<i64>,
    stored: [Num<i64>; 4],
    base: Num<i64>,
    level: Num<i16>,
    healthy: Num<i16>,
    override_building: Num<u32>,
    override_count: Num<u32>,
    override_resource: Num<u32>,
    override_amount: Num<i64>,
}

impl Columns {
    fn of(&self, table: TableId) -> Vec<usize> {
        let numbers: Vec<(TableId, usize)> = vec![
            (self.link_slot.table, self.link_slot.column),
            (self.definition_token.table, self.definition_token.column),
            (self.definition_active.table, self.definition_active.column),
            (self.controller.table, self.controller.column),
            (self.origin_list.table, self.origin_list.column),
            (self.origin_efficiency.table, self.origin_efficiency.column),
            (self.origin_route.table, self.origin_route.column),
            (self.origin_local.table, self.origin_local.column),
            (self.right_holder.table, self.right_holder.column),
            (self.oil_factor.table, self.oil_factor.column),
            (self.temporary.table, self.temporary.column),
            (self.input_gate.table, self.input_gate.column),
            (self.input_global.table, self.input_global.column),
            (self.input_origin.table, self.input_origin.column),
            (self.input_level.table, self.input_level.column),
            (self.input_modifier.table, self.input_modifier.column),
            (self.stored[0].table, self.stored[0].column),
            (self.stored[1].table, self.stored[1].column),
            (self.stored[2].table, self.stored[2].column),
            (self.stored[3].table, self.stored[3].column),
            (self.base.table, self.base.column),
            (self.level.table, self.level.column),
            (self.healthy.table, self.healthy.column),
            (self.override_building.table, self.override_building.column),
            (self.override_count.table, self.override_count.column),
            (self.override_resource.table, self.override_resource.column),
            (self.override_amount.table, self.override_amount.column),
            (self.literal.table, self.literal.column),
        ];
        numbers.into_iter().filter(|&(held, _)| held == table).map(|(_, column)| column).collect()
    }
}

#[derive(PartialEq)]
struct Shared {
    definitions: Result<(Vec<Definition>, Option<usize>), String>,
    grants: Result<BTreeMap<u32, Result<Vec<(u32, i64)>, String>>, String>,
    oil_factors: HashMap<u32, i64>,
    overrides: HashMap<u32, Vec<(u32, u32, u32, u32, i64)>>,
}

impl Shared {
    fn unread() -> Self {
        Self { definitions: Err(String::new()), grants: Err(String::new()), oil_factors: HashMap::new(), overrides: HashMap::new() }
    }
}

pub trait Source: Send + Sync {
    fn origins(&self) -> Option<&resource_origins::Content>;
    fn localisation(&self) -> &content_reader_localisation::localisation::Localisation;
}


pub struct ResourceIcons {
    content: Arc<dyn Source>,
    columns: Option<Columns>,
    shared: Shared,
    folds: Folds,
    spelling: HashMap<Vec<u8>, BTreeMap<u32, Vec<u8>>>,
    spelled_at: HashMap<u32, Vec<u8>>,
    origins: HashMap<u32, Vec<(u128, u32)>>,
    buildings: HashMap<u32, Vec<(u128, u32)>>,
    origin_keys: HashMap<u32, u128>,
    building_keys: HashMap<u32, u128>,
    whole: Result<(), String>,
    icons: BTreeMap<u32, Result<Option<Laid>, String>>,
    quiet: std::collections::HashSet<u32>,
    numbers: HashMap<u32, resource_origins::StateIcon>,
    relink: bool,
    statuses: HashMap<TableId, Option<Status>>,
    states: Dirty,
    refold: bool,
    hidden: Option<&'static str>,
    frame: u64,
    marked: u64,
    version: u64,
}

const NAME: &str = "resource_icons";
const NOT_DRAWN: &str = "no resource icon is drawn at this view";
const NO_ORIGINS: &str = "the content's resource origins are not loaded";

pub(super) fn next_version() -> u64 {
    static VERSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    VERSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

impl ResourceIcons {
    pub fn new(content: Arc<dyn Source>) -> Self {
        Self {
            content,
            columns: None,
            shared: Shared::unread(),
            folds: Folds::new(),
            spelling: HashMap::new(),
            spelled_at: HashMap::new(),
            origins: HashMap::new(),
            buildings: HashMap::new(),
            origin_keys: HashMap::new(),
            building_keys: HashMap::new(),
            whole: Ok(()),
            icons: BTreeMap::new(),
            numbers: HashMap::new(),
            quiet: std::collections::HashSet::new(),
            relink: false,
            statuses: HashMap::new(),
            states: Dirty::default(),
            refold: true,
            hidden: None,
            frame: 0,
            marked: 0,
            version: 0,
        }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, hidden: self.hidden, ..Self::new(Arc::clone(&self.content)) }
    }

    pub fn hidden(&self) -> Option<&'static str> {
        self.hidden
    }

    fn release(&mut self) {
        let empty = Self::new(Arc::clone(&self.content));
        let kept = std::mem::replace(self, empty);
        (self.hidden, self.frame, self.marked, self.version) = (kept.hidden, kept.frame, kept.marked, next_version());
    }

    pub fn marked(&self) -> u64 {
        self.marked
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn retained_icons(&self) -> usize {
        self.icons.len()
    }

    pub fn icons(&self) -> HourIcons {
        if let Some(why) = self.hidden {
            return Err(why.to_owned());
        }
        self.whole.clone().map(|()| self.icons.clone())
    }

    fn is_quiet(&self, table: TableId, key: u128) -> bool {
        table == TableId::StateBuildings && self.quiet.contains(&(key as u32))
    }

    fn mark(&mut self, state: u32) {
        if self.states.mark(state, self.frame) {
            self.marked += 1;
        }
    }

    fn mark_all(&mut self, mirrors: &Mirrors) {
        let states: Vec<u32> = self.icons.keys().copied().chain(mirrors.get(TableId::States).into_iter().flat_map(|mirror| mirror.rows().filter_map(|slot| mirror.key(slot)).map(|key| key as u32).collect::<Vec<_>>())).collect();
        for state in states {
            self.mark(state);
        }
    }

    fn names(&self) -> Vec<Vec<u8>> {
        let Some(origins) = self.content.origins() else { return Vec::new() };
        let mut names: Vec<Vec<u8>> = origins.buildings.types.iter().flat_map(|(name, building)| std::iter::once(name.as_str()).chain(building.local_resources.iter().map(|(resource, _)| resource.as_str()))).map(|name| name.as_bytes().to_ascii_lowercase()).collect();
        names.sort();
        names.dedup();
        names
    }

    fn spell_all(&mut self, mirrors: &Mirrors) {
        let columns = self.columns.expect("declared");
        self.spelling = self.names().into_iter().map(|name| (name, BTreeMap::new())).collect();
        self.spelled_at.clear();
        if let Some(mirror) = mirrors.get(TableId::LexicalTokens) {
            for slot in mirror.rows() {
                self.spell(mirror, columns, slot);
            }
        }
        self.folds = self.spelling.keys().filter_map(|name| fold_of(&self.spelling, name).map(|entry| (name.clone(), entry))).collect();
    }

    fn spell(&mut self, mirror: &Mirror, columns: Columns, slot: u32) {
        let (Some(key), Some(literal)) = (mirror.key(slot), mirror.text(columns.literal, slot).value().and_then(|id| mirror.string(id))) else { return };
        let folded = literal.to_ascii_lowercase();
        if let Some(tokens) = self.spelling.get_mut(&folded) {
            tokens.insert(key as u32, literal.to_vec());
            self.spelled_at.insert(slot, folded);
        }
    }

    fn fold_shared(&mut self, mirrors: &Mirrors) {
        let Some(columns) = self.columns else { return };
        let content = Arc::clone(&self.content);
        let Some(origins) = content.origins() else { return };
        let link = |raw: u32| slot_of(mirrors, columns, raw);
        let spelled = |token: u32| mirrors.get(TableId::LexicalTokens).and_then(|mirror| mirror.slot(u128::from(token)).and_then(|slot| mirror.text(columns.literal, slot).value()).and_then(|id| mirror.string(id)));
        let definition_rows: Vec<(u32, u32, u8)> = mirrors
            .get(TableId::ResourceDefinitions)
            .map(|mirror| mirror.rows().filter_map(|slot| Some((mirror.key(slot)? as u32, mirror.number(columns.definition_token, slot).value()?, mirror.number(columns.definition_active, slot).value().unwrap_or(0)))).collect())
            .unwrap_or_default();
        let definitions = resource_origins::definitions(origins, definition_rows.iter().map(|&(id, token, _)| (id, token)), spelled);
        let active: HashMap<u32, u8> = definition_rows.iter().map(|&(_, token, active)| (token, active)).collect();
        let folds = &self.folds;
        let grants = grant_lists(&origins.buildings, |token| active.get(&token).is_some_and(|&active| active != 0), |name: &[u8]| match folds.get(&name.to_ascii_lowercase()) {
            None => Ok(None),
            Some(Ok(token)) => Ok(Some(*token)),
            Some(Err(reason)) => Err(reason.clone()),
        });
        let mut oil_factors = HashMap::new();
        if let Some(mirror) = mirrors.get(TableId::CountryOilFactors) {
            for (key, slot) in by_key(mirror) {
                if let (Some(country), Some(factor)) = (link(key as u32), mirror.number(columns.oil_factor, slot).value()) {
                    oil_factors.insert(country, factor);
                }
            }
        }
        let mut overrides: HashMap<u32, Vec<(u32, u32, u32, u32, i64)>> = HashMap::new();
        if let Some(mirror) = mirrors.get(TableId::CountryBuildingResources) {
            for (key, slot) in by_key(mirror) {
                if let Some(country) = link((key >> 32) as u32) {
                    let number = |column: Num<u32>| mirror.number(column, slot).value().unwrap_or(0);
                    overrides.entry(country).or_default().push((key as u32, number(columns.override_building), number(columns.override_count), number(columns.override_resource), mirror.number(columns.override_amount, slot).value().unwrap_or(0)));
                }
            }
            for rows in overrides.values_mut() {
                rows.sort_by_key(|row| row.0);
            }
        }
        self.quiet = grants.as_ref().map_or_else(|_| Default::default(), |grants| grants.iter().filter(|(token, list)| **token != INFRASTRUCTURE_TOKEN && list.as_ref().is_ok_and(Vec::is_empty)).map(|(token, _)| *token).collect());
        let shared = Shared { definitions, grants, oil_factors, overrides };
        if shared.definitions != self.shared.definitions || shared.grants != self.shared.grants {
            self.shared = shared;
            self.mark_all(mirrors);
            return;
        }
        let differs = |old: &HashMap<u32, _>, new: &HashMap<u32, _>, slot: &u32| old.get(slot) != new.get(slot);
        let mut countries: Vec<u32> = self.shared.oil_factors.keys().chain(shared.oil_factors.keys()).filter(|slot| differs(&self.shared.oil_factors, &shared.oil_factors, slot)).copied().collect();
        countries.extend(self.shared.overrides.keys().chain(shared.overrides.keys()).filter(|slot| self.shared.overrides.get(slot) != shared.overrides.get(slot)).copied());
        self.shared = shared;
        if countries.is_empty() {
            return;
        }
        let Some(states) = mirrors.get(TableId::States) else { return };
        let touched: Vec<u32> = states.rows().filter(|&slot| states.number(columns.controller, slot).value().and_then(|raw| slot_of(mirrors, columns, raw)).is_some_and(|slot| countries.contains(&slot))).filter_map(|slot| states.key(slot)).map(|key| key as u32).collect();
        for state in touched {
            self.mark(state);
        }
    }

    fn whole_of(&self, mirrors: &Mirrors) -> Result<(), String> {
        for table in WHOLE {
            complete(mirrors, table)?;
            if table == TableId::ResourceDefinitions {
                self.shared.definitions.as_ref().map_err(Clone::clone)?;
            }
        }
        Ok(())
    }

    fn index(&mut self, mirrors: &Mirrors) {
        self.origins.clear();
        self.buildings.clear();
        self.origin_keys.clear();
        self.building_keys.clear();
        for (table, by_state, keys) in [(TableId::ResourceOrigins, &mut self.origins, &mut self.origin_keys), (TableId::StateBuildings, &mut self.buildings, &mut self.building_keys)] {
            let Some(mirror) = mirrors.get(table) else { continue };
            for (key, slot) in by_key(mirror) {
                by_state.entry((key >> 32) as u32).or_default().push((key, slot));
                keys.insert(slot, key);
            }
        }
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors) {
        self.frame += 1;
        if self.hidden.is_some() {
            return;
        }
        if std::mem::take(&mut self.relink) {
            self.numbers.clear();
            self.mark_all(mirrors);
        }
        if std::mem::take(&mut self.refold) {
            self.fold_shared(mirrors);
            let whole = self.whole_of(mirrors);
            if whole != self.whole {
                self.whole = whole;
                self.version = next_version();
                self.mark_all(mirrors);
            }
        }
        let Some(columns) = self.columns else { return };
        let content = Arc::clone(&self.content);
        let Some(origins) = content.origins() else { return };
        let localisation = content.localisation();
        let mut dirty = std::mem::take(&mut self.states);
        let mut changed = false;
        let _ = dirty.settle(|state| {
            let live = mirrors.get(TableId::States).and_then(|mirror| mirror.slot(u128::from(state))).is_some();
            if !live || self.whole.is_err() {
                changed |= self.icons.remove(&state).is_some();
                self.numbers.remove(&state);
                return Ok(());
            }
            let at = At { layer: self, mirrors, columns, content: origins, buildings: std::cell::OnceCell::new() };
            let laid = match resource_origins::icon(&at, state) {
                Ok(Some(icon)) if self.numbers.get(&state) == Some(&icon) => return Ok(()),
                Ok(Some(icon)) => {
                    let laid = laid_icon(&at, localisation, &icon);
                    self.numbers.insert(state, icon);
                    Ok(Some(laid))
                }
                other => {
                    self.numbers.remove(&state);
                    other.map(|_| None)
                }
            };
            self.icons.insert(state, laid);
            changed = true;
            Ok(())
        });
        self.states = dirty;
        if changed {
            self.version = next_version();
        }
    }
}

pub fn drawn<'a>(side: viewer_layer_core::playhead_host::LayerParts<'a>) -> Option<&'a ResourceIcons> {
    if !side.draws(NAME) {
        return None;
    }
    let layer = side.parts()?.1.iter().find_map(|layer| layer.any().downcast_ref::<ResourceIcons>())?;
    layer.hidden.is_none().then_some(layer)
}

fn by_key(mirror: &Mirror) -> Vec<(u128, u32)> {
    let mut rows: Vec<(u128, u32)> = mirror.rows().filter_map(|slot| Some((mirror.key(slot)?, slot))).collect();
    rows.sort_unstable();
    rows
}

fn slot_of(mirrors: &Mirrors, columns: Columns, raw: u32) -> Option<u32> {
    let mirror = mirrors.get(TableId::CountryLinks)?;
    mirror.number(columns.link_slot, mirror.slot(u128::from(raw))?).value()
}

fn fold_of(spelling: &HashMap<Vec<u8>, BTreeMap<u32, Vec<u8>>>, name: &[u8]) -> Option<Result<u32, String>> {
    let mut folds = Folds::new();
    for (&token, literal) in spelling.get(name)? {
        resource_origins::fold(&mut folds, token, literal);
    }
    folds.into_values().next()
}

pub(super) fn complete(mirrors: &Mirrors, table: TableId) -> Result<(), String> {
    let mirror = mirrors.get(table);
    let hour = mirror.and_then(Mirror::head).map_or_else(|| "?".to_owned(), |hour| hour.to_string());
    match mirror.and_then(Mirror::status) {
        Some(Status::Complete) => Ok(()),
        Some(Status::Truncated { missing }) => Err(format!("hour {hour} {table:?} is truncated ({missing} missing rows)")),
        Some(Status::Unavailable { .. }) => Err(format!("hour {hour} {table:?} is unavailable: {}", mirror.and_then(Mirror::reason).unwrap_or_default())),
        Some(Status::NotRecorded) | None => Err(format!(
            "hour {hour} {table:?} is not recorded in a {} recording",
            mirror.and_then(Mirror::capture).map_or_else(|| "?".to_owned(), |mode| mode.to_string())
        )),
    }
}

struct At<'a> {
    layer: &'a ResourceIcons,
    mirrors: &'a Mirrors,
    columns: Columns,
    content: &'a resource_origins::Content,
    buildings: std::cell::OnceCell<Vec<(u32, i16)>>,
}

impl At<'_> {
    fn row(&self, table: TableId, key: u128) -> Option<(&Mirror, u32)> {
        let mirror = self.mirrors.get(table)?;
        Some((mirror, mirror.slot(key)?))
    }

    fn number<T: viewer_layer_core::layers::Number>(&self, table: TableId, key: u128, column: Num<T>) -> Option<T> {
        let (mirror, slot) = self.row(table, key)?;
        mirror.number(column, slot).value()
    }
}

fn pair(state: u32, other: u32) -> u128 {
    (u128::from(state) << 32) | u128::from(other)
}

impl Values for At<'_> {
    fn content(&self) -> &resource_origins::Content {
        self.content
    }

    fn definitions(&self) -> &[Definition] {
        self.layer.shared.definitions.as_ref().map_or(&[], |(definitions, _)| definitions)
    }

    fn oil(&self) -> Option<usize> {
        self.layer.shared.definitions.as_ref().ok().and_then(|(_, oil)| *oil)
    }

    fn slot(&self, raw: u32) -> Option<u32> {
        slot_of(self.mirrors, self.columns, raw)
    }

    fn controller(&self, state: u32) -> Option<u32> {
        self.number(TableId::States, u128::from(state), self.columns.controller)
    }

    fn origin(&self, state: u32, list: u8, slot: u32) -> Option<Origin> {
        let mirror = self.mirrors.get(TableId::ResourceOrigins)?;
        let c = self.columns;
        self.layer.origins.get(&state)?.iter().find_map(|&(key, row)| {
            let country = key as u32;
            let row_list = mirror.number(c.origin_list, row).value()?;
            (row_list == list && self.slot(country) == Some(slot)).then(|| Origin {
                state,
                country,
                list: row_list,
                efficiency: mirror.number(c.origin_efficiency, row).value().unwrap_or(0),
                route_factor: mirror.number(c.origin_route, row).value().unwrap_or(0),
                local_resources_factor: mirror.number(c.origin_local, row).value().unwrap_or(0),
                bad: false,
                convoys_assigned: 0,
                convoys_required: 0,
            })
        })
    }

    fn right(&self, state: u32, index: u32) -> Option<u32> {
        self.number(TableId::ResourceRights, pair(state, index), self.columns.right_holder)
    }

    fn oil_factor(&self, slot: u32) -> Option<i64> {
        self.layer.shared.oil_factors.get(&slot).copied()
    }

    fn temporary(&self, state: u32, token: u32) -> Option<i64> {
        self.number(TableId::StateTemporaryResources, pair(state, token), self.columns.temporary)
    }

    fn input(&self, state: u32) -> Option<Input> {
        let (mirror, slot) = self.row(TableId::StateResourceInputs, u128::from(state))?;
        let c = self.columns;
        Some(Input {
            gate: mirror.number(c.input_gate, slot).value().unwrap_or(0),
            origin: mirror.number(c.input_origin, slot).value().unwrap_or(0),
            infrastructure_level: mirror.number(c.input_level, slot).value(),
            infrastructure_modifier: mirror.number(c.input_modifier, slot).value().unwrap_or(0),
        })
    }

    fn stored(&self, state: u32, token: u32) -> Option<(i64, i64)> {
        let (mirror, slot) = self.row(TableId::StateResources, pair(state, token))?;
        let [pool, _, _, cost] = self.columns.stored.map(|column| mirror.number(column, slot).value().unwrap_or(0));
        Some((pool, cost))
    }

    fn listed(&self, state: u32, token: u32) -> bool {
        self.row(TableId::StateBaseResources, pair(state, token)).is_some()
    }

    fn infrastructure(&self, state: u32) -> Option<(i16, i16)> {
        let (mirror, slot) = self.row(TableId::StateBuildings, pair(state, INFRASTRUCTURE_TOKEN))?;
        Some((mirror.number(self.columns.level, slot).value().unwrap_or(0), mirror.number(self.columns.healthy, slot).value().unwrap_or(0)))
    }

    fn operands(&self, state: u32, token: u32, controller: u32) -> Result<ResourceOperands, String> {
        let c = self.columns;
        let (inputs, slot) = self.row(TableId::StateResourceInputs, u128::from(state)).ok_or_else(|| format!("state {state} has no recorded resource inputs"))?;
        let stored = self.row(TableId::StateResources, pair(state, token)).map(|(mirror, row)| c.stored.map(|column| mirror.number(column, row).value().unwrap_or(0)));
        let template = self.number(TableId::StateBaseResources, pair(state, token), c.base).unwrap_or(0);
        let grants = self.layer.shared.grants.as_ref().map_err(Clone::clone)?;
        let buildings = self.buildings.get_or_init(|| match (self.mirrors.get(TableId::StateBuildings), self.layer.buildings.get(&state)) {
            (Some(mirror), Some(rows)) => rows.iter().map(|&(key, row)| (key as u32, mirror.number(c.healthy, row).value().unwrap_or(0))).collect(),
            _ => Vec::new(),
        });
        let overrides = self.layer.shared.overrides.get(&controller).map_or(&[][..], Vec::as_slice);
        let total = building_grants_from(buildings, overrides, token, |building| {
            let list = grants.get(&building).ok_or_else(|| format!("state {state} has a building type the content lacks"))?.as_ref().map_err(Clone::clone)?;
            Ok::<_, String>(grant_of(list, token))
        })?;
        let number = |column| inputs.number(column, slot).value();
        let origin = number(c.input_origin).unwrap_or(0) != 0;
        Ok(resource_operands(
            template,
            stored,
            number(c.input_gate).unwrap_or(0) != 0,
            inputs.number(c.input_global, slot).value().unwrap_or(0),
            origin.then(|| (inputs.number(c.input_level, slot).value().map(|level| (level, inputs.number(c.input_modifier, slot).value().unwrap_or(0))), false)),
            self.content.bonuses,
            total,
        ))
    }
}

impl Layer for ResourceIcons {
    fn name(&self) -> &'static str {
        NAME
    }

    fn placed(&mut self, _view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        let hidden = if !viewer_map_modes::map_mode::visibility::of(&view.map_mode).resource_icons {
            Some(NOT_DRAWN)
        } else if self.content.origins().is_none() {
            Some(NO_ORIGINS)
        } else {
            None
        };
        if hidden.is_some() {
            self.hidden = hidden;
            self.release();
            return Ok(());
        }
        self.hidden = None;
        for table in WHOLE.into_iter().chain([TableId::CountryBuildingResources]) {
            inputs.table(table, true)?;
        }
        self.columns = Some(Columns {
            link_slot: inputs.number(TableId::CountryLinks, "canonical_slot")?,
            literal: inputs.text(TableId::LexicalTokens, "literal")?,
            definition_token: inputs.number(TableId::ResourceDefinitions, "token")?,
            definition_active: inputs.number(TableId::ResourceDefinitions, "active")?,
            controller: inputs.number(TableId::States, "controller")?,
            origin_list: inputs.number(TableId::ResourceOrigins, "list")?,
            origin_efficiency: inputs.number(TableId::ResourceOrigins, "efficiency")?,
            origin_route: inputs.number(TableId::ResourceOrigins, "route_factor")?,
            origin_local: inputs.number(TableId::ResourceOrigins, "local_resources_factor")?,
            right_holder: inputs.number(TableId::ResourceRights, "holder")?,
            oil_factor: inputs.number(TableId::CountryOilFactors, "factor")?,
            temporary: inputs.number(TableId::StateTemporaryResources, "amount")?,
            input_gate: inputs.number(TableId::StateResourceInputs, "gate")?,
            input_global: inputs.number(TableId::StateResourceInputs, "global_factor")?,
            input_origin: inputs.number(TableId::StateResourceInputs, "origin")?,
            input_level: inputs.number(TableId::StateResourceInputs, "infrastructure_level")?,
            input_modifier: inputs.number(TableId::StateResourceInputs, "infrastructure_modifier")?,
            stored: [inputs.number(TableId::StateResources, "pool")?, inputs.number(TableId::StateResources, "additive")?, inputs.number(TableId::StateResources, "factor")?, inputs.number(TableId::StateResources, "cost")?],
            base: inputs.number(TableId::StateBaseResources, "amount")?,
            level: inputs.number(TableId::StateBuildings, "level")?,
            healthy: inputs.number(TableId::StateBuildings, "healthy_level")?,
            override_building: inputs.number(TableId::CountryBuildingResources, "building")?,
            override_count: inputs.number(TableId::CountryBuildingResources, "count")?,
            override_resource: inputs.number(TableId::CountryBuildingResources, "resource")?,
            override_amount: inputs.number(TableId::CountryBuildingResources, "amount")?,
        });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        if self.hidden.is_some() {
            self.release();
            return Ok(());
        }
        let marked = self.marked;
        self.icons.clear();
        self.numbers.clear();
        self.states.clear();
        self.shared = Shared::unread();
        self.whole = Err(String::new());
        self.statuses = WHOLE.into_iter().chain([TableId::CountryBuildingResources]).map(|table| (table, mirrors.get(table).and_then(Mirror::status))).collect();
        if self.columns.is_some() {
            self.spell_all(mirrors);
            self.index(mirrors);
            self.refold = true;
            self.mark_all(mirrors);
            self.settle_dirty(mirrors);
        }
        self.marked = marked;
        self.version = next_version();
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        let Some(columns) = self.columns else { return };
        if step.gap() {
            self.refold = true;
            return;
        }
        for table in WHOLE.into_iter().chain([TableId::CountryBuildingResources]) {
            let Some(changes) = step.table(table.index()) else { continue };
            if let Some((status, _)) = changes.status() {
                let held = self.statuses.insert(table, Some(status)).flatten();
                if held.is_none_or(|held| std::mem::discriminant(&held) != std::mem::discriminant(&status)) {
                    self.refold = true;
                }
            }
            let read = columns.of(table);
            let written = changes.columns().iter().any(|written| read.contains(&written.column()) && !written.slots().is_empty());
            if changes.deleted().slots().is_empty() && changes.inserted().slots().is_empty() && !written {
                continue;
            }
            match table {
                TableId::CountryLinks => {
                    self.refold = true;
                    self.relink = true;
                }
                TableId::ResourceDefinitions | TableId::CountryOilFactors | TableId::CountryBuildingResources => self.refold = true,
                TableId::LexicalTokens => {
                    let mirror = mirrors.get(table);
                    let mut changed = Vec::new();
                    for (at, &slot) in changes.deleted().slots().iter().enumerate() {
                        if let Some(folded) = self.spelled_at.remove(&slot) {
                            let key = changes.deleted().keys().and_then(|keys| keys.get(at)).copied();
                            if let (Some(key), Some(tokens)) = (key, self.spelling.get_mut(&folded)) {
                                tokens.remove(&(key as u32));
                            }
                            changed.push(folded);
                        }
                    }
                    if let Some(mirror) = mirror {
                        for &slot in changes.inserted().slots() {
                            self.spell(mirror, columns, slot);
                            if let Some(folded) = self.spelled_at.get(&slot) {
                                changed.push(folded.clone());
                            }
                        }
                    }
                    let spelled = self.shared.definitions.as_ref().ok().is_some_and(|(definitions, _)| {
                        changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).any(|&key| definitions.iter().any(|definition| u128::from(definition.token) == key))
                    });
                    self.refold |= spelled || !changed.is_empty();
                    for folded in changed {
                        match fold_of(&self.spelling, &folded) {
                            Some(entry) => self.folds.insert(folded, entry),
                            None => self.folds.remove(&folded),
                        };
                    }
                }
                TableId::States => {
                    for &key in changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()) {
                        self.mark(key as u32);
                    }
                    if let Some(mirror) = mirrors.get(table) {
                        for written in changes.columns().iter().filter(|written| written.column() == columns.controller.column) {
                            for &slot in written.slots() {
                                if let Some(key) = mirror.key(slot) {
                                    self.mark(key as u32);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            if BY_STATE.contains(&table) {
                let state_of = |key: u128| if table == TableId::StateResourceInputs { key as u32 } else { (key >> 32) as u32 };
                let deleted: Vec<(u32, u128)> = changes.deleted().slots().iter().copied().zip(changes.deleted().keys().unwrap_or_default().iter().copied()).collect();
                let inserted: Vec<(u32, u128)> = changes.inserted().slots().iter().copied().zip(changes.inserted().keys().unwrap_or_default().iter().copied()).collect();
                for &(slot, key) in &deleted {
                    if !self.is_quiet(table, key) {
                        self.mark(state_of(key));
                    }
                    let (by_state, keys) = match table {
                        TableId::ResourceOrigins => (&mut self.origins, &mut self.origin_keys),
                        TableId::StateBuildings => (&mut self.buildings, &mut self.building_keys),
                        _ => continue,
                    };
                    keys.remove(&slot);
                    if let Some(rows) = by_state.get_mut(&state_of(key)) {
                        rows.retain(|&(held, _)| held != key);
                    }
                }
                for &(slot, key) in &inserted {
                    if !self.is_quiet(table, key) {
                        self.mark(state_of(key));
                    }
                    let (by_state, keys) = match table {
                        TableId::ResourceOrigins => (&mut self.origins, &mut self.origin_keys),
                        TableId::StateBuildings => (&mut self.buildings, &mut self.building_keys),
                        _ => continue,
                    };
                    keys.insert(slot, key);
                    let rows = by_state.entry(state_of(key)).or_default();
                    let at = rows.partition_point(|&(held, _)| held < key);
                    rows.insert(at, (key, slot));
                }
                if let Some(mirror) = mirrors.get(table) {
                    for written in changes.columns().iter().filter(|written| read.contains(&written.column())) {
                        for &slot in written.slots() {
                            if let Some(key) = mirror.key(slot)
                                && !self.is_quiet(table, key)
                            {
                                self.mark(state_of(key));
                            }
                        }
                    }
                }
            }
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors);
        Ok(())
    }


    fn carried(&self) -> Carried {
        Carried { entities: self.states.len() }
    }

    fn held(&self) -> usize {
        let laid: usize = self.icons.values().map(|icon| match icon {
            Ok(Some(laid)) => laid.rows.as_ref().map_or(0, |rows| rows.iter().map(|row| row.text.capacity() + 16).sum()) + 64,
            _ => 32,
        }).sum();
        let lists = |map: &HashMap<u32, Vec<(u128, u32)>>| map.values().map(|rows| rows.capacity() * 24 + 32).sum::<usize>();
        laid + lists(&self.origins) + lists(&self.buildings) + (self.origin_keys.len() + self.building_keys.len()) * 24 + self.states.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}
