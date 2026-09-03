//! A country selection the viewer keeps between frames, and the only way to
//! get a NAME out of it.
//!
//! ## Why this is a type and not a scan
//!
//! Every consumer that takes a caller-supplied selector takes `NamedCountry`,
//! so the census is asked by the compiler at the call site. A selection kept
//! in app state had no such type: it was an owned `String` on a struct the app
//! carries between frames, and `UiRuntime::execute` published
//! `viewer.selected_country` into an XUI11 candidate that the closed-panel
//! branch then announced as available (lane UB, Codex round 50).
//!
//! Three rounds of scans followed. The last of them counted the READS -- 9
//! reader functions, 57 reads -- because a per-read rule was not reachable and
//! a per-function rule let a function that asks about one selection publish a
//! second one unasked (M4). Round 53's M5 walked past the counted rule with a
//! two-line ALIAS:
//!
//! ```text
//! let raw_selection = viewer.selected_country.as_deref();
//! let selection = raw_selection.and_then(|tag| frame.named_country(tag));
//! ...
//! country_tag: raw_selection.map(str::to_owned)   // the raw one is published
//! ```
//!
//! The read count is unchanged, the ask is still there, and the published
//! `country_tag` is a name no census admitted. UB recorded M5 as CONFIRMED,
//! OPEN and named the close: give the stored selection a TYPE the way the
//! parameter got one.
//!
//! This is that type. The TAG is private and no accessor hands it back on its
//! own: the only way to a name is [`StoredCountrySelection::named_in`], which
//! takes the frame and asks that hour's census. M5 has nothing left to alias
//! -- `as_deref` does not exist here, and neither does any other spelling that
//! yields the stored string.
//!
//! What the type still allows, because neither can publish a name:
//!
//! * COMPARISON. `is`/`matches` answer a question about a TAG the caller
//!   already holds and yield nothing. UB's own rule named a comparison as a
//!   legitimate use of a stored selection.
//! * KEYING. `PartialEq`/`Eq`/`Clone` make the selection itself the cache key,
//!   which is what M9 required: the key must be the RAW selection so a
//!   contended selection does not share a cache entry with "no selection". It
//!   is a key precisely because it is not a name.

use replay_core::{NamedCountry, WorldFrame};

/// A country TAG the viewer is holding between frames.
///
/// Holding one is not a claim that it names a country: only
/// [`Self::named_in`] can say that, and only for one frame at a time.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoredCountrySelection(Option<String>);

/// `Debug` BY HAND, and it prints no TAG.
///
/// A derived `Debug` is a door: `format!("{:?}", selection)` returns
/// `StoredCountrySelection(Some("GER"))` with no frame in sight, for an
/// ADMITTED selection as readily as a refused one, and a caller can strip the
/// wrapper back off and publish what is inside. That is M5's alias re-expressed
/// without touching the private field, and the accessor scan below cannot see
/// it because a derive generates no `pub fn` (Codex round 1, lane XE).
///
/// So this one says whether anything is selected and nothing else. A consumer
/// that needs the spelling for a message asks `refusal_detail_in`, which
/// answers only for a selection the frame has already refused.
impl std::fmt::Debug for StoredCountrySelection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(_) => formatter.write_str("StoredCountrySelection(<held>)"),
            None => formatter.write_str("StoredCountrySelection(none)"),
        }
    }
}

impl StoredCountrySelection {
    /// Nothing selected.
    pub fn none() -> Self {
        Self(None)
    }

    /// The TAG a click, a route or a restored session put here.
    ///
    /// Ungated ON PURPOSE, and it costs nothing: what arrives is what the user
    /// pointed at, and the type yields no name without a frame. The gate is on
    /// the way OUT.
    pub fn of(tag: impl Into<String>) -> Self {
        Self(Some(tag.into()))
    }

    /// The same, from an optional TAG.
    pub fn from_tag(tag: Option<impl Into<String>>) -> Self {
        Self(tag.map(Into::into))
    }

    /// Whether anything is selected. Says nothing about what.
    pub fn is_selected(&self) -> bool {
        self.0.is_some()
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.0.as_ref().is_none_or(|tag| tag.trim().is_empty())
    }

    /// Does the selection spell this TAG?
    ///
    /// A comparison, not a name: the caller already holds `tag` and learns one
    /// bit about the selection. Nothing renderable comes back.
    pub fn is(&self, tag: &str) -> bool {
        self.0.as_deref() == Some(tag)
    }

    /// The same against an optional TAG, so "nothing selected" and "no country
    /// here" compare as the same answer they always did.
    pub fn matches(&self, tag: Option<&str>) -> bool {
        self.0.as_deref() == tag
    }

    /// Does the selection spell this TAG, ignoring ASCII case?
    ///
    /// The same one bit as [`Self::is`], for the one caller that compares a
    /// stored anchor against a name the frame already admitted and has always
    /// done so case-insensitively (`design_catalog::resolve_observer_designer_view_state`).
    /// Folding it into `is` would silently loosen every other comparison.
    pub fn is_ignoring_ascii_case(&self, tag: &str) -> bool {
        self.0
            .as_deref()
            .is_some_and(|held| held.eq_ignore_ascii_case(tag))
    }

    /// THE ONE NAME-YIELDING ACCESSOR, and it takes the frame.
    ///
    /// `None` when nothing is selected AND when this hour's census says the
    /// stored TAG is reached by two ids -- the two cases a consumer must treat
    /// alike, because in both of them the selection names no country. The
    /// answer is a `NamedCountry`, so what comes out is already the type every
    /// selector-taking API in this crate demands.
    pub fn named_in<'a>(&'a self, frame: &'a WorldFrame) -> Option<NamedCountry<'a>> {
        frame.named_country(self.0.as_deref()?)
    }

    /// Why this frame yields no name, for a consumer that has to branch on it.
    /// It hands back no TAG.
    pub fn refusal_in(&self, frame: &WorldFrame) -> Option<StoredSelectionRefusal> {
        let tag = match self.0.as_deref() {
            None => return Some(StoredSelectionRefusal::NothingSelected),
            Some(tag) => tag,
        };
        if frame.country_key_is_contended(tag) {
            return Some(StoredSelectionRefusal::ContendedKey);
        }
        // AND NOTHING ELSE. A third cause was tried here -- "this hour's
        // roster does not carry the TAG" -- and it was UNREACHABLE, because
        // `WorldFrame::named_country` answers for every uncontended spelling
        // and never consults D18 (Codex round 2 finding 4). Making it reachable
        // means rebuilding the D18 roster inside a door that is read on the hot
        // per-frame UI paths, which is a cost this type must not add. The one
        // caller that needed it -- the point-of-view clear -- names that key
        // from the PICKER's own unnameable list, which is frame-derived, rather
        // than from the selection.
        None
    }

    /// The refusal AS A SENTENCE, for a diagnostic that has to name its cause.
    ///
    /// This is the only place the stored spelling comes back out, and three
    /// things keep it from being the alias M5 wrote:
    ///
    /// * it answers `None` for a selection this frame ADMITS, so the ordinary
    ///   case -- the one M5 published -- has no spelling to take;
    /// * what comes back is an owned sentence, not the TAG: a field that names
    ///   a country would have to hold the whole English clause;
    /// * `exactly_one_accessor_yields_a_name_and_it_takes_the_frame` counts
    ///   the surface of this type, so a third door is a failing test.
    ///
    /// "Degraded outcomes name the cause" is why it exists at all: a route
    /// that refuses a contended selection and says only that it has no country
    /// tells the caller nothing the missing field had not already said (lane
    /// UB, Codex round 51).
    pub fn refusal_detail_in(&self, frame: &WorldFrame) -> Option<String> {
        match self.refusal_in(frame)? {
            StoredSelectionRefusal::NothingSelected => {
                Some("no country is selected".to_string())
            }
            StoredSelectionRefusal::ContendedKey => {
                let spelling = self.0.as_deref().unwrap_or_default();
                Some(format!(
                    "{spelling:?} is a key this hour's census says more than one country \
                     wants, so it names none"
                ))
            }
        }
    }
}

/// Why a stored selection names no country this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredSelectionRefusal {
    /// Nothing is selected.
    NothingSelected,
    /// The census says two ids reach the stored TAG, so it names none.
    ContendedKey,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contended_selection_names_no_country_and_says_which_case_it_is() {
        // kills: `named_in` -- read `self.0` directly instead of routing it through `frame.named_country`, and a contended selection hands out a name.
        let contended = crate::test_identity::frame_from_json(serde_json::json!({
            "countries": { "ENG": { "_r2_tag_db_id": 12 } },
            "tag_db_id_index": {
                "country_composition_version": 2,
                "withheld_countries": {
                    "contended_keys": { "DUP": { "tag_db_ids": [441, 442] } }
                },
            },
        }));
        let selection = StoredCountrySelection::of("DUP");
        assert!(selection.is_selected(), "the selection is still held");
        assert!(selection.is("DUP"), "and a comparison still answers");
        assert!(
            selection.named_in(&contended).is_none(),
            "a key two ids reach names no country, so no consumer can be handed one"
        );
        assert_eq!(
            selection.refusal_in(&contended),
            Some(StoredSelectionRefusal::ContendedKey)
        );
        assert_eq!(
            StoredCountrySelection::none().refusal_in(&contended),
            Some(StoredSelectionRefusal::NothingSelected)
        );
        // The refusal NAMES ITS CAUSE, spelling included -- and only then.
        let detail = selection
            .refusal_detail_in(&contended)
            .expect("a refused selection carries its reason");
        assert!(
            detail.contains("DUP") && detail.contains("census"),
            "the refusal must name the key and why: {detail}"
        );
        let admitted = StoredCountrySelection::of("ENG");
        assert_eq!(
            admitted.named_in(&contended).map(|named| named.tag()),
            Some("ENG")
        );
        assert_eq!(admitted.refusal_in(&contended), None);
        // ...and an ADMITTED selection has no spelling to take out of the
        // refusal door, which is the whole of why M5's alias cannot be
        // written: the ordinary case it published is unreachable.
        assert_eq!(admitted.refusal_detail_in(&contended), None);
    }

    /// M9 (lane UB, round 53): the cache key must be the RAW selection, so a
    /// contended selection does not share a cache entry with "no selection".
    /// The selection IS the key -- that is what `PartialEq` is for here -- and
    /// keying cannot become publishing because no accessor spells it.
    #[test]
    fn a_contended_selection_is_not_the_same_cache_key_as_no_selection() {
        // kills: the `PartialEq` derive -- hand-write one comparing `is_selected()` only, and a contended selection shares M9's cache entry with "no selection".
        assert_ne!(
            StoredCountrySelection::of("DUP"),
            StoredCountrySelection::none()
        );
        assert_ne!(
            StoredCountrySelection::of("DUP"),
            StoredCountrySelection::of("GER")
        );
        assert_eq!(
            StoredCountrySelection::of("GER"),
            StoredCountrySelection::from_tag(Some("GER"))
        );
    }

    /// THE CLOSURE ITSELF: this module is the whole surface of the type, and
    /// exactly one of its methods yields a name -- the one that takes a frame.
    ///
    /// M5 aliased the raw selection past a counted-reads scan. It cannot be
    /// written against this type, because there is no method that returns the
    /// stored `String` or a `&str` borrowed from it. A new one would be a new
    /// door, so the doors are counted.
    #[test]
    fn exactly_one_accessor_yields_a_name_and_it_takes_the_frame() {
        // kills: add `pub fn tag(&self) -> Option<&str>` to the impl above, or put `Debug` back on the derive -- either is M5's alias re-opened.
        const SOURCE: &str = include_str!("stored_country_selection.rs");
        let production: Vec<&str> = SOURCE
            .lines()
            .take_while(|line| !line.trim_start().starts_with("mod tests"))
            .collect();
        // Every method this type exposes, with its return type.
        let signatures: Vec<&str> = production
            .iter()
            .map(|line| line.trim_start())
            .filter(|line| line.starts_with("pub fn "))
            .collect();
        // 11, up from 10: `is_ignoring_ascii_case` joined the COMPARISON
        // half when the designer's catalogue anchor became a selection rather
        // than a raw String -- that one comparison has always ignored ASCII
        // case, and folding it into `is` would have loosened every other.
        assert_eq!(
            signatures.len(),
            11,
            "the surface of the stored selection changed: {signatures:?}"
        );
        // A SPELLING is anything a caller can render or store as a country:
        // the stored `String`, a `&str` borrowed from it, or a `NamedCountry`.
        let yields_a_spelling: Vec<&&str> = signatures
            .iter()
            .filter(|signature| {
                let returns = signature.split("->").nth(1).unwrap_or_default();
                returns.contains("String")
                    || returns.contains("str")
                    || returns.contains("NamedCountry")
            })
            .collect();
        // EXACTLY TWO, and each one is named: the NAME, which this hour's
        // census has admitted, and the REFUSAL SENTENCE, which exists only
        // when it has not. Both take the frame, so neither can be reached
        // without asking it, and a third would be a new door.
        assert_eq!(
            yields_a_spelling.len(),
            2,
            "the doors that yield a spelling changed: {yields_a_spelling:?}"
        );
        for door in &yields_a_spelling {
            assert!(
                door.contains("frame: &WorldFrame") || door.contains("frame: &'a WorldFrame"),
                "a door that yields a spelling must take the frame: {door:?}"
            );
        }
        assert!(
            yields_a_spelling[0].contains("named_in")
                && yields_a_spelling[0].contains("<'a>(&'a self, frame: &'a WorldFrame)")
                && yields_a_spelling[0].contains("-> Option<NamedCountry<'a>>"),
            "the NAME door is `named_in` and it hands back a NamedCountry: {:?}",
            yields_a_spelling[0]
        );
        assert!(
            yields_a_spelling[1].contains("refusal_detail_in")
                && yields_a_spelling[1].contains("-> Option<String>"),
            "the REFUSAL door is `refusal_detail_in` and it hands back a sentence: {:?}",
            yields_a_spelling[1]
        );
        // ...and no `Deref`, `AsRef` or `Display` around the back. Read over
        // the PRODUCTION half only: this list is itself a string literal in
        // this file, and scanning the whole source would match itself.
        let production = production.join("\n");
        for back_door in [
            "impl std::ops::Deref for StoredCountrySelection",
            "impl AsRef<str> for StoredCountrySelection",
            "impl std::fmt::Display for StoredCountrySelection",
            "impl From<StoredCountrySelection>",
            "impl serde::Serialize for StoredCountrySelection",
        ] {
            assert!(
                !production.contains(back_door),
                "a trait impl hands the stored TAG back: {back_door}"
            );
        }
        // A DERIVE IS A DOOR TOO, and it generates no `pub fn` for the
        // inventory above to count. `Debug` derived on a newtype prints what is
        // inside it, which is the whole TAG, with no frame asked.
        let derive_line = production
            .lines()
            .find(|line| line.starts_with("#[derive(") && line.contains("Clone"))
            .expect("the type's derive line");
        // `Serialize` is on this list for the same reason `Debug` is, and it
        // is worse: `serde_json::to_string(&selection)` returns the bare TAG as
        // a JSON string with no frame in sight, and a derive generates no
        // `pub fn` for the inventory above to count (Codex round 2 finding 6).
        for generated_spelling in [
            "Debug",
            "Display",
            "Deref",
            "AsRef",
            "Into",
            "From",
            "Serialize",
            "Deserialize",
            "serde",
        ] {
            assert!(
                !derive_line.contains(generated_spelling),
                "a derive puts the stored TAG back within reach: {generated_spelling} in {derive_line}"
            );
        }
        // ...and the hand-written `Debug` prints no TAG.
        assert_eq!(
            format!("{:?}", StoredCountrySelection::of("GER")),
            "StoredCountrySelection(<held>)"
        );
        assert_eq!(
            format!("{:?}", StoredCountrySelection::none()),
            "StoredCountrySelection(none)"
        );
    }
}
