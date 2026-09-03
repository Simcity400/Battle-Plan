//! The country reference a replay row is IDENTIFIED by, and the only way to
//! build one.
//!
//! ## Why this is a type and not a scan
//!
//! A country name decoded as an identity without consulting the frame's
//! contended-key census fuses two countries that share a display TAG: the fold
//! spells every owner as its display TAG, so two D18 ids mapped to one TAG
//! arrive as two rows with the same `owner` string while the frame declares
//! that TAG contended and addressing nobody.
//!
//! Lane UB closed that class with a source scan over `world.rs`
//! (`every_country_reference_decode_is_gated_witnessed_or_named`) and ran 63
//! adversarial rounds against it. The scan works, and its own record names
//! what it cannot do: it is a TEXT scan whose windows are heuristics, it
//! classifies by SHAPE rather than by USE, and a decoder added in another
//! crate reading the raw frame is outside it. Its recorded conclusion was that
//! the strongest close was not taken -- a constructor-gated
//! `CountryIdentityRef { tag_db_id, tag }` that cannot be built from a bare
//! string, making an omission a type error rather than a test failure
//! (`ub_per_id_keying/class_closure.json`, `not_closed_by_this_gate`).
//!
//! This module is that constructor. There is exactly one expression in the
//! crate that builds a `CountryIdentityRef`, it is private to this file, and
//! the two ways to reach it both take the census and both answer `None` for a
//! key the census holds. A row field typed `CountryIdentityRef` therefore IS
//! the proof that the census was asked: reverting a gated decode to a plain
//! `leaf_string` no longer passes a scan and no longer compiles.
//!
//! `the_crate_builds_a_country_identity_ref_in_exactly_one_place` asserts the
//! single-constructor property against the crate's own sources, so the closure
//! cannot be widened by adding a second door.

use crate::world::ContendedKeys;

/// A country reference a frame's census admits as naming exactly one country.
///
/// `tag` is the display TAG. `tag_db_id` is the engine identity the ROW
/// decoded beside that name when the wire carries one -- the two halves of an
/// identity travelling as one value, so a consumer cannot take the name from
/// one row and the id from another. `None` states, in the type, that the wire
/// carries no id beside this particular name; it is not a missing measurement
/// and no consumer may read it as one.
///
/// There is no `From<String>`, no `new`, and no public field. The only way in
/// is `admitted` or `admitted_with_id`, and both consult the census.
#[derive(Clone, Debug)]
pub struct CountryIdentityRef {
    tag: String,
    tag_db_id: Option<u32>,
}

impl CountryIdentityRef {
    /// THE constructor. Private, and the only expression in the crate that
    /// names the struct's fields.
    fn admit(tag: &str, tag_db_id: Option<u32>, census: &impl ContendedKeys) -> Option<Self> {
        if census.holds(tag) {
            return None;
        }
        // ...AND THE TWO HALVES MUST BE THE SAME COUNTRY.
        //
        // Co-location is not identity. Until this check the constructor proved
        // only that a name was not contended and that an id had been decoded
        // BESIDE it, so a row spelling `GER` while carrying POL's id was
        // admitted as an identity -- and every consumer that selects by NAME
        // answered a question about GER with POL's measurement. No key is
        // contended in that frame, so nothing else in this lane could see it.
        //
        // Codex round 7 named it as one CLASS rather than another site, and it
        // is closed here rather than at each publication for the same reason
        // the census question is: this is the only expression in the crate that
        // builds the type, so a site cannot forget to ask.
        if let Some(id) = tag_db_id {
            if census.contradicts(tag, id) {
                return None;
            }
        }
        Some(Self {
            tag: tag.to_owned(),
            tag_db_id,
        })
    }

    /// WHY a pair was refused, for a caller that has to name the cause.
    ///
    /// `admit` answers `None` for two different facts now, and a caller written
    /// when `None` meant only contention reports the wrong one: a fold with no
    /// contended key at all was told that more than one country wanted `GER`
    /// (Codex round 8 finding 3). This is the same question in the same order,
    /// so the two cannot drift.
    pub(crate) fn refusal(
        tag: &str,
        tag_db_id: Option<u32>,
        census: &impl ContendedKeys,
    ) -> Option<IdentityRefusal> {
        if census.holds(tag) {
            return Some(IdentityRefusal::ContendedKey);
        }
        if tag_db_id.is_some_and(|id| census.contradicts(tag, id)) {
            return Some(IdentityRefusal::DisagreesWithTheRoster);
        }
        None
    }

    /// This TAG as an identity, or `None` when the census says two ids reach
    /// it. For the sites whose wire carries a name and no id beside it.
    pub(crate) fn admitted(tag: &str, census: &impl ContendedKeys) -> Option<Self> {
        Self::admit(tag, None, census)
    }

    /// The same, for a row that decoded the engine id for THIS name: the two
    /// halves are bound together here rather than sitting in two fields a
    /// consumer can pick apart.
    pub(crate) fn admitted_with_id(
        tag: &str,
        tag_db_id: u32,
        census: &impl ContendedKeys,
    ) -> Option<Self> {
        Self::admit(tag, Some(tag_db_id), census)
    }

    /// The TAG itself, for a map key, a comparison, a published field.
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// The engine identity this row decoded beside the name, when the wire
    /// carries one.
    pub fn tag_db_id(&self) -> Option<u32> {
        self.tag_db_id
    }

    /// The TAG, owned, for a consumer that has to store it.
    pub fn into_tag(self) -> String {
        self.tag
    }
}

/// Why this hour will not admit a `(name, id)` pair as an identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityRefusal {
    /// The census says more than one id reaches the name, so it names none.
    ContendedKey,
    /// The name and the id beside it belong to two different countries.
    DisagreesWithTheRoster,
}

impl IdentityRefusal {
    /// The cause AS A SENTENCE, for the warning a degraded outcome owes.
    pub fn detail(self, tag: &str, tag_db_id: Option<u32>) -> String {
        match self {
            Self::ContendedKey => format!(
                "{tag:?} is a contended key -- more than one country wants it this hour,                  so it identifies none"
            ),
            Self::DisagreesWithTheRoster => format!(
                "{tag:?} and the id beside it ({}) belong to two different countries this                  hour, so the pair identifies neither",
                tag_db_id.map_or_else(|| "none".to_string(), |id| id.to_string())
            ),
        }
    }
}

/// This leaf's admitted TAG, keeping the three-way answer.
///
/// DERIVED, and that is the whole rule for spelling a name out of one of
/// these: the census was asked where the identity was BUILT, so a consumer
/// that only needs the spelling -- a tooltip, a map key, a product it
/// serialises -- reads it here instead of deciding again. Nothing can turn
/// a bare string back into an identity, so the arrow only points this way.
pub trait CountryIdentityLeaf {
    fn tag(&self) -> crate::world::Leaf<&str>;
}

impl CountryIdentityLeaf for crate::world::Leaf<CountryIdentityRef> {
    fn tag(&self) -> crate::world::Leaf<&str> {
        match self {
            crate::world::Leaf::Absent => crate::world::Leaf::Absent,
            crate::world::Leaf::Unobtained => crate::world::Leaf::Unobtained,
            crate::world::Leaf::Measured(identity) => {
                crate::world::Leaf::Measured(identity.tag())
            }
        }
    }
}

/// The admitted TAGs of an identity ROSTER, keeping the three-way answer.
/// Same rule as `Leaf<CountryIdentityRef>::tag`, one list at a time.
pub trait CountryIdentityRosterLeaf {
    fn tags(&self) -> crate::world::Leaf<Vec<&str>>;
}

impl CountryIdentityRosterLeaf for crate::world::Leaf<Vec<CountryIdentityRef>> {
    fn tags(&self) -> crate::world::Leaf<Vec<&str>> {
        match self {
            crate::world::Leaf::Absent => crate::world::Leaf::Absent,
            crate::world::Leaf::Unobtained => crate::world::Leaf::Unobtained,
            crate::world::Leaf::Measured(members) => crate::world::Leaf::Measured(
                members.iter().map(CountryIdentityRef::tag).collect(),
            ),
        }
    }
}

/// EQUALITY IS THE NAME, and ordering and hashing agree with it.
///
/// A DERIVED `PartialEq` compares `(tag, id)`, and that turned two correct
/// checks into defects the moment their fields became typed: an outgoing
/// contract whose `counterpart` carries no id stopped matching its `buyer`
/// which carries one, and a GER|GER war edge whose two rows disagreed on the
/// id stopped reading as a self-edge (Codex round 2 finding 6; the round-1
/// fixes for both were `.tag()` calls a later edit could quietly undo).
///
/// Comparing the NAME is not a weakening. Within one frame an admitted TAG is
/// held by exactly one country -- that is what the census admission means --
/// so two admitted identities spelling the same TAG name the same country, and
/// their ids either agree or one row simply carried none. The id is a
/// PROVENANCE the name travels with, read through `tag_db_id()` where a join
/// needs it; it is not a second identity to disagree about.
impl PartialEq for CountryIdentityRef {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag
    }
}

impl Eq for CountryIdentityRef {}

impl PartialOrd for CountryIdentityRef {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CountryIdentityRef {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.tag.cmp(&other.tag)
    }
}

impl std::hash::Hash for CountryIdentityRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.tag.hash(state);
    }
}

impl AsRef<str> for CountryIdentityRef {
    fn as_ref(&self) -> &str {
        &self.tag
    }
}

impl std::ops::Deref for CountryIdentityRef {
    type Target = str;

    fn deref(&self) -> &str {
        &self.tag
    }
}

impl std::borrow::Borrow<str> for CountryIdentityRef {
    fn borrow(&self) -> &str {
        &self.tag
    }
}

impl std::fmt::Display for CountryIdentityRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.tag)
    }
}

impl PartialEq<str> for CountryIdentityRef {
    fn eq(&self, other: &str) -> bool {
        self.tag == other
    }
}

impl PartialEq<&str> for CountryIdentityRef {
    fn eq(&self, other: &&str) -> bool {
        self.tag == *other
    }
}

impl PartialEq<String> for CountryIdentityRef {
    fn eq(&self, other: &String) -> bool {
        &self.tag == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn census_of(contended: &[&str]) -> BTreeSet<String> {
        contended.iter().map(|tag| (*tag).to_string()).collect()
    }

    #[test]
    fn a_contended_key_yields_no_country_identity_ref() {
        // kills: `CountryIdentityRef::admit` -- delete its `if census.holds(tag) { return None; }` and a contended key is admitted as a country.
        let census = census_of(&["DUP"]);
        assert!(CountryIdentityRef::admitted("DUP", &census).is_none());
        assert!(CountryIdentityRef::admitted_with_id("DUP", 441, &census).is_none());
        assert_eq!(
            CountryIdentityRef::admitted("GER", &census).map(CountryIdentityRef::into_tag),
            Some("GER".to_string())
        );
    }

    #[test]
    fn an_admitted_ref_carries_the_id_the_row_decoded_and_says_so_when_it_did_not() {
        // kills: `CountryIdentityRef::admitted_with_id` -- forward `None` instead of `Some(tag_db_id)` to `admit` and the pair the row decoded comes apart.
        let census = census_of(&[]);
        let paired = CountryIdentityRef::admitted_with_id("GER", 7, &census).expect("admitted");
        assert_eq!((paired.tag(), paired.tag_db_id()), ("GER", Some(7)));
        let name_only = CountryIdentityRef::admitted("GER", &census).expect("admitted");
        assert_eq!((name_only.tag(), name_only.tag_db_id()), ("GER", None));
        // THE SAME COUNTRY, and equality says so, because equality on an
        // identity is the NAME. What separates these two is the PROVENANCE,
        // and provenance is read where a join needs it -- through
        // `tag_db_id()`, never through `==`.
        assert_eq!(paired, name_only, "both of them name GER");
        assert_ne!(
            paired.tag_db_id(),
            name_only.tag_db_id(),
            "and only one of them carries the id its row decoded"
        );
    }

    /// Comparing two identities compares the NAMES, so the two production
    /// checks that ask "same country?" cannot be wrong about it again.
    ///
    /// Codex round 2 finding 6: with a DERIVED `PartialEq` both mistakes stayed
    /// compilable, and the round-1 fixes for them were `.tag()` calls a later
    /// edit could quietly undo. They are now the same expression.
    #[test]
    fn comparing_two_identities_compares_the_countries_they_name() {
        // kills: replace the hand-written `impl PartialEq` with
        // `#[derive(PartialEq)]` on the struct -- an unpaired identity stops
        // equalling the paired one that names the same country, which is
        // exactly what broke the d43/d53 direction check and the war self-edge.
        let census = census_of(&[]);
        let paired = CountryIdentityRef::admitted_with_id("GER", 368, &census).expect("admitted");
        let other_id = CountryIdentityRef::admitted_with_id("GER", 441, &census).expect("admitted");
        let unpaired = CountryIdentityRef::admitted("GER", &census).expect("admitted");
        let different = CountryIdentityRef::admitted("ENG", &census).expect("admitted");

        assert_eq!(paired, unpaired, "one country, one name, one identity");
        assert_eq!(paired, other_id, "two rows disagreeing on an id is not two countries");
        assert_ne!(paired, different, "and a different name IS a different country");

        // Ordering and hashing agree with it, or a set of identities would
        // hold the same country twice.
        use std::collections::BTreeSet;
        let set: BTreeSet<CountryIdentityRef> =
            [paired.clone(), other_id, unpaired, different].into_iter().collect();
        assert_eq!(set.len(), 2, "GER once and ENG once: {set:?}");
        let mut hashed = std::collections::HashSet::new();
        hashed.insert(paired.clone());
        assert!(
            !hashed.insert(CountryIdentityRef::admitted("GER", &census).expect("admitted")),
            "Hash has to agree with Eq or a HashSet holds the country twice"
        );
    }

    /// A ROSTER that contradicts the pair, for the agreement test below.
    struct Roster {
        contended: BTreeSet<String>,
        by_tag: std::collections::BTreeMap<String, u32>,
        by_id: std::collections::BTreeMap<u32, String>,
    }

    impl Roster {
        fn of(entries: &[(&str, u32)], contended: &[&str]) -> Self {
            Self {
                contended: contended.iter().map(|tag| (*tag).to_string()).collect(),
                by_tag: entries
                    .iter()
                    .map(|(tag, id)| ((*tag).to_string(), *id))
                    .collect(),
                by_id: entries
                    .iter()
                    .map(|(tag, id)| (*id, (*tag).to_string()))
                    .collect(),
            }
        }
    }

    impl ContendedKeys for Roster {
        fn holds(&self, tag: &str) -> bool {
            self.contended.contains(tag)
        }
        fn contradicts(&self, tag: &str, id: u32) -> bool {
            self.by_tag.get(tag).is_some_and(|named| *named != id)
                || self.by_id.get(&id).is_some_and(|named| named.as_str() != tag)
        }
    }

    /// A NAME AND AN ID THAT BELONG TO TWO COUNTRIES is not an identity, and no
    /// key has to be contended for that to be true.
    ///
    /// Codex round 7: until this, the constructor proved the two halves were
    /// CO-LOCATED -- decoded beside each other on one row -- and nothing more.
    /// Every consumer that selects by NAME then answered a question about one
    /// country with another country's measurement, on a frame where the census
    /// had nothing to say.
    #[test]
    fn a_name_and_an_id_that_belong_to_two_countries_are_not_an_identity() {
        // kills: delete the `census.contradicts(tag, id)` arm in `admit` --
        // every forged pair below is admitted, and the D61, D53 intel and D53
        // graph publications carry it straight to a consumer.
        let roster = Roster::of(&[("GER", 368), ("POL", 441), ("ENG", 12)], &[]);

        // THE CONTROL: the pair the roster actually names.
        let honest = CountryIdentityRef::admitted_with_id("GER", 368, &roster)
            .expect("the roster names GER 368");
        assert_eq!((honest.tag(), honest.tag_db_id()), ("GER", Some(368)));

        // GER carrying POL's id: contradicted from both directions.
        assert!(CountryIdentityRef::admitted_with_id("GER", 441, &roster).is_none());
        // A name the roster does not carry, on an id that belongs to POL.
        assert!(CountryIdentityRef::admitted_with_id("FRA", 441, &roster).is_none());

        // UNVERIFIABLE IS NOT CONTRADICTED. A name the roster does not carry,
        // on an id it does not carry either, is what an incomplete D18 read
        // looks like -- refusing it would refuse every such hour.
        assert!(CountryIdentityRef::admitted_with_id("FRA", 900, &roster).is_some());
        // ...and a name with no id beside it has nothing to disagree with.
        assert!(CountryIdentityRef::admitted("FRA", &roster).is_some());

        // A census with no roster at all authenticates nothing and contradicts
        // nothing, which is why the trait default is false.
        let no_roster = census_of(&[]);
        assert!(CountryIdentityRef::admitted_with_id("GER", 441, &no_roster).is_some());
    }

    /// The closure itself: ONE expression in the crate builds this struct.
    ///
    /// A second constructor -- a `pub fn new`, a `From<String>`, a test helper
    /// that names the fields -- would let a caller mint an identity the census
    /// never saw, and every field typed `CountryIdentityRef` would stop being
    /// proof of anything. The scan is over the whole crate's sources, so a door
    /// opened in another module fails here too.
    #[test]
    fn the_crate_builds_a_country_identity_ref_in_exactly_one_place() {
        // kills: add one more `Self { tag: ..., tag_db_id: ... }` anywhere in this file above `mod tests` -- a second constructor is a second gate to keep.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        let mut sites: Vec<String> = Vec::new();
        let mut files = 0usize;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("a source directory").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                    continue;
                }
                files += 1;
                let source = std::fs::read_to_string(&path).expect("source");
                let file = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string();
                // The struct-literal spellings: by name from anywhere, and as
                // `Self` inside this file's own impl block. A comment cannot
                // supply either -- the line is cut at `//` first.
                // PRODUCTION ONLY, like the crate's other source scans: a test
                // that names this type in an assertion message builds nothing.
                let lines: Vec<&str> = source.lines().collect();
                let production = lines
                    .iter()
                    .position(|line| line.trim_start().starts_with("mod tests"))
                    .unwrap_or(lines.len());
                for (index, line) in lines[..production].iter().enumerate() {
                    let code = line.split("//").next().unwrap_or_default();
                    let head = code.trim_start();
                    // A DECLARATION and an IMPL HEADER spell the name followed
                    // by a brace and construct nothing. They are the only other
                    // things that can, so they are the only exceptions.
                    if head.starts_with("struct ")
                        || head.starts_with("pub struct ")
                        || head.starts_with("pub(crate) struct ")
                        || head.starts_with("impl ")
                    {
                        continue;
                    }
                    let literal = code.contains("CountryIdentityRef {")
                        || (file == "country_identity.rs" && code.contains("Some(Self {"));
                    if literal {
                        sites.push(format!("{file}:{}", index + 1));
                    }
                }
            }
        }
        assert!(files > 20, "this scan read {files} files");
        let expected = include_str!("country_identity.rs")
            .lines()
            .position(|line| line.trim() == "Some(Self {")
            .expect("the one constructor")
            + 1;
        assert_eq!(
            sites,
            vec![format!("country_identity.rs:{expected}")],
            "a country identity may be built in exactly one place, and that \
             place takes the census"
        );

        // A DERIVE BUILDS ONE TOO, and it writes no `Self {` for the scan
        // above to find. `Deserialize` is the door: an external caller feeds
        // it `{"tag":"DUP","tag_db_id":441}` and gets an identity no census
        // ever saw (Codex round 2 finding 6).
        let own = include_str!("country_identity.rs");
        let own_production = &own[..own.find("mod tests").unwrap_or(own.len())];
        let derive_line = own_production
            .lines()
            .find(|line| line.starts_with("#[derive(") && line.contains("Clone"))
            .expect("the type's derive line");
        for generated_constructor in ["Deserialize", "Default", "serde"] {
            assert!(
                !derive_line.contains(generated_constructor),
                "a derive mints identities the census never saw: {generated_constructor} in {derive_line}"
            );
        }
        for hand_written_door in [
            "impl Default for CountryIdentityRef",
            "impl From<String> for CountryIdentityRef",
            "impl From<&str> for CountryIdentityRef",
            "impl std::str::FromStr for CountryIdentityRef",
            "impl<'de> serde::Deserialize<'de> for CountryIdentityRef",
        ] {
            assert!(
                !own_production.contains(hand_written_door),
                "a trait impl mints an identity with no census: {hand_written_door}"
            );
        }
    }

    /// XC-R1, lane XC's residual, class other-lane: the country-reference
    /// SOURCE SCAN acquits a GENERIC decoder as `T`, because Rust infers the
    /// concrete types at the call sites and no scan of the text can enumerate
    /// them. Closing it by scanning would need a real parse or a caller
    /// registry.
    ///
    /// The type closes it instead, and this is the argument as a test rather
    /// than as a sentence. A generic decoder can only produce a `T` it can
    /// BUILD, so it carries a bound -- `Default`, `From<&str>`, `FromStr`,
    /// `Deserialize` -- and `CountryIdentityRef` satisfies none of them. There
    /// is nothing to enumerate: whatever `T` a caller picks, a decoder that
    /// never took the census cannot instantiate at this type.
    #[test]
    fn a_generic_decoder_cannot_be_instantiated_at_a_country_identity() {
        // kills: add `Default` (or `Deserialize`) to the struct's derive, or
        // write `impl From<&str> for CountryIdentityRef` -- either one lets a
        // generic decoder mint an identity with no census, and the probes below
        // flip to true.
        use std::marker::PhantomData;

        struct Probe<T>(PhantomData<T>);

        // THE FALLBACK ANSWER. It applies to every `T`, and loses to the
        // inherent impl below wherever that one's bound is satisfied.
        trait Buildable {
            const FROM_NOTHING: bool = false;
            const FROM_A_STRING: bool = false;
        }
        impl<T> Buildable for Probe<T> {}

        impl<T: Default> Probe<T> {
            const FROM_NOTHING: bool = true;
        }
        impl<T: for<'a> From<&'a str>> Probe<T> {
            const FROM_A_STRING: bool = true;
        }

        // The probes have to be able to say YES, or they say nothing.
        assert!(
            <Probe<String>>::FROM_NOTHING,
            "String is Default, so this probe can answer yes"
        );
        assert!(
            <Probe<String>>::FROM_A_STRING,
            "String is From<&str>, so this probe can answer yes"
        );

        // ...and they say no for the identity, at COMPILE TIME, for every
        // generic decoder in the crate at once.
        assert!(
            !<Probe<CountryIdentityRef>>::FROM_NOTHING,
            "a generic decoder could mint an identity out of nothing"
        );
        assert!(
            !<Probe<CountryIdentityRef>>::FROM_A_STRING,
            "a generic decoder could mint an identity from a bare TAG -- which \
             is exactly the decode XC-R1 says no source scan can find"
        );

        // The census is the only way in, and it is not a bound a generic
        // decoder can satisfy by accident: it takes a `&impl ContendedKeys`
        // argument the decoder would have to be handed.
        let census = census_of(&["DUP"]);
        assert!(CountryIdentityRef::admitted("DUP", &census).is_none());
        assert!(CountryIdentityRef::admitted("GER", &census).is_some());
    }
}
