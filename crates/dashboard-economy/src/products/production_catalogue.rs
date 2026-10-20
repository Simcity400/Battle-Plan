//! Production licences, industrial organisations and equipment designs,
//! prepared on their own (family `production_catalogue`).
//!
//! - Licences: each licensee's stored licence records (`owner` the licensee,
//!   `giver` the licensor), read in the daily production slot: the licensor,
//!   lent and required civilian factories, the native start date and the
//!   ordered licensed variants. A record's position is not a persistent
//!   identity; a record that stops being read only bounds its end.
//! - Organisations: every country's complete organisation roster with each
//!   one's stored size, funds, unspent points, selected policy (its token and
//!   the country's stored policy name), cooldown and unlocked traits, read in
//!   the same slot. Unknown state stays unknown, never zero.
//! - Designs: every saved design's creator and obsolete flag, from each
//!   complete read of the variant table and every writer-observed edit
//!   between reads, so an obsolete flag is dated to the hour an edit set it,
//!   else bounded by the daily read. The first read's designs predate it.
//!   No recorded field links an obsolete design to the one that replaced it.
//!
//! Each domain is observed when its own tables changed or after a gap, and
//! keeps its own refusals: a failed organisation read never refuses
//! licences or designs.

use super::keyed::{EconomyRun, HolderTag, Keyed, Observed, Published, Tags, branch_of};
use dashboard_base::{
    prepared::frame_changes::FrameChanges,
    products::{
        Product,
        equipment::{self, StockVariantName, Texts, VariantCatalogue, VariantVersion},
        production_lines::{self, RawOrganisation},
    },
};
use replay_data::{
    format::FormatVersion,
    rows::{TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{LazyLock, atomic::AtomicBool},
};
use ts_rs::TS;

pub const FAMILY: &str = "production_catalogue";
pub const QUERY: &str = "production-catalogue";

const LICENCES: [TableId; 1] = [TableId::ProductionLicenseRecords];
const ORGANISATIONS: [TableId; 3] = [
    TableId::IndustrialOrganisations,
    TableId::IndustrialOrganisationDetails,
    TableId::IndustrialOrganisationPolicies,
];

pub fn tables() -> &'static [TableId] {
    static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = vec![TableId::Campaign, TableId::Countries, TableId::CountryLinks];
        tables.extend(LICENCES);
        tables.extend(ORGANISATIONS);
        tables.extend(VariantCatalogue::TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &TABLES
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    /// A licensee's record at a roster position: the licensor's holder.
    LicenceGiver(u32, u32),
    LicenceLent(u32, u32),
    LicenceRequired(u32, u32),
    LicenceStart(u32, u32),
    /// The record's licensed variant at position `2`.
    LicenceVariant(u32, u32, u32),
    /// An organisation's holding country's holder.
    MioHolder(u64),
    MioSize(u64),
    MioFunds(u64),
    MioPoints(u64),
    /// Index of the selected policy's token in the catalogue's texts.
    MioPolicy(u64),
    /// Index of the country's stored name of that policy.
    MioPolicyName(u64),
    MioCooldown(u64),
    MioUnlocked(u64),
    /// 1 while a domain's roster (0 licences, 1 organisations, 2 designs) was
    /// read complete: without it the domain is unknown, never empty.
    Known(u8),
    /// 1 while an organisation's selected policy has no token spelling: a
    /// policy is selected, but which one is unknown.
    MioPolicyUnresolved(u64),
    /// A design's creator holder.
    DesignCreator(u64),
    /// 1 while the design is obsolete, 0 while current.
    DesignObsolete(u64),
}

impl Field {
    fn domain(self) -> u8 {
        match self {
            Self::LicenceGiver(..)
            | Self::LicenceLent(..)
            | Self::LicenceRequired(..)
            | Self::LicenceStart(..)
            | Self::LicenceVariant(..) => 0,
            Self::DesignCreator(_) | Self::DesignObsolete(_) => 2,
            Self::Known(domain) => domain,
            _ => 1,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Extra {
    /// Token spellings and stored policy names the runs index.
    pub texts: Vec<String>,
    /// Each organisation's naming operands at each read that changed them.
    pub organisations: BTreeMap<u64, Vec<RawOrganisation>>,
    /// Each design's naming reads.
    pub variants: BTreeMap<u64, Vec<VariantVersion>>,
    /// The first hour a complete read of the designs was observed.
    pub designs_from: Option<i32>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    keyed: Keyed<Field, Extra>,
    variants: VariantCatalogue,
}

impl Prepared {
    pub fn observe_changes(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let gap = self.keyed.step(frame.hour)?;
        super::keyed::observe_variants(&mut self.variants, format, frame, Some(changes));
        let changed = |tables: &[TableId]| {
            gap || [TableId::Countries, TableId::CountryLinks]
                .iter()
                .chain(tables)
                .any(|table| changes.rows_changed[table.index()])
        };
        let domains = [
            changed(&LICENCES),
            // Names follow the token dictionary: a spelling change renames.
            changed(&[ORGANISATIONS[0], ORGANISATIONS[1], ORGANISATIONS[2], TableId::LexicalTokens]),
            changed(&[TableId::VariantDefinitions, TableId::VariantChanges])
                || changes.slot_read[TableId::VariantDefinitions.index()],
        ];
        if domains.iter().any(|&domain| domain) {
            self.observe(frame, domains);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn step(&mut self, frame: &DecodedFrame) {
        self.keyed.step(frame.hour).unwrap();
        super::keyed::observe_variants(&mut self.variants, FormatVersion::CURRENT, frame, None);
        self.observe(frame, [true; 3]);
    }

    fn text(&mut self, text: String) -> i128 {
        let texts = &mut self.keyed.meta.extra.texts;
        let index = texts.iter().position(|held| *held == text).unwrap_or_else(|| {
            texts.push(text);
            texts.len() - 1
        });
        index as i128
    }

    fn observe(&mut self, frame: &DecodedFrame, domains: [bool; 3]) {
        let hour = frame.hour;
        let mut observed = Observed::new();
        let tags = match Tags::new(frame) {
            Ok(tags) => tags,
            Err(error) => {
                self.keyed
                    .refuse(format!("country identities unavailable: {error}"));
                self.keyed
                    .commit(hour, &observed, |field| domains[usize::from(field.domain())]);
                return;
            }
        };
        if domains[0] {
            self.licences(frame, &tags, &mut observed);
        }
        if domains[1] {
            self.organisations(frame, &tags, &mut observed);
        }
        if domains[2] {
            self.designs(frame, &tags, &mut observed);
        }
        self.keyed
            .commit(hour, &observed, |field| domains[usize::from(field.domain())]);
    }

    fn licences(&mut self, frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observed) {
        let hour = frame.hour;
        let rows = match frame.complete::<tables::ProductionLicenseRecords>() {
            Ok(rows) => rows,
            Err(error) => {
                self.keyed.refuse(format!("licences unavailable: {error}"));
                return;
            }
        };
        observed.insert(self.keyed.key(Field::Known(0)), 1);
        for row in rows {
            let (Some(owner), Some(giver)) = (tags.slot(row.owner), tags.slot(row.giver)) else {
                self.keyed.refuse("a licence names a country with no identity");
                continue;
            };
            self.keyed.tag(tags, owner, hour);
            self.keyed.tag(tags, giver, hour);
            let at = row.ordinal;
            for (field, value) in [
                (Field::LicenceGiver(owner, at), i128::from(giver)),
                (Field::LicenceLent(owner, at), i128::from(row.lended_cic)),
                (Field::LicenceRequired(owner, at), i128::from(row.required_cic)),
                (Field::LicenceStart(owner, at), i128::from(row.start_date)),
            ] {
                observed.insert(self.keyed.key(field), value);
            }
            for (index, &variant) in row.equipment.iter().enumerate() {
                observed.insert(
                    self.keyed.key(Field::LicenceVariant(owner, at, index as u32)),
                    i128::from(variant),
                );
            }
        }
    }

    fn organisations(&mut self, frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observed) {
        let hour = frame.hour;
        // The roster and its numbers need only the roster and the identities
        // (read by the caller); the token dictionary names them, and its
        // failure refuses the names alone.
        let (roster, countries, links) = match (
            frame.complete::<tables::IndustrialOrganisations>(),
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) {
            (Ok(roster), Ok(countries), Ok(links)) => (roster, countries, links),
            (Err(error), ..) | (_, Err(error), _) | (.., Err(error)) => {
                self.keyed.refuse(format!("organisations unavailable: {error}"));
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
        let naming = equipment::Tags::new(
            countries,
            links,
            frame.complete::<tables::CountryAppearance>().ok(),
        );
        observed.insert(self.keyed.key(Field::Known(1)), 1);
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
        for row in roster {
            let Some(holder) = tags.slot(row.country) else {
                self.keyed.refuse("an organisation's country has no identity");
                continue;
            };
            self.keyed.tag(tags, holder, hour);
            let id = row.organisation;
            observed.insert(self.keyed.key(Field::MioHolder(id)), i128::from(holder));
            // Without the dictionary the names are unknown from this read on.
            let raw = match lexical {
                Some(lexical) => production_lines::raw_organisation(row, hour, &naming, lexical),
                None => RawOrganisation { first_hour: hour, owner_unknown: true, ..Default::default() },
            };
            let versions = self.keyed.meta.extra.organisations.entry(id).or_default();
            if versions.last().is_none_or(|last| {
                (&last.token, &last.name_key, &last.icon, &last.owner_tag, last.owner_unknown)
                    != (&raw.token, &raw.name_key, &raw.icon, &raw.owner_tag, raw.owner_unknown)
            }) {
                versions.push(raw);
            }
            let Some(detail) = details.as_ref().ok().and_then(|details| {
                details
                    .binary_search_by_key(&id, |detail| detail.organisation)
                    .ok()
                    .map(|index| &details[index])
                    .filter(|detail| detail.country == row.country)
            }) else {
                continue;
            };
            // Without the dictionary a selected policy reads as unspelled.
            let state = production_lines::organisation_state(detail, None, lexical.map_or(&[][..], |rows| &rows[..]));
            for (field, value) in [
                (Field::MioSize(id), Some(i128::from(detail.state.size))),
                (Field::MioFunds(id), Some(i128::from(detail.state.funds_raw))),
                (Field::MioPoints(id), Some(i128::from(detail.state.points))),
                (Field::MioUnlocked(id), Some(detail.unlocked.len() as i128)),
                (Field::MioCooldown(id), detail.state.cooldown_hour.map(i128::from)),
            ] {
                if let Some(value) = value {
                    observed.insert(self.keyed.key(field), value);
                }
            }
            if state.policy_token.as_deref() == Some("") {
                self.keyed.refuse("a selected policy's token is not spelled");
                observed.insert(self.keyed.key(Field::MioPolicyUnresolved(id)), 1);
            }
            if let Some(token) = state.policy_token.as_ref().filter(|token| !token.is_empty()) {
                let index = self.text(token.clone());
                observed.insert(self.keyed.key(Field::MioPolicy(id)), index);
            }
            // The country's stored policy name joins by the raw token, spelled or not.
            if state.policy_token.is_some() {
                let name = policies.as_ref().ok().and_then(|rows| {
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
                    let index = self.text(name);
                    observed.insert(self.keyed.key(Field::MioPolicyName(id)), index);
                }
            }
        }
    }

    fn designs(&mut self, frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observed) {
        let hour = frame.hour;
        let rows = match frame.complete::<tables::VariantDefinitions>() {
            Ok(rows) => rows,
            Err(error) => {
                self.keyed.refuse(format!("designs unavailable: {error}"));
                return;
            }
        };
        self.keyed.meta.extra.designs_from.get_or_insert(hour);
        observed.insert(self.keyed.key(Field::Known(2)), 1);
        for row in rows {
            let Some(creator) = tags.slot(row.creator_link) else {
                self.keyed.refuse("a design's creator has no identity");
                continue;
            };
            self.keyed.tag(tags, creator, hour);
            observed.insert(self.keyed.key(Field::DesignCreator(row.id)), i128::from(creator));
            observed.insert(
                self.keyed.key(Field::DesignObsolete(row.id)),
                i128::from(row.obsolete != 0),
            );
        }
    }

    pub fn bytes(&self) -> usize {
        self.keyed.bytes()
    }

    pub fn flush(&mut self, writer: &mut store::Writer, branch: u64, cancelled: &AtomicBool) -> io::Result<()> {
        let named: BTreeSet<u64> = self
            .keyed
            .meta
            .fields
            .values()
            .filter_map(|field| match *field {
                Field::DesignCreator(id) => Some(id),
                _ => None,
            })
            .collect();
        self.keyed.meta.extra.variants = named
            .into_iter()
            .filter_map(|id| Some((id, self.variants.versions.get(&id)?.clone())))
            .collect();
        self.keyed.flush(writer, FAMILY, branch, cancelled)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum CatalogueView {
    Licences,
    Organisations,
    Designs,
}

pub struct ProductionCatalogue;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionCatalogueParams {
    pub branch_id: String,
    pub view: CatalogueView,
    /// Licences these countries grant or hold, or the organisations and
    /// designs of these countries; every country when empty.
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LicenceHistory {
    /// The licensee's holder.
    pub licensee: String,
    /// The record's roster position: not a persistent identity.
    pub ordinal: u32,
    /// The licensor's holder.
    pub licensor: Vec<EconomyRun>,
    /// Civilian factories lent and required.
    pub lent: Vec<EconomyRun>,
    pub required: Vec<EconomyRun>,
    /// The native start date's hour.
    pub start: Vec<EconomyRun>,
    /// Each licensed variant, in recorded order, as a hexadecimal handle.
    pub variants: Vec<Vec<EconomyRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationHistory {
    /// The full organisation reference, as a decimal u64.
    pub id: String,
    pub holder: Vec<EconomyRun>,
    pub size: Vec<EconomyRun>,
    /// Fixed5.
    pub funds: Vec<EconomyRun>,
    pub points: Vec<EconomyRun>,
    pub unlocked: Vec<EconomyRun>,
    /// The selected policy's token and the country's stored name of it.
    pub policy: Vec<EconomyRun>,
    pub policy_name: Vec<EconomyRun>,
    /// "1" while a policy is selected whose token is not spelled.
    pub policy_unresolved: Vec<EconomyRun>,
    pub cooldown: Vec<EconomyRun>,
    /// Its name at each naming read; null where content cannot name it.
    pub names: Vec<OrganisationLabel>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationLabel {
    pub first_hour: i32,
    pub token: String,
    pub name: Option<String>,
    /// The icon's sprite name, where the content declares one.
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DesignHistory {
    /// Hexadecimal variant handle.
    pub id: String,
    pub creator: Vec<EconomyRun>,
    pub obsolete: Vec<EconomyRun>,
    /// The equipment type's script key, its archetype's and the archetype's
    /// in-game name, from the latest naming read.
    pub token: String,
    pub archetype: Option<String>,
    pub archetype_name: Option<String>,
    pub names: Vec<StockVariantName>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProductionCatalogueReply {
    pub branch_id: String,
    pub view: CatalogueView,
    pub last_hour: Option<i32>,
    /// Hours the view's roster was read complete ("1"): outside them the
    /// view is unknown, never an empty roster.
    pub known: Vec<EconomyRun>,
    pub licences: Vec<LicenceHistory>,
    pub organisations: Vec<OrganisationHistory>,
    pub designs: Vec<DesignHistory>,
    /// The first hour the designs were read whole: designs read then
    /// predate it.
    pub designs_from: Option<i32>,
    pub holders: BTreeMap<String, Vec<HolderTag>>,
    /// Licensed variants' names at each read.
    pub variants: BTreeMap<String, Vec<StockVariantName>>,
    /// Policy runs' texts: token spellings and stored names.
    pub texts: Vec<String>,
    /// Each organisation icon's picture; null where it cannot be read.
    pub icons: BTreeMap<String, Option<String>>,
    pub warnings: Vec<String>,
}

impl Product for ProductionCatalogue {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProductionCatalogueParams;
    type Reply = ProductionCatalogueReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        for tag in &params.tags {
            dashboard_base::products::validate_tag(tag)?;
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            CatalogueView::decl(),
            ProductionCatalogueParams::decl(),
            ProductionCatalogueReply::decl(),
            LicenceHistory::decl(),
            OrganisationHistory::decl(),
            OrganisationLabel::decl(),
            DesignHistory::decl(),
        ]
    }
}

/// The content a reply's names read.
pub struct Naming<'a> {
    pub texts: Option<&'a dyn Texts>,
    pub units: Option<&'a installed_content_core::equipment::Database>,
    pub sprite: &'a dyn Fn(&str) -> bool,
    /// A declared sprite's picture as a PNG data URL.
    pub image: &'a dyn Fn(&str) -> Result<String, String>,
}

pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    params: &ProductionCatalogueParams,
    through: Option<i32>,
    naming: &Naming<'_>,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<ProductionCatalogueReply> {
    let branch = branch_of(&params.branch_id)?;
    let mut reply = ProductionCatalogueReply {
        branch_id: params.branch_id.clone(),
        view: params.view,
        last_hour: through,
        known: Vec::new(),
        licences: Vec::new(),
        organisations: Vec::new(),
        designs: Vec::new(),
        designs_from: None,
        holders: BTreeMap::new(),
        variants: BTreeMap::new(),
        texts: Vec::new(),
        icons: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let Some(through) = through else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let Some(mut published) =
        Published::<Field, Extra>::open(store, key, FAMILY, branch, through, cancelled)?
    else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let chosen: Option<BTreeSet<u32>> = (!params.tags.is_empty())
        .then(|| published.holder_spans(&params.tags).into_keys().collect());
    let holds = |chosen: &Option<BTreeSet<u32>>, runs: &[EconomyRun]| {
        chosen.as_ref().is_none_or(|chosen| {
            runs.iter()
                .any(|run| run.2.parse::<u32>().is_ok_and(|slot| chosen.contains(&slot)))
        })
    };
    let all = [(i32::MIN, i32::MAX)];
    let mut holders = BTreeSet::new();
    let mut grouped = BTreeMap::<(u64, u32), Vec<(u64, Field)>>::new();
    let domain = match params.view {
        CatalogueView::Licences => 0,
        CatalogueView::Organisations => 1,
        CatalogueView::Designs => 2,
    };
    let mut known = None;
    for (&id, field) in &published.meta.fields {
        let owner = match (*field, params.view) {
            (Field::Known(held), _) => {
                if held == domain {
                    known = Some(id);
                }
                continue;
            }
            (
                Field::LicenceGiver(owner, at)
                | Field::LicenceLent(owner, at)
                | Field::LicenceRequired(owner, at)
                | Field::LicenceStart(owner, at)
                | Field::LicenceVariant(owner, at, _),
                CatalogueView::Licences,
            ) => (u64::from(owner), at),
            (
                Field::MioHolder(mio)
                | Field::MioSize(mio)
                | Field::MioFunds(mio)
                | Field::MioPoints(mio)
                | Field::MioPolicy(mio)
                | Field::MioPolicyName(mio)
                | Field::MioCooldown(mio)
                | Field::MioUnlocked(mio)
                | Field::MioPolicyUnresolved(mio),
                CatalogueView::Organisations,
            )
            | (
                Field::DesignCreator(mio) | Field::DesignObsolete(mio),
                CatalogueView::Designs,
            ) => (mio, 0),
            _ => continue,
        };
        grouped.entry(owner).or_default().push((id, *field));
    }
    if let Some(id) = known {
        reply.known = published.text(id, &all)?;
    }
    let mut licensed = BTreeSet::new();
    let mut refused_names = 0_usize;
    let catalogue = (params.view == CatalogueView::Designs).then(|| {
        VariantCatalogue::seeded(published.meta.extra.variants.clone())
    });
    // The party key's runs, read first so a row no chosen country is party to is
    // skipped before its other keys are read.
    let party = |published: &mut Published<'_, Field, Extra>,
                 fields: &[(u64, Field)],
                 is_party: fn(&Field) -> bool|
     -> io::Result<Vec<EconomyRun>> {
        match fields.iter().find(|(_, field)| is_party(field)) {
            Some(&(id, _)) => published.text(id, &all),
            None => Ok(Vec::new()),
        }
    };
    for ((owner, at), fields) in grouped {
        published.check()?;
        match params.view {
            CatalogueView::Licences => {
                let licensee = owner as u32;
                let licensor = party(&mut published, &fields, |field| matches!(field, Field::LicenceGiver(..)))?;
                let parties = std::iter::once(licensee)
                    .chain(licensor.iter().filter_map(|run| run.2.parse().ok()))
                    .collect::<Vec<u32>>();
                if chosen
                    .as_ref()
                    .is_some_and(|chosen| !parties.iter().any(|slot| chosen.contains(slot)))
                {
                    continue;
                }
                let mut licence = LicenceHistory {
                    licensee: licensee.to_string(),
                    ordinal: at,
                    licensor,
                    lent: Vec::new(),
                    required: Vec::new(),
                    start: Vec::new(),
                    variants: Vec::new(),
                };
                let mut variants = BTreeMap::new();
                for (id, field) in fields {
                    if matches!(field, Field::LicenceGiver(..)) {
                        continue;
                    }
                    let runs = published.text(id, &all)?;
                    match field {
                        Field::LicenceLent(..) => licence.lent = runs,
                        Field::LicenceRequired(..) => licence.required = runs,
                        Field::LicenceStart(..) => licence.start = runs,
                        Field::LicenceVariant(_, _, index) => {
                            variants.insert(index, runs);
                        }
                        _ => {}
                    }
                }
                licence.variants = variants
                    .into_values()
                    .map(|runs| {
                        runs.into_iter()
                            .map(|EconomyRun(first, last, value)| {
                                let id = value.parse::<u64>().unwrap_or_default();
                                licensed.insert(id);
                                EconomyRun(first, last, format!("{id:x}"))
                            })
                            .collect()
                    })
                    .collect();
                holders.extend(parties);
                reply.licences.push(licence);
            }
            CatalogueView::Organisations => {
                let holder = party(&mut published, &fields, |field| matches!(field, Field::MioHolder(_)))?;
                if !holds(&chosen, &holder) {
                    continue;
                }
                let mut mio = OrganisationHistory {
                    id: owner.to_string(),
                    holder,
                    size: Vec::new(),
                    funds: Vec::new(),
                    points: Vec::new(),
                    unlocked: Vec::new(),
                    policy: Vec::new(),
                    policy_name: Vec::new(),
                    policy_unresolved: Vec::new(),
                    cooldown: Vec::new(),
                    names: Vec::new(),
                };
                for (id, field) in fields {
                    if matches!(field, Field::MioHolder(_)) {
                        continue;
                    }
                    let runs = published.text(id, &all)?;
                    match field {
                        Field::MioSize(_) => mio.size = runs,
                        Field::MioFunds(_) => mio.funds = runs,
                        Field::MioPoints(_) => mio.points = runs,
                        Field::MioUnlocked(_) => mio.unlocked = runs,
                        Field::MioPolicy(_) => mio.policy = runs,
                        Field::MioPolicyName(_) => mio.policy_name = runs,
                        Field::MioPolicyUnresolved(_) => mio.policy_unresolved = runs,
                        Field::MioCooldown(_) => mio.cooldown = runs,
                        _ => {}
                    }
                }
                holders.extend(mio.holder.iter().filter_map(|run| run.2.parse::<u32>().ok()));
                for raw in published
                    .meta
                    .extra
                    .organisations
                    .get(&owner)
                    .into_iter()
                    .flatten()
                {
                    let name = production_lines::organisation_name(raw, naming.texts);
                    if name.is_none() {
                        refused_names += 1;
                    }
                    let icon = production_lines::organisation_icon(raw, naming.sprite);
                    if let Some(icon) = &icon
                        && !reply.icons.contains_key(icon)
                    {
                        let image = (naming.image)(icon)
                            .map_err(|error| warnings.push(format!("{icon}: picture unavailable: {error}")))
                            .ok();
                        reply.icons.insert(icon.clone(), image);
                    }
                    mio.names.push(OrganisationLabel {
                        first_hour: raw.first_hour,
                        token: raw.token.clone(),
                        name,
                        icon,
                    });
                }
                reply.organisations.push(mio);
            }
            CatalogueView::Designs => {
                let creator = party(&mut published, &fields, |field| matches!(field, Field::DesignCreator(_)))?;
                if creator.is_empty() || !holds(&chosen, &creator) {
                    continue;
                }
                let mut design = DesignHistory {
                    id: format!("{owner:x}"),
                    creator,
                    obsolete: Vec::new(),
                    token: String::new(),
                    archetype: None,
                    archetype_name: None,
                    names: Vec::new(),
                };
                for (id, field) in fields {
                    if let Field::DesignObsolete(_) = field {
                        design.obsolete = published.text(id, &all)?;
                    }
                }
                holders.extend(design.creator.iter().filter_map(|run| run.2.parse::<u32>().ok()));
                if let Some(catalogue) = &catalogue {
                    design.names = catalogue.names(owner, naming.texts);
                    design.token = catalogue.token(owner).to_owned();
                }
                if design.names.iter().all(|name| name.name.is_none()) {
                    refused_names += 1;
                }
                if let Some(units) = naming.units
                    && !design.token.is_empty()
                {
                    match equipment::archetype_of(units, &design.token) {
                        Ok((archetype, _, _)) => {
                            design.archetype_name = naming
                                .texts
                                .and_then(|texts| equipment::localise_for(texts, None, &archetype));
                            design.archetype = Some(archetype);
                        }
                        Err(reason) => warnings.push(format!("Equipment type unavailable: {reason}")),
                    }
                }
                reply.designs.push(design);
            }
        }
    }
    if params.view == CatalogueView::Licences {
        // Licensed variants are named where the designs read them.
        let seeded = VariantCatalogue::seeded(
            published
                .meta
                .extra
                .variants
                .iter()
                .filter(|(id, _)| licensed.contains(id))
                .map(|(&id, versions)| (id, versions.clone()))
                .collect(),
        );
        for id in licensed {
            let names = seeded.names(id, naming.texts);
            if names.iter().all(|name| name.name.is_none()) {
                refused_names += 1;
            }
            reply.variants.insert(format!("{id:x}"), names);
        }
    }
    if refused_names > 0 {
        warnings.push(format!("{refused_names} names unavailable through the recorded content"));
    }
    warnings.sort();
    warnings.dedup();
    reply.designs_from = published.meta.extra.designs_from;
    reply.texts = published.meta.extra.texts.clone();
    reply.holders = holders
        .into_iter()
        .map(|slot| (slot.to_string(), published.tags(slot)))
        .collect();
    warnings.extend(published.warnings("Production catalogue"));
    reply.warnings = warnings;
    Ok(reply)
}

#[cfg(test)]
mod tests;
