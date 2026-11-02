//! Production licences, industrial organisations and equipment designs, prepared by the
//! segment driver (family `production_catalogue`) as runs on the family's series publication
//! ([`facts`], [`emit`]) and answered from it ([`query`]).
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
//! Each domain is read where its own tables or the countries' identities may have changed,
//! and keeps its own refusals: a failed organisation read never refuses licences or designs.
//! A refusal counts once for each hour it held, once per row it refused.

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
pub mod row;
pub mod served;

#[cfg(test)]
mod columnar_tests;

pub use query::query;

use super::keyed::{EconomyRun, HolderTag};
use dashboard_base::products::{
    Product,
    equipment::{self, StockVariantName, Texts, VariantCatalogue, VariantVersion},
    production_lines::{self, RawOrganisation},
};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

pub const FAMILY: &str = "production_catalogue";
pub const QUERY: &str = "production-catalogue";

const LICENCES: [TableId; 1] = [TableId::ProductionLicenseRecords];
const ORGANISATIONS: [TableId; 3] = [
    TableId::IndustrialOrganisations,
    TableId::IndustrialOrganisationDetails,
    TableId::IndustrialOrganisationPolicies,
];

/// The tables the family reads: the countries, its own, and those the variant names read.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::ProductionLicenseRecords,
    TableId::IndustrialOrganisations,
    TableId::IndustrialOrganisationDetails,
    TableId::IndustrialOrganisationPolicies,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::VariantChanges,
];

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    /// A licensee's record at a roster position: the licensor's holder.
    LicenceGiver(u32, u32),
    LicenceLent(u32, u32),
    LicenceRequired(u32, u32),
    LicenceStart(u32, u32),
    /// The record's licensed variant at position `2`.
    LicenceVariant(u32, u32, u32),
    /// 1 while the record's licensor has no identity: the licence stands, and
    /// any chosen country may be its licensor.
    LicenceUnidentified(u32, u32),
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum CatalogueView {
    Licences,
    Organisations,
    Designs,
}

impl CatalogueView {
    /// The domain the view shows: 0 licences, 1 organisations, 2 designs.
    fn domain(self) -> u8 {
        match self {
            Self::Licences => 0,
            Self::Organisations => 1,
            Self::Designs => 2,
        }
    }
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
    /// The licensor's holder; no run while it had no identity.
    pub licensor: Vec<EconomyRun>,
    /// Civilian factories lent and required.
    pub lent: Vec<EconomyRun>,
    pub required: Vec<EconomyRun>,
    /// The native start date's hour, read every hour the licence stands: its
    /// runs are the licence's lifetime.
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
    /// The icon's key in `icons`: its asset and a digest of its picture
    /// (`production_lines::picture_key`), where the content declares one.
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

impl ProductionCatalogueReply {
    /// The reply before any row: `through` the hours it answers, none before any is prepared.
    fn empty(params: &ProductionCatalogueParams, through: Option<i32>) -> Self {
        Self {
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
        }
    }
}

impl Product for ProductionCatalogue {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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
    /// An organisation's icon, chosen and drawn as one operation: its picture's key and the
    /// picture or why it is unavailable (`production_lines::organisation_shown`); `None` where
    /// no candidate is declared.
    pub icon: &'a dyn Fn(&production_lines::RawOrganisation) -> Option<(String, Result<String, String>)>,
}

/// A licence's variant runs as hexadecimal handles, each noted in `licensed`.
fn licensed_variants(
    variants: impl Iterator<Item = Vec<EconomyRun>>,
    licensed: &mut BTreeSet<u64>,
) -> Vec<Vec<EconomyRun>> {
    variants
        .map(|runs| {
            runs.into_iter()
                .map(|EconomyRun(first, last, value)| {
                    let id = value.parse::<u64>().unwrap_or_default();
                    licensed.insert(id);
                    EconomyRun(first, last, format!("{id:x}"))
                })
                .collect()
        })
        .collect()
}

/// Organisation `id` with its holder's runs and nothing else yet.
fn organisation(id: u64, holder: Vec<EconomyRun>) -> OrganisationHistory {
    OrganisationHistory {
        id: id.to_string(),
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
    }
}

/// `mio`'s name and icon at each of its naming `versions`.
fn name_organisation(
    mio: &mut OrganisationHistory,
    versions: &[RawOrganisation],
    naming: &Naming<'_>,
    icons: &mut BTreeMap<String, Option<String>>,
    warnings: &mut Vec<String>,
    refused: &mut usize,
) {
    for raw in versions {
        let name = production_lines::organisation_name(raw, naming.texts);
        if name.is_none() {
            *refused += 1;
        }
        let icon = (naming.icon)(raw).map(|(key, image)| {
            if !icons.contains_key(&key) {
                let image = image
                    .map_err(|error| warnings.push(format!("{key}: picture unavailable: {error}")))
                    .ok();
                icons.insert(key.clone(), image);
            }
            key
        });
        mio.names.push(OrganisationLabel {
            first_hour: raw.first_hour,
            token: raw.token.clone(),
            name,
            icon,
        });
    }
}

/// Design `id` with its creator's runs and nothing else yet.
fn design(id: u64, creator: Vec<EconomyRun>) -> DesignHistory {
    DesignHistory {
        id: format!("{id:x}"),
        creator,
        obsolete: Vec::new(),
        token: String::new(),
        archetype: None,
        archetype_name: None,
        names: Vec::new(),
    }
}

/// `design`'s names from `catalogue`, its equipment type and archetype.
fn name_design(
    design: &mut DesignHistory,
    id: u64,
    catalogue: Option<&VariantCatalogue>,
    naming: &Naming<'_>,
    warnings: &mut Vec<String>,
    refused: &mut usize,
) {
    if let Some(catalogue) = catalogue {
        design.names = catalogue.names(id, naming.texts);
        design.token = catalogue.token(id).to_owned();
    }
    if design.names.iter().all(|name| name.name.is_none()) {
        *refused += 1;
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
}

/// The names of the `licensed` variants, where the designs' naming reads (`versions`) hold
/// them.
fn name_licensed(
    reply: &mut ProductionCatalogueReply,
    versions: &BTreeMap<u64, Vec<VariantVersion>>,
    licensed: BTreeSet<u64>,
    naming: &Naming<'_>,
    refused: &mut usize,
) {
    let seeded = VariantCatalogue::seeded(
        versions
            .iter()
            .filter(|(id, _)| licensed.contains(id))
            .map(|(&id, versions)| (id, versions.clone()))
            .collect(),
    );
    for id in licensed {
        let names = seeded.names(id, naming.texts);
        if names.iter().all(|name| name.name.is_none()) {
            *refused += 1;
        }
        reply.variants.insert(format!("{id:x}"), names);
    }
}
