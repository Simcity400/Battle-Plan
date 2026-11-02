use dashboard_keyed::{Observer, Values, countries::Countries};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{LexicalTokenRow, TableId, tables},
};
use replay_frame::frame::{DecodedFrame, ViewIdentity};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct InternedToken {
    pub raw: u32,
    pub literal: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(super) struct FolderInventory {
    pub tokens: BTreeSet<u64>,
    pub grands: BTreeSet<u64>,
    pub tracks: BTreeSet<u32>,
    pub subs: BTreeMap<u32, BTreeSet<u64>>,
}

#[derive(Default)]
struct LexicalIndex {
    view: Option<ViewIdentity>,
    positions: HashMap<u32, usize>,
    #[cfg(test)]
    builds: usize,
}

/// Only token spelling, sparse content inventory and named refusals continue
/// between blocks. Numeric history belongs to the shared keyed scan.
#[derive(Default, Deserialize, Serialize)]
pub struct DoctrineMasteryObserver {
    pub(super) tokens: Vec<InternedToken>,
    pub(super) folders: BTreeMap<u32, FolderInventory>,
    pub(super) refusals: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(skip)]
    pub(super) index: HashMap<(u32, Option<Vec<u8>>), u64>,
    #[serde(skip)]
    lexical: LexicalIndex,
    /// The tables a native segment's keeper kept, observed a row at a time.
    #[serde(skip)]
    pub(super) mirror: super::mirror::Mirror,
}

impl DoctrineMasteryObserver {
    fn refuse(&mut self, owner: &str, why: String) {
        *self
            .refusals
            .entry(owner.to_owned())
            .or_default()
            .entry(why)
            .or_default() += 1;
    }

    fn intern(
        &mut self,
        raw: u32,
        lexical: &HashMap<u32, usize>,
        rows: &[LexicalTokenRow],
        owner: &str,
    ) -> u64 {
        if self.index.len() != self.tokens.len() {
            self.index = self
                .tokens
                .iter()
                .enumerate()
                .map(|(at, token)| ((token.raw, token.literal.clone()), at as u64 + 1))
                .collect();
        }
        let literal = lexical
            .get(&raw)
            .and_then(|&at| rows.get(at))
            .map(|row| installed_content_core::parser::c_string(&row.literal).to_vec());
        if literal.is_none() {
            self.refuse(owner, format!("token {raw} spelling is not recorded"));
        }
        let key = (raw, literal.clone());
        if let Some(&id) = self.index.get(&key) {
            return id;
        }
        let id = self.tokens.len() as u64 + 1;
        self.tokens.push(InternedToken { raw, literal });
        self.index.insert(key, id);
        id
    }

    #[cfg(test)]
    pub(super) fn lexical_builds(&self) -> usize {
        self.lexical.builds
    }

    pub(super) fn literal(&self, id: u64) -> Option<&[u8]> {
        usize::try_from(id.checked_sub(1)?)
            .ok()
            .and_then(|at| self.tokens.get(at))?
            .literal
            .as_deref()
    }
}

impl Observer for DoctrineMasteryObserver {
    const NAME: &'static str = "doctrine-mastery";
    const TABLES: &'static [TableId] = &[
        TableId::DoctrineCountries,
        TableId::DoctrineFolders,
        TableId::DoctrineTracks,
        TableId::CountryLinks,
        TableId::Countries,
        TableId::LexicalTokens,
    ];
    const READ: TableId = TableId::DoctrineTracks;

    fn native_keeper() -> Option<Box<dyn dashboard_keyed::native::Keeper<Self>>> {
        Some(Box::new(super::kept::Keeper::default()))
    }

    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::all(TableId::DoctrineCountries),
            TableFields::all(TableId::DoctrineFolders),
            TableFields::all(TableId::DoctrineTracks),
            TableFields::all(TableId::CountryLinks),
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
            TableFields::all(TableId::LexicalTokens),
        ])
    }

    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let (Some(countries), Ok(status), Ok(folders), Ok(tracks), Ok(country_rows)) = (
            Countries::read(frame),
            frame.complete::<tables::DoctrineCountries>(),
            frame.complete::<tables::DoctrineFolders>(),
            frame.complete::<tables::DoctrineTracks>(),
            frame.complete::<tables::Countries>(),
        ) else {
            for &table in Self::TABLES
                .iter()
                .filter(|&&table| table != TableId::LexicalTokens)
            {
                if let Some(why) = dashboard_base::products::table_missing_reason(frame, table) {
                    self.refuse("", format!("{table:?}: {why}"));
                }
            }
            return Ok(false);
        };
        let lexical_rows = frame.complete::<tables::LexicalTokens>().unwrap_or(&[]);
        let view = &frame.tables[TableId::LexicalTokens.index()];
        let mut lexical = std::mem::take(&mut self.lexical);
        if !lexical.view.as_ref().is_some_and(|known| known.is(view)) {
            lexical.view = Some(view.identity());
            lexical.positions = lexical_rows
                .iter()
                .enumerate()
                .map(|(at, row)| (row.token, at))
                .collect();
            #[cfg(test)]
            {
                lexical.builds += 1;
            }
        }
        let native_ids: HashMap<_, _> = country_rows
            .iter()
            .map(|country| (country.canonical_slot, country.id))
            .collect();
        let mut owners = HashMap::new();
        for row in status {
            let Some(owner) = countries.slotted(row.slot) else {
                continue;
            };
            // Doctrine-array slots are not country references. The status must
            // name the country in that canonical slot, directly or by a link.
            if native_ids.get(&row.slot) != Some(&row.country)
                && countries.slot(row.country) != Some(row.slot)
            {
                self.refuse(
                    &owner,
                    format!("doctrine slot {} country identity differs", row.slot),
                );
                continue;
            }
            // names: the presence witness distinguishes an empty inventory from
            // an absent/unassigned doctrine-array slot. No content is needed.
            values.push(format!("{owner}|present"), 1);
            owners.insert(row.slot, owner);
        }
        let mut folder_keys = BTreeSet::new();
        for folder in folders {
            let Some(owner) = owners.get(&folder.slot) else {
                continue;
            };
            let token = self.intern(folder.folder, &lexical.positions, lexical_rows, owner);
            let grand = folder
                .grand_doctrine
                .map(|raw| self.intern(raw, &lexical.positions, lexical_rows, owner))
                .unwrap_or(0);
            let mut completed = Vec::with_capacity(folder.completed_subdoctrines.len());
            for &raw in &folder.completed_subdoctrines {
                completed.push(self.intern(raw, &lexical.positions, lexical_rows, owner));
            }
            let inventory = self.folders.entry(folder.ordinal).or_default();
            inventory.tokens.insert(token);
            if grand != 0 {
                inventory.grands.insert(grand);
            }
            folder_keys.insert((folder.slot, folder.ordinal));
            values.push(
                format!("{owner}|{}|grand", folder.ordinal),
                i128::from(grand),
            );
            for (ordinal, token) in completed.into_iter().enumerate() {
                values.push(
                    format!("{owner}|{}|completed|{ordinal}", folder.ordinal),
                    i128::from(token),
                );
            }
        }
        for track in tracks {
            let Some(owner) = owners.get(&track.slot) else {
                continue;
            };
            if !folder_keys.contains(&(track.slot, track.folder)) {
                self.refuse(
                    owner,
                    format!(
                        "track {} has no witnessed folder {}",
                        track.ordinal, track.folder
                    ),
                );
            }
            let sub = track
                .sub_doctrine
                .map(|raw| self.intern(raw, &lexical.positions, lexical_rows, owner))
                .unwrap_or(0);
            let inventory = self.folders.entry(track.folder).or_default();
            inventory.tracks.insert(track.ordinal);
            if sub != 0 {
                inventory.subs.entry(track.ordinal).or_default().insert(sub);
            }
            // names: folder and track are their recorded ordered indices, not
            // country ids, inferred content tokens, or fixed vanilla lists.
            for (measure, value) in [
                ("mastery", i128::from(track.mastery)),
                ("bank", i128::from(track.mastery_bank)),
                ("daily", i128::from(track.daily_mastery)),
                ("effective", i128::from(track.effective_daily_mastery)),
                ("rewards", i128::from(track.rewards)),
                ("sub", i128::from(sub)),
            ] {
                values.push(
                    format!("{owner}|{}|{}|{measure}", track.folder, track.ordinal),
                    value,
                );
            }
        }
        self.lexical = lexical;
        Ok(true)
    }
}
