//! Occupation joins frozen at their supplying read; hourly owner/name projection.
use super::*;
use dashboard_text::metadata::CountryKeys;
#[cfg(test)]
use replay_frame::frame::FrameDecoder;
use replay_frame::frame::ViewIdentity;
use std::collections::{BTreeSet, HashSet};

type StateKey = (u32, u32, u32);
type NameCache = HashMap<u32, (Vec<u8>, Vec<u8>, Result<String, String>)>;
const HOURLY: [TableId; 3] = [TableId::States, TableId::Countries, TableId::CountryLinks];

fn hourly_resistance(format: replay_data::format::FormatVersion) -> bool {
    replay_data::cadence::cadence_in(TableId::StateResistance, format)
        == replay_data::cadence::Cadence::Hourly
}

fn resistance_read(
    format: replay_data::format::FormatVersion,
    frame: &DecodedFrame,
    hour: i32,
) -> bool {
    stored_resistance_read(format, &frame.metrics, hour)
        || hourly_resistance(format)
            && frame.tables[TableId::StateResistance.index()].sampled_at() == Some(hour)
}

/// [`resistance_read`] from a stored frame's metrics alone, before its rows are decoded.
pub(super) fn stored_resistance_read(
    format: replay_data::format::FormatVersion,
    metrics: &replay_frame::frame::Metrics,
    hour: i32,
) -> bool {
    if hourly_resistance(format) {
        // This independent worker charges its own acquisition. An untimed carry
        // does not become a new observation merely because its cadence is hourly.
        metrics.table_ns[TableId::StateResistance.index()] != 0
    } else {
        slot_read(format, metrics, TableId::StateResistance, hour)
    }
}

struct CountryInputs {
    countries: CountryKeys,
    links: Vec<CountryLinkRow>,
    views: [ViewIdentity; 2],
}

#[derive(Serialize, Deserialize)]
pub(super) struct Scan<'a> {
    /// The recording's format, which dates its daily tables
    /// ([`dashboard_text::metadata::slot_read`]).
    #[serde(skip)]
    pub(crate) format: replay_data::format::FormatVersion,
    #[serde(skip)]
    cat: Option<&'a Catalogue>,
    #[serde(skip)]
    names: StateNames,
    #[serde(skip, default = "missing_laws")]
    laws: Result<Arc<installed_content_core::occupation_laws::Laws>, String>,
    #[serde(skip)]
    red_threshold: Option<i64>,
    #[serde(skip, default = "missing_powers")]
    strength_powers: Result<(i32, i32), String>,
    #[serde(skip, default = "missing_equipment")]
    equipment: Result<Arc<installed_content_core::equipment::Database>, String>,
    pub(super) result: OccupiedScan,
    refused: Refusals,
    #[serde(skip)]
    names_cached: NameCache,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    positions: HashMap<u32, usize>,
    law_tokens: BTreeSet<String>,
    #[serde(skip)]
    state_slots: HashMap<u32, usize>,
    active: Vec<usize>,
    #[serde(skip)]
    hourly: Option<[ViewIdentity; 3]>,
    last_hour: Option<i32>,
    read_hour: Option<i32>,
    complete: bool,
    countries: Arc<CountryIndex>,
    #[serde(skip)]
    country_inputs: Option<CountryInputs>,
    country_stamp: Option<[u8; 32]>,
    country_refused: Refusals,
    pub(super) trend: trend::TrendScan,
    #[serde(skip)]
    runtime_ready: bool,
    /// How many times an observation dropped the carried state (a gap, an
    /// unreadable hour or one without resistance rows), not counting the
    /// first observation of a scan that carries nothing: after one, this
    /// scan holds what any scan of the same hours holds, however far back it
    /// began ([`super::prepared::look_back_window`]).
    #[serde(skip)]
    pub(super) resets: usize,
    #[cfg(test)]
    #[serde(skip)]
    pub(super) work: dashboard_base::prepared::observer_tests::Work,
}

fn missing_laws() -> Result<Arc<installed_content_core::occupation_laws::Laws>, String> {
    Err("Occupation laws need the recorded content stack".into())
}
fn missing_equipment() -> Result<Arc<installed_content_core::equipment::Database>, String> {
    Err("Garrison equipment needs the recorded content stack".into())
}
fn missing_powers() -> Result<(i32, i32), String> {
    Err("Garrison strength needs the recorded content defines".into())
}
impl Default for Scan<'_> {
    fn default() -> Self {
        let mut scan = Self::new(None);
        scan.runtime_ready = false;
        scan
    }
}

/// The resume stamp keeps operand equality without saving prior replay rows.
fn country_stamp(countries: &[CountryRow], links: &[CountryLinkRow]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(&(countries.len() as u64).to_le_bytes());
    for country in countries {
        hash.update(&country.id.to_le_bytes());
        hash.update(&country.canonical_slot.to_le_bytes());
        hash.update(&(country.tag.len() as u64).to_le_bytes());
        hash.update(&country.tag);
    }
    hash.update(&(links.len() as u64).to_le_bytes());
    for link in links {
        hash.update(&link.id.to_le_bytes());
        hash.update(&link.canonical_slot.to_le_bytes());
    }
    *hash.finalize().as_bytes()
}

impl<'a> Scan<'a> {
    pub(super) fn new(cat: Option<&'a Catalogue>) -> Self {
        let mut result = OccupiedScan {
            states: Vec::new(),
            observed: Vec::new(),
            laws: Vec::new(),
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
                Err(_) => match cat
                    .content
                    .layers
                    .derived::<catalogue::CachedResult<installed_content_core::defines::Defines>>()
                    .get_or_init(|| installed_content_core::defines::read(&cat.content.layers))
                {
                    Ok(defines) => {
                        fallback = defines;
                        Some(fallback.as_ref())
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
        // The occupied-state row compares the displayed strength fraction
        // with NInterface.GARRISON_STRENGTH_TO_SHOW_RED as signed fixed5 values.
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
        let strength_powers = defines
            .ok_or_else(|| "Garrison strength needs the recorded content defines".to_owned())
            .and_then(|defines| {
                Ok((
                    defines
                        .int32("NResistance", "GARRISON_STR_POW_MANPOWER")
                        .map_err(|e| e.to_string())?,
                    defines
                        .int32("NResistance", "GARRISON_STR_POW_EQUIPMENT")
                        .map_err(|e| e.to_string())?,
                ))
            });
        let laws = cat
            .ok_or_else(|| "Occupation laws need the recorded content stack".to_owned())
            .and_then(|cat| {
                cat.content
                    .layers
                    .derived::<catalogue::CachedResult<installed_content_core::occupation_laws::Laws>>()
                    .get_or_init(|| installed_content_core::occupation_laws::read(&cat.content.layers))
                    .map_err(|e| e.to_string())
            });
        let equipment = cat
            .ok_or_else(|| "Garrison equipment needs the recorded content stack".to_owned())
            .and_then(|cat| {
                cat.content
                    .layers
                    .derived::<catalogue::CachedResult<installed_content_core::equipment::Database>>()
                    .get_or_init(|| {
                        installed_content_core::equipment::Database::load_complete(&cat.content.layers)
                    })
                    .map_err(|error| error.to_string())
            });
        Self {
            format: replay_data::format::FormatVersion::default(),
            cat,
            names,
            laws,
            red_threshold,
            strength_powers,
            equipment,
            result,
            refused: Refusals::default(),
            names_cached: HashMap::new(),
            positions: HashMap::new(),
            law_tokens: BTreeSet::new(),
            state_slots: HashMap::new(),
            active: Vec::new(),
            hourly: None,
            last_hour: None,
            read_hour: None,
            complete: false,
            countries: Arc::default(),
            country_inputs: None,
            country_stamp: None,
            country_refused: Refusals::default(),
            trend: trend::TrendScan::default(),
            runtime_ready: true,
            resets: 0,
            #[cfg(test)]
            work: dashboard_base::prepared::observer_tests::Work {
                content_loads: cat.map_or(0, |cat| {
                    cat.content
                        .layers
                        .derived::<catalogue::CachedResult<installed_content_core::occupation_laws::Laws>>()
                        .loads()
                        + cat
                            .content
                            .layers
                            .derived::<catalogue::CachedResult<installed_content_core::equipment::Database>>()
                            .loads()
                        + cat
                            .content
                            .layers
                            .derived::<catalogue::CachedResult<installed_content_core::defines::Defines>>()
                            .loads()
                }),
                ..Default::default()
            },
        }
    }

    pub(super) fn bind<'b>(self, cat: Option<&'b Catalogue>) -> Scan<'b> {
        let mut scan = Scan {
            cat,
            format: self.format,
            names: self.names,
            laws: self.laws,
            red_threshold: self.red_threshold,
            strength_powers: self.strength_powers,
            equipment: self.equipment,
            result: self.result,
            refused: self.refused,
            names_cached: self.names_cached,
            positions: self.positions,
            law_tokens: self.law_tokens,
            state_slots: self.state_slots,
            active: self.active,
            hourly: self.hourly,
            last_hour: self.last_hour,
            read_hour: self.read_hour,
            complete: self.complete,
            countries: self.countries,
            country_inputs: self.country_inputs,
            country_stamp: self.country_stamp,
            country_refused: self.country_refused,
            trend: self.trend,
            runtime_ready: self.runtime_ready,
            resets: self.resets,
            #[cfg(test)]
            work: self.work,
        };
        if !scan.runtime_ready {
            let runtime = Scan::new(cat);
            scan.names = runtime.names;
            scan.laws = runtime.laws;
            scan.red_threshold = runtime.red_threshold;
            scan.strength_powers = runtime.strength_powers;
            scan.equipment = runtime.equipment;
            #[cfg(test)]
            {
                scan.work.content_loads = runtime.work.content_loads;
            }
            for warning in runtime.result.warnings {
                if !scan.result.warnings.contains(&warning) {
                    scan.result.warnings.push(warning);
                }
            }
            scan.runtime_ready = true;
        }
        scan
    }

    /// Whether a law's eligibility can read the trigger world
    /// ([`dashboard_text::triggers::RECORDED_TABLES`]): only a bound law with
    /// a visible or available trigger is evaluated through it.
    pub(super) fn reads_trigger_world(&self) -> bool {
        self.laws.as_ref().is_ok_and(|laws| {
            laws.definitions
                .values()
                .any(|law| !law.visible.is_empty() || !law.available.is_empty())
        })
    }

    /// Whether the scan holds a supplying read: the latest daily read since
    /// it last dropped the carried state.
    pub(super) fn holds_read(&self) -> bool {
        self.read_hour.is_some()
    }

    /// Retain the carried observation and active state value for the next window.
    pub(super) fn finish_window(&mut self) {
        for state in &mut self.result.states {
            self.law_tokens
                .extend(state.runs.iter().filter_map(|run| run.value.law.clone()));
            if state.runs.len() > 1 {
                state.runs.drain(..state.runs.len() - 1);
            }
        }
        if self.result.observed.len() > 1 {
            self.result.observed.drain(..self.result.observed.len() - 1);
        }
    }

    pub(super) fn gap(&mut self) {
        self.seal();
        self.active.clear();
        self.hourly = None;
        self.country_inputs = None;
        self.country_stamp = None;
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
                    .map(|reason| (reason.as_str(), identity.1))
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
        #[cfg(test)]
        {
            self.work.row_visits += states.len();
        }
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
        let mut ambiguous_settings = HashSet::new();
        for row in recorded_rows::<tables::OccupationSettings>(frame)? {
            if let Some(slot) = link_slot(links, row.country) {
                if ambiguous_settings.contains(&slot) {
                    continue;
                }
                if defaults.insert(slot, row).is_some() {
                    defaults.remove(&slot);
                    ambiguous_settings.insert(slot);
                    self.refused.note(
                        "OccupationSettings canonical country link is ambiguous",
                        format!("slot {slot}"),
                    );
                }
            } else {
                self.refused.note(
                    "OccupationSettings country link is unresolved",
                    format!("raw link {}", row.country),
                );
            }
        }
        // Index the appended group inventory once per read, resolving raw aliases.
        let mut group_laws = HashMap::new();
        let mut group_failures: HashMap<_, _> = ambiguous_settings
            .into_iter()
            .map(|slot| {
                (
                    slot,
                    "OccupationSettings canonical country link is ambiguous".to_owned(),
                )
            })
            .collect();
        for (&controller, row) in &defaults {
            let groups = match &row.group_laws {
                replay_data::rows::Recorded::NotRecorded => {
                    group_failures.insert(controller, GROUP_LAW_MISSING.to_owned());
                    continue;
                }
                replay_data::rows::Recorded::Value(None) => {
                    group_failures.insert(
                        controller,
                        "The occupied-country law inventory capture failed".into(),
                    );
                    continue;
                }
                replay_data::rows::Recorded::Value(Some(groups)) => groups,
            };
            for group in groups {
                let Some(occupied) = link_slot(links, group.occupied) else {
                    group_failures.insert(
                        controller,
                        "The occupied-country law inventory has an unresolved country link".into(),
                    );
                    continue;
                };
                if group_laws
                    .insert(
                        (controller, occupied),
                        (group.law.as_deref(), group.scope_country),
                    )
                    .is_some()
                {
                    group_failures.insert(
                        controller,
                        "The occupied-country law inventory has duplicate canonical country links"
                            .into(),
                    );
                }
            }
        }
        // Default eligibility shares one lazy world over this supplying read, and
        // each law's answer for one occupier and occupied country.
        let law_world = std::sync::OnceLock::new();
        let mut law_valid = HashMap::<(String, u32, u32), Result<bool, String>>::new();
        let mut assigned = HashMap::<StateKey, i32>::new();
        let mut unresolved_assigned = HashSet::new();
        for row in recorded_rows::<tables::StateAssignedManpower>(frame)? {
            if let Some(key) = key(row.controller, row.occupied, row.state) {
                // The manpower total sums the container with i32 wrapping.
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
        let garrisons_read = slot_read(self.format, &frame.metrics, TableId::StateGarrisons, hour);
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
            let settings = link_slot(links, state.controller).and_then(|slot| defaults.get(&slot));
            let group = match state_key {
                Some((controller, occupied, _)) => {
                    if let Some(reason) = group_failures.get(&controller) {
                        GroupLaw::Unknown(reason.clone())
                    } else if let Some(law) = group_laws.get(&(controller, occupied)) {
                        GroupLaw::Law(law.0)
                    } else if settings.is_some() {
                        GroupLaw::Missing
                    } else {
                        GroupLaw::Unknown("The occupier's occupation settings are missing".into())
                    }
                }
                None => GroupLaw::Unknown("Occupation country links are unresolved".into()),
            };
            let fallback = settings
                .ok_or_else(|| "The native database fallback law is missing".to_owned())
                .and_then(|row| match &row.fallback_law {
                    replay_data::rows::Recorded::Value(Some(bytes)) => Ok(bytes.as_slice()),
                    replay_data::rows::Recorded::Value(None) => {
                        Err("The native database fallback-law capture failed".into())
                    }
                    replay_data::rows::Recorded::NotRecorded => {
                        Err("The native database fallback law is not recorded".into())
                    }
                });
            let (law, law_missing) = law_for(
                LawInputs {
                    state: override_name,
                    key_resolved: state_key.is_some(),
                    overrides_complete: laws_complete,
                    group,
                    default,
                    fallback,
                },
                |token| {
                    let definition = self
                        .laws
                        .as_ref()
                        .map_err(Clone::clone)?
                        .find(token)?
                        .ok_or_else(|| {
                            format!("Occupation law {token} has no recorded-content declaration")
                        })?;
                    let (controller, occupied_key, _) =
                        state_key.expect("law inputs checked the country links");
                    if definition.visible.is_empty() && definition.available.is_empty() {
                        return Ok(true);
                    }
                    let occupied = group_laws
                        .get(&(controller, occupied_key))
                        .and_then(|group| group.1)
                        .and_then(|raw| link_slot(links, raw))
                        .ok_or_else(|| {
                            "The occupied-country scope is not captured or its link is unresolved"
                                .to_owned()
                        })?;
                    if definition.visible.is_empty() && controller == occupied {
                        return Ok(true);
                    }
                    let world = law_world
                        .get_or_init(|| {
                            let cat = self.cat.ok_or_else(|| {
                                "Occupation law eligibility needs recorded content".to_owned()
                            })?;
                            cat.scripted_content()?.recorded_world(frame)
                        })
                        .as_ref()
                        .map_err(Clone::clone)?;
                    law_valid
                        .entry((token.to_owned(), controller, occupied))
                        .or_insert_with(|| {
                            occupation_law_valid(world, definition, controller, occupied)
                        })
                        .clone()
                },
            );
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
            // Requirement failure or a complete assigned-manpower total of zero
            // returns zero before any equipment/define use.
            let garrison_alert = garrison.is_some_and(|row| row.serialized_flag != 0);
            let mut garrison_missing = if state_key.is_none() {
                Some("The read's controller or occupied-country link is unresolved; the garrison key cannot be established".into())
            } else if garrison.is_none() {
                Some("No matching StateGarrisons row was recorded for the read's controller, occupied country and state".into())
            } else if !garrison_alert && assigned_manpower.is_none() {
                Some("StateAssignedManpower is incomplete or its links are unresolved at this read; a zero manpower total cannot be established".into())
            } else {
                None
            };
            let strength = garrison.and_then(|row| {
                match strength::displayed(row, assigned_manpower, &self.strength_powers, |entry| {
                    let equipment = self.equipment.as_ref().map_err(Clone::clone)?;
                    let lexical = frame
                        .complete::<tables::LexicalTokens>()
                        .map_err(|e| format!("Garrison equipment dictionary unavailable: {e}"))?;
                    for token in [entry.equipment_token, entry.archetype_token] {
                        let text = lexical
                            .binary_search_by_key(&token, |row| row.token)
                            .ok()
                            .map(|index| lexical[index].literal.as_slice())
                            .filter(|text| !text.is_empty() && !text.contains(&0))
                            .ok_or_else(|| {
                                format!("Garrison equipment token {token} has no recorded literal")
                            })?;
                        if equipment.named(text).is_none() {
                            return Err(format!(
                                "Garrison equipment {} has no recorded-content definition",
                                String::from_utf8_lossy(text)
                            ));
                        }
                    }
                    Ok(())
                }) {
                    Ok(value) => Some(value),
                    Err(reason) => {
                        garrison_missing = Some(reason.clone());
                        self.refused.note(reason, format!("state {}", state.id));
                        None
                    }
                }
            });
            let value = OccupiedValues {
                name,
                name_missing,
                owner: tag(state.owner),
                occupier: tag(state.controller),
                occupied: tag(resistance.country),
                resistance_raw: resistance.level,
                compliance_raw: resistance.compliance,
                resistance_speed_raw: resistance.speed,
                compliance_speed_raw: resistance.compliance_speed,
                garrison_strength_raw: strength,
                required_manpower: garrison.map(|row| row.required_manpower),
                garrison_required_raw: garrison.map(|row| row.garrison_required),
                garrison_alert,
                garrison_red: strength
                    .and_then(|value| self.red_threshold.map(|threshold| value < threshold)),
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
                self.result.states.push(StateRuns {
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
        deferred: bool,
        previous_hour: Option<i32>,
    ) {
        let countries = self.countries.clone();
        if states_changed {
            #[cfg(test)]
            {
                self.work.row_visits += states.len();
            }
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
                if let Some(hour) = previous_hour {
                    self.result.states[index]
                        .runs
                        .last_mut()
                        .expect("held state")
                        .last_hour = hour;
                }
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
            let name_changed = self
                .names_cached
                .get(&id)
                .is_none_or(|cached| cached.0 != state.custom_name || cached.1 != state.owner_name);
            let last = self.result.states[index]
                .runs
                .last_mut()
                .expect("active state has a run");
            if !name_changed && last.value.owner.as_deref() == owner {
                if !deferred {
                    last.last_hour = hour;
                }
            } else {
                #[cfg(test)]
                {
                    self.work.history_visits += 1;
                }
                if let Some(before) = previous_hour {
                    last.last_hour = before;
                }
                let mut value = last.value.clone();
                value.owner = owner.map(str::to_owned);
                if name_changed {
                    (value.name, value.name_missing) = self.name(state);
                }
                append(&mut self.result.states[index].runs, hour, value);
                #[cfg(test)]
                {
                    self.work.emitted += 1;
                }
            }
            true
        });
        self.active = active;
    }

    pub(super) fn seal(&mut self) {
        let Some(hour) = self.last_hour else { return };
        for &index in &self.active {
            if let Some(run) = self.result.states[index].runs.last_mut() {
                run.last_hour = hour;
            }
        }
        if self.read_hour.is_some()
            && let Some(span) = self.result.observed.last_mut()
        {
            span.last_hour = hour;
        }
        self.trend.seal_through(hour);
    }

    pub(super) fn push(
        &mut self,
        frame: &DecodedFrame,
        hour: i32,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        let previous_hour = self.last_hour;
        self.trend.changed.clear();
        if self.last_hour.and_then(|hour| hour.checked_add(1)) != Some(hour) {
            self.resets += usize::from(self.last_hour.is_some());
            self.gap();
        }
        if !has_rows(frame, TableId::StateResistance) {
            self.resets += 1;
            self.gap();
            return Ok(());
        }
        let (Ok(states), Ok(countries), Ok(links)) = (
            frame.complete::<tables::States>(),
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) else {
            self.resets += 1;
            self.gap();
            return Ok(());
        };
        self.last_hour = Some(hour);
        let country_tables = [TableId::Countries, TableId::CountryLinks];
        let touched = |table: TableId| {
            changes.is_none_or(|changes| {
                changes.rows_changed[table.index()] || changes.table_read[table.index()]
            })
        };
        #[cfg(test)]
        {
            self.work.row_visits += usize::from(touched(TableId::Countries)) * countries.len()
                + usize::from(touched(TableId::CountryLinks)) * links.len();
        }
        let countries_changed = self.country_inputs.as_ref().map_or_else(
            || self.country_stamp != Some(country_stamp(countries, links)),
            |inputs| {
                let same_countries = inputs.views[0].is(&frame.tables[TableId::Countries.index()])
                    || !touched(TableId::Countries)
                    || inputs.countries.same(countries);
                let same_links = inputs.views[1].is(&frame.tables[TableId::CountryLinks.index()])
                    || !touched(TableId::CountryLinks)
                    || inputs.links == links;
                !same_countries || !same_links
            },
        );
        let views = country_tables.map(|table| frame.tables[table.index()].identity());
        if countries_changed {
            self.countries = Arc::new(country_index(countries, links, &mut self.country_refused));
            self.country_stamp = Some(country_stamp(countries, links));
        }
        match &mut self.country_inputs {
            Some(inputs) if !countries_changed => inputs.views = views,
            inputs => {
                *inputs = Some(CountryInputs {
                    countries: CountryKeys::read(countries),
                    links: links.to_vec(),
                    views,
                })
            }
        }
        // Countries' hourly balances can change without changing any naming
        // operand. Compare those operands without copying their strings, then
        // retain only typed identity/link inputs and the latest view identities.
        self.trend
            .collaboration(self.format, frame, hour, &self.countries.identities, links)?;
        if resistance_read(self.format, frame, hour) {
            if let Some(before) = previous_hour {
                for &index in &self.active {
                    self.result.states[index]
                        .runs
                        .last_mut()
                        .expect("active state")
                        .last_hour = before;
                }
            }
            self.read(frame, hour, states, links)?;
        } else if self.read_hour.is_none() {
            return Ok(());
        } else if self.hourly.as_ref().is_some_and(|views| {
            views
                .iter()
                .zip(HOURLY)
                .all(|(view, table)| view.is(&frame.tables[table.index()]))
        }) {
            if changes.is_none() {
                for &index in &self.active {
                    self.result.states[index]
                        .runs
                        .last_mut()
                        .expect("active state has a run")
                        .last_hour = hour;
                }
            }
        } else {
            let states_changed = self
                .hourly
                .as_ref()
                .is_none_or(|views| !views[0].is(&frame.tables[TableId::States.index()]));
            if states_changed || countries_changed {
                self.project(
                    hour,
                    states,
                    links,
                    states_changed,
                    changes.is_some(),
                    previous_hour,
                );
            } else if changes.is_none() {
                // Balance-only country changes still carry every held state.
                for &index in &self.active {
                    self.result.states[index]
                        .runs
                        .last_mut()
                        .expect("active state")
                        .last_hour = hour;
                }
            }
        }
        self.hourly = Some(HOURLY.map(|table| frame.tables[table.index()].identity()));
        self.trend.project(
            hour,
            &self.countries.names,
            countries_changed,
            changes.is_some(),
            previous_hour,
        );
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

    pub(super) fn snapshot(&self) -> OccupationHistory {
        let mut result = self.result.clone();
        if let Some(hour) = self.last_hour {
            for &index in &self.active {
                if let Some(run) = result.states[index].runs.last_mut() {
                    run.last_hour = hour;
                }
            }
            if self.read_hour.is_some()
                && let Some(span) = result.observed.last_mut()
            {
                span.last_hour = hour;
            }
        }
        let tokens: BTreeSet<_> = result
            .states
            .iter()
            .flat_map(|series| &series.runs)
            .filter_map(|run| run.value.law.as_ref())
            .cloned()
            .chain(self.law_tokens.iter().cloned())
            .collect();
        if let Some(cat) = self.cat {
            match &self.laws {
                Ok(laws) => {
                    if !laws.warnings.is_empty() {
                        result.warnings.push(format!(
                            "{} occupation-law files could not be read: {}",
                            laws.warnings.len(),
                            laws.warnings.join("; ")
                        ));
                    }
                    result.laws = tokens
                        .iter()
                        .map(|token| law_content(cat, laws, token))
                        .collect();
                }
                Err(e) => result
                    .warnings
                    .push(format!("Occupation law content unavailable: {e}")),
            }
            result.warnings.extend(cat.verification.iter().cloned());
            result.warnings.extend(cat.warnings.iter().cloned());
            result.warnings.extend(cat.refusal_lines());
        }
        result.states.sort_unstable_by_key(|series| series.state);
        result
            .warnings
            .extend(self.refused.lines("recorded occupation facts"));
        let mut trend = self.trend.history(self.last_hour);
        let country_warnings = self
            .country_refused
            .lines("recorded occupation country identities");
        result.warnings.extend(country_warnings.iter().cloned());
        trend.warnings.extend(country_warnings);
        OccupationHistory {
            states: result,
            trend,
        }
    }
    #[cfg(test)]
    pub(super) fn finish(self) -> OccupationHistory {
        self.snapshot()
    }
}

#[cfg(test)]
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
        TableId::LexicalTokens,
    ];
    let mut scan = Scan::new(cat);
    scan.format = replay.format;
    // Runtime law bodies read the trigger kit. Decode each block incrementally
    // once rather than seeking its keyframe at every daily read.
    let mut selected = selected.to_vec();
    if scan.reads_trigger_world() {
        selected.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
        selected.sort_unstable();
        selected.dedup();
    }
    let selected = &selected[..];
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        selected,
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                let mut decoder = FrameDecoder::for_tables(selected);
                let frames = match frames {
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
                    scan.push(decoded, frame.hour, None)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(scan.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::CountryCollaborationRow, rows::StateGarrisonRow, rows::TableRows};

    #[test]
    fn hourly_resistance_dates_only_its_own_acquisition_witness() {
        let format = replay_data::format::FormatVersion::CURRENT;
        let mut frame = dashboard_base::test_fixtures::source(1, 99, Some(1));
        frame.metrics = Default::default();
        // Even the former daily slot does not invent an hourly acquisition.
        assert!(!resistance_read(format, &frame, 99));
        frame.metrics.table_ns[TableId::StateGarrisons.index()] = 1;
        assert!(!resistance_read(format, &frame, 99));
        frame.metrics.table_ns[TableId::StateResistance.index()] = 1;
        assert!(resistance_read(format, &frame, 99));
        frame.metrics = Default::default();
        frame.tables[TableId::StateResistance.index()] = frame.tables
            [TableId::StateResistance.index()]
        .clone()
        .with_sampled_at(Some(99));
        assert!(resistance_read(format, &frame, 99));
        frame.hour = 100;
        assert!(!resistance_read(format, &frame, 100));
        frame.metrics.table_ns[TableId::StateResistance.index()] = 1;
        assert!(resistance_read(format, &frame, 100));
    }

    #[test]
    fn occupation_trend_ambiguous_canonical_slots_refuse_every_holder() {
        // The current wire writer already rejects this malformed country
        // inventory. Exercise the scan's defensive guard directly, rather
        // than constructing a recording the codec correctly cannot write.
        let mut frame = dashboard_base::test_fixtures::source(1, 100, Some(1));
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
        frame.metrics.table_ns[TableId::StateGarrisons.index()] = 1;
        frame.metrics.table_ns[TableId::CountryCollaboration.index()] = 1;
        let mut scan = Scan::new(None);
        scan.push(&frame, 100, None).unwrap();
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
        let mut frame = dashboard_base::test_fixtures::source(1, 100, Some(1));
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
        let mut scan = Scan::new(None);
        scan.push(&frame, 100, None).unwrap();
        let indexed = scan.countries.clone();
        frame.hour = 101;
        frame.metrics.table_ns.fill(0);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(2);
        }
        scan.push(&frame, 101, None).unwrap();
        assert!(Arc::ptr_eq(&indexed, &scan.countries));
        frame.hour = 102;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"Changed opaque tag".to_vec();
        }
        scan.push(&frame, 102, None).unwrap();
        assert!(!Arc::ptr_eq(&indexed, &scan.countries));
        assert_eq!(
            scan.finish().trend.series["Changed opaque tag"][0].read_hour,
            100
        );
    }

    #[test]
    fn occupied_states_state_name_failure_keeps_exact_text_and_one_diagnostic() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (cat, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[]);
        let mut scan = Scan::new(Some(&cat));
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
