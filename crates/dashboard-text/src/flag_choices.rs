//! Flag asset candidates per recorded country: one chain from a country's recorded operands
//! ([`Operands::chain`]), which preparation reads from the walk's flag inputs and the map's
//! combat and raid tooltips from one decoded hour ([`choices`]).

use crate::metadata::recorded_rows_in;
use replay_data::rows::{
    CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticalPartyRow, PoliticsRow, Recorded,
    mask, tables,
};
use replay_frame::frame::{DecodedSource, HourFrame, Rowset};
use std::{borrow::Cow, collections::BTreeMap, io};

/// The fields of each table the flag chains read.
const PARTY_TOKEN: u64 = mask::<PoliticalPartyRow>(&["token"]);
const RULING: u64 = mask::<PoliticsRow>(&["ruling_party"]);
const COUNTRY: u64 = mask::<CountryRow>(&["canonical_slot", "tag"]);
const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const LOOK: u64 = mask::<CountryAppearanceRow>(&[
    "cosmetic_tag",
    "original_tag_reference",
    "original_tag",
]);

/// Each row of `rows`, keyed by `key`.
fn by<'a, R: replay_data::rows::RowCodec + Clone, K: Ord>(
    rows: Rowset<'a, R>,
    key: impl Fn(&R) -> K,
) -> io::Result<BTreeMap<K, Cow<'a, R>>> {
    rows.iter()
        .map(|row| row.map(|row| (key(&row), row)))
        .collect()
}

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

/// The candidates the game searches a country's flag file by, in order, and why they are not
/// the whole chain.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[derive(replay_spool::Spooled)]
pub struct Chain {
    pub candidates: Vec<String>,
    /// As [`FlagChoice::refusal`].
    pub refusal: Option<String>,
}

/// A country object's CountryAppearance operands of its flag, as recorded.
#[derive(Clone, Copy, Debug)]
pub struct Appearance<'a> {
    pub cosmetic: &'a [u8],
    /// Positive: `original` is the original tag; otherwise the country's own tag is.
    pub reference: Recorded<Option<i32>>,
    pub original: Recorded<Option<&'a [u8]>>,
}

/// One country's flag operands, each its text or why it is unknown.
pub struct Operands<'a> {
    /// Empty without a cosmetic tag.
    pub cosmetic: Result<&'a str, String>,
    /// The original-or-own tag: unknown, it withholds only the chain's tag tail.
    pub original: Result<&'a str, String>,
    /// The ruling party's ideology group token.
    pub group: Result<Option<&'a str>, String>,
}

impl<'a> Operands<'a> {
    /// Country object `owner`'s operands from its own tag, its appearance, its
    /// `Politics.ruling_party` (the inner None a null pointer) and the ruling party's token;
    /// a part it has no row of is None.
    pub fn of(
        owner: u32,
        own: &'a [u8],
        appearance: Option<Appearance<'a>>,
        ruling: Option<Option<u32>>,
        token: Option<&'a [u8]>,
    ) -> Self {
        let missing = || format!("country {owner} flag needs its CountryAppearance row");
        Self {
            cosmetic: appearance.ok_or_else(missing).and_then(|appearance| {
                std::str::from_utf8(appearance.cosmetic)
                    .map_err(|_| "country cosmetic tag is not UTF-8".to_owned())
            }),
            original: appearance
                .ok_or_else(missing)
                .and_then(|appearance| original(own, appearance)),
            group: group(owner, ruling, token),
        }
    }

    /// The chain every reader searches.
    pub fn chain(self) -> Chain {
        let candidates = match (&self.cosmetic, &self.group) {
            (Ok(cosmetic), Ok(group)) => installed_content_country_names::country_names::flag_keys(
                self.original.as_ref().ok().copied(),
                cosmetic,
                *group,
            ),
            _ => Vec::new(),
        };
        let refusal = self
            .cosmetic
            .err()
            .or_else(|| self.group.err())
            .or_else(|| self.original.err());
        Chain {
            candidates,
            refusal,
        }
    }
}

/// The original-or-own tag: the positive reference's original tag, else `own`. An unread or
/// historically absent original cannot be inferred from the name tag.
fn original<'a>(own: &'a [u8], appearance: Appearance<'a>) -> Result<&'a str, String> {
    let reference = (*appearance
        .reference
        .require("CountryAppearance.original_tag_reference")?)
    .ok_or("CountryAppearance.original_tag_reference capture refused")?;
    let bytes = if reference > 0 {
        (*appearance
            .original
            .require("CountryAppearance.original_tag")?)
        .ok_or("CountryAppearance.original_tag capture refused")?
    } else {
        own
    };
    std::str::from_utf8(bytes).map_err(|_| "country original-or-own tag is not UTF-8".into())
}

/// The ruling ideology group: the capture authenticates the ordinary group, whose active flag is
/// initialized to 1 and never changed by authored content, and refuses the null group. Even an
/// empty accepted token is an active group, so the qualified candidate still appends its
/// underscore.
fn group(
    owner: u32,
    ruling: Option<Option<u32>>,
    token: Option<&[u8]>,
) -> Result<Option<&str>, String> {
    let ruling = ruling.ok_or_else(|| format!("country {owner} flag needs its Politics row"))?;
    let ruling =
        ruling.ok_or_else(|| format!("country {owner} flag has a null ruling-party pointer"))?;
    let token = token.ok_or_else(|| {
        format!("country {owner} flag needs PoliticalParties ruling group {ruling}")
    })?;
    std::str::from_utf8(token)
        .map(Some)
        .map_err(|_| format!("country {owner} flag ruling group is not UTF-8"))
}

/// Each country's `Politics.ruling_party` and the party, by country object.
#[allow(clippy::type_complexity)]
fn ruling_rows(
    frame: HourFrame<'_>,
) -> io::Result<(
    BTreeMap<u32, Option<u32>>,
    BTreeMap<(u32, u32), Cow<'_, PoliticalPartyRow>>,
)> {
    let parties = by(
        recorded_rows_in::<tables::PoliticalParties>(frame, PARTY_TOKEN)?,
        |row| (row.country, row.id),
    )?;
    let politics = recorded_rows_in::<tables::Politics>(frame, RULING)?
        .iter()
        .map(|row| row.map(|row| (row.country, row.ruling_party)))
        .collect::<io::Result<_>>()?;
    Ok((politics, parties))
}

/// The flag's ruling ideology group, retaining a missing or unsupported join
/// instead of skipping the group-qualified candidate.
pub fn ruling_groups(
    frame: &dyn DecodedSource,
) -> io::Result<BTreeMap<u32, Result<Option<String>, String>>> {
    ruling_groups_in(HourFrame::Decoded(frame))
}

/// [`ruling_groups`] of any frame's tables, a walk's views among them.
pub fn ruling_groups_in(
    frame: HourFrame<'_>,
) -> io::Result<BTreeMap<u32, Result<Option<String>, String>>> {
    let (politics, parties) = ruling_rows(frame)?;
    // A country's object is its key.
    Ok(recorded_rows_in::<tables::Countries>(frame, 0)?
        .keys()
        .map(|key| {
            let id = key as u32;
            let ruling = politics.get(&id).copied();
            let token = ruling
                .flatten()
                .and_then(|party| parties.get(&(id, party)))
                .map(|party| party.token.as_slice());
            let group = group(id, ruling, token).map(|group| group.map(str::to_owned));
            (id, group)
        })
        .collect())
}

/// Flag asset candidates per country at one hour, shared with the combat UI.
pub fn choices(frame: &dyn DecodedSource) -> io::Result<Vec<FlagChoice>> {
    choices_in(HourFrame::Decoded(frame))
}

/// [`choices`] of any frame's tables, a walk's views among them.
pub fn choices_in(frame: HourFrame<'_>) -> io::Result<Vec<FlagChoice>> {
    let links: BTreeMap<_, _> = recorded_rows_in::<tables::CountryLinks>(frame, LINK)?
        .iter()
        .map(|r| r.map(|r| (r.id, r.canonical_slot)))
        .collect::<io::Result<_>>()?;
    let countries = by(
        recorded_rows_in::<tables::Countries>(frame, COUNTRY)?,
        |r| r.canonical_slot,
    )?;
    let (politics, parties) = ruling_rows(frame)?;
    let appearances = by(
        recorded_rows_in::<tables::CountryAppearance>(frame, LOOK)?,
        |r| r.country,
    )?;
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
            // Atlas cells belong to the resolved country object. An alias link
            // supplies the same object's captured appearance and party.
            let owner = country.id;
            let appearance = appearances.get(&owner).map(|row| Appearance {
                cosmetic: &row.cosmetic_tag,
                reference: row.original_tag_reference,
                original: match &row.original_tag {
                    Recorded::NotRecorded => Recorded::NotRecorded,
                    Recorded::Value(original) => Recorded::Value(original.as_deref()),
                },
            });
            let ruling = politics.get(&owner).copied();
            let token = ruling
                .flatten()
                .and_then(|party| parties.get(&(owner, party)))
                .map(|party| party.token.as_slice());
            let Chain {
                candidates,
                refusal,
            } = Operands::of(owner, &country.tag, appearance, ruling, token).chain();
            Some(FlagChoice {
                raw,
                tag: String::from_utf8_lossy(&country.tag).into_owned(),
                candidates,
                refusal,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests;
