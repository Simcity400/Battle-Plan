//! Verbatim copies of the naval regions' whole observations before their changes were followed.
use super::*;
use crate::shared;
use dashboard_base::prepared::{frame_changes::FrameChanges, prepass::Family};
use dashboard_keyed::countries::Countries;
use replay::container::TableFields;
use replay_data::rows::{NavalRegionalMineRow, RowCodec, TableSchema};
use replay_derive::naval_mine_levels::Levels;
use serde::{Deserialize, Serialize};
use std::io;

/// The owner part of the count of vectors the hour holds but cannot name.
const UNNAMED: &str = "?";

fn save<T: TableSchema>(
    frame: &DecodedFrame,
    values: &mut Values,
    reduce: impl Fn(&T::Row) -> T::Row,
) -> io::Result<()> {
    let Ok(rows) = frame.complete::<T>() else {
        return Ok(());
    };
    let prefix = format!("input|{}", T::ID.index());
    values.push(format!("{prefix}|known"), 1);
    for row in rows {
        let row = reduce(row);
        let delta = row
            .delta(None)?
            .ok_or_else(|| io::Error::other("Prepared naval row has no insertion"))?;
        let name = format!("{prefix}|{}", delta.key);
        values.push(format!("{name}|length"), delta.values.len() as i128);
        values.push(format!("{name}|fields"), delta.recorded_fields as i128);
        values.push(format!("{name}|mask"), i128::from(delta.mask));
        for (at, chunk) in delta.values.chunks(8).enumerate() {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            values.push(format!("{name}|{at}"), i128::from(u64::from_le_bytes(word)));
        }
    }
    Ok(())
}

/// Naval supremacy observed whole every hour as before its changes were followed.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Supremacy {
    pub refusals: BTreeMap<String, u64>,
}
impl Observer for Supremacy {
    type Content = ();

    fn content(
        _source: &dashboard_base::Source,
        _options: Option<&dashboard_core::config::ContentOptions>,
        _check: &mut dyn FnMut() -> std::io::Result<()>,
    ) -> std::io::Result<Self::Content> {
        Ok(())
    }

    const NAME: &'static str = NavalSupremacyObserver::NAME;
    const TABLES: &'static [TableId] = NavalSupremacyObserver::TABLES;
    const READ: TableId = NavalSupremacyObserver::READ;
    fn family() -> &'static Family {
        NavalSupremacyObserver::family()
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        NavalSupremacyObserver::fields()
    }
    fn changed(changes: &FrameChanges) -> bool {
        NavalSupremacyObserver::changed(changes)
    }
    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let Ok(currents) = frame.complete::<tables::NavalSupremacy>() else {
            shared::note(&mut self.refusals, "NavalSupremacy is not recorded");
            return Ok(false);
        };
        if let Some(hour) = frame.tables[Self::READ.index()].sampled_at() {
            values.push("epoch".into(), i128::from(hour));
        }
        let countries = Countries::read(frame);
        // names: TAG@slot|region|raw:<reference>, with raw references distinct;
        // input-current|region|raw is an unresolved-identity operand, not a label.
        // input|table|row|length/fields/mask/word holds reduced diplomacy rows.
        for row in currents {
            let name = shared::country_part(frame, countries.as_ref(), row.country)
                .filter(|n| !n.contains('|'))
                .map_or_else(
                    || format!("input-current|{}|{}", row.region, row.country),
                    |owner| format!("{owner}|{}|raw:{}", row.region, row.country),
                );
            values.push(name, i128::from(row.current));
            values.push(
                format!("region|{}|slot", row.region),
                i128::from(row.region_slot),
            );
        }
        save::<tables::CountryLinks>(frame, values, Clone::clone)?;
        save::<tables::Countries>(frame, values, |r| rows::CountryRow {
            id: r.id,
            canonical_slot: r.canonical_slot,
            tag: r.tag.clone(),
            ..Default::default()
        })?;
        save::<tables::FactionMembers>(frame, values, Clone::clone)?;
        save::<tables::Subjects>(frame, values, |r| rows::SubjectRow {
            overlord: r.overlord,
            subject: r.subject,
            ..Default::default()
        })?;
        save::<tables::WarRelations>(frame, values, |r| rows::WarRelationRow {
            first_country: r.first_country,
            second_country: r.second_country,
            ..Default::default()
        })?;
        save::<tables::AirCountryRelations>(frame, values, Clone::clone)?;
        save::<tables::RelationTuples>(frame, values, |r| rows::RelationTupleRow {
            owner_country: r.owner_country,
            target_country: r.target_country,
            type_id: r.type_id,
            ordinal: r.ordinal,
            first_country: r.first_country,
            second_country: r.second_country,
            ..Default::default()
        })?;
        save::<tables::CountryCounterInputs>(frame, values, |r| {
            rows::CountryCounterInputRow {
                country: r.country,
                forward_visibility: r.forward_visibility,
                directed_wars: r.directed_wars.clone(),
                ..Default::default()
            }
        })?;
        save::<tables::ReservedCountryNaming>(frame, values, |r| {
            rows::ReservedCountryNamingRow {
                tag: r.tag.clone(),
                faction: r.faction,
                reverse_visibility: r.reverse_visibility.clone(),
                ..Default::default()
            }
        })?;
        Ok(true)
    }
}

/// Minefields observed whole every hour as before their changes were followed.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Minefields {
    pub refusals: BTreeMap<String, u64>,
}
impl Observer for Minefields {
    type Content = ();

    fn content(
        _source: &dashboard_base::Source,
        _options: Option<&dashboard_core::config::ContentOptions>,
        _check: &mut dyn FnMut() -> std::io::Result<()>,
    ) -> std::io::Result<Self::Content> {
        Ok(())
    }

    const NAME: &'static str = MinefieldsObserver::NAME;
    const TABLES: &'static [TableId] = MinefieldsObserver::TABLES;
    const READ: TableId = MinefieldsObserver::READ;
    fn family() -> &'static Family {
        MinefieldsObserver::family()
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        MinefieldsObserver::fields()
    }
    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let (Some(countries), Ok(rows), Ok(links)) = (
            Countries::read(frame),
            frame.complete::<tables::NavalRegionalMines>(),
            frame.complete::<tables::CountryLinks>(),
        ) else {
            shared::note(
                &mut self.refusals,
                "Minefield amounts or country identities are not recorded",
            );
            return Ok(false);
        };
        let links: BTreeMap<_, _> = links.iter().map(|r| (r.id, r.canonical_slot)).collect();
        let mut groups = BTreeMap::<u32, Vec<&NavalRegionalMineRow>>::new();
        // Owners whose vectors this hour holds but cannot name.
        let mut unnamed = BTreeSet::new();
        for row in rows {
            if let Some(slot) = countries.slot(row.country) {
                groups.entry(slot).or_default().push(row);
            } else {
                unnamed.insert(row.country);
                shared::note(
                    &mut self.refusals,
                    "Minefield owner country link is unavailable",
                );
            }
        }
        let mut unnamed = unnamed.len() as i128;
        for (slot, group) in groups {
            let Some(owner) = countries.slotted(slot).filter(|s| !s.contains('|')) else {
                unnamed += 1;
                shared::note(&mut self.refusals, "Minefield owner tag is unavailable");
                continue;
            };
            let raw = group[0].country;
            let group: Vec<_> = group.into_iter().copied().collect();
            let admitted = (|| -> Result<u32, String> {
                let levels = Levels::from_rows(&group, &links)?;
                let witnesses: Vec<_> = group.iter().filter(|r| r.kind == 0).collect();
                let [witness] = witnesses.as_slice() else {
                    return Err(
                        "Regional mine owner completeness witness is missing or duplicated".into(),
                    );
                };
                for row in &group {
                    if row.country != raw {
                        return Err("Regional mine vector duplicates a canonical country".into());
                    }
                    if row.kind == 1 {
                        let _ = levels.amount(raw, row.region)?;
                    }
                }
                Ok(witness.region)
            })();
            match admitted {
                Ok(extent) => {
                    // names: OWNER|extent and OWNER|region; zeros stay sparse.
                    values.push(format!("{owner}|extent"), i128::from(extent));
                    for row in group.iter().filter(|r| r.kind == 1 && r.amount != 0) {
                        values.push(format!("{owner}|{}", row.region), i128::from(row.amount));
                    }
                }
                Err(why) => {
                    // Dated: the owner is known to hold an unadmitted vector.
                    values.push(format!("{owner}|refused"), 1);
                    shared::note(&mut self.refusals, why);
                }
            }
        }
        if unnamed > 0 {
            values.push(format!("{UNNAMED}|refused"), unnamed);
        }
        Ok(true)
    }
}
