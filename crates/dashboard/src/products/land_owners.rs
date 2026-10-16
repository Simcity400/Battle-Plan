//! One compact, campaign-wide ownership catalogue; no per-hour history retained.
//! Names are resolved at each country's first observed hour in the user's game
//! language (`installed_content::country_names`); without game content they are tags.
#[cfg(test)]
use super::LoadContext;
use super::Product;
#[cfg(test)]
use crate::Source;
#[cfg(test)]
use crate::content::ContentKey;
#[cfg(test)]
use crate::metadata::validate_campaign;
use crate::{
    CountryMetadata, LandOwners,
    metadata::{CountryKeys, short_text},
};
use installed_content::{country_names::CountryNames, localisation::Localisation};
#[cfg(test)]
use replay::frame::FrameDecoder;
use replay::{
    container::Replay,
    frame::{TableView, ViewIdentity},
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::Arc,
};
use ts_rs::TS;

/// Includes positive ownership observations from every recorded branch. Control
/// and cores do not establish ownership; unavailable intervals add no countries.
///
/// One entry per (tag, slot), witnessed on the lowest branch that saw it own
/// land (its first observation there), so the reply scales with countries and
/// not with countries x branches. Entries are ordered by tag, then witness
/// branch, then slot: the first entry of a tag is its lowest (branch, slot)
/// observation, as when every branch was listed.
#[cfg(test)]
pub fn land_owners(
    replay: &Replay,
    localisation: Option<&Localisation>,
    cancelled: &AtomicBool,
) -> io::Result<LandOwners> {
    let check = || {
        if cancelled.load(Ordering::Acquire) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Country discovery cancelled",
            ))
        } else {
            Ok(())
        }
    };
    let mut scan = Scan::new(replay);
    let selected = [
        TableId::Campaign,
        TableId::States,
        TableId::CountryLinks,
        TableId::Countries,
        TableId::CountryAppearance,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::Autonomy,
    ];
    crate::blocks::scan_blocks(replay, &replay.index, &selected, cancelled, |blocks| {
        for (_, frames) in blocks {
            check()?;
            let frames = match frames {
                Ok(frames) => frames,
                Err(_) => {
                    scan.incomplete = true;
                    continue;
                }
            };
            let mut decoder = FrameDecoder::for_tables(&selected);
            for frame in frames {
                check()?;
                let decoded = match decoder.advance(&frame) {
                    Ok(decoded) if validate_campaign(decoded, replay).is_ok() => decoded,
                    _ => {
                        scan.incomplete = true;
                        break;
                    }
                };
                scan.observe(decoded, localisation, None)?;
            }
        }
        Ok(())
    })?;
    check()?;
    Ok(scan.reply())
}

/// Recording-wide positive ownership evidence. It grows only with new tag/slot witnesses.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Scan {
    #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_map")]
    countries: HashMap<u32, Witnesses>,
    #[serde(skip)]
    owners: Option<BTreeSet<u32>>,
    #[allow(clippy::type_complexity)]
    #[serde(skip)]
    country_inputs: Option<(CountryKeys, ViewIdentity, Arc<HashMap<u32, u32>>)>,
    pub(crate) incomplete: bool,
    unnamed: usize,
    warnings: Vec<String>,
    pub(crate) complete: bool,
    #[cfg(test)]
    #[serde(skip)]
    work: crate::prepared::observer_tests::Work,
}
/// Native tag bytes are lookup keys; insertion ordinals preserve the original
/// per-slot order when distinct tags have the same shortened display text.
#[derive(Default, Serialize, Deserialize)]
struct Witnesses {
    #[serde(with = "crate::prepared_production::map_pairs")]
    entries: HashMap<Vec<u8>, (u64, usize, CountryMetadata)>,
}
impl Scan {
    pub(crate) fn new(replay: &Replay) -> Self {
        Self {
            incomplete: !replay.gaps.is_empty(),
            ..Default::default()
        }
    }
    pub(crate) fn observe(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        localisation: Option<&Localisation>,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        if self.owners.is_some()
            && self.country_inputs.is_some()
            && changes.is_some_and(|changes| {
                [TableId::States, TableId::Countries, TableId::CountryLinks]
                    .iter()
                    .all(|table| !changes.rows_changed[table.index()])
            })
        {
            return Ok(());
        }
        let states = match &*frame.tables[replay::rows::TableId::States.index()] {
            TableView::Complete(rows) => rows.rows::<tables::States>(),
            TableView::Truncated { rows, .. } => {
                self.incomplete = true;
                rows.rows::<tables::States>()
            }
            TableView::Unavailable { .. } => None,
        };
        let (Some(states), Ok(links), Ok(rows)) = (
            states,
            frame.complete::<tables::CountryLinks>(),
            frame.complete::<tables::Countries>(),
        ) else {
            self.incomplete = true;
            self.owners = None;
            self.country_inputs = None;
            return Ok(());
        };
        let refresh = self.owners.is_none()
            || changes.is_none_or(|changes| changes.rows_changed[TableId::States.index()]);
        let mut ownership_changed = false;
        if refresh {
            #[cfg(test)]
            {
                self.work.row_visits += states.len();
            }
            // Retain current ownership operands, so a link-only refresh never
            // rereads States. This projection is runtime-only, not history.
            let owners = states.iter().map(|state| state.owner).collect();
            ownership_changed = self.owners.as_ref() != Some(&owners);
            self.owners = Some(owners);
        }
        let touched = changes.is_none_or(|changes| {
            changes.rows_changed[TableId::Countries.index()]
                || changes.table_read[TableId::Countries.index()]
        });
        #[cfg(test)]
        {
            if touched || self.country_inputs.is_none() {
                self.work.row_visits += rows.len();
            }
        }
        let view = &frame.tables[TableId::CountryLinks.index()];
        let same_countries = self
            .country_inputs
            .as_ref()
            .is_some_and(|(keys, _, _)| !touched || keys.same(rows));
        let same_links = self
            .country_inputs
            .as_ref()
            .is_some_and(|(_, links, _)| links.is(view));
        if changes.is_some_and(|changes| !changes.rows_changed[TableId::CountryLinks.index()])
            && !ownership_changed
            && same_countries
            && same_links
        {
            return Ok(());
        }
        match &mut self.country_inputs {
            Some((keys, links_view, slots)) => {
                if !same_countries {
                    *keys = CountryKeys::read(rows);
                }
                if !same_links {
                    #[cfg(test)]
                    {
                        self.work.row_visits += links.len();
                    }
                    *links_view = view.identity();
                    *slots = Arc::new(crate::metadata::link_slots(links));
                }
            }
            inputs => {
                #[cfg(test)]
                {
                    self.work.row_visits += links.len();
                }
                *inputs = Some((
                    CountryKeys::read(rows),
                    view.identity(),
                    Arc::new(crate::metadata::link_slots(links)),
                ));
            }
        }
        let (countries, _, slots) = self
            .country_inputs
            .as_ref()
            .expect("current country operands");
        let mut owners: BTreeSet<_> = self
            .owners
            .as_ref()
            .expect("ownership operands")
            .iter()
            .filter_map(|raw| {
                let slot = slots.get(raw).copied();
                self.incomplete |= slot.is_none() && *raw != 0;
                slot
            })
            .collect();
        // Operands are read only for a frame that adds a catalogue entry.
        let mut names = None;
        for (id, canonical_slot, tag) in countries.iter() {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let Some(slot) = slots
                .get(&id)
                .copied()
                .filter(|slot| *slot == canonical_slot)
            else {
                self.incomplete = true;
                continue;
            };
            if tag.is_empty() || !owners.remove(&slot) {
                continue;
            }
            let witnesses = self.countries.entry(slot).or_default();
            let known = witnesses.entries.get(tag);
            if known.is_some_and(|(branch, _, _)| *branch <= frame.branch) {
                continue;
            }
            let ordinal = known.map_or(witnesses.entries.len(), |(_, ordinal, _)| *ordinal);
            let entry = (
                frame.branch,
                ordinal,
                CountryMetadata {
                    id: slot.to_string(),
                    tag: short_text(&String::from_utf8_lossy(tag), 512),
                    name: match localisation.map(|localisation| {
                        names
                            .get_or_insert_with(|| CountryNames::read(frame, localisation))
                            .name(slot)
                    }) {
                        Some(Ok(name)) => short_text(&name, 512),
                        Some(Err(_)) => {
                            self.unnamed += 1;
                            short_text(&String::from_utf8_lossy(tag), 512)
                        }
                        None => short_text(&String::from_utf8_lossy(tag), 512),
                    },
                    observed_branch_id: frame.branch.to_string(),
                    observed_hour: frame.hour,
                },
            );
            #[cfg(test)]
            {
                self.work.emitted += 1;
            }
            witnesses.entries.insert(tag.to_vec(), entry);
        }
        self.incomplete |= !owners.is_empty();
        Ok(())
    }
    pub(crate) fn reply(&self) -> LandOwners {
        let mut countries: Vec<_> = self
            .countries
            .iter()
            .flat_map(|(slot, witnesses)| {
                let mut entries: Vec<_> = witnesses.entries.values().collect();
                entries.sort_unstable_by_key(|(_, ordinal, _)| *ordinal);
                entries
                    .into_iter()
                    .map(|(branch, _, country)| (*branch, *slot, country.clone()))
            })
            .collect();
        let mut warnings = Vec::new();
        if self.incomplete {
            warnings.push("Some ownership history is unavailable. Only countries with recorded land ownership are listed.".into());
        }
        if self.unnamed > 0 {
            warnings.push(format!(
            "{} country names could not be resolved from the recorded content and show their tag.",self.unnamed
        ));
        }
        warnings.extend(self.warnings.iter().cloned());
        countries.sort_unstable_by(|a, b| (&a.2.tag, a.0, a.1).cmp(&(&b.2.tag, b.0, b.1)));
        LandOwners {
            countries: countries
                .into_iter()
                .map(|(_, _, country)| country)
                .collect(),
            warnings,
        }
    }
    pub(crate) fn warning(&mut self, reason: String) {
        if !self.warnings.contains(&reason) {
            self.warnings.push(reason);
        }
    }
    pub(crate) fn saved_bytes(&self) -> io::Result<Vec<u8>> {
        crate::prepared::series::compress(serde_json::to_vec(self)?.as_slice(), 3)
    }
    pub(crate) fn read(reader: impl io::Read) -> io::Result<Self> {
        Ok(serde_json::from_reader(zstd::stream::read::Decoder::new(
            reader,
        )?)?)
    }
}

pub struct LandOwnersProduct;

/// Land ownership is campaign-wide; there are no parameters.
#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LandOwnersParams {}

impl Product for LandOwnersProduct {
    const KEY: &'static str = "land-owners";
    const CONTENT_DEPENDENT: bool = true;
    type Params = LandOwnersParams;
    type Reply = LandOwners;

    #[cfg(test)]
    fn load(
        source: &Source,
        _params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let key = crate::content::content_key(context.content.as_ref());
        let state = source.state::<Option<(Option<ContentKey>, LandOwners)>>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        if state.as_ref().is_none_or(|(cached, _)| *cached != key) {
            let localisation = crate::content::name_localisation(source, context.content.as_ref());
            let mut owners =
                land_owners(&source.replay(), localisation.as_deref().ok(), cancelled)?;
            owners.warnings.extend(localisation.err());
            *state = Some((key, owners));
        }
        Ok(state.as_ref().expect("land owners cached").1.clone())
    }
}

#[cfg(test)]
mod tests;
