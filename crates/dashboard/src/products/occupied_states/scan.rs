//! Daily occupation joins frozen at their read; hourly owner/name projection.
use super::*;
use replay::frame::{FrameDecoder, ViewIdentity};
use std::collections::{BTreeSet, HashSet};

type StateKey = (u32, u32, u32);
type NameCache = HashMap<u32, (Vec<u8>, Vec<u8>, Result<String, String>)>;
const HOURLY: [TableId; 3] = [TableId::States, TableId::Countries, TableId::CountryLinks];

struct CountryInputs {
    countries: Vec<(u32, u32, Vec<u8>)>,
    links: Vec<CountryLinkRow>,
    views: [ViewIdentity; 2],
}

pub(super) struct Scan<'a> {
    cat: Option<&'a Catalogue>,
    names: StateNames,
    red_threshold: Option<i64>,
    pub(super) result: OccupiedStatesReply,
    refused: Refusals,
    names_cached: NameCache,
    positions: HashMap<u32, usize>,
    state_slots: HashMap<u32, usize>,
    active: Vec<usize>,
    hourly: Option<[ViewIdentity; 3]>,
    last_hour: Option<i32>,
    read_hour: Option<i32>,
    complete: bool,
    countries: Arc<CountryIndex>,
    country_inputs: Option<CountryInputs>,
    country_refused: Refusals,
    trend: trend::TrendScan,
}

impl<'a> Scan<'a> {
    pub(super) fn new(branch: u64, cat: Option<&'a Catalogue>) -> Self {
        let mut result = OccupiedStatesReply {
            branch_id: branch.to_string(),
            decimals: 5,
            states: Vec::new(),
            observed: Vec::new(),
            laws: Vec::new(),
            strength_missing: STRENGTH_MISSING.into(),
            warnings: Vec::new(),
        };
        let names = cat
            .and_then(|cat| match cat.cached_states() {
                Ok(content) => Some(content.names.clone()),
                Err(error) => {
                    result
                        .warnings
                        .push(format!("State template names unavailable: {error}"));
                    None
                }
            })
            .unwrap_or_default();
        // The threshold needs only the defines: read them directly when the cached
        // state content (names, provinces) is unavailable.
        let fallback;
        let defines = match cat {
            None => None,
            Some(cat) => match cat.cached_states() {
                Ok(content) => Some(&content.defines),
                Err(_) => match installed_content::defines::read(&cat.content.layers) {
                    Ok(defines) => {
                        fallback = defines;
                        Some(&fallback)
                    }
                    Err(error) => {
                        result
                            .warnings
                            .push(format!("Garrison defines unavailable: {error}"));
                        None
                    }
                },
            },
        };
        // P: occupied-state row 14155CC50 compares the displayed strength fraction
        // with NInterface.GARRISON_STRENGTH_TO_SHOW_RED at 14331F240.
        // 14155CE88/14155CE9C load and compare signed fixed5 qwords.
        let red_threshold = defines.and_then(|defines| {
            match defines.fixed5("NInterface", "GARRISON_STRENGTH_TO_SHOW_RED") {
                Ok(value) => Some(value),
                Err(e) => {
                    result
                        .warnings
                        .push(format!("Garrison red threshold unavailable: {e}"));
                    None
                }
            }
        });
        Self {
            cat,
            names,
            red_threshold,
            result,
            refused: Refusals::default(),
            names_cached: HashMap::new(),
            positions: HashMap::new(),
            state_slots: HashMap::new(),
            active: Vec::new(),
            hourly: None,
            last_hour: None,
            read_hour: None,
            complete: false,
            countries: Arc::default(),
            country_inputs: None,
            country_refused: Refusals::default(),
            trend: trend::TrendScan::default(),
        }
    }

    pub(super) fn gap(&mut self) {
        self.active.clear();
        self.hourly = None;
        self.country_inputs = None;
        self.read_hour = None;
        self.trend.gap();
    }

    fn name(&mut self, state: &StateRow) -> (Option<String>, Option<String>) {
        let cached = self.names_cached.entry(state.id).or_insert_with(|| {
            (
                state.custom_name.clone(),
                state.owner_name.clone(),
                name_of(state, self.cat, &self.names),
            )
        });
        if cached.0 != state.custom_name || cached.1 != state.owner_name {
            *cached = (
                state.custom_name.clone(),
                state.owner_name.clone(),
                name_of(state, self.cat, &self.names),
            );
        }
        if let Err(reason) = &cached.2 {
            self.refused.note(reason, format!("state {}", state.id));
        }
        (
            cached.2.as_ref().ok().cloned(),
            cached.2.as_ref().err().cloned(),
        )
    }

    fn note_link(&mut self, field: &str, raw: u32, state: u32, links: &[CountryLinkRow]) {
        let refused = link_slot(links, raw)
            .and_then(|slot| self.countries.identities.get(&slot).map(|&id| (slot, id)))
            .and_then(|identity| {
                self.countries
                    .name_refusals
                    .get(&identity)
                    .map(|&reason| (reason, identity.1))
            });
        if let Some((reason, country)) = refused {
            self.refused
                .note(reason, format!("state {state}, country {country}"));
        } else {
            self.refused.note(
                format!("{field} cannot resolve through Countries and CountryLinks"),
                format!("state {state}, raw link {raw}"),
            );
        }
    }

    fn read(
        &mut self,
        frame: &DecodedFrame,
        hour: i32,
        states: &[StateRow],
        links: &[CountryLinkRow],
    ) -> io::Result<()> {
        let countries = self.countries.clone();
        let tag = |raw| {
            link_slot(links, raw)
                .and_then(|slot| countries.identities.get(&slot).map(|&id| (slot, id)))
                .and_then(|identity| countries.names.get(&identity))
                .cloned()
        };
        let key = |controller, occupied, state| -> Option<StateKey> {
            Some((
                link_slot(links, controller)?,
                link_slot(links, occupied)?,
                state,
            ))
        };
        self.state_slots = states
            .iter()
            .enumerate()
            .map(|(slot, state)| (state.id, slot))
            .collect();
        let mut garrisons = HashMap::new();
        for row in recorded_rows::<tables::StateGarrisons>(frame)? {
            if let Some(key) = key(row.controller, row.occupied, row.state) {
                garrisons.insert(key, row);
            } else {
                self.refused.note(
                    "StateGarrisons country link is unresolved",
                    format!(
                        "state {}, controller {}, occupied {}",
                        row.state, row.controller, row.occupied
                    ),
                );
            }
        }
        let mut laws = HashMap::new();
        for row in recorded_rows::<tables::StateOccupationLaws>(frame)? {
            if let Some(key) = key(row.controller, row.occupied, row.state) {
                laws.insert(key, row);
            } else {
                self.refused.note(
                    "StateOccupationLaws country link is unresolved",
                    format!(
                        "state {}, controller {}, occupied {}",
                        row.state, row.controller, row.occupied
                    ),
                );
            }
        }
        let mut defaults = HashMap::new();
        for row in recorded_rows::<tables::OccupationSettings>(frame)? {
            if let Some(slot) = link_slot(links, row.country) {
                defaults.insert(slot, row);
            } else {
                self.refused.note(
                    "OccupationSettings country link is unresolved",
                    format!("raw link {}", row.country),
                );
            }
        }
        let mut assigned = HashMap::<StateKey, i32>::new();
        let mut unresolved_assigned = HashSet::new();
        for row in recorded_rows::<tables::StateAssignedManpower>(frame)? {
            if let Some(key) = key(row.controller, row.occupied, row.state) {
                // P: manpower total 140C5A0F0 sums the container with i32 wrapping.
                let total = assigned.entry(key).or_default();
                *total = total.wrapping_add(row.amount);
            } else {
                unresolved_assigned.insert(row.state);
                self.refused.note(
                    "StateAssignedManpower country link is unresolved",
                    format!(
                        "state {}, controller {}, occupied {}",
                        row.state, row.controller, row.occupied
                    ),
                );
            }
        }
        let assigned_complete = frame.complete::<tables::StateAssignedManpower>().is_ok();
        let laws_complete = frame.complete::<tables::StateOccupationLaws>().is_ok();
        let garrisons_read = slot_read(&frame.metrics, TableId::StateGarrisons, hour);
        self.trend.begin(
            hour,
            countries.clone(),
            links,
            recorded_rows::<tables::StateGarrisons>(frame)?
                .iter()
                .filter(|_| garrisons_read),
            garrisons_read && frame.complete::<tables::StateGarrisons>().is_ok(),
        );
        self.active.clear();
        self.complete = frame.complete::<tables::StateResistance>().is_ok();
        for resistance in recorded_rows::<tables::StateResistance>(frame)?
            .iter()
            .filter(|row| occupied(row))
        {
            let Some(state) = self
                .state_slots
                .get(&resistance.state)
                .map(|&slot| &states[slot])
            else {
                self.complete = false;
                self.refused.note(
                    "active resistance names a state absent from States",
                    resistance.state.to_string(),
                );
                self.trend.missing_state(resistance.state);
                continue;
            };
            // Every occupation key and tag below uses ONLY this read's controller
            // and CountryLinks snapshot. Hourly projection never changes them.
            let state_key = key(state.controller, resistance.country, state.id);
            self.trend
                .state(resistance, state, &countries.identities, links);
            let garrison = state_key.and_then(|key| garrisons.get(&key));
            let assigned_manpower = state_key
                .filter(|_| {
                    garrison.is_some()
                        && assigned_complete
                        && !unresolved_assigned.contains(&state.id)
                })
                .map(|key| assigned.get(&key).copied().unwrap_or(0));
            let override_name = state_key
                .and_then(|key| laws.get(&key))
                .map(|row| row.name.as_slice());
            let default = link_slot(links, state.controller)
                .and_then(|slot| defaults.get(&slot))
                .map(|row| (row.default_law_present != 0, row.law.as_slice()));
            let (law, law_missing) =
                law_for(override_name, state_key.is_some(), laws_complete, default);
            if let Some(reason) = &law_missing {
                self.refused.note(
                    reason,
                    format!(
                        "state {}, controller {}, occupied {}",
                        state.id, state.controller, resistance.country
                    ),
                );
            }
            let (name, name_missing) = self.name(state);
            // P: 140FE01E0(...,true) returns zero for +0x60, or a complete
            // assigned-manpower total of zero. Otherwise equipment is unrecorded.
            let garrison_alert = garrison.is_some_and(|row| row.serialized_flag != 0);
            let zero_strength =
                garrison.is_some() && (garrison_alert || assigned_manpower == Some(0));
            let garrison_missing = if state_key.is_none() {
                Some("The read's controller or occupied-country link is unresolved; the garrison key cannot be established".into())
            } else if garrison.is_none() {
                Some("No matching StateGarrisons row was recorded for the read's controller, occupied country and state".into())
            } else if !garrison_alert && assigned_manpower.is_none() {
                Some("StateAssignedManpower is incomplete or its links are unresolved at this read; a zero manpower total cannot be established".into())
            } else {
                None
            };
            let value = OccupiedValues {
                name,
                name_missing,
                owner: tag(state.owner),
                occupier: tag(state.controller),
                occupied: tag(resistance.country),
                resistance_raw: resistance.level.to_string(),
                compliance_raw: resistance.compliance.to_string(),
                resistance_speed_raw: resistance.speed.to_string(),
                compliance_speed_raw: resistance.compliance_speed.to_string(),
                garrison_strength_raw: zero_strength.then(|| "0".into()),
                required_manpower: garrison.map(|row| row.required_manpower),
                garrison_required_raw: garrison.map(|row| row.garrison_required.to_string()),
                garrison_alert,
                garrison_red: if zero_strength {
                    self.red_threshold.map(|threshold| 0 < threshold)
                } else {
                    None
                },
                assigned_manpower,
                garrison_missing,
                law,
                law_missing,
            };
            self.complete &= value.occupier.is_some() && value.occupied.is_some();
            for (field, raw, missing) in [
                ("owner", state.owner, value.owner.is_none()),
                (
                    "occupier at the read",
                    state.controller,
                    value.occupier.is_none(),
                ),
                (
                    "occupied country at the read",
                    resistance.country,
                    value.occupied.is_none(),
                ),
            ] {
                if missing {
                    self.note_link(field, raw, state.id, links);
                }
            }
            let index = *self.positions.entry(state.id).or_insert_with(|| {
                let index = self.result.states.len();
                self.result.states.push(OccupiedSeries {
                    state: state.id,
                    runs: Vec::new(),
                });
                index
            });
            append(&mut self.result.states[index].runs, hour, value);
            self.active.push(index);
        }
        self.read_hour = Some(hour);
        self.trend.finish_read();
        Ok(())
    }

    fn project(
        &mut self,
        hour: i32,
        states: &[StateRow],
        links: &[CountryLinkRow],
        states_changed: bool,
    ) {
        let countries = self.countries.clone();
        if states_changed {
            self.state_slots = states
                .iter()
                .enumerate()
                .map(|(slot, state)| (state.id, slot))
                .collect();
        }
        let mut active = std::mem::take(&mut self.active);
        active.retain(|&index| {
            let id = self.result.states[index].state;
            let Some(state) = self.state_slots.get(&id).map(|&slot| &states[slot]) else {
                self.complete = false;
                self.refused.note(
                    "read occupation state absent from hourly States",
                    id.to_string(),
                );
                return false;
            };
            let owner = link_slot(links, state.owner)
                .and_then(|slot| countries.identities.get(&slot).map(|&id| (slot, id)))
                .and_then(|identity| countries.names.get(&identity))
                .map(String::as_str);
            if owner.is_none() {
                self.note_link("owner", state.owner, id, links);
            }
            let cached = self
                .names_cached
                .get(&id)
                .expect("read cached every active name");
            let name_changed = cached.0 != state.custom_name || cached.1 != state.owner_name;
            let last = self.result.states[index]
                .runs
                .last_mut()
                .expect("active state has a run");
            if !name_changed && last.value.owner.as_deref() == owner {
                last.last_hour = hour;
            } else {
                let mut value = last.value.clone();
                value.owner = owner.map(str::to_owned);
                if name_changed {
                    (value.name, value.name_missing) = self.name(state);
                }
                append(&mut self.result.states[index].runs, hour, value);
            }
            true
        });
        self.active = active;
    }

    pub(super) fn push(&mut self, frame: &DecodedFrame, hour: i32) -> io::Result<()> {
        if self.last_hour.and_then(|hour| hour.checked_add(1)) != Some(hour) {
            self.gap();
        }
        self.last_hour = Some(hour);
        if !has_rows(frame, TableId::StateResistance) {
            self.gap();
            return Ok(());
        }
        let (Ok(states), Ok(countries), Ok(links)) = (
            frame.complete::<tables::States>(),
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) else {
            self.gap();
            return Ok(());
        };
        let country_tables = [TableId::Countries, TableId::CountryLinks];
        let countries_changed = self.country_inputs.as_ref().is_none_or(|inputs| {
            let same_countries = inputs.views[0].is(&frame.tables[TableId::Countries.index()])
                || inputs.countries.len() == countries.len()
                    && inputs
                        .countries
                        .iter()
                        .zip(countries)
                        .all(|((id, slot, tag), row)| {
                            *id == row.id && *slot == row.canonical_slot && *tag == row.tag
                        });
            let same_links = inputs.views[1].is(&frame.tables[TableId::CountryLinks.index()])
                || inputs.links == links;
            !same_countries || !same_links
        });
        let views = country_tables.map(|table| frame.tables[table.index()].identity());
        if countries_changed {
            self.countries = Arc::new(country_index(countries, links, &mut self.country_refused));
            self.country_inputs = Some(CountryInputs {
                countries: countries
                    .iter()
                    .map(|row| (row.id, row.canonical_slot, row.tag.clone()))
                    .collect(),
                links: links.to_vec(),
                views,
            });
        } else {
            self.country_inputs
                .as_mut()
                .expect("unchanged inputs were saved")
                .views = views;
        }
        // Countries' hourly balances can change without changing any naming
        // operand. Compare those operands without copying their strings, then
        // retain only typed identity/link inputs and the latest view identities.
        self.trend
            .collaboration(frame, hour, &self.countries.identities, links)?;
        if slot_read(&frame.metrics, TableId::StateResistance, hour) {
            self.read(frame, hour, states, links)?;
        } else if self.read_hour.is_none() {
            return Ok(());
        } else if self.hourly.as_ref().is_some_and(|views| {
            views
                .iter()
                .zip(HOURLY)
                .all(|(view, table)| view.is(&frame.tables[table.index()]))
        }) {
            for &index in &self.active {
                self.result.states[index]
                    .runs
                    .last_mut()
                    .expect("active state has a run")
                    .last_hour = hour;
            }
        } else {
            let states_changed = self
                .hourly
                .as_ref()
                .is_none_or(|views| !views[0].is(&frame.tables[TableId::States.index()]));
            self.project(hour, states, links, states_changed);
        }
        self.hourly = Some(HOURLY.map(|table| frame.tables[table.index()].identity()));
        self.trend
            .project(hour, &self.countries.names, countries_changed);
        let read = self.read_hour.expect("a read precedes every observation");
        if let Some(last) = self.result.observed.last_mut()
            && last.last_hour.checked_add(1) == Some(hour)
            && last.read_hour == read
            && last.complete == self.complete
        {
            last.last_hour = hour;
        } else {
            self.result.observed.push(OccupiedObservation {
                first_hour: hour,
                last_hour: hour,
                read_hour: read,
                complete: self.complete,
            });
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> OccupationHistory {
        let tokens: BTreeSet<_> = self
            .result
            .states
            .iter()
            .flat_map(|series| &series.runs)
            .filter_map(|run| run.value.law.as_ref())
            .cloned()
            .collect();
        if let Some(cat) = self.cat {
            match installed_content::occupation_laws::read(&cat.content.layers) {
                Ok(laws) => {
                    if !laws.warnings.is_empty() {
                        self.result.warnings.push(format!(
                            "{} occupation-law files could not be read: {}",
                            laws.warnings.len(),
                            laws.warnings.join("; ")
                        ));
                    }
                    self.result.laws = tokens
                        .iter()
                        .map(|token| law_content(cat, &laws, token))
                        .collect();
                }
                Err(e) => self
                    .result
                    .warnings
                    .push(format!("Occupation law content unavailable: {e}")),
            }
            self.result
                .warnings
                .extend(cat.verification.iter().cloned());
            self.result.warnings.extend(cat.warnings.iter().cloned());
            self.result.warnings.extend(cat.refusal_lines());
        }
        self.result
            .states
            .sort_unstable_by_key(|series| series.state);
        self.result
            .warnings
            .extend(self.refused.lines("recorded occupation facts"));
        let mut trend = self.trend.finish();
        let country_warnings = self
            .country_refused
            .lines("recorded occupation country identities");
        self.result
            .warnings
            .extend(country_warnings.iter().cloned());
        trend.warnings.extend(country_warnings);
        OccupationHistory {
            states: self.result,
            trend,
        }
    }
}

pub(in crate::products) fn scan_history(
    replay: &Replay,
    branch: u64,
    cat: Option<&Catalogue>,
    cancelled: &AtomicBool,
) -> io::Result<OccupationHistory> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::States,
        TableId::StateResistance,
        TableId::StateGarrisons,
        TableId::StateOccupationLaws,
        TableId::OccupationSettings,
        TableId::StateAssignedManpower,
        TableId::CountryCollaboration,
    ];
    let mut scan = Scan::new(branch, cat);
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        let mut decoder = FrameDecoder::for_tables(&selected);
        let frames = match replay.read_block_selected(block, &selected) {
            Ok(frames) => frames,
            Err(e) => {
                scan.result.warnings.push(format!(
                    "Occupied states block {} unavailable: {e}",
                    block.first_hour
                ));
                scan.trend.unavailable(format!(
                    "Occupation history block {} unavailable: {e}",
                    block.first_hour
                ));
                scan.gap();
                continue;
            }
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Occupied states request cancelled",
                ));
            }
            let decoded = match decoder.advance(&frame) {
                Ok(decoded) => decoded,
                Err(e) => {
                    scan.result.warnings.push(format!(
                        "Occupied states at {} unavailable: {e}",
                        frame.hour
                    ));
                    scan.trend.unavailable(format!(
                        "Occupation history at {} unavailable: {e}",
                        frame.hour
                    ));
                    scan.gap();
                    break;
                }
            };
            validate_campaign(decoded, replay)?;
            scan.push(decoded, frame.hour)?;
        }
    }
    Ok(scan.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{CountryCollaborationRow, StateGarrisonRow, TableRows};

    #[test]
    fn occupation_trend_ambiguous_canonical_slots_refuse_every_holder() {
        // The current wire writer already rejects this malformed country
        // inventory. Exercise the scan's defensive guard directly, rather
        // than constructing a recording the codec correctly cannot write.
        let mut frame = crate::test_fixtures::source(1, 100, Some(1));
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![
                CountryRow {
                    id: 12,
                    canonical_slot: 7,
                    tag: b"Second opaque tag".to_vec(),
                    ..Default::default()
                },
                CountryRow {
                    id: 9001,
                    canonical_slot: 7,
                    tag: b"First opaque tag".to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 7,
                },
                CountryLinkRow {
                    id: 12,
                    canonical_slot: 7,
                },
                CountryLinkRow {
                    id: 9001,
                    canonical_slot: 7,
                },
            ]));
        *frame.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 1,
                controller: 11,
                owner: 11,
                ..Default::default()
            }]));
        *frame.tables[TableId::StateResistance.index()] =
            TableView::Complete(TableRows::StateResistance(vec![StateResistanceRow {
                state: 1,
                country: 11,
                active: 1,
                level: 10,
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryCollaboration.index()] =
            TableView::Complete(TableRows::CountryCollaboration(vec![
                CountryCollaborationRow {
                    source: 11,
                    target: 12,
                    progress: 12345,
                },
            ]));
        frame.metrics.table_ns[TableId::StateResistance.index()] = 1;
        let mut scan = Scan::new(1, None);
        scan.push(&frame, 100).unwrap();
        assert!(scan.countries.identities.is_empty());
        assert!(scan.countries.names.is_empty());
        let history = scan.finish();
        assert!(history.trend.series.is_empty());
        assert_eq!(
            history.trend.warnings,
            [
                "1 recorded occupation trend facts not shown: active resistance controller link is unresolved; state 1, raw link 11",
                "2 recorded occupation country identities not shown: Countries canonical slot is ambiguous; slot 7, country 12, slot 7, country 9001",
            ]
        );
        assert_eq!(
            history.trend.collaboration_warnings,
            [
                "1 recorded occupation trend facts not shown: CountryCollaboration link is unresolved; 11 → 12"
            ]
        );
    }

    #[test]
    fn occupation_trend_identity_cache_ignores_hourly_balances_but_rebuilds_for_tag_changes() {
        let mut frame = crate::test_fixtures::source(1, 100, Some(1));
        *frame.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 1,
                controller: 11,
                owner: 11,
                ..Default::default()
            }]));
        *frame.tables[TableId::StateResistance.index()] =
            TableView::Complete(TableRows::StateResistance(vec![StateResistanceRow {
                state: 1,
                country: 11,
                active: 1,
                level: 10,
                ..Default::default()
            }]));
        *frame.tables[TableId::StateGarrisons.index()] =
            TableView::Complete(TableRows::StateGarrisons(vec![StateGarrisonRow {
                controller: 11,
                occupied: 11,
                state: 1,
                ..Default::default()
            }]));
        frame.metrics.table_ns[TableId::StateResistance.index()] = 1;
        let mut scan = Scan::new(1, None);
        scan.push(&frame, 100).unwrap();
        let indexed = scan.countries.clone();
        frame.hour = 101;
        frame.metrics.table_ns.fill(0);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(2);
        }
        scan.push(&frame, 101).unwrap();
        assert!(Arc::ptr_eq(&indexed, &scan.countries));
        frame.hour = 102;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"Changed opaque tag".to_vec();
        }
        scan.push(&frame, 102).unwrap();
        assert!(!Arc::ptr_eq(&indexed, &scan.countries));
        assert_eq!(
            scan.finish().trend.series["Changed opaque tag"][0].read_hour,
            100
        );
    }

    #[test]
    fn occupied_states_state_name_failure_keeps_exact_text_and_one_diagnostic() {
        let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[]);
        let mut scan = Scan::new(1, Some(&cat));
        let state = StateRow {
            id: 41,
            ..Default::default()
        };
        assert_eq!(
            scan.name(&state).1.as_deref(),
            Some("No recorded-content state template")
        );
        let reply = scan.finish().states;
        assert_eq!(
            reply
                .warnings
                .iter()
                .filter(|line| line.contains("No recorded-content state template"))
                .count(),
            1
        );
        assert!(reply.warnings.iter().all(|line| {
            !line.contains("State template names unavailable: State template names unavailable:")
        }));
    }
}
