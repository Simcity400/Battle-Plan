//! The raw D68 header, split into its two grains BEFORE anything can refuse
//! either of them.
//!
//! ## Why this is a type and not a fixture
//!
//! The D68 header is mixed grain. `player`, `ideology` and `cosmetic_tag` are
//! composed out of the country identities, so a fold this reader cannot admit
//! may not publish them; `dlcs`, `start_date_raw`, `save_version`, `session`
//! and `version` are SESSION constants the country account says nothing about,
//! and discarding them cost rc_vanilla7 its measured DLC word for a refusal
//! that was never about it (lane UB, M11/M14).
//!
//! The refusal used to walk the raw object leaf by leaf:
//!
//! ```text
//! for leaf in SESSION_HEADER_COUNTRY_LEAVES {
//!     if header.contains_key(leaf) { header.insert(leaf.into(), Value::Null); }
//! }
//! ```
//!
//! That loop is a PREDICATE over the shapes a raw JSON header can take, and a
//! predicate over an infinite space cannot be completed by enumeration. UB's
//! final mutation M34 added one arm to it -- skip nulling a `player` that is
//! not an object -- and the seven-factor covering array behind
//! `a_missing_country_account_costs_the_d68_header_its_country_and_not_its_session`
//! could not see it, because the TYPE of a carried country leaf was not one of
//! its factors. UB recorded the mutation as a KNOWN LIMIT and named the close:
//! split the header into a country half and a session half before the refusal,
//! so the refusal drops the country half wholesale and never sees a leaf.
//!
//! This module is that split. `refuse_country` is `self.country.take()`: a
//! whole-half drop with nothing to branch on. M34 cannot be written here --
//! there is no loop to add an arm to, no `contains_key` to negate, and no
//! access to a leaf's JSON at the point of refusal. The shapes the leaves
//! carry are read once, AFTER the refusal has already decided, by the builder
//! that turns them into leaves.

use serde_json::{Map, Value};

/// The composed half: one slot per leaf the country identities compose,
/// always all three, each holding whatever JSON the raw header carried there
/// (`None` = the header did not carry the key).
///
/// Three named fields and not a map, so `carried` is a total walk of the
/// struct's own slots rather than a key test that an arm could skip.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionHeaderCountryRaw {
    player: Option<Value>,
    ideology: Option<Value>,
    cosmetic_tag: Option<Value>,
}

/// The leaves the country identities compose. Derived from the struct, so the
/// list and the fields cannot drift apart.
/// Read by the tests that prove the split and the refusal cover exactly these
/// three, so it is compiled with them.
#[cfg(test)]
pub(crate) const SESSION_HEADER_COUNTRY_LEAVES: [&str; 3] =
    ["player", "ideology", "cosmetic_tag"];

impl SessionHeaderCountryRaw {
    fn take_from(header: &mut Map<String, Value>) -> Self {
        Self {
            player: header.remove("player"),
            ideology: header.remove("ideology"),
            cosmetic_tag: header.remove("cosmetic_tag"),
        }
    }

    /// (name, slot) for every composed leaf, in the order the header names
    /// them.
    fn leaves(&self) -> [(&'static str, &Option<Value>); 3] {
        [
            ("player", &self.player),
            ("ideology", &self.ideology),
            ("cosmetic_tag", &self.cosmetic_tag),
        ]
    }

    /// The composed leaves this header actually carried, named at the SPLIT.
    /// The refusal reports these; it does not look at the header again.
    fn carried(&self) -> Vec<&'static str> {
        self.leaves()
            .into_iter()
            .filter(|(_, slot)| slot.is_some())
            .map(|(name, _)| name)
            .collect()
    }

    pub(crate) fn player(&self) -> Option<&Value> {
        self.player.as_ref()
    }

    pub(crate) fn ideology(&self) -> Option<&Value> {
        self.ideology.as_ref()
    }

    pub(crate) fn cosmetic_tag(&self) -> Option<&Value> {
        self.cosmetic_tag.as_ref()
    }
}

/// The raw D68 header as this reader found it, already split.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SessionHeaderRaw {
    /// The frame carries no `session_header` key.
    Absent,
    /// The frame carries it as `null`, or a refusal took the whole thing --
    /// the domain's own "unavailable" spelling either way.
    Unavailable,
    /// Present and not an object: nothing in it can be shown to carry no
    /// country identity, so a refusal takes the whole thing. It keeps the JSON
    /// type name, because the reader that used to walk this member reported
    /// what it found instead of the object it wanted, and a degraded outcome
    /// names its cause (Codex round 1 finding 6, lane XE).
    Unwalkable(&'static str),
    /// Walked and SPLIT. `country` is `None` once a refusal has dropped it.
    Split {
        country: Option<SessionHeaderCountryRaw>,
        session: Map<String, Value>,
    },
}

impl SessionHeaderRaw {
    /// Split the raw member out of the frame. The two grains are apart from
    /// this point on, and nothing downstream sees the undivided object.
    pub(crate) fn split(member: Option<Value>) -> Self {
        match member {
            None => Self::Absent,
            Some(Value::Null) => Self::Unavailable,
            Some(Value::Object(mut header)) => {
                let country = SessionHeaderCountryRaw::take_from(&mut header);
                Self::Split {
                    country: Some(country),
                    // WHAT IS LEFT, and that is the definition: a member this
                    // reader does not compose out of the country identities is
                    // a session constant, which is exactly what the leaf loop
                    // this replaces left alone.
                    session: header,
                }
            }
            Some(other) => Self::Unwalkable(crate::world::json_type_name(&other)),
        }
    }

    /// The account refusal, and the whole of it: the country half is DROPPED.
    ///
    /// There is no leaf here and no shape to branch on. The names in the
    /// warnings come from the split, not from a second look at the header, so
    /// a rule that skipped a leaf could change what is REPORTED and never what
    /// is refused.
    pub(crate) fn refuse_country(&mut self, reason: &str, warnings: &mut Vec<String>) {
        match self {
            // Nothing was carried, so nothing is withheld and nothing is
            // claimed. Round 32's guarantee -- a header beside no account still
            // refuses -- lives in `withheld_countries_account_refusal`, which
            // is asked BEFORE the split and sees the member.
            Self::Absent | Self::Unavailable => {}
            // THE VARIANT SURVIVES THE REFUSAL. Collapsing it to
            // `Unavailable` here took the JSON type with it, so a frame that
            // refused the account AND carried a non-object header degraded
            // with no word about what it found -- the same silence finding 6
            // closed, one branch over (Codex round 2 finding 5). There is
            // nothing to drop: the country half never existed.
            Self::Unwalkable(_) => {
                warnings.push(format!("session_header: {reason}"));
            }
            Self::Split { country, .. } => {
                // The member is NAMED whatever it turns out to have carried:
                // the fold was judged, and a refusal that withholds nothing
                // still has to say it was made.
                warnings.push(format!("session_header: {reason}"));
                let Some(dropped) = country.take() else {
                    return;
                };
                for leaf in dropped.carried() {
                    warnings.push(format!("session_header.{leaf}: {reason}"));
                }
            }
        }
    }

    /// The composed half, or `None` when the header carried none or a refusal
    /// dropped it.
    pub(crate) fn country(&self) -> Option<&SessionHeaderCountryRaw> {
        match self {
            Self::Split { country, .. } => country.as_ref(),
            _ => None,
        }
    }

    /// The session-constant half, for the builder to read its leaves out of.
    pub(crate) fn session(&self) -> Option<&Map<String, Value>> {
        match self {
            Self::Split { session, .. } => Some(session),
            _ => None,
        }
    }

    /// Whether the frame carried the member at all -- the difference between
    /// `DomainAbsent` and `DomainUnavailable`.
    pub(crate) fn was_carried(&self) -> bool {
        !matches!(self, Self::Absent)
    }

    /// The JSON type a present-but-unwalkable header carried, for the builder
    /// to name as the cause. `None` for every other shape.
    pub(crate) fn unwalkable_type(&self) -> Option<&'static str> {
        match self {
            Self::Unwalkable(found) => Some(found),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn split_of(value: Value) -> SessionHeaderRaw {
        SessionHeaderRaw::split(Some(value))
    }

    #[test]
    fn the_split_puts_each_leaf_in_exactly_one_half() {
        // kills: `SessionHeaderRaw::split` -- leave `player` in the session map as well as taking it into the country half, and a refusal stops removing it.
        let raw = split_of(json!({
            "player": {"tag_db_id": 368, "country_tag": "D04"},
            "ideology": "fascism",
            "cosmetic_tag": "GER_reich",
            "dlcs": 257535,
            "start_date_raw": 1,
            "save_version": 2,
            "session": 3,
            "version": "1.16.4",
        }));
        let country = raw.country().expect("the composed half");
        assert!(country.player().is_some());
        assert!(country.ideology().is_some());
        assert!(country.cosmetic_tag().is_some());
        let session = raw.session().expect("the session half");
        for composed in SESSION_HEADER_COUNTRY_LEAVES {
            assert!(
                !session.contains_key(composed),
                "{composed} is in both halves"
            );
        }
        for constant in ["dlcs", "start_date_raw", "save_version", "session", "version"] {
            assert!(session.contains_key(constant), "{constant} left the header");
        }
    }

    /// UB's M34, and the shape family behind it, made unrepresentable.
    ///
    /// M34 skipped nulling a `player` that is not an object. Here the refusal
    /// never sees a leaf's JSON: it drops the half. So the property holds for
    /// EVERY shape a country leaf can take -- which is what the seven-factor
    /// covering array could not reach -- and the session half is untouched for
    /// every one of them.
    #[test]
    fn a_refusal_drops_the_country_half_whatever_shape_its_leaves_carry() {
        // kills: `SessionHeaderRaw::refuse_country` -- replace `country.take()` with a per-leaf loop over the raw map (UB M34) and the non-object `player` shapes survive the refusal.
        let shapes = [
            json!(null),
            json!(7),
            json!("D04"),
            json!([1, 2]),
            json!({}),
            json!({"tag_db_id": 368, "country_tag": "D04"}),
            json!(true),
            json!(1.5),
        ];
        for player in &shapes {
            for ideology in &shapes {
                let mut raw = split_of(json!({
                    "player": player,
                    "ideology": ideology,
                    "cosmetic_tag": "GER_reich",
                    "dlcs": 257535,
                    "version": "1.16.4",
                }));
                let mut warnings = Vec::new();
                raw.refuse_country("the account is unreadable", &mut warnings);
                assert!(
                    raw.country().is_none(),
                    "the composed half survived a refusal for player={player} ideology={ideology}"
                );
                let session = raw.session().expect("the session half survives");
                assert_eq!(session.get("dlcs"), Some(&json!(257535)));
                assert_eq!(session.get("version"), Some(&json!("1.16.4")));
                assert!(
                    warnings.iter().any(|w| w == "session_header.player: the account is unreadable"),
                    "the refusal must name every composed leaf the header carried: {warnings:?}"
                );
            }
        }
    }

    #[test]
    fn a_header_this_reader_cannot_walk_is_refused_whole_and_named() {
        // kills: `split` -- build `Unwalkable` without the `json_type_name` it found, and the header degrades with no cause.
        let mut raw = split_of(json!(7));
        assert_eq!(raw, SessionHeaderRaw::Unwalkable("number"));
        assert_eq!(raw.unwalkable_type(), Some("number"));
        let mut warnings = Vec::new();
        raw.refuse_country("the account is unreadable", &mut warnings);
        assert_eq!(warnings, vec!["session_header: the account is unreadable"]);
        // ...AND THE TYPE SURVIVES IT. A refusal takes the country half, and an
        // unwalkable header has none; what it does have is the word for what it
        // found, which the builder still needs to say.
        assert_eq!(raw, SessionHeaderRaw::Unwalkable("number"));
        assert_eq!(raw.unwalkable_type(), Some("number"));
        assert!(raw.was_carried(), "an unwalkable header was still carried");
    }

    #[test]
    fn an_absent_header_is_neither_refused_nor_named() {
        // kills: `split` -- map a MISSING member onto `Unwalkable` rather than `Absent`, and an absent header claims a wrong type it never had.
        let mut raw = SessionHeaderRaw::split(None);
        let mut warnings = Vec::new();
        raw.refuse_country("the account is unreadable", &mut warnings);
        assert_eq!(raw, SessionHeaderRaw::Absent);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!raw.was_carried());

        let mut null = split_of(json!(null));
        let mut warnings = Vec::new();
        null.refuse_country("the account is unreadable", &mut warnings);
        assert_eq!(null, SessionHeaderRaw::Unavailable);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(null.was_carried(), "a null header was still carried");
    }

    /// A header carrying only SOME composed leaves names only those.
    #[test]
    fn a_refusal_names_the_composed_leaves_the_header_carried_and_no_others() {
        // kills: `refuse_country` -- name every composed leaf unconditionally instead of the ones the half held, and an absent leaf is reported as refused.
        let mut raw = split_of(json!({"ideology": "fascism", "session": 3}));
        let mut warnings = Vec::new();
        raw.refuse_country("no account", &mut warnings);
        assert_eq!(
            warnings,
            vec![
                "session_header: no account".to_string(),
                "session_header.ideology: no account".to_string(),
            ]
        );
    }
}
