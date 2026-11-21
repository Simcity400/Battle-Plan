
use crate::rows::{
    CountryAppearanceRow, CountryLinkRow, CountryRow, LocalPlayerRow, PoliticalPartyRow,
    PoliticsRow, Recorded,
};

pub fn original_tag<'a>(
    country: &'a CountryRow,
    appearance: &'a CountryAppearanceRow,
) -> Result<&'a str, String> {
    original_tag_of(
        &appearance.original_tag_reference,
        &appearance.original_tag,
        &country.tag,
    )
}

pub fn original_tag_of<'a>(
    reference: &Recorded<Option<i32>>,
    original: &'a Recorded<Option<Vec<u8>>>,
    own: &'a [u8],
) -> Result<&'a str, String> {
    let reference = reference
        .require("original tag reference of country appearance")?
        .ok_or("original tag reference of country appearance capture refused")?;
    let bytes = if reference > 0 {
        original
            .require("original tag of country appearance")?
            .as_ref()
            .ok_or("original tag of country appearance capture refused")?
    } else {
        own
    };
    std::str::from_utf8(bytes).map_err(|_| "country original-or-own tag is not UTF-8".into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlagOperands<'a> {
    pub tag: &'a str,
    pub cosmetic: &'a str,
    pub original: Option<&'a str>,
    pub group: &'a str,
}

pub fn viewed_country<'a>(
    local: &[LocalPlayerRow],
    links: &[CountryLinkRow],
    countries: &'a [CountryRow],
    appearances: &'a [CountryAppearanceRow],
    politics: &[PoliticsRow],
    parties: &'a [PoliticalPartyRow],
) -> Result<FlagOperands<'a>, String> {
    let [player] = local else {
        return Err("local player has no single row".into());
    };
    let raw = player
        .viewing_country()
        .ok_or("the recording client views no country")?;
    let slot = links
        .iter()
        .rev()
        .find(|link| link.id == raw)
        .ok_or("the viewed country has no country links row")?
        .canonical_slot;
    let country = countries
        .iter()
        .rev()
        .find(|country| country.canonical_slot == slot)
        .ok_or("the viewed country has no Countries row")?;
    if (country.id as i32) <= 0 {
        return Err("the viewed country has no country object".into());
    }
    let owner = country.id;
    let tag = std::str::from_utf8(&country.tag).map_err(|_| "country tag is not UTF-8")?;
    let appearance = appearances
        .iter()
        .rev()
        .find(|row| row.country == owner)
        .ok_or_else(|| format!("country {owner} flag needs its country appearance row"))?;
    let cosmetic = std::str::from_utf8(&appearance.cosmetic_tag)
        .map_err(|_| "country cosmetic tag is not UTF-8")?;
    let ruling = politics
        .iter()
        .rev()
        .find(|row| row.country == owner)
        .ok_or_else(|| format!("country {owner} flag needs its Politics row"))?
        .ruling_party
        .ok_or_else(|| format!("country {owner} flag has a null ruling-party pointer"))?;
    let party = parties
        .iter()
        .rev()
        .find(|row| row.country == owner && row.id == ruling)
        .ok_or_else(|| {
            format!("country {owner} flag needs political parties ruling group {ruling}")
        })?;
    let group = std::str::from_utf8(&party.token)
        .map_err(|_| format!("country {owner} flag ruling group is not UTF-8"))?;
    Ok(FlagOperands {
        tag,
        cosmetic,
        original: original_tag(country, appearance).ok(),
        group,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::Recorded;

    struct Rows {
        local: Vec<LocalPlayerRow>,
        links: Vec<CountryLinkRow>,
        countries: Vec<CountryRow>,
        appearances: Vec<CountryAppearanceRow>,
        politics: Vec<PoliticsRow>,
        parties: Vec<PoliticalPartyRow>,
    }

    impl Rows {
        fn new() -> Self {
            Self {
                local: vec![LocalPlayerRow {
                    country: 9,
                    observer_country: 0,
                }],
                links: vec![
                    CountryLinkRow {
                        id: 7,
                        canonical_slot: 3,
                    },
                    CountryLinkRow {
                        id: 9,
                        canonical_slot: 3,
                    },
                ],
                countries: vec![CountryRow {
                    id: 7,
                    canonical_slot: 3,
                    tag: b"mod-country-long-tag".to_vec(),
                    ..Default::default()
                }],
                appearances: vec![CountryAppearanceRow {
                    country: 7,
                    cosmetic_tag: b"variants/red".to_vec(),
                    original_tag_reference: Recorded::Value(Some(0)),
                    original_tag: Recorded::Value(None),
                    ..Default::default()
                }],
                politics: vec![PoliticsRow {
                    country: 7,
                    ruling_party: Some(2),
                    ..Default::default()
                }],
                parties: vec![
                    PoliticalPartyRow {
                        country: 7,
                        id: 1,
                        token: b"other".to_vec(),
                        ..Default::default()
                    },
                    PoliticalPartyRow {
                        country: 7,
                        id: 2,
                        token: b"synthetic_group".to_vec(),
                        ..Default::default()
                    },
                ],
            }
        }

        fn viewed(&self) -> Result<FlagOperands<'_>, String> {
            viewed_country(
                &self.local,
                &self.links,
                &self.countries,
                &self.appearances,
                &self.politics,
                &self.parties,
            )
        }
    }

    #[test]
    fn the_viewed_country_resolves_through_its_link_to_the_country_objects_operands() {
        let rows = Rows::new();
        assert_eq!(
            rows.viewed().unwrap(),
            FlagOperands {
                tag: "mod-country-long-tag",
                cosmetic: "variants/red",
                original: Some("mod-country-long-tag"),
                group: "synthetic_group",
            }
        );
        let mut rows = Rows::new();
        rows.local[0] = LocalPlayerRow {
            country: 0,
            observer_country: 7,
        };
        assert_eq!(rows.viewed().unwrap().tag, "mod-country-long-tag");
        let mut rows = Rows::new();
        rows.appearances[0].original_tag_reference = Recorded::Value(Some(4));
        rows.appearances[0].original_tag = Recorded::Value(Some(b"ORIGINAL".to_vec()));
        assert_eq!(rows.viewed().unwrap().original, Some("ORIGINAL"));
        rows.appearances[0].original_tag = Recorded::NotRecorded;
        let viewed = rows.viewed().unwrap();
        assert_eq!((viewed.original, viewed.cosmetic), (None, "variants/red"));
    }

    #[test]
    fn a_missing_operand_names_why_no_flag_can_be_chosen() {
        let mut rows = Rows::new();
        rows.local[0] = LocalPlayerRow::default();
        assert!(rows.viewed().unwrap_err().contains("views no country"));
        let mut rows = Rows::new();
        rows.politics[0].ruling_party = None;
        assert!(rows.viewed().unwrap_err().contains("null ruling-party"));
        let mut rows = Rows::new();
        rows.parties.pop();
        assert!(rows.viewed().unwrap_err().contains("ruling group 2"));
        let mut rows = Rows::new();
        rows.appearances.clear();
        assert!(rows.viewed().unwrap_err().contains("country appearance row"));
        let mut rows = Rows::new();
        rows.countries[0].id = 0;
        assert!(rows.viewed().unwrap_err().contains("no country object"));
        let rows = Rows {
            local: Vec::new(),
            ..Rows::new()
        };
        assert!(rows.viewed().is_err());
    }
}
