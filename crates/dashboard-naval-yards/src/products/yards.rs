//! Shared wire types and content-independent naming operands for the yard products.
mod changes;
#[cfg(test)]
pub(crate) mod reference;
pub(crate) mod management;
mod naming;
mod ship_names;
mod window;
use dashboard_base::{
    catalogue::{self, Catalogue, View},
    products::check_reply_budget,
};
pub use dashboard_keyed::query::params;
use dashboard_keyed::{
    Values,
    countries::Countries,
    query::{KeyedSeries, KeyedWindow, Query},
};
use dashboard_text::{
    scripted_text,
    variant_names::{self, RawVariant, Tags, Texts},
};
pub use naming::*;
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{self, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
use ts_rs::TS;
pub use window::*;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalYardsParams {
    pub branch_id: String,
    pub tag: String,
    #[ts(optional)]
    pub first_hour: Option<i32>,
    #[ts(optional)]
    pub last_hour: Option<i32>,
}
impl NavalYardsParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid naval yards branch")?;
        dashboard_core::protocol::validate_tag(&self.tag)?;
        if self
            .first_hour
            .zip(self.last_hour)
            .is_some_and(|(a, b)| a > b)
        {
            return Err("Invalid naval yards window");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct YardChoice {
    pub id: String,
    pub name: String,
}

/// A catalogue, not a moment's membership. The window dates every inclusion.
#[derive(Clone, Debug, Serialize, TS)]
pub struct YardGroup {
    pub country: String,
    pub id: String,
    pub name: String,
    pub ships: Vec<YardChoice>,
}

pub fn types() -> Vec<String> {
    vec![
        NavalYardsParams::decl(),
        YardChoice::decl(),
        YardGroup::decl(),
        KeyedWindow::decl(),
        KeyedSeries::decl(),
        dashboard_base::products::country_groups::HourSpan::decl(),
    ]
}

/// Naming operands and supplying dates persist; runtime projections stay source-bound.
#[derive(Default, Serialize, Deserialize)]
pub struct State {
    pub tokens: Vec<Operand>,
    #[serde(skip)]
    index: HashMap<String, usize>,
    #[serde(skip)]
    refusal_counts: HashMap<(String, usize), usize>,
    dates: BTreeMap<usize, i32>,
    last_hour: Option<i32>,
    #[serde(skip)]
    designs: Option<Result<HashMap<u64, Operand>, String>>,
    #[serde(skip)]
    management: management::Shared,
    #[serde(skip)]
    ships: HashMap<u64, ShipProjection>,
    #[serde(skip)]
    ship_views: Vec<replay_frame::frame::ViewIdentity>,
    #[serde(skip)]
    ship_refusals: Option<Vec<(usize, usize)>>,
    /// The last whole observation's projection, which the next hour's changes are found against.
    #[serde(skip)]
    engine: Option<Box<changes::Engine>>,
    /// About what tokens interned since the catalogue was last published hold.
    #[serde(skip)]
    unpublished: usize,
}

struct ShipProjection {
    values: Values,
    refusals: Vec<(usize, usize)>,
}

impl State {
    pub fn intern(&mut self, operand: Operand) -> usize {
        let key = serde_json::to_string(&operand).expect("serializable naming operand");
        self.intern_key(&key, || operand)
    }
    /// The token of the operand whose interning key is `key`, if interned.
    fn interned(&mut self, key: &str) -> Option<usize> {
        if self.index.len() != self.tokens.len() {
            self.index = self
                .tokens
                .iter()
                .enumerate()
                .map(|(id, operand)| {
                    (
                        serde_json::to_string(operand).expect("serializable naming operand"),
                        id,
                    )
                })
                .collect();
        }
        self.index.get(key).copied()
    }
    /// Intern the operand whose interning key is `key`.
    fn intern_key(&mut self, key: &str, operand: impl FnOnce() -> Operand) -> usize {
        if self.index.len() != self.tokens.len() {
            self.index = self
                .tokens
                .iter()
                .enumerate()
                .map(|(id, operand)| {
                    (
                        serde_json::to_string(operand).expect("serializable naming operand"),
                        id,
                    )
                })
                .collect();
        }
        if let Some(&id) = self.index.get(key) {
            return id;
        }
        let id = self.tokens.len();
        self.tokens.push(operand());
        self.index.insert(key.to_owned(), id);
        self.unpublished = self.unpublished.saturating_add(key.len());
        id
    }
    pub fn label(&mut self, values: &mut Values, key: String, operand: Operand) {
        let id = self.intern(operand);
        values.push(key, id as i128);
    }
    pub fn refuse(
        &mut self,
        values: &mut Values,
        owner: &str,
        why: impl Into<String>,
        count: usize,
    ) {
        if count == 0 {
            return;
        }
        let id = self.intern(Operand::Refusal(why.into()));
        if let Some(refusals) = &mut self.ship_refusals {
            refusals.push((id, count));
            return;
        }
        self.refusal(values, owner, id, count);
    }
    fn refusal(&mut self, values: &mut Values, owner: &str, id: usize, count: usize) {
        let count_value = count;
        let count = self
            .refusal_counts
            .entry((owner.to_owned(), id))
            .or_default();
        *count += count_value;
        values.push(format!("{owner}|refusal|{id}"), *count as i128);
    }
    /// Daily siblings have their own read channels, even on equal-value reads.
    pub fn sources(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        sources: &[TableId],
        owners: &[String],
        values: &mut Values,
    ) {
        self.refusal_counts.clear();
        self.date(format, frame, sources);
        for &table in sources {
            let known = known(frame, table);
            for owner in owners {
                values.push(format!("{owner}|known|{table:?}"), i128::from(known));
                if !known {
                    let reason = dashboard_base::products::table_missing_reason(frame, table)
                        .unwrap_or_else(|| "Table not recorded".into());
                    self.refuse(values, owner, format!("{table:?}: {reason}"), 1);
                }
                if known
                    && dated(table, format)
                    && let Some(&read) = self.dates.get(&table.index())
                {
                    values.push(format!("{owner}|read|{table:?}"), i128::from(read));
                }
            }
        }
    }
    /// Date each source's supplying read at the hour, as [`Self::sources`] does.
    fn date(&mut self, format: FormatVersion, frame: &DecodedFrame, sources: &[TableId]) {
        if !self
            .management
            .continues(self.last_hour, frame.branch, frame.hour)
        {
            self.dates.clear();
        }
        self.last_hour = Some(frame.hour);
        for &table in sources {
            if !known(frame, table) {
                self.dates.remove(&table.index());
            } else if dashboard_text::metadata::slot_read(format, &frame.metrics, table, frame.hour)
            {
                self.dates.insert(table.index(), frame.hour);
            } else if let Some(hour) = frame.tables[table.index()].sampled_at() {
                // A same-cadence sibling can witness an equal read later than
                // the table's own carried sample stamp.
                self.dates
                    .entry(table.index())
                    .and_modify(|read| *read = (*read).max(hour))
                    .or_insert(hour);
            }
        }
    }

    pub fn design(&mut self, frame: &DecodedFrame, handle: u64) -> Result<Operand, String> {
        if self.designs.is_none() {
            // Bind country tags once per changed naming census, not once per ship.
            let read = || -> Result<HashMap<u64, Operand>, String> {
                let variants = frame
                    .complete::<tables::VariantDefinitions>()
                    .map_err(|why| why.to_string())?;
                let countries = frame
                    .complete::<tables::Countries>()
                    .map_err(|why| why.to_string())?;
                let links = frame
                    .complete::<tables::CountryLinks>()
                    .map_err(|why| why.to_string())?;
                let lexical = frame
                    .complete::<tables::LexicalTokens>()
                    .unwrap_or_default();
                let tags = Tags::new(
                    countries,
                    links,
                    frame.complete::<tables::CountryAppearance>().ok(),
                );
                Ok(variants
                    .iter()
                    .map(|definition| {
                        // Dates are separate channels, never an interned token per hour.
                        let raw = variant_names::raw_variant(definition, 0, &tags, lexical);
                        (definition.id, Operand::Design(raw))
                    })
                    .collect())
            };
            self.designs = Some(read());
        }
        self.designs
            .as_ref()
            .expect("design naming cache bound")
            .as_ref()
            .map_err(Clone::clone)?
            .get(&handle)
            .cloned()
            .ok_or_else(|| format!("Design {handle}: VariantDefinitions has no matching design"))
    }
    pub fn design_value(
        &mut self,
        frame: &DecodedFrame,
        values: &mut Values,
        owner: &str,
        prefix: &str,
        field: &str,
        handle: u64,
    ) {
        values.push(format!("{prefix}|{field}"), i128::from(handle));
        if handle == 0 {
            return;
        }
        match self.design(frame, handle) {
            Ok(operand) => self.label(values, format!("{prefix}|{field}Name"), operand),
            Err(why) => self.refuse(values, owner, why, 1),
        }
    }
    pub fn ship(
        &mut self,
        frame: &DecodedFrame,
        values: &mut Values,
        owner: &str,
        prefix: &str,
        ship: u64,
    ) {
        if self.ship_views.len() != management::TABLES.len()
            || self
                .ship_views
                .iter()
                .zip(management::TABLES)
                .any(|(view, table)| !view.is(&frame.tables[table.index()]))
        {
            self.ships.clear();
            self.ship_views = management::TABLES
                .iter()
                .map(|table| frame.tables[table.index()].identity())
                .collect();
        }
        let projected = match self.ships.remove(&ship) {
            Some(projected) => projected,
            None => {
                let mut values = Values::default();
                self.ship_refusals = Some(Vec::new());
                self.project_ship(frame, &mut values, ship);
                ShipProjection {
                    values,
                    refusals: self.ship_refusals.take().expect("ship refusal collection"),
                }
            }
        };
        for (field, value) in projected.values.iter() {
            values.push(format!("{prefix}{field}"), value);
        }
        for &(id, count) in &projected.refusals {
            self.refusal(values, owner, id, count);
        }
        self.ships.insert(ship, projected);
    }
    fn project_ship(&mut self, frame: &DecodedFrame, values: &mut Values, ship: u64) {
        dashboard_base::prepared::prepass::note("naval-yards.ship");
        let (prefix, owner) = ("", "");
        let row = find(frame.complete::<tables::Ships>().ok(), ship, |row| row.id);
        let model = find(frame.complete::<tables::ShipModels>().ok(), ship, |row| {
            row.ship
        });
        let hull = dashboard_forces::products::naval_management::projection::hull(row, model);
        if let Some(hp) = hull.current_raw {
            values.push(format!("{prefix}|hull"), i128::from(hp));
        } else {
            self.refuse(values, owner, "Current ship hull not recorded", 1);
        }
        if let Some(hp) = hull.maximum_raw {
            values.push(format!("{prefix}|maximum"), i128::from(hp));
        } else {
            self.refuse(values, owner, "Maximum ship hull not recorded", 1);
        }
        if let Some(model) = model {
            self.design_value(frame, values, owner, prefix, "design", model.variant);
        }
        if let Some(row) = row {
            let token = row.class_id as u32;
            let spelling = find(
                frame.complete::<tables::LexicalTokens>().ok(),
                token,
                |row| row.token,
            )
            .ok_or_else(|| {
                format!("Ship class token {token}: LexicalTokens does not supply its spelling")
            })
            .and_then(|row| {
                std::str::from_utf8(&row.literal)
                    .map(str::to_owned)
                    .map_err(|_| "Ship class token is not UTF-8".to_owned())
            });
            self.label(
                values,
                format!("{prefix}|class"),
                Operand::Class {
                    token,
                    spelling,
                    index: model
                        .and_then(|model| {
                            model.subunit_index.require("ShipModels.subunit_index").ok()
                        })
                        .copied()
                        .flatten(),
                },
            );
            values.push(format!("{prefix}|task"), i128::from(row.task_force));
            if let Some(name) = find(
                frame.complete::<tables::TaskForceCounterInputs>().ok(),
                row.task_force,
                |row| row.task_force,
            )
            .and_then(|row| row.name.require("TaskForceCounterInputs.name").ok())
            .and_then(|bytes| bytes.as_ref())
            {
                self.label(
                    values,
                    format!("{prefix}|taskName"),
                    Operand::Task {
                        handle: row.task_force,
                        bytes: name.clone(),
                    },
                );
            } else {
                self.refuse(values, owner, "Task force name not recorded", 1);
            }
            if let Some(order) = find(
                frame.complete::<tables::NavalRepairTaskForces>().ok(),
                row.task_force,
                |row| row.task_force,
            ) {
                values.push(
                    format!("{prefix}|repairMode"),
                    i128::from(order.repair_mode),
                );
                self.label(
                    values,
                    format!("{prefix}|policy"),
                    Operand::Policy {
                        port: false,
                        value: order.repair_mode,
                    },
                );
                values.push(
                    format!("{prefix}|repairSplit"),
                    i128::from(order.repair_split),
                );
                values.push(
                    format!("{prefix}|repairTarget"),
                    i128::from(order.repair_target),
                );
                values.push(
                    format!("{prefix}|rejoin"),
                    i128::from(order.merge_after_repair),
                );
                if let Ok(tasks) = frame.complete::<tables::TaskForces>() {
                    values.push(
                        format!("{prefix}|rejoinResolves"),
                        i128::from(
                            find(Some(tasks), order.merge_after_repair, |row| row.id).is_some(),
                        ),
                    );
                }
                self.design_value(
                    frame,
                    values,
                    owner,
                    prefix,
                    "refitIntent",
                    order.refit_variant,
                );
                self.design_value(
                    frame,
                    values,
                    owner,
                    prefix,
                    "afterRepairIntent",
                    order.refit_after_repair,
                );
                if let Ok(children) = frame.complete::<tables::NavalRepairChildren>() {
                    let first = children.partition_point(|child| child.task_force < row.task_force);
                    for child in children[first..]
                        .iter()
                        .take_while(|child| child.task_force == row.task_force)
                    {
                        values.push(
                            format!("{prefix}|child:{}", child.ordinal),
                            i128::from(child.child),
                        );
                        if let Ok(tasks) = frame.complete::<tables::TaskForces>() {
                            values.push(
                                format!("{prefix}|childResolves:{}", child.ordinal),
                                i128::from(find(Some(tasks), child.child, |row| row.id).is_some()),
                            );
                        }
                    }
                }
            }
        }
        if let Some(row) = row {
            if let Ok(Some(bytes)) = row.name.require("Ships.name") {
                self.label(
                    values,
                    format!("{prefix}|name"),
                    Operand::ShipCache {
                        handle: ship,
                        bytes: bytes.clone(),
                    },
                );
                return;
            }
            if let Ok(Some(naming)) = row.naming.require("Ships.naming") {
                let group = ship_names::group(frame, naming, model);
                self.label(
                    values,
                    format!("{prefix}|nameRebuilt"),
                    Operand::ShipRebuilt {
                        handle: ship,
                        naming: naming.clone(),
                        group,
                    },
                );
                return;
            }
        }
        self.refuse(
            values,
            owner,
            "Ship name unavailable: Ships cache and naming operands not recorded",
            1,
        );
    }
}

pub fn find<T, K: Ord>(rows: Option<&[T]>, key: K, by: impl Fn(&T) -> K) -> Option<&T> {
    let rows = rows?;
    rows.binary_search_by_key(&key, by).ok().map(|at| &rows[at])
}

/// Whether `table` holds its own complete rows at the hour.
fn known(frame: &DecodedFrame, table: TableId) -> bool {
    matches!(&*frame.tables[table.index()], replay_frame::frame::TableView::Complete(rows) if rows.table() == table)
}

/// Whether a known source carries the date of its supplying read.
fn dated(table: TableId, format: FormatVersion) -> bool {
    !matches!(
        replay_data::cadence::cadence_in(table, format),
        replay_data::cadence::Cadence::Hourly
    )
}

pub fn owners(frame: &DecodedFrame, countries: &Countries) -> Vec<String> {
    let mut owners: Vec<_> = frame
        .complete::<tables::Countries>()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| countries.slotted(row.canonical_slot))
        .collect();
    owners.sort();
    owners.dedup();
    owners
}

pub fn fields(tables: &[TableId]) -> io::Result<Vec<TableFields>> {
    tables
        .iter()
        .map(|&table| {
            let names: &[&str] = match table {
                TableId::Countries => &["canonical_slot", "tag"],
                TableId::CountryAppearance => {
                    &["name_tag", "original_tag", "original_tag_reference"]
                }
                TableId::ReservedCountryNaming => {
                    &["tag", "original_tag", "original_tag_reference"]
                }
                TableId::Ships => &["task_force", "strength", "class_id", "name", "naming"],
                TableId::ShipModels => &["counter", "variant", "subunit_index"],
                TableId::NavalRepairTaskForces => &[
                    "repair_mode",
                    "repair_split",
                    "repair_target",
                    "merge_after_repair",
                    "refit_variant",
                    "refit_after_repair",
                ],
                TableId::TaskForces => &["roster_owner"],
                TableId::TaskForceCounterInputs => &["name"],
                TableId::CountryFactories => &["naval_total_raw"],
                TableId::NavalBases => &["country"],
                TableId::ProductionLines => &["cost_raw", "active_factories", "variant"],
                TableId::Provinces => &["state"],
                TableId::States => &["owner", "custom_name", "owner_name"],
                _ => return Ok(TableFields::all(table)),
            };
            TableFields::named(table, names)
        })
        .collect()
}

/// Whether a name belongs to the requested country (`TAG@slot`, or a tag at any slot).
pub fn selected(name: &str, tag: &str) -> bool {
    name.split('|')
        .next()
        .is_some_and(|part| dashboard_keyed::countries::selected(part, tag))
}

pub fn checked<T: Serialize>(reply: &T) -> io::Result<serde_json::Value> {
    check_reply_budget(reply)?;
    serde_json::to_value(reply).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorded_mod_names_design_suffixes_and_policy_keys_follow_layered_localisation() {
        let (_, base) = catalogue::tests::folder_catalogue(&[(
            "localisation/base_l_english.yml",
            "\u{feff}l_english:\n mod_port:0 \"Base port\"\n mod_class:0 \"Base class\"\n mod_design:0 \"Base design\"\n",
        )]);
        let (_, overlay) = catalogue::tests::folder_catalogue(&[(
            "localisation/replace/overlay_l_english.yml",
            "\u{feff}l_english:\n mod_port:0 \"Overlay port\"\n mod_class:0 \"Overlay class\"\n mod_design:0 \"Overlay design\"\n NAVY_DOCKYARD_REPAIR_POLICY_-3:0 \"Overlay priority\"\n",
        )]);
        let layers = installed_content_core::ResolvedLayers::from_folders(&[
            (&base.0, &[][..]),
            (&overlay.0, &[][..]),
        ])
        .unwrap();
        let cat = Arc::new(catalogue::tests::layers_catalogue(layers));
        let port = Operand::Port(PortName {
            province: 9,
            override_name: Ok(b"mod_port".to_vec()),
            owner_tag: Err("unused owner".into()),
            state: Err("unused state".into()),
        });
        assert_eq!(resolve(&port, Some(&cat)).unwrap().name, "Overlay port");
        let class = Operand::Class {
            token: 712,
            spelling: Ok("mod_class".into()),
            index: None,
        };
        assert_eq!(resolve(&class, Some(&cat)).unwrap().id, "mod_class");
        assert_eq!(resolve(&class, Some(&cat)).unwrap().name, "Overlay class");
        let design = Operand::Design(RawVariant {
            id: u64::MAX,
            token: "mod_design".into(),
            suffix: "Mark synthetic".into(),
            append: true,
            ..Default::default()
        });
        assert_eq!(
            resolve(&design, Some(&cat)).unwrap().name,
            "Overlay design Mark synthetic"
        );
        assert_eq!(
            resolve(
                &Operand::Policy {
                    port: true,
                    value: -3
                },
                Some(&cat)
            )
            .unwrap()
            .name,
            "Overlay priority"
        );
        let hidden = installed_content_core::ResolvedLayers::from_folders(&[
            (&base.0, &[][..]),
            (&overlay.0, &["localisation"][..]),
        ])
        .unwrap();
        let hidden = Arc::new(catalogue::tests::layers_catalogue(hidden));
        assert_eq!(
            resolve(&class, Some(&hidden)).unwrap().name,
            "Overlay class"
        );
        assert!(
            resolve(
                &Operand::Class {
                    token: 900,
                    spelling: Ok("unrecorded_class".into()),
                    index: None
                },
                Some(&hidden)
            )
            .is_err()
        );
        assert!(resolve(&design, None).is_err());
    }

    #[test]
    fn counted_refusals_accumulate_and_interning_survives_restore_without_per_hour_names() {
        let mut state = State::default();
        let mut values = Values::default();
        state.refuse(&mut values, "MOD@7", "Missing operand", 2);
        state.refuse(&mut values, "MOD@7", "Missing operand", 3);
        assert_eq!(values.iter().last().unwrap().1, 5);
        let mut restored: State =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(
            restored.intern(Operand::Refusal("Missing operand".into())),
            0
        );
        assert_eq!(restored.tokens.len(), 1);
    }
}
