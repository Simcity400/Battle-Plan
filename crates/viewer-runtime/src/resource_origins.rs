
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use dashboard_text::state_resources::{
    Links, ONE, StateResources, base_amount, infrastructure_factor, resource_bonuses,
};
pub use dashboard_text::state_resources::{
    ResourceOperands, building_grants_from, grant_lists, grant_of, resource_operands,
};
pub const INFRASTRUCTURE_TOKEN: u32 = engine_native_parts::cbuilding_database::INFRASTRUCTURE_TOKEN;
use content_reader_layers::ResolvedLayers;
use content_reader_key_names::buildings::Buildings;
use content_reader_defines::defines::Defines;
use replay_data::{
    rows::ResourceOriginRow, rows::StateResourceInputRow, rows::StateResourceRow,
    rows::TableSchema, rows::tables,
};
use replay_frame::frame::DecodedFrame;

pub struct Content {
    pub resources: Vec<content_reader_key_names::resources::Resource>,
    pub buildings: Buildings,
    pub bonuses: [i64; 2],
    pub oil: String,
    pub warnings: Vec<String>,
}

impl Content {
    pub fn load(layers: &ResolvedLayers, defines: &Defines) -> io::Result<Self> {
        let (resources, notes) = content_reader_key_names::resources::read(layers)?;
        if let Some(gap) = resources.gap() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, gap.to_string()));
        }
        Ok(Self {
            resources: resources.declarations,
            buildings: content_reader_key_names::buildings::read(layers)?,
            bonuses: resource_bonuses(defines)?,
            oil: defines.text("NGame", "OIL_RESOURCE")?,
            warnings: notes.into_iter().map(|(_, note)| note).collect(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub index: u32,
    pub token: u32,
    pub name: String,
    pub icon_frame: Option<u32>,
}

impl Definition {
    pub fn name_key(&self) -> String {
        format!(
            "PRODUCTION_MATERIALS_{}",
            content_reader_localisation::localisation::crt_case_text(&self.name, true, false)
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Operands {
    pub full: i64,
    pub grants: i64,
    pub oil: i64,
    pub infrastructure: i64,
    pub route: i64,
    pub local: i64,
    pub cost: i64,
    pub damage: i64,
    pub temporary: i64,
    pub efficiency: i64,
}

fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

pub fn nominal(operands: &Operands) -> i64 {
    let o = operands;
    let scaled = mul(mul(o.infrastructure, o.full), o.route);
    mul(mul(o.grants, o.oil).wrapping_add(scaled), o.local).wrapping_sub(o.cost)
}

pub fn delivered(operands: &Operands) -> i64 {
    let o = operands;
    let current = o.full.max(0);
    let scaled = mul(mul(o.infrastructure, current), o.route);
    let produced = mul(mul(o.grants, o.oil).wrapping_add(scaled), o.local)
        .wrapping_sub(mul(mul(current, o.damage), o.local))
        .wrapping_add(o.temporary)
        .max(0);
    mul(produced, o.efficiency).wrapping_sub(o.cost)
}

pub fn whole(value: i64) -> i32 {
    dashboard_text::triggers::fixed_round(value) as i32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    pub state: u32,
    pub country: u32,
    pub list: u8,
    pub efficiency: i64,
    pub route_factor: i64,
    pub local_resources_factor: i64,
    pub bad: bool,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
}

impl From<&ResourceOriginRow> for Origin {
    fn from(row: &ResourceOriginRow) -> Self {
        Self {
            state: row.state,
            country: row.country,
            list: row.list,
            efficiency: row.efficiency,
            route_factor: row.route_factor,
            local_resources_factor: row.local_resources_factor,
            bad: row.bad,
            convoys_assigned: row.convoys_assigned,
            convoys_required: row.convoys_required,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Input {
    pub gate: u8,
    pub origin: u8,
    pub infrastructure_level: Option<i32>,
    pub infrastructure_modifier: i64,
}

impl From<&StateResourceInputRow> for Input {
    fn from(row: &StateResourceInputRow) -> Self {
        Self {
            gate: row.gate,
            origin: row.origin,
            infrastructure_level: row.infrastructure_level,
            infrastructure_modifier: row.infrastructure_modifier,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Amounts {
    pub definition: usize,
    pub nominal: i64,
    pub delivered: i64,
    pub operands: Operands,
    pub origin: Origin,
    pub holder: Option<u32>,
    pub damaged: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controlled {
    pub controller: u32,
    pub slot: u32,
    pub origin: Option<Origin>,
}

pub trait Values {
    fn content(&self) -> &Content;
    fn definitions(&self) -> &[Definition];
    fn oil(&self) -> Option<usize>;
    fn slot(&self, raw: u32) -> Option<u32>;
    fn controller(&self, state: u32) -> Option<u32>;
    fn origin(&self, state: u32, list: u8, slot: u32) -> Option<Origin>;
    fn right(&self, state: u32, index: u32) -> Option<u32>;
    fn oil_factor(&self, slot: u32) -> Option<i64>;
    fn temporary(&self, state: u32, token: u32) -> Option<i64>;
    fn input(&self, state: u32) -> Option<Input>;
    fn stored(&self, state: u32, token: u32) -> Option<(i64, i64)>;
    fn listed(&self, state: u32, token: u32) -> bool;
    fn infrastructure(&self, state: u32) -> Option<(i16, i16)>;
    fn operands(&self, state: u32, token: u32, controller: u32) -> Result<ResourceOperands, String>;
}

fn slot_of(values: &dyn Values, raw: u32) -> Result<u32, String> {
    values.slot(raw).ok_or_else(|| format!("CountryLinks lacks {raw}"))
}

pub fn controlled(values: &dyn Values, state: u32) -> Result<Controlled, String> {
    let controller = values
        .controller(state)
        .ok_or_else(|| format!("state {state} has no States row"))?;
    let slot = slot_of(values, controller)?;
    Ok(Controlled {
        controller,
        slot,
        origin: values.origin(state, 0, slot),
    })
}

pub fn has_icon(values: &dyn Values, state: u32) -> Result<bool, String> {
    let definitions = values.definitions();
    for definition in definitions {
        if values.listed(state, definition.token)
            || values
                .stored(state, definition.token)
                .is_some_and(|(pool, cost)| pool > 0 || cost != 0)
        {
            return Ok(true);
        }
    }
    if definitions.is_empty() {
        return Ok(false);
    }
    let input = values
        .input(state)
        .ok_or_else(|| format!("state {state} has no recorded resource inputs"))?;
    if input.gate != 0 {
        return Ok(true);
    }
    let controlled = controlled(values, state)?;
    if controlled.origin.is_none() {
        return Ok(false);
    }
    for definition in definitions {
        if values.operands(state, definition.token, controlled.slot)?.grants > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

fn origin_for(
    values: &dyn Values,
    state: u32,
    definition: &Definition,
    controlled: &Controlled,
) -> Result<(Origin, Option<u32>), String> {
    let own = controlled
        .origin
        .ok_or_else(|| format!("state {state}'s controller holds no resource origin for it"))?;
    let Some(holder) = values.right(state, definition.index) else {
        return Ok((own, None));
    };
    let holder_slot = slot_of(values, holder)?;
    if holder_slot == controlled.slot {
        return Ok((own, None));
    }
    let origin = values.origin(state, 1, holder_slot).ok_or_else(|| {
        format!(
            "state {state}'s {} rights holder {holder} has no recorded origin for it",
            definition.name
        )
    })?;
    Ok((origin, Some(holder)))
}

pub fn amounts(values: &dyn Values, state: u32, definition: usize) -> Result<Amounts, String> {
    amounts_for(values, state, definition, &controlled(values, state)?)
}

fn amounts_for(
    values: &dyn Values,
    state: u32,
    definition: usize,
    controlled: &Controlled,
) -> Result<Amounts, String> {
    let found = &values.definitions()[definition];
    let (origin, holder) = origin_for(values, state, found, controlled)?;
    let state_operands = values.operands(state, found.token, controlled.slot)?;
    let input = values
        .input(state)
        .ok_or_else(|| format!("state {state} has no recorded resource inputs"))?;
    if input.origin == 0 {
        return Err(format!(
            "state {state}'s infrastructure level was not read: its resource inputs record no controller origin, which ResourceOrigins holds"
        ));
    }
    let bonus = values.content().bonuses[0];
    let damaged = values
        .infrastructure(state)
        .map_or(0, |(level, healthy)| i32::from(level) - i32::from(healthy));
    let oil = if values.oil() == Some(definition) {
        ONE.wrapping_add(values.oil_factor(controlled.slot).unwrap_or(0))
    } else {
        ONE
    };
    let operands = Operands {
        full: base_amount(&state_operands),
        grants: state_operands.grants,
        oil,
        infrastructure: infrastructure_factor(
            input
                .infrastructure_level
                .map(|level| (level, input.infrastructure_modifier)),
            bonus,
        ),
        route: origin.route_factor,
        local: ONE.wrapping_add(origin.local_resources_factor),
        cost: state_operands.cost,
        damage: i64::from(damaged).wrapping_mul(bonus),
        temporary: values.temporary(state, found.token).unwrap_or(0),
        efficiency: origin.efficiency,
    };
    Ok(Amounts {
        definition,
        nominal: nominal(&operands),
        delivered: delivered(&operands),
        operands,
        origin,
        holder,
        damaged,
    })
}

pub fn icon(values: &dyn Values, state: u32) -> Result<Option<StateIcon>, String> {
    if !has_icon(values, state)? {
        return Ok(None);
    }
    let controlled = controlled(values, state)?;
    if controlled.origin.is_none() {
        return Err(format!(
            "state {state} has resources but its controller holds no resource origin for it"
        ));
    }
    let mut rows = Vec::new();
    let mut holders = Vec::new();
    for definition in 0..values.definitions().len() {
        let amounts = amounts_for(values, state, definition, &controlled)?;
        if amounts.nominal <= 0 {
            continue;
        }
        let holder = match amounts.holder {
            Some(holder) => Some((slot_of(values, holder)?, holder)),
            None => None,
        };
        holders.push(holder);
        rows.push(IconRow {
            definition,
            nominal: amounts.nominal,
            delivered: amounts.delivered,
            flag: holder.map(|(_, raw)| raw),
        });
    }
    let flag = match holders.first() {
        Some(Some((slot, raw)))
            if holders
                .iter()
                .all(|holder| holder.is_some_and(|(other, _)| other == *slot)) =>
        {
            Some(*raw)
        }
        _ => None,
    };
    if flag.is_some() {
        for row in &mut rows {
            row.flag = None;
        }
    }
    Ok(Some(StateIcon { rows, flag }))
}

pub type Folds = BTreeMap<Vec<u8>, Result<u32, String>>;

pub fn fold(folds: &mut Folds, token: u32, literal: &[u8]) {
    let folded = literal.to_ascii_lowercase();
    match folds.get(&folded) {
        None => {
            folds.insert(folded, Ok(token));
        }
        Some(Ok(held)) if *held != token => {
            let reason = format!("{} folds to several tokens", String::from_utf8_lossy(literal));
            folds.insert(folded, Err(reason));
        }
        Some(_) => {}
    }
}

pub fn definitions<'s>(
    content: &Content,
    rows: impl IntoIterator<Item = (u32, u32)>,
    spelled: impl Fn(u32) -> Option<&'s [u8]>,
) -> Result<(Vec<Definition>, Option<usize>), String> {
    let mut rows: Vec<(u32, u32)> = rows.into_iter().collect();
    rows.sort_by_key(|&(id, _)| id);
    let mut definitions = Vec::new();
    for (id, token) in rows {
        let declared = (id as usize)
            .checked_sub(1)
            .and_then(|at| content.resources.get(at))
            .ok_or_else(|| {
                format!(
                    "recorded resource {id} has no declaration in the content's resource files"
                )
            })?;
        let spelling = spelled(token)
            .ok_or_else(|| format!("recorded resource {id}'s token {token} is not spelled"))?;
        if !spelling.eq_ignore_ascii_case(declared.token.as_bytes()) {
            return Err(format!(
                "recorded resource {id} is {}, but declaration {id} of the content is {}",
                String::from_utf8_lossy(spelling),
                declared.token
            ));
        }
        definitions.push(Definition {
            index: id,
            token,
            name: declared.token.clone(),
            icon_frame: declared.icon_frame,
        });
    }
    let oil = definitions
        .iter()
        .position(|definition| definition.name.eq_ignore_ascii_case(&content.oil));
    Ok((definitions, oil))
}

struct Joins<'h> {
    links: &'h BTreeMap<u32, u32>,
    folds: &'h Folds,
}

impl Links for Joins<'_> {
    fn slot(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    fn token_of(&self, name: &[u8]) -> Result<Option<u32>, String> {
        match self.folds.get(&name.to_ascii_lowercase()) {
            None => Ok(None),
            Some(Ok(token)) => Ok(Some(*token)),
            Some(Err(reason)) => Err(reason.clone()),
        }
    }
}

fn table<T: TableSchema>(frame: &DecodedFrame) -> Result<&[T::Row], String> {
    frame.complete::<T>().map_err(|error| error.to_string())
}

pub struct Hour<'a> {
    content: &'a Content,
    links: BTreeMap<u32, u32>,
    folds: Folds,
    resources: StateResources<'a>,
    definitions: Vec<Definition>,
    oil: Option<usize>,
    controllers: BTreeMap<u32, u32>,
    origins: BTreeMap<u32, Vec<&'a ResourceOriginRow>>,
    rights: BTreeMap<(u32, u32), u32>,
    oil_factors: BTreeMap<u32, i64>,
    temporary: BTreeMap<(u32, u32), i64>,
    inputs: BTreeMap<u32, &'a StateResourceInputRow>,
    stored: BTreeMap<(u32, u32), &'a StateResourceRow>,
    listed: BTreeSet<(u32, u32)>,
    infrastructure: BTreeMap<u32, (i16, i16)>,
    routes: BTreeMap<(u32, u32), u8>,
}

impl<'a> Hour<'a> {
    pub fn new(frame: &'a DecodedFrame, content: &'a Content) -> Result<Self, String> {
        let links: BTreeMap<u32, u32> = table::<tables::CountryLinks>(frame)?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let mut folds = Folds::new();
        let mut spelled = BTreeMap::new();
        for row in table::<tables::LexicalTokens>(frame)? {
            spelled.insert(row.token, row.literal.as_slice());
            fold(&mut folds, row.token, &row.literal);
        }
        let (definitions, oil) = definitions(
            content,
            table::<tables::ResourceDefinitions>(frame)?
                .iter()
                .map(|row| (row.id, row.token)),
            |token| spelled.get(&token).copied(),
        )?;
        let mut origins: BTreeMap<u32, Vec<&ResourceOriginRow>> = BTreeMap::new();
        for row in table::<tables::ResourceOrigins>(frame)? {
            origins.entry(row.state).or_default().push(row);
        }
        let mut oil_factors = BTreeMap::new();
        for row in table::<tables::CountryOilFactors>(frame)? {
            if let Some(&slot) = links.get(&row.country) {
                oil_factors.insert(slot, row.factor);
            }
        }
        let infrastructure_token = INFRASTRUCTURE_TOKEN;
        let infrastructure = table::<tables::StateBuildings>(frame)?
            .iter()
            .filter(|row| row.building_type == infrastructure_token)
            .map(|row| (row.location, (row.level, row.healthy_level)))
            .collect();
        let mut routes = BTreeMap::new();
        for row in table::<tables::TradeRoutes>(frame)? {
            if row.class == ORIGIN_ROUTE_CLASS
                && let Some(&slot) = links.get(&row.country)
            {
                routes.insert((slot, row.other), row.kind);
            }
        }
        Ok(Self {
            content,
            resources: StateResources::new(frame, &content.buildings, content.bonuses),
            controllers: table::<tables::States>(frame)?
                .iter()
                .map(|row| (row.id, row.controller))
                .collect(),
            origins,
            rights: table::<tables::ResourceRights>(frame)?
                .iter()
                .map(|row| ((row.state, row.resource), row.holder))
                .collect(),
            oil_factors,
            temporary: table::<tables::StateTemporaryResources>(frame)?
                .iter()
                .map(|row| ((row.state, row.resource), row.amount))
                .collect(),
            inputs: table::<tables::StateResourceInputs>(frame)?
                .iter()
                .map(|row| (row.state, row))
                .collect(),
            stored: table::<tables::StateResources>(frame)?
                .iter()
                .map(|row| ((row.state, row.resource), row))
                .collect(),
            listed: table::<tables::StateBaseResources>(frame)?
                .iter()
                .map(|row| (row.state, row.resource))
                .collect(),
            infrastructure,
            routes,
            links,
            folds,
            definitions,
            oil,
        })
    }

    pub fn definitions(&self) -> &[Definition] {
        &self.definitions
    }

    pub fn states(&self) -> impl Iterator<Item = u32> + '_ {
        self.controllers.keys().copied()
    }

    pub fn slot(&self, raw: u32) -> Result<u32, String> {
        slot_of(self, raw)
    }

    fn joins(&self) -> Joins<'_> {
        Joins {
            links: &self.links,
            folds: &self.folds,
        }
    }

    pub fn controlled(&self, state: u32) -> Result<Controlled, String> {
        controlled(self, state)
    }

    pub fn has_icon(&self, state: u32) -> Result<bool, String> {
        has_icon(self, state)
    }

    pub fn amounts(&self, state: u32, definition: usize) -> Result<Amounts, String> {
        amounts(self, state, definition)
    }

    pub fn icon(&self, state: u32) -> Result<Option<StateIcon>, String> {
        icon(self, state)
    }

    pub fn listed(&self, state: u32, definition: usize) -> bool {
        self.listed
            .contains(&(state, self.definitions[definition].token))
    }

    pub fn temporary(&self, state: u32, definition: usize) -> Option<i64> {
        self.temporary
            .get(&(state, self.definitions[definition].token))
            .copied()
    }

    pub fn route_kind(&self, origin: Origin) -> Result<u8, String> {
        let slot = self.slot(origin.country)?;
        self.routes
            .get(&(slot, origin.state))
            .copied()
            .ok_or_else(|| {
                format!(
                    "the route of country {}'s resource origin in state {} is not recorded",
                    origin.country, origin.state
                )
            })
    }
}

impl Values for Hour<'_> {
    fn content(&self) -> &Content {
        self.content
    }

    fn definitions(&self) -> &[Definition] {
        &self.definitions
    }

    fn oil(&self) -> Option<usize> {
        self.oil
    }

    fn slot(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    fn controller(&self, state: u32) -> Option<u32> {
        self.controllers.get(&state).copied()
    }

    fn origin(&self, state: u32, list: u8, slot: u32) -> Option<Origin> {
        self.origins
            .get(&state)
            .into_iter()
            .flatten()
            .find(|origin| origin.list == list && self.links.get(&origin.country) == Some(&slot))
            .map(|origin| Origin::from(*origin))
    }

    fn right(&self, state: u32, index: u32) -> Option<u32> {
        self.rights.get(&(state, index)).copied()
    }

    fn oil_factor(&self, slot: u32) -> Option<i64> {
        self.oil_factors.get(&slot).copied()
    }

    fn temporary(&self, state: u32, token: u32) -> Option<i64> {
        self.temporary.get(&(state, token)).copied()
    }

    fn input(&self, state: u32) -> Option<Input> {
        self.inputs.get(&state).map(|row| Input::from(*row))
    }

    fn stored(&self, state: u32, token: u32) -> Option<(i64, i64)> {
        self.stored.get(&(state, token)).map(|row| (row.pool, row.cost))
    }

    fn listed(&self, state: u32, token: u32) -> bool {
        self.listed.contains(&(state, token))
    }

    fn infrastructure(&self, state: u32) -> Option<(i16, i16)> {
        self.infrastructure.get(&state).copied()
    }

    fn operands(&self, state: u32, token: u32, controller: u32) -> Result<ResourceOperands, String> {
        self.resources
            .operands(state, token, Some(controller), &self.joins())
            .map_err(|why| why.to_string())
    }
}

pub fn state_amounts(
    frame: &DecodedFrame,
    content: &Content,
    state: u32,
) -> Result<Vec<(Definition, i64)>, String> {
    let links: BTreeMap<u32, u32> = table::<tables::CountryLinks>(frame)?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let mut folds = Folds::new();
    let mut spelled = BTreeMap::new();
    for row in table::<tables::LexicalTokens>(frame)? {
        spelled.insert(row.token, row.literal.as_slice());
        fold(&mut folds, row.token, &row.literal);
    }
    let (definitions, _) = definitions(
        content,
        table::<tables::ResourceDefinitions>(frame)?
            .iter()
            .map(|row| (row.id, row.token)),
        |token| spelled.get(&token).copied(),
    )?;
    let controller = table::<tables::States>(frame)?
        .iter()
        .find(|row| row.id == state)
        .map(|row| row.controller)
        .ok_or_else(|| format!("state {state} has no States row"))?;
    let controller = links
        .get(&controller)
        .copied()
        .ok_or_else(|| format!("CountryLinks lacks {controller}"))?;
    let resources = StateResources::new(frame, &content.buildings, content.bonuses);
    let joins = Joins {
        links: &links,
        folds: &folds,
    };
    definitions
        .into_iter()
        .map(|definition| {
            let amount = resources
                .amount(state, definition.token, Some(controller), &joins)
                .map_err(|why| why.to_string())?;
            Ok((definition, amount))
        })
        .collect()
}

pub const ORIGIN_ROUTE_CLASS: u8 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateIcon {
    pub rows: Vec<IconRow>,
    pub flag: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconRow {
    pub definition: usize,
    pub nominal: i64,
    pub delivered: i64,
    pub flag: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowColour {
    Red,
    Green,
    Plain,
}

impl RowColour {
    pub fn of(delivered: i64, nominal: i64) -> Self {
        if delivered < 0 || delivered < nominal {
            Self::Red
        } else if delivered > nominal {
            Self::Green
        } else {
            Self::Plain
        }
    }

    pub fn code(self) -> Option<char> {
        match self {
            Self::Red => Some('R'),
            Self::Green => Some('G'),
            Self::Plain => None,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod fixture;

#[cfg(test)]
mod tests;
