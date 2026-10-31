//! An hour observed by its changes: only the rows they name are observed again.

use super::{Field, Prepared};
use dashboard_base::{
    metadata::CountryKeys,
    prepared::frame_changes::FrameChanges,
    products::{equipment, production_lines},
};
use replay_data::rows::{
    CountryLinkRow, CountryRow, DeltaRow, IndustrialOrganisationDetailsRow,
    IndustrialOrganisationPoliciesRow, IndustrialOrganisationRow, LexicalTokenRow, TableId, tables,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
};

const LICENSEE_UNIDENTIFIED: &str = "a licence's licensee has no identity";
const LICENSOR_UNIDENTIFIED: &str = "a licence's licensor has no identity";
const ORGANISATION_COUNTRY: &str = "an organisation's country has no identity";
const POLICY_UNSPELLED: &str = "a selected policy's token is not spelled";
const DESIGN_CREATOR: &str = "a design's creator has no identity";

/// Each domain's tables that its rows read besides the country identities.
const INPUTS: [&[TableId]; 3] = [
    &[TableId::ProductionLicenseRecords],
    &[
        TableId::IndustrialOrganisations,
        TableId::IndustrialOrganisationDetails,
        TableId::IndustrialOrganisationPolicies,
        TableId::LexicalTokens,
    ],
    &[TableId::VariantDefinitions],
];

/// What observing the next hour by its changes needs; a restore observes whole.
#[derive(Default)]
pub(super) struct Open {
    /// None where the next observation reads the identities again.
    identity: Option<Identity>,
    /// None where the domain's next observation is whole.
    domains: [Option<Domain>; 3],
    /// Whether a domain's keys were all ended and none was observed since.
    ended: [bool; 3],
    /// Rows observed, counted in tests.
    #[cfg(test)]
    pub(super) rows: usize,
}

impl Open {
    pub(super) fn bytes(&self) -> usize {
        self.identity.as_ref().map_or(0, Identity::bytes)
            + self
                .domains
                .iter()
                .flatten()
                .map(Domain::bytes)
                .sum::<usize>()
    }
}

/// The country identities as `keyed::Tags` reads them.
struct Identity {
    keys: CountryKeys,
    links: BTreeMap<u32, u32>,
    tags: BTreeMap<u32, Vec<u8>>,
    /// Each slot's raw links, ascending.
    raws: BTreeMap<u32, Vec<u32>>,
}

impl Identity {
    fn read(countries: &[CountryRow], links: &[CountryLinkRow]) -> Self {
        let links: BTreeMap<u32, u32> = links
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let mut raws: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for (&raw, &slot) in &links {
            raws.entry(slot).or_default().push(raw);
        }
        Self {
            keys: CountryKeys::read(countries),
            links,
            tags: countries
                .iter()
                .map(|row| (row.canonical_slot, row.tag.clone()))
                .collect(),
            raws,
        }
    }

    fn slot(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    fn tag(&self, slot: u32) -> Option<String> {
        self.tags
            .get(&slot)
            .and_then(|tag| std::str::from_utf8(tag).ok())
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
    }

    fn bytes(&self) -> usize {
        self.keys.heap_bytes()
            + self.links.len() * 48
            + self.tags.values().map(|tag| tag.len() + 64).sum::<usize>()
            + self.raws.len() * 64
            + self.links.len() * 4
    }
}

/// A domain at its last observation.
#[derive(Default)]
struct Domain {
    /// Which of its [`INPUTS`] were complete.
    complete: Vec<bool>,
    /// Each refusing row's reasons, by row key.
    refusing: BTreeMap<u128, Vec<&'static str>>,
    /// How many rows stood for each reason.
    standing: BTreeMap<&'static str, u64>,
    /// Organisations only: the rows each organisation joins.
    joins: Joins,
}

impl Domain {
    fn bytes(&self) -> usize {
        self.complete.len()
            + self.refusing.len() * 64
            + self.standing.len() * 48
            + self.joins.bytes()
    }

    /// The row's reasons, replacing those it stood for.
    fn refusals(&mut self, row: u128, reasons: Vec<&'static str>) {
        if let Some(held) = self.refusing.remove(&row) {
            for reason in held {
                let count = self.standing.get_mut(reason).expect("a standing reason");
                *count -= 1;
                if *count == 0 {
                    self.standing.remove(reason);
                }
            }
        }
        if !reasons.is_empty() {
            for &reason in &reasons {
                *self.standing.entry(reason).or_default() += 1;
            }
            self.refusing.insert(row, reasons);
        }
    }
}

/// Each organisation's policy roster country and spelled tokens.
#[derive(Default)]
struct Joins {
    of: HashMap<u64, (Option<u32>, Vec<u32>)>,
    countries: BTreeSet<(u32, u64)>,
    tokens: BTreeSet<(u32, u64)>,
}

impl Joins {
    fn bytes(&self) -> usize {
        self.of.len() * 64 + (self.countries.len() + self.tokens.len()) * 40
    }

    fn set(&mut self, id: u64, country: Option<u32>, tokens: Vec<u32>) {
        if let Some((country, tokens)) = self.of.remove(&id) {
            if let Some(country) = country {
                self.countries.remove(&(country, id));
            }
            for token in tokens {
                self.tokens.remove(&(token, id));
            }
        }
        if country.is_none() && tokens.is_empty() {
            return;
        }
        if let Some(country) = country {
            self.countries.insert((country, id));
        }
        for &token in &tokens {
            self.tokens.insert((token, id));
        }
        self.of.insert(id, (country, tokens));
    }

    fn joining<'a>(set: &'a BTreeSet<(u32, u64)>, key: u32) -> impl Iterator<Item = u64> + 'a {
        set.range((key, 0)..=(key, u64::MAX)).map(|&(_, id)| id)
    }
}

/// The rows `changes` names in `table`, or None where it cannot name them.
fn changed_rows<'c>(
    changes: Option<&'c FrameChanges>,
    domain: &Domain,
    index: usize,
    table: TableId,
) -> Option<(&'c [DeltaRow], &'c [u128])> {
    let changes = changes?;
    if changes.rows_changed[table.index()] && !domain.complete[index] {
        return None;
    }
    changes.rows(table)
}

fn keys_of<'c>(rows: Option<(&'c [DeltaRow], &'c [u128])>) -> impl Iterator<Item = u128> + 'c {
    rows.into_iter().flat_map(|(changed, deleted)| {
        changed
            .iter()
            .map(|row| row.key)
            .chain(deleted.iter().copied())
    })
}

/// The licence fields a record's licensee holds at a roster position.
type LicenceFields = BTreeMap<Field, i128>;

/// One organisation's fields after its holder, in the order observed.
const ORGANISATION_STATE: [fn(u64) -> Field; 8] = [
    Field::MioSize,
    Field::MioFunds,
    Field::MioPoints,
    Field::MioUnlocked,
    Field::MioCooldown,
    Field::MioPolicyUnresolved,
    Field::MioPolicy,
    Field::MioPolicyName,
];

impl Prepared {
    pub(super) fn observe(
        &mut self,
        frame: &DecodedFrame,
        domains: [bool; 3],
        changes: Option<&FrameChanges>,
    ) {
        let hour = frame.hour;
        let identity = match self.identity(frame, changes) {
            Ok(identity) => identity,
            Err(reason) => {
                self.keyed.refuse(reason);
                self.open.identity = None;
                self.open.domains = Default::default();
                for domain in 0..3 {
                    if domains[domain] {
                        self.end(domain as u8, hour, &HashSet::new());
                    }
                }
                return;
            }
        };
        let changes = changes.filter(|_| !identity);
        let identity = self.open.identity.take().expect("identities just read");
        if domains[0] {
            self.licences(frame, &identity, changes);
        }
        if domains[1] {
            self.organisations(frame, &identity, changes);
        }
        if domains[2] {
            self.designs(frame, &identity, changes);
        }
        self.open.identity = Some(identity);
    }

    /// Whether the identities were read again, or why they cannot be.
    fn identity(
        &mut self,
        frame: &DecodedFrame,
        changes: Option<&FrameChanges>,
    ) -> Result<bool, String> {
        let unavailable = |error: io::Error| {
            format!(
                "country identities unavailable: {}",
                io::Error::other(error)
            )
        };
        let countries = frame.complete::<tables::Countries>().map_err(unavailable)?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(unavailable)?;
        let kept = self.open.identity.as_ref().is_some_and(|held| {
            changes.is_some_and(|changes| {
                !changes.rows_changed[TableId::CountryLinks.index()]
                    && (!changes.rows_changed[TableId::Countries.index()]
                        || held
                            .keys
                            .kept_changes(countries.len(), changes.rows(TableId::Countries))
                        || held.keys.same(countries))
            })
        });
        if kept {
            return Ok(false);
        }
        self.open.identity = Some(Identity::read(countries, links));
        self.open.domains = Default::default();
        Ok(true)
    }

    /// Publish a field's value, or end it; `seen` collects published keys.
    fn put(
        &mut self,
        field: Field,
        value: Option<i128>,
        hour: i32,
        seen: &mut Option<HashSet<u64>>,
    ) {
        match value {
            Some(value) => {
                let key = self.keyed.key(field);
                self.keyed
                    .values
                    .observe_if_changed(key, hour, Some(value), false, false);
                if let Some(seen) = seen {
                    seen.insert(key);
                }
            }
            // A keyed field has a series state.
            None => {
                if let Some(key) = self.keyed.lookup(&field) {
                    self.keyed
                        .values
                        .observe_if_changed(key, hour, None, false, false);
                }
            }
        }
    }

    /// End every held key of `domain` but those `seen`.
    fn end(&mut self, domain: u8, hour: i32, seen: &HashSet<u64>) {
        if self.open.ended[usize::from(domain)] {
            return;
        }
        let ended: Vec<u64> = self
            .keyed
            .values
            .active()
            .filter(|key| !seen.contains(key))
            .filter(|key| {
                self.keyed
                    .meta
                    .fields
                    .get(key)
                    .is_some_and(|field| field.domain() == domain)
            })
            .collect();
        for key in ended {
            self.keyed.values.observe(key, hour, None, false);
        }
        if seen.is_empty() {
            self.open.ended[usize::from(domain)] = true;
        }
    }

    /// A domain's roster read failed: it is unknown from this hour.
    fn fail(&mut self, domain: u8, hour: i32) {
        self.open.domains[usize::from(domain)] = None;
        self.end(domain, hour, &HashSet::new());
    }

    fn known(&mut self, domain: u8, hour: i32, seen: &mut Option<HashSet<u64>>) {
        self.open.ended[usize::from(domain)] = false;
        self.put(Field::Known(domain), Some(1), hour, seen);
    }

    /// Count standing refusals, end what a whole observation missed, keep the domain.
    fn close(
        &mut self,
        index: u8,
        frame: &DecodedFrame,
        mut domain: Domain,
        seen: Option<HashSet<u64>>,
    ) {
        for (&reason, &count) in &domain.standing {
            self.keyed.refuse_times(reason, count);
        }
        if let Some(seen) = seen {
            self.end(index, frame.hour, &seen);
        }
        domain.complete = INPUTS[usize::from(index)]
            .iter()
            .map(|&table| matches!(*frame.tables[table.index()], TableView::Complete(_)))
            .collect();
        self.open.domains[usize::from(index)] = Some(domain);
    }

    fn licences(
        &mut self,
        frame: &DecodedFrame,
        identity: &Identity,
        changes: Option<&FrameChanges>,
    ) {
        let hour = frame.hour;
        let rows = match frame.complete::<tables::ProductionLicenseRecords>() {
            Ok(rows) => rows,
            Err(error) => {
                self.keyed.refuse(format!("licences unavailable: {error}"));
                self.fail(0, hour);
                return;
            }
        };
        let held = self.open.domains[0].take();
        let changed = held
            .as_ref()
            .and_then(|domain| changed_rows(changes, domain, 0, TableId::ProductionLicenseRecords));
        let (mut domain, mut seen, records) = match (held, changed) {
            (Some(domain), Some(changed)) => {
                // Records of one licensee slot and position share fields.
                let mut positions = BTreeSet::new();
                let mut records = BTreeSet::new();
                for key in keys_of(Some(changed)) {
                    records.insert(key);
                    let (owner, at) = ((key >> 32) as u32, key as u32);
                    if let Some(slot) = identity.slot(owner) {
                        positions.insert((slot, at));
                    }
                }
                for &(slot, at) in &positions {
                    for &raw in identity.raws.get(&slot).into_iter().flatten() {
                        records.insert(u128::from(raw) << 32 | u128::from(at));
                    }
                }
                (domain, None, (Some(records), positions))
            }
            _ => (
                Domain::default(),
                Some(HashSet::new()),
                (None, BTreeSet::new()),
            ),
        };
        let (records, positions) = records;
        let mut fields: BTreeMap<(u32, u32), LicenceFields> = positions
            .iter()
            .map(|&position| (position, BTreeMap::new()))
            .collect();
        let row_at = |key: u128| {
            rows.binary_search_by_key(&key, |row| {
                u128::from(row.owner) << 32 | u128::from(row.ordinal)
            })
            .ok()
            .map(|index| &rows[index])
        };
        let mut observe =
            |state: &mut Self,
             key: u128,
             row: Option<&replay_data::rows::ProductionLicenseRecordRow>| {
                #[cfg(test)]
                {
                    state.open.rows += 1;
                }
                let Some(row) = row else {
                    domain.refusals(key, Vec::new());
                    return;
                };
                let Some(owner) = identity.slot(row.owner) else {
                    domain.refusals(key, vec![LICENSEE_UNIDENTIFIED]);
                    return;
                };
                let giver = identity.slot(row.giver);
                domain.refusals(
                    key,
                    giver.map_or_else(|| vec![LICENSOR_UNIDENTIFIED], |_| Vec::new()),
                );
                state.keyed.tag_text(owner, identity.tag(owner), hour);
                let at = row.ordinal;
                let licensor = match giver {
                    Some(giver) => {
                        state.keyed.tag_text(giver, identity.tag(giver), hour);
                        (Field::LicenceGiver(owner, at), i128::from(giver))
                    }
                    None => (Field::LicenceUnidentified(owner, at), 1),
                };
                let position = fields.entry((owner, at)).or_default();
                for (field, value) in [
                    licensor,
                    (Field::LicenceLent(owner, at), i128::from(row.lended_cic)),
                    (
                        Field::LicenceRequired(owner, at),
                        i128::from(row.required_cic),
                    ),
                    (Field::LicenceStart(owner, at), i128::from(row.start_date)),
                ] {
                    state.keyed.key(field);
                    position.insert(field, value);
                }
                for (index, &variant) in row.equipment.iter().enumerate() {
                    let field = Field::LicenceVariant(owner, at, index as u32);
                    state.keyed.key(field);
                    position.insert(field, i128::from(variant));
                }
            };
        match &records {
            Some(records) => {
                for &key in records {
                    observe(self, key, row_at(key));
                }
            }
            None => {
                for row in rows {
                    observe(
                        self,
                        u128::from(row.owner) << 32 | u128::from(row.ordinal),
                        Some(row),
                    );
                }
            }
        }
        for ((owner, at), now) in fields {
            if records.is_some() {
                // The fields the position held and no longer shows end.
                let mut held: Vec<Field> = [
                    Field::LicenceGiver(owner, at),
                    Field::LicenceUnidentified(owner, at),
                    Field::LicenceLent(owner, at),
                    Field::LicenceRequired(owner, at),
                    Field::LicenceStart(owner, at),
                ]
                .into_iter()
                .collect();
                held.extend(
                    self.keyed
                        .keys_in(
                            Field::LicenceVariant(owner, at, 0)
                                ..=Field::LicenceVariant(owner, at, u32::MAX),
                        )
                        .map(|(field, _)| *field),
                );
                for field in held {
                    if !now.contains_key(&field) {
                        self.put(field, None, hour, &mut seen);
                    }
                }
            }
            for (field, value) in now {
                self.put(field, Some(value), hour, &mut seen);
            }
        }
        // A licence whose licensee has no identity is left out: the roster is not known whole.
        if domain.standing.contains_key(LICENSEE_UNIDENTIFIED) {
            self.open.ended[0] = false;
            self.put(Field::Known(0), None, hour, &mut seen);
        } else {
            self.known(0, hour, &mut seen);
        }
        self.close(0, frame, domain, seen);
    }

    fn organisations(
        &mut self,
        frame: &DecodedFrame,
        identity: &Identity,
        changes: Option<&FrameChanges>,
    ) {
        let hour = frame.hour;
        // A failed token dictionary refuses only the names.
        let (roster, countries, links) = match (
            frame.complete::<tables::IndustrialOrganisations>(),
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) {
            (Ok(roster), Ok(countries), Ok(links)) => (roster, countries, links),
            (Err(error), ..) | (_, Err(error), _) | (.., Err(error)) => {
                self.keyed
                    .refuse(format!("organisations unavailable: {error}"));
                self.fail(1, hour);
                return;
            }
        };
        let lexical = match frame.complete::<tables::LexicalTokens>() {
            Ok(lexical) => Some(lexical),
            Err(error) => {
                self.keyed
                    .refuse(format!("organisation names unavailable: {error}"));
                None
            }
        };
        let details = frame.complete::<tables::IndustrialOrganisationDetails>();
        if let Err(error) = &details {
            self.keyed
                .refuse(format!("organisation state unavailable: {error}"));
        }
        let policies = frame.complete::<tables::IndustrialOrganisationPolicies>();
        if let Err(error) = &policies {
            self.keyed
                .refuse(format!("organisation policies unavailable: {error}"));
        }
        let held = self.open.domains[1].take();
        // Organisations to observe again, and whether their naming may have changed.
        let changed = held.as_ref().and_then(|domain| {
            let at = |index| changed_rows(changes, domain, index, INPUTS[1][index]);
            let (roster, details, policies, lexical) = (at(0)?, at(1)?, at(2)?, at(3)?);
            let mut changed: BTreeMap<u64, bool> = BTreeMap::new();
            for key in keys_of(Some(roster)) {
                changed.insert(key as u64, true);
            }
            for key in keys_of(Some(lexical)) {
                for id in Joins::joining(&domain.joins.tokens, key as u32) {
                    changed.insert(id, true);
                }
            }
            for key in keys_of(Some(details)) {
                changed.entry(key as u64).or_insert(false);
            }
            for key in keys_of(Some(policies)) {
                for id in Joins::joining(&domain.joins.countries, key as u32) {
                    changed.entry(id).or_insert(false);
                }
            }
            Some(changed)
        });
        let (mut domain, mut seen) = match (held, &changed) {
            (Some(domain), Some(_)) => (domain, None),
            _ => (Domain::default(), Some(HashSet::new())),
        };
        let mut naming = None;
        let context = Organisations {
            countries,
            links,
            appearance: frame.complete::<tables::CountryAppearance>().ok(),
            lexical,
            details: details.as_ref().ok().copied(),
            policies: policies.as_ref().ok().copied(),
        };
        self.known(1, hour, &mut seen);
        match changed {
            Some(changed) => {
                for (id, named) in changed {
                    let row = roster
                        .binary_search_by_key(&id, |row| row.organisation)
                        .ok()
                        .map(|index| &roster[index]);
                    self.organisation(
                        &context,
                        &mut naming,
                        identity,
                        &mut domain,
                        id,
                        row,
                        named,
                        hour,
                        &mut seen,
                    );
                }
            }
            None => {
                for row in roster {
                    self.organisation(
                        &context,
                        &mut naming,
                        identity,
                        &mut domain,
                        row.organisation,
                        Some(row),
                        true,
                        hour,
                        &mut seen,
                    );
                }
            }
        }
        self.close(1, frame, domain, seen);
    }

    /// One organisation, its naming read again where `named`.
    #[allow(clippy::too_many_arguments)]
    fn organisation<'f>(
        &mut self,
        at: &Organisations<'f>,
        naming: &mut Option<equipment::Tags<'f>>,
        identity: &Identity,
        domain: &mut Domain,
        id: u64,
        row: Option<&IndustrialOrganisationRow>,
        named: bool,
        hour: i32,
        seen: &mut Option<HashSet<u64>>,
    ) {
        #[cfg(test)]
        {
            self.open.rows += 1;
        }
        let key = u128::from(id);
        let Some((row, holder)) = row.and_then(|row| Some((row, identity.slot(row.country))))
        else {
            domain.refusals(key, Vec::new());
            domain.joins.set(id, None, Vec::new());
            self.end_organisation(id, true, hour, seen);
            return;
        };
        let Some(holder) = holder else {
            domain.refusals(key, vec![ORGANISATION_COUNTRY]);
            domain.joins.set(id, None, Vec::new());
            self.end_organisation(id, true, hour, seen);
            return;
        };
        self.keyed.tag_text(holder, identity.tag(holder), hour);
        self.put(Field::MioHolder(id), Some(i128::from(holder)), hour, seen);
        if named {
            // Without the dictionary the names are unknown from this read on.
            let raw = match at.lexical {
                Some(lexical) => {
                    let naming = naming.get_or_insert_with(|| {
                        equipment::Tags::new(at.countries, at.links, at.appearance)
                    });
                    production_lines::raw_organisation(row, hour, naming, lexical)
                }
                None => production_lines::RawOrganisation {
                    first_hour: hour,
                    owner_unknown: true,
                    ..Default::default()
                },
            };
            let versions = self.keyed.meta.extra.organisations.entry(id).or_default();
            if versions.last().is_none_or(|last| {
                (
                    &last.token,
                    &last.name_key,
                    &last.icon,
                    &last.owner_tag,
                    last.owner_unknown,
                ) != (
                    &raw.token,
                    &raw.name_key,
                    &raw.icon,
                    &raw.owner_tag,
                    raw.owner_unknown,
                )
            }) {
                let bytes = raw.token.len()
                    + raw.name_key.len()
                    + raw.icon.len()
                    + raw.owner_tag.as_ref().map_or(0, String::len)
                    + 160;
                versions.push(raw);
                self.keyed.note_change(bytes);
            }
        }
        let detail = at.details.and_then(|details| {
            details
                .binary_search_by_key(&id, |detail| detail.organisation)
                .ok()
                .map(|index| &details[index])
                .filter(|detail| detail.country == row.country)
        });
        let Some(detail) = detail else {
            domain.refusals(key, Vec::new());
            domain.joins.set(id, None, vec![row.token]);
            self.end_organisation(id, false, hour, seen);
            return;
        };
        // Without the dictionary a selected policy reads as unspelled.
        let state = production_lines::organisation_state(detail, None, at.lexical.unwrap_or(&[]));
        let mut values: [Option<i128>; 8] = [
            Some(i128::from(detail.state.size)),
            Some(i128::from(detail.state.funds_raw)),
            Some(i128::from(detail.state.points)),
            Some(detail.unlocked.len() as i128),
            detail.state.cooldown_hour.map(i128::from),
            None,
            None,
            None,
        ];
        let mut reasons = Vec::new();
        if state.policy_token.as_deref() == Some("") {
            reasons.push(POLICY_UNSPELLED);
            values[5] = Some(1);
        }
        if let Some(token) = state
            .policy_token
            .as_ref()
            .filter(|token| !token.is_empty())
        {
            values[6] = Some(self.text(token.clone()));
        }
        // The country's stored policy name joins by the raw token, spelled or not.
        if state.policy_token.is_some() {
            let name = at.policies.and_then(|rows| {
                rows.binary_search_by_key(&detail.country, |row| row.country)
                    .ok()
                    .and_then(|index| {
                        rows[index]
                            .policies
                            .iter()
                            .find(|policy| policy.token == detail.state.policy)
                    })
                    .map(|policy| String::from_utf8_lossy(&policy.name).into_owned())
            });
            if let Some(name) = name.filter(|name| !name.is_empty()) {
                values[7] = Some(self.text(name));
            }
        }
        for (field, value) in ORGANISATION_STATE.iter().zip(values) {
            self.put(field(id), value, hour, seen);
        }
        domain.refusals(key, reasons);
        domain.joins.set(
            id,
            state.policy_token.is_some().then_some(detail.country),
            vec![row.token, detail.state.policy],
        );
    }

    /// An organisation's fields end: all of them, or those after its holder.
    fn end_organisation(
        &mut self,
        id: u64,
        holder: bool,
        hour: i32,
        seen: &mut Option<HashSet<u64>>,
    ) {
        if seen.is_some() {
            // A whole observation ends what it did not see.
            return;
        }
        if holder {
            self.put(Field::MioHolder(id), None, hour, seen);
        }
        for field in ORGANISATION_STATE {
            self.put(field(id), None, hour, seen);
        }
    }

    fn designs(
        &mut self,
        frame: &DecodedFrame,
        identity: &Identity,
        changes: Option<&FrameChanges>,
    ) {
        let hour = frame.hour;
        let rows = match frame.complete::<tables::VariantDefinitions>() {
            Ok(rows) => rows,
            Err(error) => {
                self.keyed.refuse(format!("designs unavailable: {error}"));
                self.fail(2, hour);
                return;
            }
        };
        if self.keyed.meta.extra.designs_from.is_none() {
            self.keyed.meta.extra.designs_from = Some(hour);
            self.keyed.note_change(16);
        }
        let held = self.open.domains[2].take();
        let changed = held
            .as_ref()
            .and_then(|domain| changed_rows(changes, domain, 0, TableId::VariantDefinitions))
            .map(|changed| {
                keys_of(Some(changed))
                    .map(|key| key as u64)
                    .collect::<BTreeSet<u64>>()
            });
        let (mut domain, mut seen) = match (held, &changed) {
            (Some(domain), Some(_)) => (domain, None),
            _ => (Domain::default(), Some(HashSet::new())),
        };
        self.known(2, hour, &mut seen);
        let mut observe =
            |state: &mut Self, id: u64, row: Option<&replay_data::rows::VariantDefinitionRow>| {
                #[cfg(test)]
                {
                    state.open.rows += 1;
                }
                let creator = row.map(|row| (row, identity.slot(row.creator_link)));
                match creator {
                    Some((row, Some(creator))) => {
                        domain.refusals(u128::from(id), Vec::new());
                        state.keyed.tag_text(creator, identity.tag(creator), hour);
                        // A newly published design publishes its whole naming history.
                        if state.keyed.lookup(&Field::DesignCreator(id)).is_none() {
                            let bytes = state
                                .variants
                                .versions
                                .get(&id)
                                .map_or(0, |versions| super::keyed::json_len(versions) + 24);
                            state.keyed.note_change(bytes);
                        }
                        state.put(
                            Field::DesignCreator(id),
                            Some(i128::from(creator)),
                            hour,
                            &mut seen,
                        );
                        state.put(
                            Field::DesignObsolete(id),
                            Some(i128::from(row.obsolete != 0)),
                            hour,
                            &mut seen,
                        );
                    }
                    Some((_, None)) | None => {
                        domain.refusals(
                            u128::from(id),
                            if creator.is_some() {
                                vec![DESIGN_CREATOR]
                            } else {
                                Vec::new()
                            },
                        );
                        state.put(Field::DesignCreator(id), None, hour, &mut seen);
                        state.put(Field::DesignObsolete(id), None, hour, &mut seen);
                    }
                }
            };
        match changed {
            Some(changed) => {
                for id in changed {
                    let row = rows
                        .binary_search_by_key(&id, |row| row.id)
                        .ok()
                        .map(|index| &rows[index]);
                    observe(self, id, row);
                }
            }
            None => {
                for row in rows {
                    observe(self, row.id, Some(row));
                }
            }
        }
        self.close(2, frame, domain, seen);
    }

    fn text(&mut self, text: String) -> i128 {
        let texts = &mut self.keyed.meta.extra.texts;
        if let Some(index) = texts.iter().position(|held| *held == text) {
            return index as i128;
        }
        let bytes = text.len();
        texts.push(text);
        let index = texts.len() - 1;
        self.keyed.note_change(bytes);
        index as i128
    }
}

/// The tables an hour's organisations read.
struct Organisations<'f> {
    countries: &'f [CountryRow],
    links: &'f [CountryLinkRow],
    appearance: Option<&'f [replay_data::rows::CountryAppearanceRow]>,
    lexical: Option<&'f [LexicalTokenRow]>,
    details: Option<&'f [IndustrialOrganisationDetailsRow]>,
    policies: Option<&'f [IndustrialOrganisationPoliciesRow]>,
}
