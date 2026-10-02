//! Scripted localisation (`[ROOT.GetName]`) evaluated over one recorded hour.
//!
//! CTextBase::ProcessString (1412B9CA0) expands every `[` .. first `]` of a
//! text in a CContextLocalizationText. Each bracket starts from the saved
//! root context (RestoreContext 1410D0900). `[(...)]` is the conditional
//! form (1412B98C0) and `[?...]` the variable form (1412BA1F0); anything else
//! splits at every `.`: each segment but the last promotes the scope
//! (PromoteScope 1410CF710), the last is a property (GetPropertyText
//! 1410CD900), whose text is processed again one level deeper, up to
//! `NGame.MAX_SCRIPTED_LOC_RECURSION` (at the limit the text is kept as it
//! is). A `[` with no `]` after it ends the text, dropping it from the last
//! bracket on.
//!
//! The views this serves (idea names 140FBCF90 -> 1410CF560, idea
//! descriptions 140FBAE40, focus names 1402D2250, the stability breakdown
//! 1410CF390) build a fresh CEventScope with only the country set
//! (default_construct 140528800: ROOT, FROM and PREV are itself), so the
//! root context is the event scope (kind 1) of that country.
//!
//! Names match without case at their exact length (`_strnicmp`, 142549958);
//! country tags and scripted localisation names match exactly. What the
//! recording cannot answer is refused by name with the missing input.

use crate::scope::RecordedScopes;
use installed_content::{
    country_names::CountryNames,
    localisation::{Localisation, crt_case_text},
};
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{FactionMemberRow, TableSchema, tables},
};

mod balance;
mod entries;
pub(crate) use entries::{StateNames, read_state_content, state_name_from};
mod properties;
mod random;

use properties::Party;

/// A context object of CContextLocalizationText.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Object {
    /// Kind 1: the root event scope, whose country is the view's.
    Event,
    /// Kind 3: a country by canonical slot.
    Country(u32),
    /// Kind 4: a state, `None` for a null state object.
    State(Option<u32>),
    /// Kind 15: an explicit province text object, outside CEventScope.
    Province(Option<u32>),
    /// Kind 16: the province template's terrain ordinal, zero for null.
    Terrain(i32),
    /// Kind 8: a character by id.
    Character(u32),
}

/// A CEventScope link: ROOT (+0x18), FROM (+0x20) or PREV (+0x28).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// The scope itself, as a default-constructed scope links (140528800).
    Itself,
    /// Another scope of the same [`EventScopes`], by index.
    Scope(usize),
    /// A scope the caller cannot describe; promoting through it is refused.
    Unknown,
}

/// What a CEventScope holds, as text reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventObject {
    Country(u32),
    /// A state by id; SetState (14052ECE0) clears the scope's country.
    State(u32),
    Province(u32),
    /// A unit leader by full reference: set_unit_leader (14052EFD0) stores
    /// its character (CUnitLeader+0xF48) at +0x50 and clears the country.
    UnitLeader(u64),
    /// A character by id; SetCharacter (14052DAF0) clears the country.
    Character(u32),
}

/// One CEventScope as text reads it: its object and its links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventScope {
    pub this: EventObject,
    pub root: Link,
    pub from: Link,
    pub prev: Link,
    /// CEventScope+0x0C, used by an unseeded random_list (140A9CD70).
    pub random_seed: u32,
}

impl EventScope {
    /// A default-constructed scope (140528800) set to `this`: its ROOT, FROM
    /// and PREV are itself.
    pub fn of(this: EventObject) -> Self {
        Self {
            this,
            root: Link::Itself,
            from: Link::Itself,
            prev: Link::Itself,
            random_seed: 1, // 140528800; setters preserve the seed words.
        }
    }
}

/// The event scopes a text is expanded in. The first is the caller's
/// (CContextLocalizationText+0x378), which RestoreContext (1410D0900) makes
/// active again at every bracket; ROOT, FROM and PREV promotions make the
/// linked scope active (141A81250), so later segments follow its links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventScopes {
    pub scopes: Vec<EventScope>,
    /// The thread-local script variables set while the text is expanded
    /// (InitializeRaidEventScope 141580600 sets these three).
    pub variables: ScopeVariables,
}

/// Thread-local script variables a caller has set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScopeVariables {
    pub target_state: Option<u32>,
    pub target_province: Option<u32>,
    pub target_leader: Option<u64>,
}

impl EventScopes {
    /// `scopes`, with no script variables set.
    /// Fresh scopes (140528800); description completion clears temporary
    /// globals (1401D20F0). Event-chain consumers need their own target arrays.
    pub fn new(scopes: Vec<EventScope>) -> Self {
        Self {
            scopes,
            variables: ScopeVariables::default(),
        }
    }

    /// The index of the scope that `link` of scope `at` names.
    fn linked(&self, at: usize, link: Link, name: &str) -> Result<usize, String> {
        match link {
            Link::Itself => Ok(at),
            Link::Scope(to) if to < self.scopes.len() => Ok(to),
            Link::Scope(to) => Err(format!("{name}: event scope {to} is not given")),
            Link::Unknown => Err(format!(
                "{name}: the event scope's {name} link is not known here"
            )),
        }
    }
}

/// Content facts the evaluator needs besides localisation.
pub struct ScriptedContent {
    /// `common/scripted_localisation` names ([`installed_content::scripted_localisation`]).
    pub names: installed_content::scripted_localisation::Names,
    /// `NGame.MAX_SCRIPTED_LOC_RECURSION` (1412B9CA0 compares the depth).
    pub recursion: i32,
    /// `common/country_tag_aliases` names (the tag alias database
    /// 143316AA8), which promotion step 3 tries after the country tags.
    pub tag_aliases: std::sync::Arc<installed_content::tag_aliases::TagAliases>,
    /// The recording client's game language ([`crate::catalogue::Catalogue`]):
    /// text the game stored already localised is in it.
    pub recorded_language: String,
    /// The content stack the triggers of scripted localisation entries and
    /// state names read ([`entries`]).
    pub layers: std::sync::Arc<installed_content::content::ResolvedContent>,
    pub(crate) environment: std::sync::OnceLock<Result<entries::Environment, String>>,
}

impl ScriptedContent {
    /// The scripted localisation names, with the files the port cannot read
    /// to their end, `NGame.MAX_SCRIPTED_LOC_RECURSION` and the tag aliases
    /// of `content`, or why they cannot be read. `recorded_language` is the
    /// recording client's language, without its `l_`.
    pub fn read(
        content: std::sync::Arc<installed_content::content::ResolvedContent>,
        recorded_language: String,
    ) -> Result<Self, String> {
        let layers = &content.layers;
        let names = installed_content::scripted_localisation::names(layers)
            .map_err(|error| format!("common/scripted_localisation is unreadable: {error}"))?;
        let recursion = installed_content::defines::read(layers)
            .and_then(|defines| defines.int32("NGame", "MAX_SCRIPTED_LOC_RECURSION"))
            .map_err(|error| format!("NGame.MAX_SCRIPTED_LOC_RECURSION is unavailable: {error}"))?;
        let tag_aliases = installed_content::tag_aliases::read(layers)
            .map_err(|error| format!("common/country_tag_aliases is unreadable: {error}"))?;
        Ok(Self {
            names,
            recursion,
            tag_aliases: std::sync::Arc::new(tag_aliases),
            recorded_language,
            layers: content,
            environment: std::sync::OnceLock::new(),
        })
    }
}

/// Text the game stored already localised, or as a literal, without its key:
/// `what` names the field. The game copies these bytes, so they are exact for
/// a viewer in the recording client's language (`recorded`, without its
/// `l_`). A game in another language would have stored its own translation of
/// a key that was not kept, so there they are refused rather than shown in the
/// recording's language; empty text is the same in every language.
pub fn stored_text(
    localisation: &Localisation,
    recorded: &str,
    bytes: &[u8],
    what: &str,
) -> Result<String, String> {
    if !bytes.is_empty() && recorded != localisation.language {
        return Err(format!(
            "{what} is text the game stored in {recorded} without its key, and the viewer's language is {}",
            localisation.language
        ));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{what} is not UTF-8"))
}

/// One hour's scripted text context for a country.
pub struct ScriptedText<'a> {
    localisation: &'a Localisation,
    content: &'a ScriptedContent,
    scopes: RecordedScopes,
    names: CountryNames<'a>,
    frame: &'a DecodedFrame,
    root: u32,
    world: std::cell::OnceCell<Result<crate::triggers::World<'a>, String>>,
    /// Entries whose text is being expanded in their own scope, outermost
    /// first (1410CF560 restarts at depth 0).
    nested: std::cell::Cell<i32>,
}

/// `_strnicmp` at the exact length (142549958).
fn named(segment: &str, name: &str) -> bool {
    segment.eq_ignore_ascii_case(name)
}

/// Whether the frame holds the whole table.
fn complete<T: TableSchema>(frame: &DecodedFrame) -> bool {
    matches!(&*frame.tables[T::ID.index()], TableView::Complete(_))
}

fn rows<T: TableSchema>(frame: &DecodedFrame) -> (&[T::Row], String) {
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => (
            rows.rows::<T>().unwrap_or_default(),
            "the complete table has no row for it".into(),
        ),
        TableView::Truncated { rows, missing } => (
            rows.rows::<T>().unwrap_or_default(),
            format!("the table is truncated ({missing} rows missing)"),
        ),
        TableView::Unavailable { reason } => (&[], format!("the table is unavailable: {reason}")),
    }
}

impl<'a> ScriptedText<'a> {
    /// The context of `root` (a canonical country slot) at `frame`'s hour,
    /// the original-tag lists read from this frame alone
    /// ([`RecordedScopes::read`]).
    pub fn new(
        frame: &'a DecodedFrame,
        localisation: &'a Localisation,
        content: &'a ScriptedContent,
        root: u32,
    ) -> Self {
        Self::with_scopes(
            frame,
            localisation,
            content,
            root,
            RecordedScopes::read(frame, std::sync::Arc::clone(&content.tag_aliases)),
        )
    }

    /// [`Self::new`] with the hour's scopes already read, for example with
    /// the original-tag lists derived from the branch
    /// ([`RecordedScopes::read_with`]).
    pub fn with_scopes(
        frame: &'a DecodedFrame,
        localisation: &'a Localisation,
        content: &'a ScriptedContent,
        root: u32,
        scopes: RecordedScopes,
    ) -> Self {
        Self {
            localisation,
            content,
            scopes,
            names: CountryNames::read(frame, localisation),
            frame,
            root,
            world: std::cell::OnceCell::new(),
            nested: std::cell::Cell::new(0),
        }
    }

    /// Expands `text` as ProcessString (1412B9CA0) does from depth 0. The
    /// view's event scope is its country, as ROOT, THIS, FROM and PREV
    /// (140528800).
    pub fn expand(&self, text: &str) -> Result<String, String> {
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(self.root))]);
        self.expand_in(text, &scopes)
    }

    /// SScopeLocalizer (1402F5F40): expand in the caller's event scopes, the
    /// first being the caller's own (raid triggers, 1413931A0).
    pub fn expand_in(&self, text: &str, scopes: &EventScopes) -> Result<String, String> {
        if scopes.scopes.is_empty() {
            return Err("scripted text needs an event scope".into());
        }
        self.process(text, 0, scopes, 0, Object::Event)
    }

    /// An explicit province text context (kind 15, 1410CCE20), with the
    /// caller's event scope for event promotions. Provinces are not event
    /// scope targets (141A81250), and cannot bind scripted entries (1410CD900).
    pub fn expand_province_in(
        &self,
        text: &str,
        province: u32,
        scopes: &EventScopes,
    ) -> Result<String, String> {
        if scopes.scopes.is_empty() {
            return Err("scripted text needs an event scope".into());
        }
        let (provinces, gap) = rows::<tables::Provinces>(self.frame);
        let found = provinces.iter().any(|row| row.id == province);
        if !found && !complete::<tables::Provinces>(self.frame) {
            return Err(format!("province {province}: Provinces {gap}"));
        }
        self.process(
            text,
            0,
            scopes,
            0,
            Object::Province(found.then_some(province)),
        )
    }

    /// ProcessString with the context's saved event scope `start`.
    fn process(
        &self,
        text: &str,
        depth: i32,
        scopes: &EventScopes,
        start: usize,
        context: Object,
    ) -> Result<String, String> {
        if depth >= self.content.recursion {
            return Ok(text.to_owned());
        }
        let mut out = String::new();
        let mut rest = text;
        loop {
            let Some(open) = rest.find('[') else {
                out.push_str(rest);
                return Ok(out);
            };
            // No `]` after it: 1412B9E55 leaves before appending the text
            // before the `[`.
            let Some(length) = rest[open + 1..].find(']') else {
                return Ok(out);
            };
            let inner = &rest[open + 1..open + 1 + length];
            out.push_str(&rest[..open]);
            if inner.len() >= 2 && inner.starts_with('(') && inner.ends_with(')') {
                return Err(format!(
                    "the conditional scripted text [{inner}] (1412B98C0) is not evaluated"
                ));
            }
            if let Some(variable) = inner.strip_prefix('?') {
                return Err(format!(
                    "[?{variable}]: the variable form's value formatting (1412BA1F0) is not ported"
                ));
            }
            let value = self.bracket(inner, scopes, start, context)?;
            out.push_str(&self.process(&value, depth + 1, scopes, start, context)?);
            rest = &rest[open + 1 + length + 1..];
        }
    }

    /// One `[A.B.Property]`, from the context's saved event scope `start`
    /// (RestoreContext 1410D0900 before each bracket).
    fn bracket(
        &self,
        inner: &str,
        scopes: &EventScopes,
        start: usize,
        context: Object,
    ) -> Result<String, String> {
        let segments: Vec<&str> = inner.split('.').collect();
        let (property, promotions) = segments.split_last().unwrap_or((&"", &[]));
        let mut object = context;
        let mut active = start;
        for segment in promotions {
            object = self
                .promote(object, segment, scopes, &mut active)
                .map_err(|reason| format!("[{inner}]: {reason}"))?;
        }
        self.property(object, property, scopes, active)
            .map_err(|reason| format!("[{inner}]: {reason}"))
    }

    /// PromoteScope (1410CF710) for one segment; `active` is the event scope
    /// CContextLocalizationText+0x378 holds.
    fn promote(
        &self,
        object: Object,
        segment: &str,
        scopes: &EventScopes,
        active: &mut usize,
    ) -> Result<Object, String> {
        // 1. The current kind's table.
        match object {
            Object::Country(slot) => {
                if named(segment, "Capital") || named(segment, "OriginalCapital") {
                    return self.capital(slot, named(segment, "OriginalCapital"));
                }
                if named(segment, "Overlord") {
                    // No overlord: logged, the context is unchanged (141A7DC60).
                    return Ok(self.overlord(slot)?.map_or(object, Object::Country));
                }
                if named(segment, "FactionLeader") {
                    return Ok(self.faction_leader(slot)?.map_or(object, Object::Country));
                }
            }
            Object::State(state) => {
                if named(segment, "Owner") || named(segment, "Controller") {
                    let Some(id) = state else {
                        // The handler runs on a null object: nothing changes.
                        return Ok(object);
                    };
                    return self.state_country(id, named(segment, "Owner"));
                }
                if named(segment, "Occupied") {
                    return Err(
                        "Occupied: a state's occupier promotion (141A85580) is not ported".into(),
                    );
                }
            }
            Object::Character(id) => {
                if named(segment, "Owner") {
                    return self.character_country(id);
                }
            }
            Object::Province(province) => {
                // Province member table/dispatcher 141A83DC0 / 141A83F00:
                // Terrain uses the template +0xA8; Controller uses +0x188.
                if named(segment, "Terrain") || named(segment, "Controller") {
                    let Some(id) = province else {
                        return Ok(object);
                    };
                    return if named(segment, "Terrain") {
                        self.province_terrain(id).map(Object::Terrain)
                    } else {
                        let (provinces, gap) = rows::<tables::Provinces>(self.frame);
                        let row = provinces
                            .iter()
                            .find(|row| row.id == id)
                            .ok_or_else(|| format!("province {id}: Provinces {gap}"))?;
                        self.scopes
                            .slot(row.controller)
                            .map(Object::Country)
                            .ok_or_else(|| {
                                format!(
                                    "province {id}'s controller link {} resolves to no country",
                                    row.controller
                                )
                            })
                    };
                }
            }
            Object::Event | Object::Terrain(_) => {}
        }
        // 1 in kind 1, and 2 from every kind: the event-scope table.
        if named(segment, "Player") {
            return self.local_player().map(Object::Country);
        }
        // 141A81250 on the active scope: THIS keeps it; ROOT, FROM and PREV
        // follow its +0x18, +0x20 and +0x28. The chosen scope becomes the
        // active one (+0x378) and its object the context.
        let current = scopes.scopes[*active];
        for (name, link) in [
            ("THIS", Link::Itself),
            ("Root", current.root),
            ("From", current.from),
            ("Prev", current.prev),
        ] {
            if named(segment, name) {
                let chosen = scopes.linked(*active, link, name)?;
                *active = chosen;
                return self.event_object(scopes.scopes[chosen].this, name);
            }
        }
        // 3. A country tag (exact), a character of the current country, a
        //    state id (1410CF9C0). Each is found, proven absent, or refused.
        if self.scopes.country(segment).is_some() {
            // ResolveScriptCountryTag (140BA5D10), as a script scope.
            return self.scopes.resolve_tag(segment).map(Object::Country);
        }
        // Then the tag alias database (143316AA8): AliasResolve (1407141F0)
        // in the scope 1410CF9C0 builds for ResolveScriptCountryTag
        // (140BA5D10): a default scope (140528800, its ROOT, FROM and PREV
        // itself) set with clearing to the local player's country.
        if self.scopes.alias_entry(segment).is_some() {
            let local = self
                .local_player()
                .map_err(|why| format!("tag alias {segment}: {why}"))?;
            let root = crate::triggers::Scope::Country(local);
            let scopes = crate::triggers::Scopes {
                prev: Some(root),
                from: Some(root),
                bare: true,
                ..crate::triggers::Scopes::of(root)
            };
            return self
                .world()
                .and_then(|world| world.resolve_tag(segment, scopes))
                .map_err(|why| format!("tag alias {segment}: {why}"))?
                .map(Object::Country)
                .ok_or_else(|| format!("tag alias {segment} names no country this hour"));
        }
        if let Some(refusal) = self.scopes.alias(segment) {
            return Err(refusal);
        }
        if !complete::<tables::Countries>(self.frame) {
            let (_, gap) = rows::<tables::Countries>(self.frame);
            return Err(format!(
                "{segment}: whether it is a country tag (140BA53F0) is unknown: Countries {gap}"
            ));
        }
        if let Some(id) = self.character_named(object, segment, scopes.scopes[*active].this)? {
            return Ok(Object::Character(id));
        }
        let digits = segment.strip_prefix(['-', '+']).unwrap_or(segment);
        if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
            let id = installed_content::fixed5::decimal_i32(segment.as_bytes());
            if id <= 0 {
                // Consumed, the context unchanged.
                return Ok(object);
            }
            let (states, gap) = rows::<tables::States>(self.frame);
            if states.iter().any(|state| state.id == id as u32) {
                return Ok(Object::State(Some(id as u32)));
            }
            if complete::<tables::States>(self.frame) {
                // A positive id no state holds is a null state object.
                return Ok(Object::State(None));
            }
            return Err(format!("state {segment} is unknown: States {gap}"));
        }
        // 4. Kind 1 only: saved event targets. 1410CFF00 has its own object
        // selector (including text kinds outside this trigger port).
        if object == Object::Event {
            return self.saved_promotion(segment);
        }
        // 5. The kind-2 names, on the current object.
        if named(segment, "Country") && matches!(object, Object::Country(_)) {
            return Ok(object);
        }
        // 6. Unknown: logged, the context unchanged.
        Ok(object)
    }

    /// 1410CFF00: local tooltip/saved, global tooltip/saved; a missing or
    /// invalid target returns false and PromoteScope leaves its object alone.
    fn saved_promotion(&self, name: &str) -> Result<Object, String> {
        use crate::triggers::saved_event_targets::{Reader, Selected};
        let selected = self.scopes.saved_selection(name.as_bytes(), Reader::Text)?;
        match selected {
            Selected::Missing | Selected::Caller => Ok(Object::Event),
            Selected::State(id) => {
                if id <= 0 {
                    return Ok(Object::State(None));
                }
                let (states, gap) = rows::<tables::States>(self.frame);
                let found = states.iter().any(|r| r.id == id as u32);
                if !found && !complete::<tables::States>(self.frame) {
                    return Err(format!(
                        "saved event target {name}: state {id} is unknown: States {gap}"
                    ));
                }
                Ok(Object::State(found.then_some(id as u32)))
            }
            Selected::Country(raw) => self
                .scopes
                .saved_country(raw, name.as_bytes())?
                .map(Object::Country)
                .ok_or_else(|| {
                    format!("saved event target {name}: country {raw} has no recorded object")
                }),
            Selected::Character(id) => Ok(Object::Character(id)),
            Selected::Leader(_) => Err(Reader::Text.refusal(name.as_bytes(), "unit leader")),
            Selected::Unsupported(kind) => Err(Reader::Text.refusal(name.as_bytes(), kind)),
        }
    }

    /// GetPropertyText (1410CD900) in the promoted context.
    fn property(
        &self,
        object: Object,
        name: &str,
        scopes: &EventScopes,
        active: usize,
    ) -> Result<String, String> {
        let property = match object {
            Object::Event => self.date_property(name)?,
            Object::Country(slot) => self.country_property(slot, name)?,
            Object::State(state) => {
                const STATE: [&str; 4] = [
                    "GetName",
                    "GetID",
                    "GetCapitalVictoryPointName",
                    "GetContinent",
                ];
                match (STATE.iter().find(|entry| named(name, entry)), state) {
                    (Some(_), None) => Some("NULL".to_owned()),
                    (Some(&"GetID"), Some(id)) => Some(id.to_string()),
                    (Some(&"GetName"), Some(id)) => Some(self.state_name(id)?),
                    (Some(entry), Some(id)) => {
                        return Err(format!(
                            "{entry} of state {id}: the native victory-point/continent text selector (141A85400) is not ported"
                        ));
                    }
                    (None, _) => None,
                }
            }
            Object::Character(id) => self.character_property(id, name)?,
            Object::Province(province) => {
                // 141A83EF0 registers only GetName; 141A83DD0 passes 0,
                // disabling the state-name fallback of 140E67930.
                return if named(name, "GetName") {
                    province.map_or_else(|| Ok("NULL".into()), |id| self.province_name(id))
                } else {
                    Ok(String::new()) // kind 15 has no entry fallback (1410CD900).
                };
            }
            Object::Terrain(ordinal) => {
                return if named(name, "GetName") {
                    self.terrain_name(ordinal)
                } else {
                    Ok(String::new()) // kind 16 has no entry fallback (1410CD900).
                };
            }
        };
        let Some(text) = property else {
            // A miss: a scripted localisation of that name (1410CE090), else
            // empty text.
            return self.entry(object, name, scopes, active);
        };
        Ok(text)
    }

    fn localise(&self, key: &str) -> Result<String, String> {
        self.localisation
            .localise(key.as_bytes())
            .map_err(|refusal| refusal.to_string())
    }

    /// [`stored_text`] in the recording client's language.
    fn stored_text(&self, bytes: &[u8], what: &str) -> Result<String, String> {
        stored_text(
            self.localisation,
            &self.content.recorded_language,
            bytes,
            what,
        )
    }

    /// The country table's properties (dispatcher 141A7B350), `None` for a
    /// name the table lacks.
    fn country_property(&self, slot: u32, name: &str) -> Result<Option<String>, String> {
        let tag = self
            .scopes
            .tag(slot)
            .ok_or_else(|| format!("country slot {slot} has no tag"))?
            .to_owned();
        let names = |suffix: &str| self.names.name_with(slot, suffix);
        // 141A7B350 cases 37..41 probe the current tag without ideology.
        let non_ideology = |suffix: &str| -> Result<String, String> {
            let key = format!("{tag}{suffix}");
            if self.localisation.get(&key).is_some() {
                self.localise(&key)
            } else {
                names(suffix)
            }
        };
        let is = |entry: &str| named(name, entry);
        // SsoString_UppercaseFirstCharacter (1424B6FE0): the first byte's
        // toupper in the "C" locale ([`crt_case_text`]).
        let upper_first = |text: String| crt_case_text(&text, true, true);
        Ok(Some(if is("GetName") {
            names("")?
        } else if is("GetNameDef") {
            names("_DEF")?
        } else if is("GetNameDefCap") {
            upper_first(names("_DEF")?)
        } else if is("GetAdjective") {
            names("_ADJ")?
        } else if is("GetAdjectiveCap") {
            upper_first(names("_ADJ")?)
        } else if is("GetTag") {
            tag
        } else if is("GetFlag") {
            format!("\u{12}{tag}")
        } else if is("GetNameWithFlag") {
            format!("\u{12}{tag} {}", upper_first(names("_DEF")?))
        } else if let Some(&(entry, suffix, cap)) = OLD_NAMES.iter().find(|(entry, ..)| is(entry)) {
            let text = self.old_name(slot, entry, suffix)?;
            if cap { upper_first(text) } else { text }
        } else if is("GetNonIdeologyName") {
            non_ideology("")?
        } else if is("GetNonIdeologyNameDef") {
            non_ideology("_DEF")?
        } else if is("GetNonIdeologyNameDefCap") {
            upper_first(non_ideology("_DEF")?)
        } else if is("GetNonIdeologyAdjective") {
            non_ideology("_ADJ")?
        } else if is("GetNonIdeologyAdjectiveCap") {
            upper_first(non_ideology("_ADJ")?)
        } else if is("GetRulingIdeology") || is("GetRulingIdeologyNoun") {
            let Some(group) = self.names.ruling_group(slot)? else {
                return Err(format!("{name}: {tag} has no ruling party"));
            };
            let key = if is("GetRulingIdeologyNoun") {
                format!("{group}_noun")
            } else {
                group.to_owned()
            };
            // String_ToLowerInPlace (1424B8790), the "C" locale's tolower.
            crt_case_text(&self.localise(&key)?, false, false)
        } else if is("GetRulingParty") || is("GetRulingPartyLong") {
            self.ruling_party(slot, &tag, is("GetRulingPartyLong"))?
        } else if is("GetLeader") {
            self.party_leader(slot, Party::Ruling)?
        } else if let Some(group) = properties::leader_group(name) {
            self.party_leader(slot, Party::Token(group))?
        } else if let Some(group) = properties::party_group(name) {
            match self.party(slot, Party::Named(group))? {
                Some(party) => self.party_name(party, &tag, true)?,
                None => String::new(),
            }
        } else if is("GetPartySupport") {
            self.party_support(slot)?
        } else if is("GetLastElection") {
            let (politics, gap) = rows::<tables::Politics>(self.frame);
            let row = politics
                .iter()
                .find(|row| self.scopes.slot(row.country) == Some(slot))
                .ok_or_else(|| format!("GetLastElection: Politics {gap}"))?;
            self.date_text(row.last_election)?
        } else if PRONOUNS.iter().any(|entry| is(entry)) {
            self.country_pronoun(slot, name)?
        } else if is("GetManpower") {
            self.manpower(slot)?.to_string()
        } else if is("GetAgency") {
            self.agency(slot)?
        } else if let Some(text) = self.balance(slot, name)? {
            text
        } else if is("GetFactionName") {
            self.faction_name(slot)?
        } else if let Some(entry) = COUNTRY_UNPORTED.iter().find(|(entry, _)| is(entry)) {
            return Err(format!("{}: {}", entry.0, entry.1));
        } else {
            return Ok(None);
        }))
    }

    fn character_property(&self, id: u32, name: &str) -> Result<Option<String>, String> {
        let is = |entry: &str| named(name, entry);
        if is("GetName") {
            // 141A72900 case 0 copies +0x28 directly; party leaders
            // instead use 140FB5FD0 with the token fallback.
            let (characters, gap) = rows::<tables::Characters>(self.frame);
            let row = characters
                .iter()
                .find(|row| row.id == id)
                .ok_or_else(|| format!("character {id} has no Characters row: {gap}"))?;
            return String::from_utf8(row.name.clone())
                .map(Some)
                .map_err(|_| format!("character {id}'s name is not UTF-8"));
        }
        if is("GetIdeology") || is("GetIdeologyGroup") {
            return Err(format!(
                "{name}: a character's country-leader ideology is not recorded per character"
            ));
        }
        if PRONOUNS.iter().any(|entry| is(entry)) {
            return self
                .pronoun(name, self.character_gender(id)? == 2)
                .map(Some);
        }
        Ok(None)
    }

    /// CCharacter::GetDisplayName (140FB5FD0): the stored name (+0x28),
    /// else the saved name key (+0x48) through the localiser.
    fn character_name(&self, id: u32) -> Result<String, String> {
        let (characters, gap) = rows::<tables::Characters>(self.frame);
        let row = characters
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("character {id} has no Characters row: {gap}"))?;
        if !row.name.is_empty() {
            return String::from_utf8(row.name.clone())
                .map_err(|_| format!("character {id}'s name is not UTF-8"));
        }
        self.localisation
            .localise(&row.token)
            .map_err(|refusal| refusal.to_string())
    }

    /// Capital (+0x1018) or OriginalCapital (+0x101C, else +0x1018): a null
    /// state when invalid, so a property then reads `NULL`.
    fn capital(&self, slot: u32, original: bool) -> Result<Object, String> {
        let (politics, gap) = rows::<tables::Politics>(self.frame);
        let row = politics
            .iter()
            .find(|row| self.scopes.slot(row.country) == Some(slot))
            .ok_or_else(|| {
                format!("the capital of country slot {slot} is unknown: Politics {gap}")
            })?;
        let capital = if original && row.original_capital != 0 {
            row.original_capital
        } else {
            row.capital
        };
        Ok(Object::State(
            u32::try_from(capital).ok().filter(|id| *id > 0),
        ))
    }

    fn overlord(&self, slot: u32) -> Result<Option<u32>, String> {
        let (appearance, gap) = rows::<tables::CountryAppearance>(self.frame);
        let row = appearance
            .iter()
            .find(|row| self.scopes.slot(row.country) == Some(slot))
            .ok_or_else(|| {
                format!("the overlord of country slot {slot} is unknown: CountryAppearance {gap}")
            })?;
        // 141A7DC60: a positive overlord (+0x188) becomes the context; none
        // is logged and leaves it unchanged.
        let link = match u32::try_from(row.overlord) {
            Ok(link) if link > 0 => link,
            _ => return Ok(None),
        };
        let (_, links) = rows::<tables::CountryLinks>(self.frame);
        self.scopes.slot(link).map(Some).ok_or_else(|| {
            format!(
                "the overlord link {link} of country slot {slot} resolves to no country: CountryLinks {links}"
            )
        })
    }

    /// The row of country `slot` in table `T`, whose rows name their country
    /// by `link`; `None` only where the frame proves there is none: the table
    /// is whole, and CountryLinks is whole or every row's link resolves.
    fn country_row<T: TableSchema>(
        &self,
        slot: u32,
        link: impl Fn(&T::Row) -> u32,
    ) -> Result<Option<&'a T::Row>, String> {
        let (found, gap) = rows::<T>(self.frame);
        if let Some(row) = found
            .iter()
            .find(|row| self.scopes.slot(link(row)) == Some(slot))
        {
            return Ok(Some(row));
        }
        if !complete::<T>(self.frame) {
            return Err(format!("{:?} {gap}", T::ID));
        }
        if !complete::<tables::CountryLinks>(self.frame)
            && let Some(row) = found
                .iter()
                .find(|row| self.scopes.slot(link(row)).is_none())
        {
            let (_, links) = rows::<tables::CountryLinks>(self.frame);
            return Err(format!(
                "a {:?} row's country link {} resolves to no country: CountryLinks {links}",
                T::ID,
                link(row)
            ));
        }
        Ok(None)
    }

    /// The country's faction membership (+0x290), `None` without one.
    fn faction_member(&self, slot: u32) -> Result<Option<&'a FactionMemberRow>, String> {
        self.country_row::<tables::FactionMembers>(slot, |row| row.country)
            .map_err(|reason| format!("the faction of country slot {slot} is unknown: {reason}"))
    }

    /// The first member (ordinal 0, the faction's +0x58) of the country's
    /// faction, `None` without a faction (141A7DC60 then leaves the context
    /// unchanged).
    fn faction_leader(&self, slot: u32) -> Result<Option<u32>, String> {
        let Some(member) = self.faction_member(slot)? else {
            return Ok(None);
        };
        let (members, gap) = rows::<tables::FactionMembers>(self.frame);
        let leader = members
            .iter()
            .find(|row| row.faction == member.faction && row.ordinal == 0)
            .ok_or_else(|| {
                format!(
                    "faction {} has no first member: FactionMembers {gap}",
                    member.faction
                )
            })?;
        let (_, links) = rows::<tables::CountryLinks>(self.frame);
        self.scopes.slot(leader.country).map(Some).ok_or_else(|| {
            format!(
                "the leader link {} of faction {} resolves to no country: CountryLinks {links}",
                leader.country, member.faction
            )
        })
    }

    fn state_country(&self, id: u32, owner: bool) -> Result<Object, String> {
        let (states, gap) = rows::<tables::States>(self.frame);
        let state = states
            .iter()
            .find(|state| state.id == id)
            .ok_or_else(|| format!("state {id} has no States row: {gap}"))?;
        let link = if owner { state.owner } else { state.controller };
        self.scopes
            .slot(link)
            .map(Object::Country)
            .ok_or_else(|| format!("state {id}'s country link {link} resolves to no country"))
    }

    fn character_country(&self, id: u32) -> Result<Object, String> {
        let (characters, gap) = rows::<tables::Characters>(self.frame);
        let row = characters
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("character {id} has no Characters row: {gap}"))?;
        self.scopes
            .slot(row.country)
            .map(Object::Country)
            .ok_or_else(|| {
                format!(
                    "character {id}'s country link {} resolves to no country",
                    row.country
                )
            })
    }

    /// The context 141A81250 selects from an event scope: its character
    /// (+0x50, 140529310) first, then its country (+8), then its state
    /// (+0xA8).
    fn event_object(&self, object: EventObject, name: &str) -> Result<Object, String> {
        Ok(match object {
            EventObject::Country(slot) => Object::Country(slot),
            EventObject::State(id) => Object::State(Some(id)),
            EventObject::Character(id) => Object::Character(id),
            EventObject::UnitLeader(reference) => {
                let (leaders, gap) = rows::<tables::UnitLeaders>(self.frame);
                let row = leaders
                    .iter()
                    .find(|row| {
                        (u64::from(row.id) << 32 | u64::from(row.reference_type)) == reference
                    })
                    .ok_or_else(|| {
                        format!(
                            "{name}: unit leader {reference:#x} is not recorded: UnitLeaders {gap}"
                        )
                    })?;
                Object::Character(row.character_id)
            }
            EventObject::Province(id) => {
                return Err(format!(
                    "{name}: province {id} is a text object, not a native event-scope target (141A81250)"
                ));
            }
        })
    }

    /// The local player's country as 1410CF9C0 / 141A81250 take it: CGameState+0x520
    /// when signed-positive, else +0x524 (LocalPlayer, viewing_country),
    /// through CountryLinks; refused by name where the hour does not say.
    fn local_player(&self) -> Result<u32, String> {
        let (players, gap) = rows::<tables::LocalPlayer>(self.frame);
        if !complete::<tables::LocalPlayer>(self.frame) {
            return Err(format!(
                "the local player's country (CGameState+0x520, +0x524) is not recorded this hour: LocalPlayer {gap}"
            ));
        }
        let [player] = players else {
            return Err(format!(
                "LocalPlayer holds {} rows where the game state holds one local player",
                players.len()
            ));
        };
        let raw = if (player.country as i32) > 0 {
            player.country
        } else {
            player.observer_country
        };
        // ResolveCountryReference (140BA56A0) maps zero to slot zero.
        if raw == 0 {
            return Ok(0);
        }
        self.scopes
            .slot(raw)
            .ok_or_else(|| format!("the local player's country link {raw} has no CountryLinks row"))
    }

    /// A character of the current country, else of the active event scope's
    /// country (+8) when it has one (1410CF9C0 -> 1410D6BF0), whose script
    /// token is `segment`; `None` only where the frame proves there is none.
    /// A state, character or unit leader scope holds no country: default
    /// construction (140528800) leaves +8 zero and SetState, SetCharacter and
    /// set_unit_leader (14052EFD0) clear it (14052F3F0), so the search is
    /// skipped. For a province scope the country is not known, so only a
    /// token no character has is proven no character.
    fn character_named(
        &self,
        object: Object,
        segment: &str,
        event: EventObject,
    ) -> Result<Option<u32>, String> {
        let country = match (object, event) {
            (Object::Country(slot), _) | (_, EventObject::Country(slot)) => Some(slot),
            (_, EventObject::State(_) | EventObject::Character(_) | EventObject::UnitLeader(_)) => {
                return Ok(None);
            }
            (_, EventObject::Province(_)) => None,
        };
        let (characters, gap) = rows::<tables::Characters>(self.frame);
        let (lexical, lexical_gap) = rows::<tables::LexicalTokens>(self.frame);
        let links_whole = complete::<tables::CountryLinks>(self.frame);
        let mut unknown = None;
        for row in characters {
            match self.scopes.slot(row.country) {
                _ if country.is_none() => {}
                Some(slot) if Some(slot) == country => {}
                None if !links_whole => {
                    let (_, links) = rows::<tables::CountryLinks>(self.frame);
                    unknown.get_or_insert_with(|| {
                        format!(
                            "character {}'s country link {} resolves to no country: CountryLinks {links}",
                            row.id, row.country
                        )
                    });
                    continue;
                }
                _ => continue,
            }
            // A negative token id is no token.
            let Ok(token) = u32::try_from(row.name_token_id) else {
                continue;
            };
            match lexical.binary_search_by_key(&token, |entry| entry.token) {
                Ok(index) if lexical[index].literal == segment.as_bytes() => {
                    if country.is_none() {
                        return Err(format!(
                            "{segment} names character {}, and the event scope's country (+8), which 1410CF9C0 searches, is not known here",
                            row.id
                        ));
                    }
                    return Ok(Some(row.id));
                }
                Ok(_) => {}
                Err(_) => {
                    unknown.get_or_insert_with(|| {
                        format!(
                            "character {}'s script token {token} is not spelled: LexicalTokens {lexical_gap}",
                            row.id
                        )
                    });
                }
            }
        }
        let unknown = if complete::<tables::Characters>(self.frame) {
            unknown
        } else {
            Some(format!("Characters {gap}"))
        };
        match unknown {
            Some(reason) => Err(match country {
                Some(country) => format!(
                    "{segment}: whether it names a character of country slot {country} is unknown: {reason}"
                ),
                None => format!("{segment}: whether it names a character is unknown: {reason}"),
            }),
            None => Ok(None),
        }
    }

    /// GetRulingParty (14118F860) or its long form (14118F810).
    fn ruling_party(&self, slot: u32, tag: &str, long: bool) -> Result<String, String> {
        let (politics, gap) = rows::<tables::Politics>(self.frame);
        let politics = politics
            .iter()
            .find(|row| self.scopes.slot(row.country) == Some(slot))
            .ok_or_else(|| format!("the ruling party of {tag} is unknown: Politics {gap}"))?;
        let Some(party) = politics.ruling_party else {
            return Err(format!("{tag} has no ruling party"));
        };
        let (parties, gap) = rows::<tables::PoliticalParties>(self.frame);
        let party = parties
            .iter()
            .find(|row| row.country == politics.country && row.id == party)
            .ok_or_else(|| {
                format!("the ruling party of {tag} has no PoliticalParties row: {gap}")
            })?;
        self.party_name(party, tag, long)
    }

    fn party_name(
        &self,
        party: &replay::rows::PoliticalPartyRow,
        tag: &str,
        long: bool,
    ) -> Result<String, String> {
        if party.name_default == 0 {
            let key = if long { &party.long_name } else { &party.name };
            return self
                .localisation
                .localise(key)
                .map_err(|refusal| refusal.to_string());
        }
        let (appearance, _) = rows::<tables::CountryAppearance>(self.frame);
        let original = appearance
            .iter()
            .find(|row| row.country == party.country)
            .map(|row| String::from_utf8_lossy(&row.name_tag).into_owned());
        let party_tag = self
            .scopes
            .slot(party.tag)
            .and_then(|slot| self.scopes.tag(slot))
            .map(str::to_owned);
        let naming = crate::products::party_history::PartyNaming {
            token: String::from_utf8_lossy(&party.token).into_owned(),
            tag: party_tag,
            original_tag: original,
            key: None,
        };
        let defines = |key: &str| self.localisation.get(key).is_some();
        let key = crate::products::party_history::party_key(&naming, defines).ok_or_else(|| {
            format!("the ruling party of {tag}: its tag or original tag is not recorded")
        })?;
        // 14118EAD0 probes `_long` only on a `_party` key it found, not on
        // the group token it falls back to.
        let long_key = format!("{key}_long");
        let key = if long && key != naming.token && defines(&long_key) {
            long_key
        } else {
            key
        };
        self.localise(&key)
    }

    /// GetFactionName (141A7B350 case 0x12): the faction's name, else the
    /// executable's key `NO_FACTION` through the localiser (142230F20), which
    /// copies it as its bytes when the content lacks it (142230940).
    fn faction_name(&self, slot: u32) -> Result<String, String> {
        let Some(member) = self.faction_member(slot)? else {
            return self.localise(NO_FACTION);
        };
        let (factions, gap) = rows::<tables::Factions>(self.frame);
        let faction = factions
            .iter()
            .find(|row| row.id == member.faction)
            .ok_or_else(|| format!("faction {} has no Factions row: {gap}", member.faction))?;
        if faction.name.is_empty() {
            return Err(format!(
                "faction {} has no custom name, and its template name is not recorded",
                member.faction
            ));
        }
        self.localisation
            .localise(&faction.name)
            .map_err(|refusal| refusal.to_string())
    }
}

/// The key GetFactionName localises without a faction (141A7B350 case 0x12).
const NO_FACTION: &str = "NO_FACTION";

/// The ten pronoun functions (country IDs 27-36, character 1-10).
const PRONOUNS: [&str; 10] = [
    "GetSheHe",
    "GetSheHeCap",
    "GetHerHim",
    "GetHerHimCap",
    "GetHerHis",
    "GetHerHisCap",
    "GetHersHis",
    "GetHersHisCap",
    "GetHerselfHimself",
    "GetHerselfHimselfCap",
];

/// Country functions of 141A7B350 (table 14309CA20) this port cannot answer,
/// with what is missing. Their inputs are content or recorded; what they lack
/// is the native description renderer.
const COUNTRY_UNPORTED: [(&str, &str); 4] = [
    (
        "GetActiveRangeModDesc",
        "140A04DA0 formats the active range's content modifier block (+0x28) through modifier_accumulator::FormatDescription (14054E050) under PROGRESS_SECTION_MODIFIER; that renderer is ported only in the viewer",
    ),
    (
        "GetActiveRangeRuleDesc",
        "140A05130 describes the active range's rules (+0x238, virtual +0x50) under PROGRESS_SECTION_RULE; rule descriptions are not ported",
    ),
    (
        "GetActiveRangeActivationEffect",
        "140A04500 runs the effect description (virtual +0x40) of the active range's activation effect (+0x630) under PROGRESS_SECTION_ACTIVATION_EFFECT; effect descriptions are not ported",
    ),
    (
        "GetActiveRangeDeactivationEffect",
        "140A047C0 runs the effect description of the active range's deactivation effect (+0x688); effect descriptions are not ported",
    ),
];

/// GetOld* (141A7B350 cases 5-9): the property, the suffix of the current
/// name it falls back to, and whether the first byte is uppercased.
const OLD_NAMES: [(&str, &str, bool); 5] = [
    ("GetOldName", "", false),
    ("GetOldNameDef", "_DEF", false),
    ("GetOldNameDefCap", "_DEF", true),
    ("GetOldAdjective", "_ADJ", false),
    ("GetOldAdjectiveCap", "_ADJ", true),
];

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_wiring;
