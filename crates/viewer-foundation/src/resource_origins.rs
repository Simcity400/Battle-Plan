//! What the resources map mode reads of a state's resource origins, from one recorded hour: which
//! states carry a resource icon, whose origin each resource reads under resource rights, and each
//! resource's nominal and delivered amounts (the icon's `MAX` and `VAL`, the hover's nominal and
//! delivered figures).
//!
//! A state's resources are read through its controller's own origin (`ResourceOrigins` list 0).
//! A resource whose rights the controller gave away (`ResourceRights`) reads the holder's origin
//! for the state instead (list 1). Per resource, with every product fixed5 and truncated:
//!
//! - nominal = (grants x oil + full x infrastructure x route) x local - cost
//! - delivered = max(0, (grants x oil + current x infrastructure x route) x local
//!   - current x damage x local + temporary) x efficiency - cost
//!
//! `full` is the state's base amount ([`base_amount`]), `current` the same clamped at zero,
//! `grants` the state's building grants under the controller's overrides, `oil` one plus the
//! controller's `production_oil_factor` for the content's `NGame.OIL_RESOURCE` (one otherwise),
//! `infrastructure` the controller's infrastructure factor ([`infrastructure_factor`]), `route`,
//! `local` (one plus `local_resources_factor`) and `efficiency` the origin's own, `damage` the
//! infrastructure building's damaged levels times `INFRASTRUCTURE_RESOURCE_BONUS`, `temporary`
//! the state's temporary resources and `cost` its resource cost.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use dashboard_text::state_resources::{
    Links, ONE, StateResources, base_amount, infrastructure_factor, resource_bonuses,
};
use installed_content_core::{ResolvedLayers, buildings::Buildings, defines::Defines};
use replay_data::{
    rows::ResourceOriginRow, rows::StateResourceInputRow, rows::StateResourceRow,
    rows::TableSchema, rows::tables,
};
use replay_frame::frame::DecodedFrame;

/// What the origins read of the recorded content stack.
pub struct Content {
    /// Every resource declaration in load order, which is the resource database's order:
    /// declaration `i` is database index `i + 1`.
    pub resources: Vec<installed_content_core::resources::Resource>,
    /// Building types, for their `local_resources_<resource>` grants.
    pub buildings: Buildings,
    /// `NBuildings` `INFRASTRUCTURE_RESOURCE_BONUS` and `SUPPLY_ROUTE_RESOURCE_BONUS`, fixed5.
    pub bonuses: [i64; 2],
    /// `NGame.OIL_RESOURCE`: the resource `production_oil_factor` scales.
    pub oil: String,
    /// What the resource files misread, for the warning area.
    pub warnings: Vec<String>,
}

impl Content {
    pub fn load(layers: &ResolvedLayers, defines: &Defines) -> io::Result<Self> {
        let (resources, notes) = installed_content_core::resources::read(layers)?;
        Ok(Self {
            resources,
            buildings: installed_content_core::buildings::read(layers)?,
            bonuses: resource_bonuses(defines)?,
            oil: defines.text("NGame", "OIL_RESOURCE")?,
            warnings: notes.into_iter().map(|(_, note)| note).collect(),
        })
    }
}

/// One recorded resource joined to its declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    /// The one-based database index, which `ResourceRights` names.
    pub index: u32,
    /// The resource's lexer token, which the state tables name.
    pub token: u32,
    /// The declared key.
    pub name: String,
    /// The resource strip's one-based frame, `None` when the declaration has none.
    pub icon_frame: Option<u32>,
}

impl Definition {
    /// `PRODUCTION_MATERIALS_` and the upper-cased key: the resource's name.
    pub fn name_key(&self) -> String {
        format!(
            "PRODUCTION_MATERIALS_{}",
            installed_content_localisation::localisation::crt_case_text(&self.name, true, false)
        )
    }
}

/// The operands of one resource read through one origin, all fixed5.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Operands {
    /// The state's base amount, unclamped.
    pub full: i64,
    /// The building grants under the controller's overrides: the origin's building pool.
    pub grants: i64,
    /// One plus the controller's `production_oil_factor` for the oil resource, else one.
    pub oil: i64,
    pub infrastructure: i64,
    /// The origin's supply-route factor.
    pub route: i64,
    /// One plus the origin's `local_resources_factor`.
    pub local: i64,
    pub cost: i64,
    /// The damaged infrastructure levels times `INFRASTRUCTURE_RESOURCE_BONUS`.
    pub damage: i64,
    pub temporary: i64,
    /// The origin's stored efficiency.
    pub efficiency: i64,
}

fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

/// The nominal amount: the icon's `MAX`.
pub fn nominal(operands: &Operands) -> i64 {
    let o = operands;
    let scaled = mul(mul(o.infrastructure, o.full), o.route);
    mul(mul(o.grants, o.oil).wrapping_add(scaled), o.local).wrapping_sub(o.cost)
}

/// The delivered amount: the icon's `VAL`.
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

/// A fixed5 value to the nearest whole number, halves away from zero, as the icon and the hover
/// print it.
pub fn whole(value: i64) -> i32 {
    dashboard_text::triggers::fixed_round(value) as i32
}

/// One resource of a state, read through the origin its rights choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Amounts<'a> {
    /// Index into [`Hour::definitions`].
    pub definition: usize,
    pub nominal: i64,
    pub delivered: i64,
    pub operands: Operands,
    /// The origin read: the controller's own, or the rights holder's.
    pub origin: &'a ResourceOriginRow,
    /// The rights holder's raw country reference, when it is not the controller.
    pub holder: Option<u32>,
    /// The infrastructure building's damaged levels (level minus healthy level).
    pub damaged: i32,
}

/// A state's controller and its own origin for the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controlled<'a> {
    /// Raw country reference and canonical slot.
    pub controller: u32,
    pub slot: u32,
    /// `None` when the controller holds no origin for the state.
    pub origin: Option<&'a ResourceOriginRow>,
}

/// A lowercase spelling to its lexer token, or why several tokens share it.
type Folds = BTreeMap<Vec<u8>, Result<u32, String>>;

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

/// One recorded hour's resource origins and the state operands they read.
pub struct Hour<'a> {
    content: &'a Content,
    links: BTreeMap<u32, u32>,
    folds: Folds,
    resources: StateResources<'a>,
    definitions: Vec<Definition>,
    /// Index into `definitions` of the oil resource, if the database has it.
    oil: Option<usize>,
    controllers: BTreeMap<u32, u32>,
    /// By state, in table order.
    origins: BTreeMap<u32, Vec<&'a ResourceOriginRow>>,
    /// (state, database index) to the holder's raw reference.
    rights: BTreeMap<(u32, u32), u32>,
    /// By canonical slot.
    oil_factors: BTreeMap<u32, i64>,
    /// (state, token).
    temporary: BTreeMap<(u32, u32), i64>,
    inputs: BTreeMap<u32, &'a StateResourceInputRow>,
    stored: BTreeMap<(u32, u32), &'a StateResourceRow>,
    /// (state, token) the state's template lists.
    listed: BTreeSet<(u32, u32)>,
    /// By state: the infrastructure building's level and healthy level.
    infrastructure: BTreeMap<u32, (i16, i16)>,
    /// (canonical slot, state) to the origin route's kind.
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
            let folded = row.literal.to_ascii_lowercase();
            let held = folds.get(&folded).cloned();
            match held {
                None => {
                    folds.insert(folded, Ok(row.token));
                }
                Some(Ok(token)) if token != row.token => {
                    let reason = format!(
                        "{} folds to several tokens",
                        String::from_utf8_lossy(&row.literal)
                    );
                    folds.insert(folded, Err(reason));
                }
                Some(_) => {}
            }
        }
        let mut definitions = Vec::new();
        let mut rows: Vec<_> = table::<tables::ResourceDefinitions>(frame)?
            .iter()
            .collect();
        rows.sort_by_key(|row| row.id);
        for row in rows {
            let declared = (row.id as usize)
                .checked_sub(1)
                .and_then(|at| content.resources.get(at))
                .ok_or_else(|| {
                    format!(
                        "recorded resource {} has no declaration in the content's resource files",
                        row.id
                    )
                })?;
            let spelling = spelled.get(&row.token).ok_or_else(|| {
                format!(
                    "recorded resource {}'s token {} is not spelled",
                    row.id, row.token
                )
            })?;
            if !spelling.eq_ignore_ascii_case(declared.token.as_bytes()) {
                return Err(format!(
                    "recorded resource {} is {}, but declaration {} of the content is {}",
                    row.id,
                    String::from_utf8_lossy(spelling),
                    row.id,
                    declared.token
                ));
            }
            definitions.push(Definition {
                index: row.id,
                token: row.token,
                name: declared.token.clone(),
                icon_frame: declared.icon_frame,
            });
        }
        let oil = definitions
            .iter()
            .position(|definition| definition.name.eq_ignore_ascii_case(&content.oil));
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
        let infrastructure_token = engine_native::cbuilding_database::INFRASTRUCTURE_TOKEN;
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

    /// The recorded resources in database order.
    pub fn definitions(&self) -> &[Definition] {
        &self.definitions
    }

    /// Every recorded state, in id order.
    pub fn states(&self) -> impl Iterator<Item = u32> + '_ {
        self.controllers.keys().copied()
    }

    /// A raw country reference's canonical slot.
    pub fn slot(&self, raw: u32) -> Result<u32, String> {
        self.links
            .get(&raw)
            .copied()
            .ok_or_else(|| format!("CountryLinks lacks {raw}"))
    }

    fn joins(&self) -> Joins<'_> {
        Joins {
            links: &self.links,
            folds: &self.folds,
        }
    }

    /// The state's controller and the controller's own origin for it.
    pub fn controlled(&self, state: u32) -> Result<Controlled<'a>, String> {
        let controller = *self
            .controllers
            .get(&state)
            .ok_or_else(|| format!("state {state} has no States row"))?;
        let slot = self.slot(controller)?;
        let origin = self
            .origins
            .get(&state)
            .into_iter()
            .flatten()
            .copied()
            .find(|origin| origin.list == 0 && self.links.get(&origin.country) == Some(&slot));
        Ok(Controlled {
            controller,
            slot,
            origin,
        })
    }

    /// Whether the state carries a resource icon: some resource its template lists, of a
    /// positive pool, with a building pool above zero in the controller's origin, or of a
    /// non-zero cost; or its additive gate is set.
    pub fn has_icon(&self, state: u32) -> Result<bool, String> {
        for definition in &self.definitions {
            let key = (state, definition.token);
            let stored = self.stored.get(&key);
            if self.listed.contains(&key) || stored.is_some_and(|row| row.pool > 0 || row.cost != 0)
            {
                return Ok(true);
            }
        }
        if self.definitions.is_empty() {
            return Ok(false);
        }
        let input = self
            .inputs
            .get(&state)
            .ok_or_else(|| format!("state {state} has no recorded resource inputs"))?;
        if input.gate != 0 {
            return Ok(true);
        }
        let controlled = self.controlled(state)?;
        if controlled.origin.is_none() {
            return Ok(false);
        }
        for definition in &self.definitions {
            let operands = self
                .resources
                .operands(
                    state,
                    definition.token,
                    Some(controlled.slot),
                    &self.joins(),
                )
                .map_err(|why| why.to_string())?;
            if operands.grants > 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The origin the resource reads: the controller's own, or, for a resource whose rights the
    /// controller gave to another country, that holder's origin for the state.
    fn origin_for(
        &self,
        state: u32,
        definition: &Definition,
        controlled: &Controlled<'a>,
    ) -> Result<(&'a ResourceOriginRow, Option<u32>), String> {
        let own = controlled
            .origin
            .ok_or_else(|| format!("state {state}'s controller holds no resource origin for it"))?;
        let Some(&holder) = self.rights.get(&(state, definition.index)) else {
            return Ok((own, None));
        };
        let holder_slot = self.slot(holder)?;
        if holder_slot == controlled.slot {
            return Ok((own, None));
        }
        let origin = self
            .origins
            .get(&state)
            .into_iter()
            .flatten()
            .copied()
            .find(|origin| {
                origin.list == 1 && self.links.get(&origin.country) == Some(&holder_slot)
            })
            .ok_or_else(|| {
                format!(
                    "state {state}'s {} rights holder {holder} has no recorded origin for it",
                    definition.name
                )
            })?;
        Ok((origin, Some(holder)))
    }

    /// One resource of the state, through the origin its rights choose. The state's controller
    /// must hold an origin for it ([`Hour::controlled`]).
    pub fn amounts(&self, state: u32, definition: usize) -> Result<Amounts<'a>, String> {
        let controlled = self.controlled(state)?;
        let found = &self.definitions[definition];
        let (origin, holder) = self.origin_for(state, found, &controlled)?;
        let state_operands = self
            .resources
            .operands(state, found.token, Some(controlled.slot), &self.joins())
            .map_err(|why| why.to_string())?;
        let input = self
            .inputs
            .get(&state)
            .ok_or_else(|| format!("state {state} has no recorded resource inputs"))?;
        if input.origin == 0 {
            return Err(format!(
                "state {state}'s infrastructure level was not read: its resource inputs record no controller origin, which ResourceOrigins holds"
            ));
        }
        let bonus = self.content.bonuses[0];
        let damaged = self
            .infrastructure
            .get(&state)
            .map_or(0, |&(level, healthy)| i32::from(level) - i32::from(healthy));
        let oil = if self.oil == Some(definition) {
            ONE.wrapping_add(self.oil_factors.get(&controlled.slot).copied().unwrap_or(0))
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
            temporary: self
                .temporary
                .get(&(state, found.token))
                .copied()
                .unwrap_or(0),
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

    /// Whether the state's template lists the resource.
    pub fn listed(&self, state: u32, definition: usize) -> bool {
        self.listed
            .contains(&(state, self.definitions[definition].token))
    }

    /// Whether the state reads temporary resources and holds some of `definition`'s.
    pub fn temporary(&self, state: u32, definition: usize) -> Option<i64> {
        self.temporary
            .get(&(state, self.definitions[definition].token))
            .copied()
    }

    /// The kind of `origin`'s route (`TradeRoutes` class 0 of its list owner and state): 0 none,
    /// 1 land, 2 naval, 3 other.
    pub fn route_kind(&self, origin: &ResourceOriginRow) -> Result<u8, String> {
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

/// `TradeRoutes`' class of a resource origin's route.
pub const ORIGIN_ROUTE_CLASS: u8 = 0;

/// What one state's resource icon shows: its rows (resources of a positive nominal amount, in
/// database order) and the flags of resource rights.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateIcon {
    pub rows: Vec<IconRow>,
    /// The holder every row's rights go to, when that is one country other than the
    /// controller: the window's own flag, and then no row has one.
    pub flag: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconRow {
    /// Index into [`Hour::definitions`].
    pub definition: usize,
    pub nominal: i64,
    pub delivered: i64,
    /// The row's own flag: its rights holder's raw reference, when the window shows none.
    pub flag: Option<u32>,
}

/// The colour code a row's value is written in: red below the nominal amount (or below
/// zero), green above it, none at it.
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

    /// The colour code's letter.
    pub fn code(self) -> Option<char> {
        match self {
            Self::Red => Some('R'),
            Self::Green => Some('G'),
            Self::Plain => None,
        }
    }
}

impl Hour<'_> {
    /// The state's icon, `Ok(None)` for a state without one. A state with an icon whose
    /// controller holds no origin for it is refused: the game's value would read a missing
    /// origin.
    pub fn icon(&self, state: u32) -> Result<Option<StateIcon>, String> {
        if !self.has_icon(state)? {
            return Ok(None);
        }
        let controlled = self.controlled(state)?;
        if controlled.origin.is_none() {
            return Err(format!(
                "state {state} has resources but its controller holds no resource origin for it"
            ));
        }
        let mut rows = Vec::new();
        let mut holders = Vec::new();
        for definition in 0..self.definitions.len() {
            let amounts = self.amounts(state, definition)?;
            if amounts.nominal <= 0 {
                continue;
            }
            let holder = match amounts.holder {
                Some(holder) => Some((self.slot(holder)?, holder)),
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
}

#[cfg(any(test, feature = "test-support"))]
pub mod fixture;

#[cfg(test)]
mod tests;
