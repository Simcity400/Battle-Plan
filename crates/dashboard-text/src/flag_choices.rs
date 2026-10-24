//! Flag asset candidates per recorded country, read from one decoded hour:
//! shared by the country flag product and the map's combat and raid tooltips.

use crate::metadata::recorded_rows;
use replay_data::rows::tables;
use replay_frame::frame::DecodedSource;
use std::{collections::BTreeMap, io};

pub struct FlagChoice {
    pub raw: u32,
    pub tag: String,
    pub candidates: Vec<String>,
    /// Why the native candidate chain is unknown. A known cosmetic candidate
    /// can still supply the complete flag when only its later tag is unknown.
    pub refusal: Option<String>,
}

impl FlagChoice {
    pub fn key(&self, exists: impl Fn(&str) -> bool) -> Result<&str, String> {
        self.candidates
            .iter()
            .find(|key| exists(key))
            .map(String::as_str)
            .ok_or_else(|| {
                self.refusal
                    .clone()
                    .unwrap_or_else(|| "country has no resolved flag artwork".into())
            })
    }
}

/// The flag's ruling ideology group, retaining a missing or unsupported join
/// instead of skipping the group-qualified candidate.
pub fn ruling_groups(
    frame: &dyn DecodedSource,
) -> io::Result<BTreeMap<u32, Result<Option<String>, String>>> {
    let parties: BTreeMap<_, _> = recorded_rows::<tables::PoliticalParties>(frame)?
        .iter()
        .map(|row| ((row.country, row.id), row.token.as_slice()))
        .collect();
    let politics: BTreeMap<_, _> = recorded_rows::<tables::Politics>(frame)?
        .iter()
        .map(|row| (row.country, row.ruling_party))
        .collect();
    Ok(recorded_rows::<tables::Countries>(frame)?
        .iter()
        .map(|row| {
            let country = row.id;
            let group = (|| {
                let ruling = politics
                    .get(&country)
                    .ok_or_else(|| format!("country {country} flag needs its Politics row"))?;
                let ruling = ruling.as_ref().ok_or_else(|| {
                    format!("country {country} flag has a null ruling-party pointer")
                })?;
                let token = parties.get(&(country, *ruling)).ok_or_else(|| {
                    format!("country {country} flag needs PoliticalParties ruling group {ruling}")
                })?;
                let token = std::str::from_utf8(token)
                    .map_err(|_| format!("country {country} flag ruling group is not UTF-8"))?;
                // Capture authenticates the ordinary group, whose active flag is
                // initialized to 1 and never changed by authored content. The null
                // group is refused. Even an empty accepted token is an active group,
                // so the qualified candidate still appends its underscore.
                Ok(Some(token.to_owned()))
            })();
            (country, group)
        })
        .collect())
}

/// Flag asset candidates per country at one hour, shared with the combat UI.
pub fn choices(frame: &dyn DecodedSource) -> io::Result<Vec<FlagChoice>> {
    let links: BTreeMap<_, _> = recorded_rows::<tables::CountryLinks>(frame)?
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    let countries: BTreeMap<_, _> = recorded_rows::<tables::Countries>(frame)?
        .iter()
        .map(|r| (r.canonical_slot, r))
        .collect();
    let groups = ruling_groups(frame)?;
    let appearances: BTreeMap<_, _> = recorded_rows::<tables::CountryAppearance>(frame)?
        .iter()
        .map(|r| (r.country, r))
        .collect();
    Ok(links
        .into_iter()
        .filter_map(|(raw, slot)| {
            let country =
                crate::triggers::refusal::reference(countries.get(&slot), slot != 0, || {
                    crate::triggers::refusal::missing_reference(
                        replay_data::rows::TableId::Countries,
                        Some(crate::triggers::Store::Country(slot)),
                    )
                })?;
            // Atlas population tests the object's own country reference.
            if (country.id as i32) <= 0 {
                return None;
            }
            let tag = String::from_utf8_lossy(&country.tag).into_owned();
            // Atlas cells belong to the resolved country object. An alias link
            // supplies the same object's captured appearance and party.
            let owner = country.id;
            let appearance = appearances.get(&owner);
            let cosmetic = appearance
                .ok_or_else(|| format!("country {owner} flag needs its CountryAppearance row"))
                .and_then(|row| {
                    std::str::from_utf8(&row.cosmetic_tag)
                        .map_err(|_| "country cosmetic tag is not UTF-8".to_owned())
                });
            let original = appearance
                .ok_or_else(|| format!("country {owner} flag needs its CountryAppearance row"))
                .and_then(|row| installed_content_country_names::country_names::original_tag(country, row));
            let group = groups
                .get(&owner)
                .cloned()
                .unwrap_or_else(|| Err(format!("country {owner} flag needs its ruling group")));
            let candidates = match (&cosmetic, &group) {
                (Ok(cosmetic), Ok(group)) => installed_content_country_names::country_names::flag_keys(
                    original.as_ref().ok().copied(),
                    cosmetic,
                    group.as_deref(),
                ),
                _ => Vec::new(),
            };
            let refusal = cosmetic
                .err()
                .or_else(|| group.err())
                .or_else(|| original.err());
            Some(FlagChoice {
                raw,
                tag,
                candidates,
                refusal,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{
        CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticalPartyRow, PoliticsRow, Recorded,
        TableId, TableRows,
    };
    use replay_frame::frame::{DecodedFrame, Metrics, TableView};

    #[test]
    fn decoded_source_keeps_mod_art_candidates_aliases_and_unknown_tail() {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 17,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
        };
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                [100, 150]
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: 1,
                    })
                    .into(),
            ));
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 100,
                canonical_slot: 1,
                tag: b"D01".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 100,
                cosmetic_tag: b"MOD_COS".to_vec(),
                original_tag_reference: Recorded::Value(Some(200)),
                original_tag: Recorded::Value(Some(b"ORG".to_vec())),
                ..Default::default()
            }]));
        *frame.tables[TableId::Politics.index()] =
            TableView::Complete(TableRows::Politics(vec![PoliticsRow {
                country: 100,
                ruling_party: Some(7),
                ..Default::default()
            }]));
        *frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 100,
                id: 7,
                token: b"mod_group".to_vec(),
                ..Default::default()
            }]));
        let source: &dyn DecodedSource = &frame;
        let flags = choices(source).unwrap();
        assert_eq!(
            flags.iter().map(|flag| flag.raw).collect::<Vec<_>>(),
            [100, 150]
        );
        for flag in &flags {
            assert_eq!(flag.tag, "D01");
            assert_eq!(
                flag.candidates,
                ["MOD_COS_mod_group", "MOD_COS", "ORG_mod_group", "ORG"]
            );
            assert_eq!(flag.key(|key| key == "ORG_mod_group"), Ok("ORG_mod_group"));
            assert_eq!(flag.refusal, None);
        }
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].original_tag_reference = Recorded::Value(None);
        }
        for flag in choices(&frame).unwrap() {
            assert_eq!(flag.candidates, ["MOD_COS_mod_group", "MOD_COS"]);
            assert_eq!(flag.key(|key| key == "MOD_COS"), Ok("MOD_COS"));
            assert_eq!(
                flag.key(|_| false).unwrap_err(),
                "CountryAppearance.original_tag_reference capture refused"
            );
        }
    }
}
