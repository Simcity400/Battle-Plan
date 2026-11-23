
use super::values::{self, ONE};
use super::*;
use content_reader_country_names::country_names::CountryNames;
use content_reader_localisation::localisation::Argument as Arg;
use replay_data::{
    rows::CharacterOperativeRow, rows::PORTRAIT_LARGE, rows::ProvinceRow, rows::Recorded,
    rows::StrategicOperativeRow, rows::portrait_entry,
};
use viewer_foundation::color_inheritance::active_subject;
use replay_derive::directed_wars;
use viewer_foundation::tooltip_text::localise;

const OPERATIVE_PORTRAIT_ROLE: u32 = 4;
const OPERATIVE_CATEGORY: i32 = 3;
const ON_MISSION: u32 = 3;
const COOLDOWN: u32 = 1;

pub(super) const BUILD_NETWORK: u32 = 1;
pub(super) const QUIET_NETWORK: u32 = 2;
pub(super) const COUNTER_INTELLIGENCE: u32 = 3;
pub(super) const ROOT_OUT: u32 = 4;
pub(super) const BOOST_IDEOLOGY: u32 = 5;
pub(super) const CONTROL_TRADE: u32 = 6;
pub(super) const DIPLOMATIC_PRESSURE: u32 = 7;
pub(super) const PROPAGANDA: u32 = 8;

pub(super) struct Hour<'h, 'f> {
    pub frame: &'f DecodedFrame,
    pub world: &'h World<'f>,
    pub ours: u32,
    pub defines: &'h IntelDefines,
    pub assets: &'h IntelAssets,
    pub raids: &'h RaidAssets,
    pub localisation: &'h Localisation,
    pub language: &'h str,
}

#[derive(Clone, Copy)]
enum Item<'f> {
    Shield(u32),
    Operative(&'f OperativeMissionRow),
    Operation(&'f OperationRow),
    Offer(&'f replay_data::rows::AgencyBufferedOffer),
}

#[derive(Default)]
struct Gathered<'f> {
    items: Vec<Item<'f>>,
    operations: i64,
    more: bool,
}

type SortKey = (u8, Vec<u8>, i64, u32);

type RootOutStacks = BTreeMap<(u32, i32), Result<Vec<(u64, i64)>, String>>;

pub(super) struct Shared<'h, 'f> {
    pub tables: Tables<'f>,
    pub missions: super::missions::Inputs<'f>,
    pub names: CountryNames<'h>,
    provinces: HashMap<u32, &'f ProvinceRow>,
    root_out: RootOutStacks,
}

impl<'h, 'f> Hour<'h, 'f> {
    pub fn slot(&self, raw: u32) -> Option<u32> {
        self.world.slot(raw)
    }

    pub fn build(&self, out: &mut IntelIcons) {
        let frame = self.frame;
        let report = &mut out.report;
        let tables = Tables::read(frame);
        let provinces: HashMap<u32, &'f ProvinceRow> = frame
            .complete::<tables::Provinces>()
            .map(|rows| rows.iter().map(|row| (row.id, row)).collect())
            .unwrap_or_default();
        let capitals = complete::<tables::Politics>(frame, "Politics").map(|rows| {
            rows.iter()
                .filter_map(|row| Some((self.slot(row.country)?, row.capital)))
                .collect::<HashMap<u32, i32>>()
        });
        let mut raw_by_slot: RawBySlot = BTreeMap::new();
        if let Ok(links) = frame.complete::<tables::CountryLinks>() {
            for link in links {
                raw_by_slot
                    .entry(link.canonical_slot)
                    .or_default()
                    .push(link.id);
            }
        }
        for raws in raw_by_slot.values_mut() {
            raws.sort_unstable();
        }
        let mut gathered: BTreeMap<u32, Gathered<'f>> = BTreeMap::new();
        let mut named: Slots = BTreeSet::new();
        let capital_state = |slot: u32| -> Result<Option<u32>, String> {
            let capitals = capitals.as_ref().map_err(Clone::clone)?;
            Ok(capitals
                .get(&slot)
                .filter(|&&state| state > 0)
                .map(|&state| state as u32))
        };
        let controllers: Result<HashMap<u32, Option<u32>>, String> =
            complete::<tables::States>(frame, "States").map(|rows| {
                rows.iter()
                    .map(|row| (row.id, self.slot(row.controller)))
                    .collect()
            });
        let held_capital = |slot: u32| -> Result<Option<u32>, String> {
            let Some(state) = capital_state(slot)? else {
                return Ok(None);
            };
            let controllers = controllers.as_ref().map_err(Clone::clone)?;
            Ok((controllers.get(&state).copied().flatten() == Some(slot)).then_some(state))
        };
        for slot in self.friends(&raw_by_slot, report) {
            match held_capital(slot) {
                Ok(Some(state)) => {
                    named.insert(slot);
                    gathered
                        .entry(state)
                        .or_default()
                        .items
                        .push(Item::Shield(slot));
                }
                Ok(None) => report.note(
                    "a friendly country controls no capital state, so its shield is not drawn",
                ),
                Err(why) => report.note(format!("a shield's capital is unknown: {why}")),
            }
        }
        match &tables.missions {
            Ok(rows) => {
                let rows: &'f [OperativeMissionRow] = rows;
                for row in rows
                    .iter()
                    .filter(|row| self.slot(row.roster_country) == Some(self.ours))
                {
                    let state = match self.operative_state(row, &capital_state) {
                        Ok(Some(state)) => state,
                        Ok(None) => continue,
                        Err(why) => {
                            report.note(format!("an operative's state is unknown: {why}"));
                            continue;
                        }
                    };
                    named.extend(self.slot(row.owner_country));
                    named.extend(row.target_country.and_then(|raw| self.slot(raw)));
                    named.extend(row.pending_country.and_then(|raw| self.slot(raw)));
                    gathered
                        .entry(state)
                        .or_default()
                        .items
                        .push(Item::Operative(row));
                }
            }
            Err(why) => report.note(format!("operative icons are not drawn: {why}")),
        }
        let withheld = match frame.complete::<tables::LocalPlayer>() {
            Ok([player]) if player.viewing_country().and_then(|raw| self.slot(raw)) == Some(self.ours) => None,
            Ok([_]) => Some("operation and offer entries are withheld: AgencyOperationBuffers holds only the recording client's buffer, and the map is viewed as another country".to_owned()),
            Ok(rows) => Some(format!("operation and offer entries are withheld: LocalPlayer holds {} rows where the game holds one", rows.len())),
            Err(why) => Some(format!("operation and offer entries are withheld: the recording client's viewpoint is unknown: {why}")),
        };
        match (&tables.buffers, withheld) {
            (_, Some(why)) => report.note(why),
            (Ok(buffers), None) if buffers.len() == 1 && buffers[0].snapshot == 0 => {
                let buffer = &buffers[0];
                let by_id: Result<HashMap<u64, &'f OperationRow>, _> = tables.operations.as_ref()
                    .map(|rows| rows.iter().map(|row| (row.id, row)).collect());
                for buffered in buffer.completed.iter().chain(&buffer.running).chain(&buffer.ready).chain(&buffer.unready) {
                    let Some(state) = buffered.state.filter(|&state| state != 0) else {
                        continue;
                    };
                    let entry = gathered.entry(state).or_default();
                    if entry.operations >= i64::from(self.defines.max_stack) {
                        entry.more = true;
                        continue;
                    }
                    entry.operations += 1;
                    let Some(&row) = by_id.as_ref().ok().and_then(|rows| rows.get(&buffered.operation)) else {
                        report.note("an admitted operation entry is withheld: its buffered reference has no recorded Operations row");
                        continue;
                    };
                    named.extend(self.slot(row.target));
                    entry.items.push(Item::Operation(row));
                }
                let offers = match &buffer.offers {
                    Recorded::Value(offers) => offers.as_slice(),
                    Recorded::NotRecorded => {
                        report.note("offer entries are withheld: AgencyOperationBuffers did not record offers");
                        &[]
                    }
                };
                for offer in offers {
                    let Some(state) = offer.state.filter(|&state| state != 0) else { continue; };
                    let entry = gathered.entry(state).or_default();
                    if entry.operations >= i64::from(self.defines.max_stack) {
                        entry.more = true;
                        continue;
                    }
                    entry.operations += 1;
                    named.extend(self.slot(offer.country));
                    entry.items.push(Item::Offer(offer));
                }
            }
            (Err(why), None) => report.note(format!("operation entries are withheld: AgencyOperationBuffers are unavailable: {why}")),
            _ => report.note("operation entries are withheld: AgencyOperationBuffers has no shared client snapshot"),
        }
        let shared = Shared {
            names: CountryNames::read_selected(frame, self.localisation, named),
            root_out: self.root_out_stacks(&tables),
            missions: super::missions::Inputs::new(self, &tables),
            tables,
            provinces,
        };
        for (state, gathered) in gathered {
            if gathered.items.is_empty() && !gathered.more {
                continue;
            }
            let anchor = match self.anchor(state, &shared.provinces) {
                Ok(anchor) => anchor,
                Err(why) => {
                    report.note(format!("a state's icon has no anchor: {why}"));
                    continue;
                }
            };
            let mut entries: Vec<(SortKey, Entry)> = gathered
                .items
                .iter()
                .map(|item| self.entry(*item, &shared, report))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut entries: Vec<Entry> = entries.into_iter().map(|(_, entry)| entry).collect();
            if gathered.more {
                entries.push(Entry {
                    kind: EntryKind::More,
                    key: EntryKey::More,
                    writes: Vec::new(),
                    tooltip: Arc::new(tooltips::more()),
                });
            }
            out.states.push(StateIcon {
                state,
                anchor,
                entries,
            });
        }
    }

    fn friends(&self, raw_by_slot: &RawBySlot, report: &mut IntelReport) -> Vec<u32> {
        let frame = self.frame;
        let mut out = vec![self.ours];
        let related = match self.related() {
            Ok(related) => related,
            Err(why) => {
                report.note(format!(
                    "the related countries' shields are not drawn: {why}"
                ));
                return out;
            }
        };
        if related.is_empty() {
            return out;
        }
        let wars = raw_by_slot
            .get(&self.ours)
            .and_then(|raws| raws.first().copied())
            .ok_or_else(|| "the viewing country has no CountryLinks row".to_owned())
            .and_then(|raw| directed_wars::Owner::new(frame, raw));
        for slot in related {
            let at_war = raw_by_slot
                .get(&slot)
                .and_then(|raws| raws.first().copied())
                .ok_or_else(|| "it has no CountryLinks row".to_owned())
                .and_then(|raw| wars.as_ref().map_err(Clone::clone)?.at_war(raw));
            match at_war {
                Ok(false) => out.push(slot),
                Ok(true) => {}
                Err(why) => report.note(format!(
                    "whether a related country is at war with the viewer is unknown, so its shield is not drawn: {why}"
                )),
            }
        }
        out
    }

    fn related(&self) -> Result<BTreeSet<u32>, String> {
        let frame = self.frame;
        let members = complete::<tables::FactionMembers>(frame, "FactionMembers")?;
        let subjects = complete::<tables::Subjects>(frame, "Subjects")?;
        let faction_of: HashMap<u32, u64> = members
            .iter()
            .filter_map(|member| Some((self.slot(member.country)?, member.faction)))
            .collect();
        let mut related = BTreeSet::new();
        if let Some(&faction) = faction_of.get(&self.ours).filter(|&&faction| faction != 0) {
            related.extend(
                faction_of
                    .iter()
                    .filter(|(_, member)| **member == faction)
                    .map(|(slot, _)| *slot),
            );
        }
        for subject in subjects
            .iter()
            .filter(|row| active_subject(row, frame.hour))
        {
            let (overlord, vassal) = (self.slot(subject.overlord), self.slot(subject.subject));
            if overlord == Some(self.ours) {
                related.extend(vassal);
            }
            if vassal == Some(self.ours) {
                related.extend(overlord);
            }
        }
        related.remove(&self.ours);
        Ok(related)
    }

    fn operative_state(
        &self,
        row: &OperativeMissionRow,
        capital_state: &dyn Fn(u32) -> Result<Option<u32>, String>,
    ) -> Result<Option<u32>, String> {
        let positive =
            |state: Option<i32>| state.filter(|&state| state > 0).map(|state| state as u32);
        match row.assignment {
            ON_MISSION => match row.variant {
                Some(BUILD_NETWORK | QUIET_NETWORK | ROOT_OUT | BOOST_IDEOLOGY | PROPAGANDA) => {
                    Ok(positive(row.target_state))
                }
                Some(COUNTER_INTELLIGENCE | CONTROL_TRADE | DIPLOMATIC_PRESSURE) => {
                    let raw = row
                        .target_country
                        .ok_or("its mission has no target country")?;
                    let slot = self.slot(raw).ok_or_else(|| {
                        format!("its target country {raw} has no CountryLinks row")
                    })?;
                    capital_state(slot)
                }
                Some(variant) => Err(format!(
                    "its mission variant {variant} is not one of the eight"
                )),
                None => Ok(None),
            },
            COOLDOWN if row.pending_variant.is_some() => Ok(positive(row.pending_state)),
            _ => Ok(None),
        }
    }

    fn anchor(&self, state: u32, provinces: &HashMap<u32, &ProvinceRow>) -> Result<Anchor, String> {
        let order = self
            .world
            .state_order(state)
            .map_err(|why| why.to_string())?;
        let mut best: Option<(u32, i32)> = None;
        for &province in order {
            let points = provinces
                .get(&province)
                .map(|row| row.victory_points)
                .ok_or_else(|| {
                    format!("state {state}'s province {province} has no Provinces row")
                })?;
            if best.is_none_or(|(_, most)| points > most) {
                best = Some((province, points));
            }
        }
        match best {
            Some((province, points)) if points > 0 => Ok(Anchor::Province(province)),
            _ => self
                .raids
                .geometry
                .state_centre(order)
                .map(|[x, z]| Anchor::Centre([x as f32, 0.0, z as f32]))
                .ok_or_else(|| format!("state {state} has no provinces to centre on")),
        }
    }

    fn root_out_stacks(&self, tables: &Tables<'f>) -> RootOutStacks {
        let Ok(rows) = tables.missions.clone() else {
            return BTreeMap::new();
        };
        let mut groups = RootOutStacks::new();
        for row in rows.iter().filter(|row| {
            row.assignment == ON_MISSION
                && row.variant == Some(ROOT_OUT)
                && row.target_state.is_some()
        }) {
            let Some(owner) = self.slot(row.owner_country) else {
                continue;
            };
            let key = (owner, row.target_state.unwrap_or_default());
            let base = self.root_out_base(row);
            let group = groups.entry(key).or_insert_with(|| Ok(Vec::new()));
            match base {
                Ok(value) => {
                    if let Ok(list) = group {
                        list.push((row.id, value));
                    }
                }
                Err(why) => {
                    if group.is_ok() {
                        *group = Err(why);
                    }
                }
            }
        }
        let factor = self.defines.root_out.as_ref().map(|[_, factor]| *factor);
        groups
            .into_iter()
            .map(|(key, group)| {
                let stacked = group.and_then(|list| {
                    let factor = factor?;
                    Ok(values::root_out_stacked(&list, factor))
                });
                (key, stacked)
            })
            .collect()
    }

    fn root_out_base(&self, row: &OperativeMissionRow) -> Result<i64, String> {
        let [base, _] = *self.defines.root_out.as_ref().map_err(Clone::clone)?;
        let country = row
            .root_out_country_factor
            .ok_or("an operative's country root-out factor is not recorded")?;
        let operative = row
            .root_out_operative_factor
            .ok_or("an operative's own root-out factor is not recorded")?;
        let nationality = match row.root_out_nationality {
            None => return Err("an operative's nationality is not recorded".into()),
            Some(1) => self.assets.nationality.clone()?,
            Some(_) => 0,
        };
        Ok(values::root_out_base(base, country, operative, nationality))
    }

    fn entry(
        &self,
        item: Item<'f>,
        shared: &Shared<'h, 'f>,
        report: &mut IntelReport,
    ) -> (SortKey, Entry) {
        let name_bytes = |slot: Option<u32>, report: &mut IntelReport| -> Vec<u8> {
            match slot.map(|slot| shared.names.name(slot)) {
                Some(Ok(name)) => name.into_bytes(),
                Some(Err(why)) => {
                    report.note(format!(
                        "an entry's order is undecided: its country's name: {why}"
                    ));
                    Vec::new()
                }
                None => {
                    report
                        .note("an entry's order is undecided: its country has no CountryLinks row");
                    Vec::new()
                }
            }
        };
        match item {
            Item::Shield(slot) => {
                let key = (0, name_bytes(Some(slot), report), 0, 0);
                (key, self.shield(slot, shared))
            }
            Item::Operative(row) => {
                let key = (
                    1,
                    name_bytes(self.slot(row.owner_country), report),
                    i64::from(row.ordinal),
                    0,
                );
                (key, self.operative(row, shared, report))
            }
            Item::Operation(row) => {
                let (entry, priority) = self.operation(row);
                (
                    (2, Vec::new(), i64::from(priority), row.definition_token),
                    entry,
                )
            }
            Item::Offer(row) => (
                (
                    2,
                    Vec::new(),
                    i64::from(self.defines.priorities.can_start),
                    row.definition,
                ),
                self.offer(row),
            ),
        }
    }

    fn strategic(
        &self,
        shared: &Shared<'h, 'f>,
        slot: u32,
    ) -> Result<Option<&'f StrategicOperativeRow>, String> {
        let rows = shared.tables.strategic.clone()?;
        Ok(rows.iter().find(|row| self.slot(row.country) == Some(slot)))
    }

    fn shield(&self, slot: u32, shared: &Shared<'h, 'f>) -> Entry {
        let mut writes = Vec::new();
        let level = self
            .defines
            .activity
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|activity| {
                let row = self.strategic(shared, slot)?;
                let sum = row.map_or(0, |row| {
                    row.mission_activity.wrapping_add(row.operation_activity)
                });
                let level = values::activity_level(sum, activity);
                Ok((level, values::activity_color(level, activity)))
            });
        match &level {
            Ok((_, color)) => writes.push(Write::Color("shield_icn", *color)),
            Err(why) => writes.push(Write::Withheld(
                "shield_icn",
                format!("its activity tint is undecided: {why}"),
            )),
        }
        let defense = self.agency_defense(slot, shared);
        match &defense {
            Ok(defense) => writes.push(Write::Text(
                "rating_txt",
                values::round_whole(*defense).to_string(),
            )),
            Err(why) => writes.push(Write::Withheld(
                "rating_txt",
                format!("its rating is undecided: {why}"),
            )),
        }
        Entry {
            kind: EntryKind::Shield,
            key: EntryKey::Shield(slot),
            writes,
            tooltip: Arc::new(tooltips::shield(
                self,
                slot,
                shared,
                defense,
                level.map(|(level, _)| level),
            )),
        }
    }

    fn agency_defense(&self, slot: u32, shared: &Shared<'h, 'f>) -> Result<i64, String> {
        let defines = self.defines.counter.as_ref().map_err(Clone::clone)?;
        let row = shared
            .tables
            .defense
            .clone()?
            .iter()
            .find(|row| self.slot(row.country) == Some(slot))
            .ok_or("the country has no RaidIntelInputs row")?;
        values::agency_defense(values::counter_rating(row, defines)?, defines)
    }

    fn operative(
        &self,
        row: &'f OperativeMissionRow,
        shared: &Shared<'h, 'f>,
        _report: &mut IntelReport,
    ) -> Entry {
        let mut writes = Vec::new();
        for (child, value) in [
            ("frame_select_hl_icn", row.map_selected),
            ("frame_warn_hl_icn", row.map_warning),
        ] {
            writes.push(match value {
                Recorded::Value(Some(value)) => Write::Shown(child, value != 0),
                Recorded::Value(None) => {
                    Write::Withheld(child, "its operative UI state was not captured".into())
                }
                Recorded::NotRecorded => {
                    Write::Withheld(child, "its operative UI state was not recorded".into())
                }
            });
        }
        let character = shared.tables.characters.clone().map(|rows| {
            rows.iter().find(|character| {
                character.resolved
                    && character.category == OPERATIVE_CATEGORY
                    && character.reference == row.id
            })
        });
        match self.portrait(row, character.clone(), shared) {
            Ok(sprite) => writes.push(Write::Portrait("portrait_icn", sprite)),
            Err(why) => writes.push(Write::Withheld("portrait_icn", why)),
        }
        writes.push(Write::Enabled("portrait_icn", row.assignment == ON_MISSION));
        let progress = if row.assignment == COOLDOWN {
            writes.push(Write::Shown("mission_icon", false));
            writes.push(Write::Shown("mission_status_icn", false));
            writes.push(Write::Shown("leader_enroute", true));
            row.cooldown_end_hour
                .ok_or_else(|| "the cooldown's end is not recorded".to_owned())
                .and_then(|end| {
                    Ok(values::cooldown_text(
                        end.wrapping_sub(self.frame.hour),
                        &localise(self.localisation, "DAY_UNIT_SHORT", &[])?,
                        &localise(self.localisation, "HOUR_UNIT_SHORT", &[])?,
                    ))
                })
        } else {
            writes.push(Write::Shown("leader_enroute", false));
            let variant = row.variant.unwrap_or_default();
            writes.push(Write::Shown("mission_icon", true));
            writes.push(Write::Frame("mission_icon", variant as i32));
            if variant == BUILD_NETWORK {
                match self.network(row, shared) {
                    Ok(network) => {
                        writes.push(Write::Shown("mission_status_icn", true));
                        writes.push(Write::Frame(
                            "mission_status_icn",
                            values::gain_frame(network.map_or(0, |row| row.gain_raw)),
                        ));
                    }
                    Err(why) => writes.push(Write::Withheld("mission_status_icn", why)),
                }
            } else {
                writes.push(Write::Shown("mission_status_icn", false));
            }
            self.progress(row, shared)
        };
        match progress {
            Ok(text) => writes.push(Write::Text("mission_progress_text", text)),
            Err(why) => writes.push(Write::Withheld("mission_progress_text", why)),
        }
        Entry {
            kind: EntryKind::Operative,
            key: EntryKey::Operative(row.roster_country, row.id),
            writes,
            tooltip: Arc::new(tooltips::operative(self, row, character, shared)),
        }
    }

    fn portrait(
        &self,
        row: &OperativeMissionRow,
        character: Result<Option<&CharacterOperativeRow>, String>,
        shared: &Shared<'h, 'f>,
    ) -> Result<Vec<u8>, String> {
        if let Some(character) = character? {
            let holders = shared.tables.portraits.clone()?;
            if let Some(holder) = holders
                .iter()
                .find(|holder| holder.character == character.character)
            {
                match portrait_entry(&holder.portraits, OPERATIVE_PORTRAIT_ROLE, PORTRAIT_LARGE) {
                    Some(entry) if entry.is_set != 0 && entry.kind == 0 => {
                        return Ok(entry.identity.clone());
                    }
                    Some(entry) if entry.is_set != 0 => {
                        return Err(format!(
                            "the operative's large portrait is of descriptor kind {}, a texture this icon does not load",
                            entry.kind
                        ));
                    }
                    _ => {}
                }
            }
        }
        Ok(row.portrait.clone())
    }

    pub(super) fn network(
        &self,
        row: &OperativeMissionRow,
        shared: &Shared<'h, 'f>,
    ) -> Result<Option<&'f replay_data::rows::IntelNetworkRow>, String> {
        let rows = shared.tables.networks.clone()?;
        let target = row
            .target_country
            .and_then(|raw| self.slot(raw))
            .ok_or("the mission's target country has no CountryLinks row")?;
        let state = row.target_state.ok_or("the mission has no target state")?;
        Ok(rows.iter().find(|network| {
            network.state_id == state
                && self.slot(network.owner_country) == Some(self.ours)
                && self.slot(network.target_country) == Some(target)
        }))
    }

    fn progress(
        &self,
        row: &OperativeMissionRow,
        shared: &Shared<'h, 'f>,
    ) -> Result<String, String> {
        let target = || {
            row.target_country
                .and_then(|raw| self.slot(raw))
                .ok_or_else(|| "the mission's target country has no CountryLinks row".to_owned())
        };
        let owner = || {
            self.slot(row.owner_country)
                .ok_or_else(|| "the operative's owner has no CountryLinks row".to_owned())
        };
        let share = |value: i64, max: i64| {
            values::divide(value, max)
                .ok_or_else(|| "the progress's division has no result".to_owned())
        };
        match row.variant.unwrap_or_default() {
            BUILD_NETWORK | QUIET_NETWORK => Ok(values::percent_points(
                self.network(row, shared)?
                    .map_or(0, |row| row.local_strength_raw),
            )),
            COUNTER_INTELLIGENCE => {
                let defines = self.defines.counter.as_ref().map_err(Clone::clone)?;
                let target = target()?;
                let inputs = shared
                    .tables
                    .defense
                    .clone()?
                    .iter()
                    .find(|inputs| self.slot(inputs.country) == Some(target))
                    .ok_or("the target country has no RaidIntelInputs row")?;
                let contribution = values::counter_contributions(inputs, defines)?
                    .into_iter()
                    .find(|(handle, _)| *handle == row.id)
                    .ok_or("the operative is not among its target's recorded counteroperatives")?;
                Ok(content_reader_parser::fixed5::fixed5_to_string(contribution.1, 2))
            }
            ROOT_OUT => {
                let key = (
                    owner()?,
                    row.target_state.ok_or("the mission has no target state")?,
                );
                let stacked = shared
                    .root_out
                    .get(&key)
                    .ok_or("the operative's root-out group is not recorded")?
                    .as_ref()
                    .map_err(Clone::clone)?;
                let (_, efficiency) = stacked
                    .iter()
                    .find(|(handle, _)| *handle == row.id)
                    .ok_or("the operative is not in its root-out group")?;
                Ok(values::percent(*efficiency))
            }
            BOOST_IDEOLOGY => {
                let target = target()?;
                let (mut total, mut matching) = (0_i64, 0_i64);
                for party in shared.tables.parties.clone()? {
                    if self.slot(party.country) != Some(target) {
                        continue;
                    }
                    if party.token.eq_ignore_ascii_case(&row.ideology) {
                        matching = party.popularity;
                    }
                    total = total.wrapping_add(party.popularity);
                }
                Ok(values::percent(values::ideology_share(matching, total)))
            }
            CONTROL_TRADE => {
                let max = self.defines.control_trade_max.clone()?;
                let owner = owner()?;
                let influence = self
                    .strategic(shared, target()?)?
                    .and_then(|row| {
                        row.trade_influence
                            .iter()
                            .find(|entry| self.slot(entry.owner) == Some(owner))
                    })
                    .map_or(0, |entry| entry.value);
                Ok(values::percent(share(influence, max)?))
            }
            DIPLOMATIC_PRESSURE => {
                let max = self.defines.diplomatic_max.clone()?;
                let owner = owner()?;
                let acceptance = self
                    .strategic(shared, target()?)?
                    .and_then(|row| {
                        row.diplomatic_pressure
                            .iter()
                            .find(|entry| self.slot(entry.owner) == Some(owner))
                    })
                    .map_or(0, |entry| {
                        replay_data::fixed_point::q15_to_fixed5(entry.acceptance)
                    });
                Ok(values::percent(share(acceptance, max)?))
            }
            PROPAGANDA => {
                let [stability_max, war_support_max] = self.defines.propaganda_max.clone()?;
                let target = target()?;
                let inputs = shared
                    .tables
                    .stability
                    .clone()?
                    .iter()
                    .find(|inputs| self.slot(inputs.country) == Some(target))
                    .ok_or("the target country has no CountryStabilityInputs row")?;
                let stability = share(
                    replay_data::fixed_point::q15_to_fixed5(inputs.propaganda_stability_q15),
                    stability_max,
                )?;
                let war_support = share(
                    replay_data::fixed_point::q15_to_fixed5(inputs.propaganda_war_support_q15),
                    war_support_max,
                )?;
                Ok(values::percent(stability.min(war_support)))
            }
            variant => Err(format!("mission variant {variant} has no progress")),
        }
    }

    fn offer(&self, row: &replay_data::rows::AgencyBufferedOffer) -> Entry {
        let definition = (|| {
            let spelling = super::super::raids::spelling(self.world, row.definition, "operation")?;
            self.assets
                .operation(spelling)?
                .ok_or_else(|| format!("operation {spelling} is not in the content"))
        })();
        let mut writes = vec![
            Write::Shown("operation_icon", false),
            Write::Shown("phase_circular_progress_icon", false),
            Write::Shown("phase_btn", false),
            Write::Shown("operation_completed_overlay", false),
            Write::Shown("operation_completed_warning", false),
            Write::Shown("operation_equipment_overlay", false),
            Write::Shown("portrait_btn", true),
        ];
        match &definition {
            Ok(definition) => writes.push(Write::Sprite(
                "portrait_btn",
                definition.map_icon.as_bytes().to_vec(),
            )),
            Err(why) => writes.push(Write::Withheld("portrait_btn", why.clone())),
        }
        match row.duration {
            Some(days) => match localise(
                self.localisation,
                "ABILITY_DUR_DAYS",
                &[("num", Arg::Text(&days.to_string()))],
            ) {
                Ok(text) => writes.push(Write::Text("operation_progress_text", text)),
                Err(why) => writes.push(Write::Withheld("operation_progress_text", why)),
            },
            None => writes.push(Write::Withheld(
                "operation_progress_text",
                "its duration operands were not captured".into(),
            )),
        }
        Entry {
            kind: EntryKind::Operation,
            key: EntryKey::Offer(row.definition, row.country),
            writes,
            tooltip: Arc::new(tooltips::offer(self, row, definition)),
        }
    }

    fn operation(&self, row: &'f OperationRow) -> (Entry, i32) {
        let hour = self.frame.hour;
        let state = values::operation_state(row, hour);
        let value = matches!(state, OperationState::Preparing).then(|| values::preparation(row));
        let prepared = matches!(value, Some(Ok(ONE)));
        let priorities = self.defines.priorities;
        let priority = match state {
            OperationState::Finished => priorities.completed,
            OperationState::Running { .. } => priorities.in_progress,
            OperationState::Preparing if prepared => priorities.prepared,
            OperationState::Preparing => priorities.default,
        };
        let definition = self.definition(row);
        let map_icon = definition
            .as_ref()
            .map(|definition| definition.map_icon.as_bytes().to_vec());
        let mut writes = Vec::new();
        if matches!(state, OperationState::Preparing) {
            writes.push(Write::Shown("operation_icon", false));
        } else {
            writes.push(Write::Shown("operation_icon", true));
            match &map_icon {
                Ok(sprite) => writes.push(Write::Sprite("operation_icon", sprite.clone())),
                Err(why) => writes.push(Write::Withheld("operation_icon", why.to_string())),
            }
            writes.push(Write::Frame("operation_icon", 1));
        }
        let (completed, running) = match state {
            OperationState::Finished => (true, None),
            OperationState::Running { elapsed, remaining } => {
                (remaining == 0, Some((elapsed, remaining)))
            }
            OperationState::Preparing => (false, None),
        };
        match running.filter(|_| !completed) {
            Some((elapsed, _)) => {
                writes.push(Write::Shown("phase_circular_progress_icon", true));
                writes.push(Write::Progress(
                    "phase_circular_progress_icon",
                    values::phase_progress(elapsed, row.duration_days),
                ));
            }
            None => writes.push(Write::Shown("phase_circular_progress_icon", false)),
        }
        match self.phase_sprite(row) {
            Ok(Some(sprite)) => {
                writes.push(Write::Shown("portrait_btn", false));
                writes.push(Write::Shown("phase_btn", true));
                writes.push(Write::Sprite("phase_btn", sprite));
            }
            Ok(None) => {
                writes.push(Write::Shown("phase_btn", false));
                writes.push(Write::Shown("portrait_btn", true));
                match &map_icon {
                    Ok(sprite) => writes.push(Write::Sprite("portrait_btn", sprite.clone())),
                    Err(why) => writes.push(Write::Withheld("portrait_btn", why.to_string())),
                }
            }
            Err(why) => {
                writes.push(Write::Shown("portrait_btn", false));
                writes.push(Write::Withheld("phase_btn", why));
            }
        }
        let text = match (state, &value) {
            (OperationState::Preparing, Some(Ok(ONE))) => {
                localise(self.localisation, "PREPARED", &[])
                    .map(|text| values::coloured('Y', &text))
            }
            (OperationState::Preparing, Some(Ok(value))) => Ok(values::percent(*value)),
            (OperationState::Preparing, Some(Err(why))) => {
                Err(format!("its preparation is undecided: {why}"))
            }
            (OperationState::Running { remaining, .. }, _) if remaining != 0 => {
                running_duration_text(self.localisation, remaining)
            }
            _ => localise(self.localisation, "FINISHED", &[])
                .map(|text| values::coloured('G', &text)),
        };
        match text {
            Ok(text) => writes.push(Write::Text("operation_progress_text", text)),
            Err(why) => writes.push(Write::Withheld("operation_progress_text", why)),
        }
        let detected = row
            .detected
            .require("Operations.detected")
            .map(|detected| *detected != 0);
        if completed {
            match detected {
                Ok(detected) => {
                    writes.push(Write::Shown("operation_completed_overlay", true));
                    writes.push(Write::Frame(
                        "operation_completed_overlay",
                        if detected { 2 } else { 1 },
                    ));
                    writes.push(Write::Shown("operation_completed_warning", detected));
                    if detected {
                        writes.push(Write::Frame("operation_completed_warning", 3));
                    }
                }
                Err(why) => {
                    writes.push(Write::Withheld("operation_completed_overlay", why.clone()));
                    writes.push(Write::Withheld("operation_completed_warning", why));
                }
            }
        } else {
            writes.push(Write::Shown("operation_completed_overlay", false));
            writes.push(Write::Shown("operation_completed_warning", false));
        }
        match &value {
            Some(Ok(value)) => {
                writes.push(Write::Shown("operation_equipment_overlay", true));
                writes.push(Write::Frame(
                    "operation_equipment_overlay",
                    if *value == ONE { 2 } else { 1 },
                ));
            }
            Some(Err(why)) => writes.push(Write::Withheld(
                "operation_equipment_overlay",
                format!("its preparation is undecided: {why}"),
            )),
            None => writes.push(Write::Shown("operation_equipment_overlay", false)),
        }
        let entry = Entry {
            kind: EntryKind::Operation,
            key: EntryKey::Operation(row.id),
            writes,
            tooltip: Arc::new(tooltips::operation(
                self,
                row,
                definition.ok(),
                state,
                value,
            )),
        };
        (entry, priority)
    }

    pub fn definition(
        &self,
        row: &OperationRow,
    ) -> Result<&'h content_reader_military::intelligence::Operation, String> {
        let spelling =
            super::super::raids::spelling(self.world, row.definition_token, "operation")?;
        self.assets
            .operation(spelling)?
            .ok_or_else(|| format!("operation {spelling} is not in the content"))
    }

    fn phase_sprite(&self, row: &OperationRow) -> Result<Option<Vec<u8>>, String> {
        let Some(index) = values::current_phase(row, self.frame.hour) else {
            return Ok(None);
        };
        let token = *row
            .phases
            .get(index)
            .ok_or_else(|| format!("the operation's phase {index} is not recorded"))?;
        let spelling = super::super::raids::spelling(self.world, token, "operation phase")?;
        let phase = self
            .assets
            .phase(spelling)?
            .ok_or_else(|| format!("operation phase {spelling} is not in the content"))?;
        Ok(Some(phase.map_sprite().as_bytes().to_vec()))
    }

    pub fn state_name(&self, shared: &Shared<'h, 'f>, state: u32) -> Result<String, String> {
        let row = shared
            .tables
            .states
            .clone()?
            .iter()
            .find(|row| row.id == state);
        let (custom, owner) = row.map_or((&[][..], &[][..]), |row| {
            (row.custom_name.as_slice(), row.owner_name.as_slice())
        });
        viewer_foundation::tooltip_text::state_name(
            self.localisation,
            self.language,
            state,
            custom,
            owner,
            || {
                let key = self
                    .raids
                    .tooltip
                    .state_names
                    .get(&state)
                    .ok_or_else(|| format!("state {state} is not in the content"))?
                    .clone()?;
                localise(self.localisation, &key, &[])
            },
        )
    }
}

pub(super) fn running_duration_text(
    localisation: &content_reader_localisation::localisation::Localisation,
    remaining: i32,
) -> Result<String, String> {
    let number = values::coloured('G', &remaining.to_string());
    localise(
        localisation,
        "ABILITY_DUR_DAYS",
        &[("num", Arg::Text(&number))],
    )
}
