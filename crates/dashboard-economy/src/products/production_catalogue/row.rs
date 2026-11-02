//! The family's row census, kept as the oracle its columnar preparation ([`super::facts`],
//! [`super::emit`]) and readers ([`super::query`]) are held to: each hour of a row frame
//! observed into keyed runs and a catalogue, and the reply read back from them.
//!
//! An observed hour observes again only the rows its recorded changes name ([`observe`]).

mod observe;

use super::*;
use crate::products::keyed::{self, Published, branch_of, clip_text, selected_spans};
use dashboard_base::prepared::{frame_changes::FrameChanges, series::CatalogueWriter};
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeSet, io, sync::atomic::AtomicBool};

impl Field {
    fn domain(self) -> u8 {
        match self {
            Self::LicenceGiver(..)
            | Self::LicenceUnidentified(..)
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

/// [`Extra`] without the designs' naming reads, which the variant catalogue holds.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Held {
    pub texts: Vec<String>,
    pub organisations: BTreeMap<u64, Vec<RawOrganisation>>,
    pub designs_from: Option<i32>,
}

/// [`Extra`] borrowed, its fields in its order.
#[derive(Serialize)]
struct ExtraView<'a> {
    texts: &'a Vec<String>,
    organisations: &'a BTreeMap<u64, Vec<RawOrganisation>>,
    variants: Named<'a>,
    designs_from: &'a Option<i32>,
}

struct Named<'a> {
    ids: Vec<u64>,
    versions: &'a BTreeMap<u64, Vec<VariantVersion>>,
}

impl Serialize for Named<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(
            self.ids
                .iter()
                .filter_map(|id| Some((id, self.versions.get(id)?))),
        )
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    keyed: keyed::Keyed<Field, Held>,
    variants: VariantCatalogue,
    /// What observing the next hour by its changes needs; never saved.
    #[serde(skip)]
    open: observe::Open,
    #[serde(skip)]
    catalogue: CatalogueWriter,
    /// The catalogue's base range length, once a flush saw it.
    #[serde(skip)]
    base: Option<usize>,
}

impl Prepared {
    pub fn observe_changes(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let gap = self.keyed.step(frame.hour)?;
        self.observe_variants(format, frame, Some(changes));
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
            self.observe(frame, domains, (!gap).then_some(changes));
        }
        Ok(())
    }

    /// One hour observed whole.
    #[cfg(test)]
    pub(crate) fn step(&mut self, frame: &DecodedFrame) {
        self.keyed.step(frame.hour).unwrap();
        self.observe_variants(FormatVersion::CURRENT, frame, None);
        self.observe(frame, [true; 3], None);
    }

    fn observe_variants(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        changes: Option<&FrameChanges>,
    ) {
        let changed = keyed::observe_variants(&mut self.variants, format, frame, changes);
        let bytes = changed
            .iter()
            .map(|(_, version)| keyed::json_len(version) + 24)
            .sum();
        self.keyed.note_change(bytes);
    }

    /// Bytes the next commit publishes and releases.
    pub fn bytes(&self) -> usize {
        self.keyed.pending()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let ids = self
            .keyed
            .keys_in(Field::DesignCreator(0)..=Field::DesignCreator(u64::MAX))
            .map(|(field, _)| match *field {
                Field::DesignCreator(id) => id,
                _ => unreachable!("a design creator range holds only design creators"),
            })
            .collect();
        self.keyed.flush_values(writer, FAMILY, branch, cancelled)?;
        let held = &self.keyed.meta.extra;
        let extra = ExtraView {
            texts: &held.texts,
            organisations: &held.organisations,
            variants: Named {
                ids,
                versions: &self.variants.versions,
            },
            designs_from: &held.designs_from,
        };
        self.keyed
            .publish_catalogue(writer, FAMILY, branch, &mut self.catalogue, &mut self.base, &extra)?;
        self.keyed.published();
        Ok(())
    }
}

/// Other keys a catalogue reply reads together at most: it holds at most this many keys' whole
/// histories at once, before each is clipped to its row's spans.
const OTHER_KEYS: usize = 1024;

/// The reply the row census's publication answers.
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
    let chosen: Option<BTreeMap<u32, Vec<(i32, i32)>>> =
        (!params.tags.is_empty()).then(|| published.holder_spans(&params.tags));
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
                | Field::LicenceUnidentified(owner, at)
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
    let catalogue = (params.view == CatalogueView::Designs)
        .then(|| VariantCatalogue::seeded(published.meta.extra.variants.clone()));
    let is_party: fn(&Field) -> bool = match params.view {
        CatalogueView::Licences => |field| matches!(field, Field::LicenceGiver(..)),
        CatalogueView::Organisations => |field| matches!(field, Field::MioHolder(_)),
        CatalogueView::Designs => |field| matches!(field, Field::DesignCreator(_)),
    };
    // The keys a row shows the values of: neither its party nor its licensor's absence.
    let valued =
        |field: &Field| !is_party(field) && !matches!(field, Field::LicenceUnidentified(..));
    let rows: Vec<((u64, u32), Vec<(u64, Field)>)> = grouped.into_iter().collect();
    // Every row's party key is read first, all together, so a row no chosen country is party
    // to is skipped before its other keys are read; those are then read together too. The rows
    // are built in order, so a failure refuses the reply where reading key by key did.
    let party_keys: Vec<Option<u64>> = rows
        .iter()
        .map(|(_, fields)| {
            fields
                .iter()
                .find(|(_, field)| is_party(field))
                .map(|&(id, _)| id)
        })
        .collect();
    let read = published.runs_each(&party_keys.iter().flatten().copied().collect::<Vec<_>>())?;
    let mut read = read.into_iter();
    let parties: Vec<io::Result<Vec<EconomyRun>>> = party_keys
        .iter()
        .map(|key| match key {
            Some(_) => read
                .next()
                .expect("a read for every party key")
                .map(|runs| published.clipped_text(runs, &all)),
            None => Ok(Vec::new()),
        })
        .collect();
    // With a selection, the hours each licence's licensor had no identity: any chosen
    // country may be it.
    let unidentified_keys: Vec<Option<u64>> = rows
        .iter()
        .map(|(_, fields)| {
            fields
                .iter()
                .find(|(_, field)| {
                    chosen.is_some() && matches!(field, Field::LicenceUnidentified(..))
                })
                .map(|&(id, _)| id)
        })
        .collect();
    let read =
        published.runs_each(&unidentified_keys.iter().flatten().copied().collect::<Vec<_>>())?;
    let mut read = read.into_iter();
    let unidentified: Vec<io::Result<Vec<EconomyRun>>> = unidentified_keys
        .iter()
        .map(|key| match key {
            Some(_) => read
                .next()
                .expect("a read for every unidentified key")
                .map(|runs| published.clipped_text(runs, &all)),
            None => Ok(Vec::new()),
        })
        .collect();
    // Each row's spans, none for a skipped row, up to the first party that could not be read.
    let mut spans: Vec<Option<Vec<(i32, i32)>>> = Vec::new();
    for ((((owner, _), _), party), unidentified) in rows.iter().zip(&parties).zip(&unidentified) {
        let (Ok(party), Ok(unidentified)) = (party, unidentified) else {
            break;
        };
        let selected = match params.view {
            CatalogueView::Licences => {
                selected_spans(chosen.as_ref(), &[*owner as u32], party, unidentified)
            }
            _ => selected_spans(chosen.as_ref(), &[], party, &[]),
        };
        let skipped =
            selected.is_empty() || (params.view == CatalogueView::Designs && party.is_empty());
        spans.push((!skipped).then_some(selected));
    }
    // Each row's other keys, none for a skipped row, read for a group of rows at a time.
    let other_keys: Vec<Vec<u64>> = rows
        .iter()
        .enumerate()
        .map(|(index, (_, fields))| match spans.get(index) {
            Some(Some(_)) => fields
                .iter()
                .filter(|(_, field)| valued(field))
                .map(|&(id, _)| id)
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    let mut others = Vec::new().into_iter();
    // The first row whose other keys are not yet read.
    let mut unread = 0;
    let mut parties = parties.into_iter();
    let mut unidentified = unidentified.into_iter();
    let mut spans = spans.into_iter();
    for (index, ((owner, at), fields)) in rows.into_iter().enumerate() {
        published.check()?;
        let party = parties.next().expect("a party read for every row")?;
        unidentified
            .next()
            .expect("an unidentified read for every row")?;
        let Some(spans) = spans.next().expect("spans for every party read") else {
            continue;
        };
        if index >= unread {
            let mut keys = Vec::new();
            unread = index;
            while let Some(row) = other_keys.get(unread) {
                if !keys.is_empty() && keys.len() + row.len() > OTHER_KEYS {
                    break;
                }
                keys.extend_from_slice(row);
                unread += 1;
            }
            others = published.runs_each(&keys)?.into_iter();
        }
        // The next non-party key's runs within the row's spans.
        let mut text = || -> io::Result<Vec<EconomyRun>> {
            let runs = others.next().expect("a read for every other key")?;
            Ok(published.clipped_text(runs, &spans))
        };
        match params.view {
            CatalogueView::Licences => {
                let licensee = owner as u32;
                let licensor = clip_text(&party, &spans);
                let parties = std::iter::once(licensee)
                    .chain(licensor.iter().filter_map(|run| run.2.parse().ok()))
                    .collect::<Vec<u32>>();
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
                for (_, field) in fields {
                    if !valued(&field) {
                        continue;
                    }
                    let runs = text()?;
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
                licence.variants = licensed_variants(variants.into_values(), &mut licensed);
                holders.extend(parties);
                reply.licences.push(licence);
            }
            CatalogueView::Organisations => {
                let holder = clip_text(&party, &spans);
                let mut mio = organisation(owner, holder);
                for (_, field) in fields {
                    if !valued(&field) {
                        continue;
                    }
                    let runs = text()?;
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
                let versions = published.meta.extra.organisations.get(&owner);
                name_organisation(
                    &mut mio,
                    versions.map_or(&[][..], Vec::as_slice),
                    naming,
                    &mut reply.icons,
                    &mut warnings,
                    &mut refused_names,
                );
                reply.organisations.push(mio);
            }
            CatalogueView::Designs => {
                let creator = clip_text(&party, &spans);
                let mut design = design(owner, creator);
                for (_, field) in fields {
                    if !valued(&field) {
                        continue;
                    }
                    let runs = text()?;
                    if let Field::DesignObsolete(_) = field {
                        design.obsolete = runs;
                    }
                }
                holders.extend(
                    design
                        .creator
                        .iter()
                        .filter_map(|run| run.2.parse::<u32>().ok()),
                );
                name_design(
                    &mut design,
                    owner,
                    catalogue.as_ref(),
                    naming,
                    &mut warnings,
                    &mut refused_names,
                );
                reply.designs.push(design);
            }
        }
    }
    if params.view == CatalogueView::Licences {
        name_licensed(
            &mut reply,
            &published.meta.extra.variants,
            licensed,
            naming,
            &mut refused_names,
        );
    }
    if refused_names > 0 {
        warnings.push(format!(
            "{refused_names} names unavailable through the recorded content"
        ));
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
#[cfg(test)]
pub(super) mod equivalence;
