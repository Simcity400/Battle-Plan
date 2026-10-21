//! Content facts every product shares: the recording's localisation in the
//! user's game language and its interface sprites, read once per source and
//! folders, plus the texture decoding every image reply uses. Everything is
//! resolved through the recorded content stack; nothing names a vanilla asset.

use crate::invalid;
use base64::Engine;
use dashboard_core::findings::{Finding, Kind};
use installed_content_core::{
    ResolvedLayers, content::ResolvedContent, gui::native::resources, gui::native::sprites::Plain,
};
use installed_content_gfx::gfx::SpriteSet;
use installed_content_image::{Rgba8Image, native::Decode};
use installed_content_localisation::{localisation::Argument, localisation::Localisation};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, OnceLock},
};

/// The retained compressed artwork budget shared by product content caches.
pub const MAX_RETAINED_ARTWORK_BYTES: usize = 64 * 1024 * 1024;

/// A fixed-content error retains its native kind and message on cache hits.
#[derive(Clone, Debug)]
pub struct ContentError {
    kind: io::ErrorKind,
    raw_os_error: Option<i32>,
    message: Arc<str>,
}
impl From<io::Error> for ContentError {
    fn from(error: io::Error) -> Self {
        Self {
            kind: error.kind(),
            raw_os_error: error.raw_os_error(),
            message: error.to_string().into(),
        }
    }
}
impl ContentError {
    pub fn error(&self) -> io::Error {
        match self.raw_os_error {
            Some(code) => io::Error::from_raw_os_error(code),
            None => io::Error::new(self.kind, self.message.to_string()),
        }
    }
}

/// One demanded database per recorded-content binding, including a failed read.
/// The owning Source/ResolvedLayers supplies the key and lifetime; this policy
/// only serialises a cold load and retains its result. Cancellation is retryable.
/// The text refusals and readers the read produced are retained too, and every
/// later use credits them to its consumer scopes ([`own_refusals`]), so which
/// consumer read first does not decide whose refusals they are.
pub struct CachedResult<T> {
    value: Mutex<Option<Result<Arc<T>, ContentError>>>,
    noted: Mutex<OwnRefusals>,
    #[cfg(any(test, feature = "test-support"))]
    loads: std::sync::atomic::AtomicUsize,
}
impl<T> Default for CachedResult<T> {
    fn default() -> Self {
        Self {
            value: Mutex::new(None),
            noted: Mutex::default(),
            #[cfg(any(test, feature = "test-support"))]
            loads: Default::default(),
        }
    }
}
impl<T> CachedResult<T> {
    pub fn get_or_init(&self, read: impl FnOnce() -> io::Result<T>) -> io::Result<Arc<T>> {
        let mut value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        if value.is_none() {
            #[cfg(any(test, feature = "test-support"))]
            self.loads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // The read's notes reach the enclosing scopes as they happen.
            let (result, noted) = capture(read);
            if result
                .as_ref()
                .is_err_and(|error| error.kind() == io::ErrorKind::Interrupted)
            {
                return result.map(Arc::new);
            }
            *self.noted.lock().unwrap_or_else(|error| error.into_inner()) = noted;
            *value = Some(result.map(Arc::new).map_err(ContentError::from));
        } else {
            credit(&self.noted.lock().unwrap_or_else(|error| error.into_inner()));
        }
        value
            .as_ref()
            .expect("database result retained")
            .as_ref()
            .map(Arc::clone)
            .map_err(ContentError::error)
    }

    /// A result already loaded, for tests that supply their own database.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loaded(value: Arc<T>) -> Self {
        let result = Self::default();
        *result.value.lock().unwrap() = Some(Ok(value));
        result
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn loads(&self) -> usize {
        self.loads.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Fixed projections of recorded operands within one content binding.
/// Each key uses the database result policy, including named failures and
/// retryable cancellation. The Source/ResolvedLayers slot owns the binding.
pub struct KeyedResults<K, T> {
    values: Mutex<BTreeMap<K, Arc<CachedResult<T>>>>,
}
impl<K, T> Default for KeyedResults<K, T> {
    fn default() -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
        }
    }
}
impl<K: Ord + Clone, T> KeyedResults<K, T> {
    pub fn get_or_init(
        &self,
        key: &K,
        read: impl FnOnce(&K) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        let result = {
            let mut values = self
                .values
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            Arc::clone(values.entry(key.clone()).or_default())
        };
        result.get_or_init(|| read(key))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn loads(&self, key: &K) -> usize {
        self.values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            .map_or(0, |result| result.loads())
    }
}

/// Retry the fallback sprite only when the requested name is absent.
/// A declared but refused sprite stays refused.
pub fn created_gui_sprite<'a>(
    sprites: &'a SpriteSet,
    name: &str,
) -> io::Result<&'a installed_content_gfx::gfx::SpriteType> {
    if let Some(sprite) = sprites.find(name).map_err(invalid)? {
        return Ok(sprite);
    }
    let fallback = std::str::from_utf8(installed_content_core::gui::native::resources::FALLBACK_SPRITE)
        .expect("the executable's fallback sprite literal is ASCII");
    sprites
        .lookup(fallback)
        .map_err(|why| invalid(format!("{name} is not declared, and its fallback: {why}")))
}

/// Text icons search the GFX_ prefix before the bare authored alias.
/// A declared but unsupported sprite is a refusal; only an absent declaration
/// takes the second lookup.
pub fn text_icon_sprite<'a>(
    sprites: &'a SpriteSet,
    alias: &str,
) -> io::Result<&'a installed_content_gfx::gfx::SpriteType> {
    let prefixed = format!("GFX_{alias}");
    if let Some(sprite) = sprites.find(&prefixed).map_err(invalid)? {
        return Ok(sprite);
    }
    sprites.lookup(alias).map_err(invalid)
}

/// How the game's view of a name treats `[...]` scripted text. A text box
/// stores the string and draws brackets as glyphs; a scripted view evaluates
/// them before passing the text to the box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// The name reaches its text box through the plain localiser only:
    /// brackets are glyphs. Used for party, character, ideology, technology
    /// and faction names, trait lists, idea slots, character seats and idea
    /// categories.
    Plain,
    /// The view evaluates text in a country's event scope. Used for idea
    /// and focus names and descriptions, idea tooltip modifier lines and
    /// their postfixes and custom tooltips, and stability breakdown names.
    /// With the country ([`localised_in`]) the brackets are evaluated
    /// ([`crate::scripted_text`]); without it a closed `[...]` is refused.
    /// An unclosed `[` stops evaluation before the preceding text is
    /// appended, so that text is dropped.
    Scripted,
}

/// [`localised_with`] with no arguments.
pub fn localised(localisation: &Localisation, key: &str, view: View) -> Result<String, String> {
    localised_with(localisation, key, &[], view)
}

/// Saved byte strings take the C-string path:
/// an embedded NUL ends the key before UTF-8 decoding or content lookup.
pub fn localised_bytes(
    localisation: &Localisation,
    key: &[u8],
    view: View,
) -> Result<String, String> {
    let text = localisation.localise(key).map_err(|why| why.to_string())?;
    let (text, _) = scripted(&text, view, Kind::Unsupported)?;
    plain_text(&text)
}

/// `key` through the game's localiser (`installed_content_localisation::localisation`), with its
/// substitution, depth and 0x1800-byte buffer. A key the content lacks is copied
/// as its own bytes. Then read as `view` shows it ([`View`]) and made
/// plain text ([`plain_text`]). The error names the key and why.
pub fn localised_with(
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
    view: View,
) -> Result<String, String> {
    localised_with_reason(localisation, key, arguments, view, Kind::Unsupported)
        .0
        .map_err(|finding| finding.message)
}

/// A product's text and typed findings, obtained with one substitution walk.
pub fn localised_details(
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
    view: View,
    country_kind: Kind,
) -> (Option<String>, Vec<Finding>) {
    let (text, mut findings) =
        localised_with_reason(localisation, key, arguments, view, country_kind);
    let text = text.map_err(|finding| findings.push(*finding)).ok();
    (text, findings)
}

fn localised_with_reason(
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
    view: View,
    country_kind: Kind,
) -> (Result<String, Box<Finding>>, Vec<Finding>) {
    localised_with_renderer(localisation, key, arguments, |text| {
        scripted(text, view, country_kind).map_err(|reason| {
            let aggregate = if country_kind == Kind::Offline {
                "scripted localisation needs recorded country context; unavailable offline"
            } else {
                "scripted localisation is not evaluated because this product does not pass a country"
            };
            Box::new(Finding::grouped(
                country_kind,
                key,
                aggregate,
                format!("localisation {key} holds {reason}"),
            ))
        })
    })
}

fn localised_with_renderer(
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
    render: impl FnOnce(&str) -> Result<(String, Option<&'static str>), Box<Finding>>,
) -> (Result<String, Box<Finding>>, Vec<Finding>) {
    let (text, fallbacks) = localisation.localise_with_fallbacks(key.as_bytes(), arguments);
    // Keep substitution recovery findings alongside the recovered text.
    let mut findings: Vec<_> = fallbacks
        .into_iter()
        .map(|reason| Finding::new(Kind::Content, key, reason))
        .collect();
    let text = text
        .map_err(|why| {
            Box::new(Finding::grouped(
                Kind::Refused,
                key,
                "native localisation text refused",
                why.to_string(),
            ))
        })
        .and_then(|text| {
            let (text, fallback) = render(&text)?;
            if let Some(reason) = fallback {
                // The game drops this text during scripted evaluation.
                findings.push(Finding::new(
                    Kind::Content,
                    key,
                    format!("localisation {key} holds {reason}"),
                ));
            }
            plain_text(&text).map_err(|why| {
                Box::new(Finding::grouped(
                    Kind::Unsupported,
                    key,
                    &format!("localisation holds {why}"),
                    format!("localisation {key} holds {why}"),
                ))
            })
        });
    (text, findings)
}

/// The text as `view` shows its `[`s ([`View`]).
fn scripted(
    text: &str,
    view: View,
    country_kind: Kind,
) -> Result<(String, Option<&'static str>), String> {
    let Some(open) = text.find('[') else {
        return Ok((text.to_owned(), None));
    };
    let closed = text[open + 1..].find(']');
    let country_reason = if country_kind == Kind::Offline {
        "recorded country context is unavailable offline"
    } else {
        "this product does not pass that country"
    };
    match (view, closed) {
        (View::Plain, _) => Ok((text.to_owned(), None)),
        (View::Scripted, None) => Ok((
            String::new(),
            Some("an unclosed scripted expression; the game omits the text"),
        )),
        (View::Scripted, Some(length)) => Err(format!(
            "the scripted expression {:?}, which the game evaluates for the country the text is shown for, and {country_reason}",
            &text[open..open + length + 2],
        )),
    }
}

/// [`localised_with`] in a Scripted view evaluated for a country: the
/// brackets go through [`crate::scripted_text::ScriptedText`], then the
/// text is made plain.
pub fn localised_in(
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
    scripted: &crate::scripted_text::ScriptedText<'_>,
) -> Result<String, String> {
    localised_in_details(localisation, key, arguments, scripted)
        .0
        .map_err(|finding| finding.message)
}

pub fn localised_in_details(
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
    scripted: &crate::scripted_text::ScriptedText<'_>,
) -> (Result<String, Box<Finding>>, Vec<Finding>) {
    localised_with_renderer(localisation, key, arguments, |text| {
        scripted
            .expand(text)
            .map(|text| (text, None))
            .map_err(|why| {
                Box::new(Finding::grouped(
                    Kind::Unsupported,
                    key,
                    "recorded-country scripted text unavailable",
                    format!("localisation {key} holds {why}"),
                ))
            })
    })
}

/// Stored game text (`installed_content_localisation::localisation`) as plain text, read as the
/// game's text layout reads it (`installed_content_localisation::localisation::pieces`): colour
/// codes are removed, inline images (a text icon or an inline flag) are left
/// out of the plain text, and the 0x14 and 0x1A spacing controls, which have
/// no plain-text form, are refused by name.
pub fn plain_text(text: &str) -> Result<String, String> {
    use installed_content_localisation::{localisation::Piece, localisation::pieces};
    let mut clean = String::new();
    for piece in pieces(text) {
        match piece {
            Piece::Text(text) => clean.push_str(text),
            Piece::Colour(_) | Piece::Icon(_) | Piece::Flag(_) => (),
            Piece::Control14 | Piece::Control1A(_) => {
                return Err(format!(
                    "{}, which plain text cannot show",
                    piece.describe()
                ));
            }
        }
    }
    Ok(clean)
}

/// Issues by reason, with what each names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusals(BTreeMap<String, BTreeSet<String>>);

impl Refusals {
    pub fn merge(&mut self, other: &Self) {
        for (reason, identities) in &other.0 {
            self.0
                .entry(reason.clone())
                .or_default()
                .extend(identities.iter().cloned());
        }
    }

    /// Counts `identity` under `reason` once, however often it is met.
    pub fn note(&mut self, reason: impl Into<String>, identity: impl Into<String>) {
        self.0
            .entry(reason.into())
            .or_default()
            .insert(identity.into());
    }

    /// One counted warning per reason, without enumerating every identity.
    pub fn counts(&self, what: &str) -> Vec<String> {
        self.0
            .iter()
            .map(|(reason, identities)| format!("{} {what} not shown: {reason}", identities.len()))
            .collect()
    }

    /// One warning per reason, naming every `what` it affects.
    pub fn lines(&self, what: &str) -> Vec<String> {
        self.diagnostic_lines(&format!("{what} not shown"))
    }

    /// Counted issues without implying that the affected values were omitted.
    pub fn diagnostic_lines(&self, what: &str) -> Vec<String> {
        self.counted_diagnostic_lines(what)
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }

    pub fn counted_diagnostic_lines(&self, what: &str) -> Vec<(String, usize)> {
        self.0
            .iter()
            .map(|(reason, identities)| {
                (
                    format!(
                        "{} {what}: {reason}; {}",
                        identities.len(),
                        identities.iter().cloned().collect::<Vec<_>>().join(", ")
                    ),
                    identities.len(),
                )
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A lazily read content input whose reader warnings a consumer's own lines
/// carry once the consumer has used it ([`Catalogue::reader_warning_lines`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Reader {
    /// Scripted localisation ([`Catalogue::scripted_content`]).
    Scripted,
    /// State templates ([`Catalogue::cached_states`]), however scripted text reaches them.
    States,
    /// Faction templates, as scripted text reads them.
    FactionTemplates,
}

/// The text refusals one consumer's own catalogue calls produced, and the
/// lazily read inputs those calls used. A prepared family keeps it in its
/// saved state, so a family restored from a commit reports what the same
/// family continued in memory would.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnRefusals {
    texts: Refusals,
    readers: BTreeSet<Reader>,
}

impl OwnRefusals {
    fn is_empty(&self) -> bool {
        self.texts.is_empty() && self.readers.is_empty()
    }

    pub fn merge(&mut self, other: &Self) {
        self.texts.merge(&other.texts);
        self.readers.extend(other.readers.iter().copied());
    }
}

/// One consumer scope on this thread. A capture ([`capture`]) collects
/// without changing what the catalogue reports.
struct Active {
    own: OwnRefusals,
    reports: bool,
}

thread_local! {
    /// The consumer scopes active on this thread, innermost last.
    static SCOPES: std::cell::RefCell<Vec<Active>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn scoped<T>(own: &mut OwnRefusals, reports: bool, run: impl FnOnce() -> T) -> T {
    // Pops the scope back into `own` on return and when `run` unwinds.
    struct Exit<'a>(&'a mut OwnRefusals);
    impl Drop for Exit<'_> {
        fn drop(&mut self) {
            if let Some(active) = SCOPES.with(|scopes| scopes.borrow_mut().pop()) {
                *self.0 = active.own;
            }
        }
    }
    SCOPES.with(|scopes| {
        scopes.borrow_mut().push(Active {
            own: std::mem::take(own),
            reports,
        });
    });
    let _exit = Exit(own);
    run()
}

/// Runs `run` as one consumer of catalogue text. Every text refusal a
/// catalogue notes on this thread meanwhile, and every lazy reader used,
/// joins `own`; memoised reads credit theirs on each use ([`CachedResult`]).
/// Inside, [`Catalogue::refusal_lines`] reports `own` rather than the
/// catalogue's shared tally, which every consumer of the source changes at
/// its own pace. Enclosing scopes receive what inner calls produce; the
/// innermost scope reports.
pub fn own_refusals<T>(own: &mut OwnRefusals, run: impl FnOnce() -> T) -> T {
    scoped(own, true, run)
}

/// What `run` noted, which reaches the enclosing scopes as well, for a memo
/// or another thread to credit ([`credit`]) to later users.
pub fn capture<T>(run: impl FnOnce() -> T) -> (T, OwnRefusals) {
    let mut own = OwnRefusals::default();
    let value = scoped(&mut own, false, run);
    (value, own)
}

/// Credits `noted` to every scope active on this thread, as if the calls
/// that produced it ran here.
pub fn credit(noted: &OwnRefusals) {
    if noted.is_empty() {
        return;
    }
    SCOPES.with(|scopes| {
        for active in scopes.borrow_mut().iter_mut() {
            active.own.merge(noted);
        }
    });
}

/// Notes that the consumers active on this thread used `reader`.
pub fn used(reader: Reader) {
    SCOPES.with(|scopes| {
        for active in scopes.borrow_mut().iter_mut() {
            active.own.readers.insert(reader);
        }
    });
}

fn note_own(reason: &str, identity: &str) {
    SCOPES.with(|scopes| {
        for active in scopes.borrow_mut().iter_mut() {
            active.own.texts.note(reason, identity);
        }
    });
}

/// `read` of the innermost reporting scope, when one is active.
fn reporting<T>(read: impl FnOnce(&OwnRefusals) -> T) -> Option<T> {
    SCOPES.with(|scopes| {
        scopes
            .borrow()
            .iter()
            .rev()
            .find(|active| active.reports)
            .map(|active| read(&active.own))
    })
}

/// Named, counted refusals whose storage is bounded with a product's reply.
/// Check after each scan step and before turning them into reply warnings.
/// Reaching the bound refuses the whole product; it never drops individual
/// warnings to make an otherwise successful reply fit.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct BoundedRefusals {
    refused: Refusals,
    bytes: usize,
    overflow: bool,
    budget: usize,
}

impl BoundedRefusals {
    pub fn new(budget: usize) -> Self {
        Self {
            refused: Refusals::default(),
            bytes: 0,
            overflow: false,
            budget,
        }
    }

    pub fn note(&mut self, reason: impl Into<String>, identity: impl Into<String>) {
        if self.overflow {
            return;
        }
        let reason = reason.into();
        let identity = identity.into();
        if self
            .refused
            .0
            .get(&reason)
            .is_some_and(|identities| identities.contains(&identity))
        {
            return;
        }
        // Each unique note pays for both JSON strings, including escaping,
        // and the warning's count/label/separators. Repeated reasons paying
        // again makes this a conservative bound on Refusals::lines output.
        let available = self.budget.saturating_sub(self.bytes + 64);
        let Some(size) = crate::transport::json_len(&(&reason, &identity), available) else {
            self.overflow = true;
            return;
        };
        let Some(bytes) = self.bytes.checked_add(size + 64) else {
            self.overflow = true;
            return;
        };
        if bytes > self.budget {
            self.overflow = true;
            return;
        }
        self.bytes = bytes;
        self.refused.note(reason, identity);
    }

    pub fn check(&self) -> io::Result<usize> {
        if self.overflow {
            Err(io::Error::other(
                "Product refusals exceed the complete-response budget",
            ))
        } else {
            Ok(self.bytes)
        }
    }

    pub fn merge(&mut self, other: &Self) {
        if other.overflow {
            self.overflow = true;
        }
        for (reason, identities) in &other.refused.0 {
            for identity in identities {
                self.note(reason, identity);
            }
        }
    }

    pub fn lines(&self, what: &str) -> Vec<String> {
        self.refused.lines(what)
    }
}

/// A string the engine built for display, recorded as bytes, as plain text.
/// Stored colour codes and icons use the shared game localiser; unsupported
/// controls and invalid UTF-8 stay refusals instead of invented text.
pub fn display_text(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "is not UTF-8".to_owned())?;
    plain_text(text)
        .map(|text| text.trim().to_owned())
        .map_err(|reason| format!("holds {reason}"))
}

/// One source's shared content facts for one set of folders.
pub struct Catalogue {
    pub content: Arc<ResolvedContent>,
    /// Shared with country naming (`content::localisation`): parsed once per source.
    pub localisation: Arc<Localisation>,
    pub sprites: SpriteSet,
    /// Shared native templates, loaded only when a consumer needs GUI declarations.
    guis: OnceLock<Result<installed_content_core::gui::native::Guis, String>>,
    /// The current client's texture decode settings, used by native GUI artwork.
    pub gui_decode: Result<installed_content_image::native::Decode, String>,
    /// Notes from verifying the recording, such as unverified content allowed by the user.
    pub verification: Vec<String>,
    /// Standing notes about the recorded content, such as native reader logs.
    pub warnings: Vec<String>,
    /// The recording client's game language (the replay identity's, without
    /// its `l_`): the language of text the game stored already localised.
    pub recorded_language: String,
    /// Every key whose text [`Catalogue::localised_with`] or
    /// [`Catalogue::localise_in`] refused so far, by reason: a fact of the
    /// content, reported by [`Catalogue::refusal_lines`].
    refused: Mutex<Refusals>,
    findings: Option<Mutex<BTreeSet<Finding>>>,
    /// Scripted localisation names and the recursion limit, read on first
    /// use ([`Catalogue::scripted_content`]).
    scripted: OnceLock<Result<crate::scripted_text::ScriptedContent, String>>,
    war_names: OnceLock<Result<installed_content_wargoals::wargoals::Names, String>>,
    /// State templates are usable without the scripted-localisation inputs.
    pub state_content: Arc<OnceLock<Result<crate::scripted_text::StateContent, String>>>,
    #[cfg(any(test, feature = "test-support"))]
    content_loads: std::sync::atomic::AtomicUsize,
}

impl Catalogue {
    /// Load the complete native registry once, retaining its template order and
    /// post-load pass. A failure refuses GUI-dependent fields; text stays usable.
    pub fn guis(&self) -> io::Result<&installed_content_core::gui::native::Guis> {
        self.guis
            .get_or_init(|| {
                #[cfg(any(test, feature = "test-support"))]
                self.content_loads
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                installed_content_core::gui::native::read_layers(&self.content.layers)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(invalid)
    }

    /// Whether the native GUI registry has been read, for tests that check a
    /// read never reaches it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn guis_loaded(&self) -> bool {
        self.guis.get().is_some()
    }

    pub fn war_names(&self) -> Result<&installed_content_wargoals::wargoals::Names, String> {
        self.war_names
            .get_or_init(|| {
                #[cfg(any(test, feature = "test-support"))]
                self.content_loads
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                installed_content_wargoals::wargoals::read_names(&self.content.layers)
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
    /// A catalogue over resolved content and its localisation, as the per-source
    /// loader builds it.
    pub fn from_content(
        content: Arc<ResolvedContent>,
        localisation: Arc<Localisation>,
        verification: Vec<String>,
        mut warnings: Vec<String>,
        recorded_language: String,
        gui_decode: Result<installed_content_image::native::Decode, String>,
    ) -> io::Result<Self> {
        let sprites = installed_content_gfx::gfx::read(&content.layers)?;
        for (label, count) in [
            ("logged localisation diagnostics", localisation.logged.len()),
            ("refused localisation forms", localisation.refused.len()),
            ("duplicate localisation keys", localisation.duplicates.len()),
        ] {
            if count > 0 {
                warnings.push(format!("Recorded content contains {count} {label}."));
            }
        }
        // What the game does not define in the interface files, named and counted; a widget
        // asking for a refused sprite is told why (`SpriteSet::find`).
        if !sprites.refused.is_empty() {
            warnings.push(format!(
                "Recorded content contains {} refusals in its interface files: {}.",
                sprites.refused.len(),
                sprites.refused.join("; ")
            ));
        }
        Ok(Self {
            content,
            localisation,
            sprites,
            guis: OnceLock::new(),
            gui_decode,
            verification,
            warnings,
            recorded_language,
            refused: Mutex::default(),
            findings: None,
            scripted: OnceLock::new(),
            war_names: OnceLock::new(),
            state_content: Arc::new(OnceLock::new()),
            #[cfg(any(test, feature = "test-support"))]
            content_loads: Default::default(),
        })
    }

    /// A content-only catalogue, without a recording or identity verification.
    pub fn of_layers(layers: ResolvedLayers, language: &str) -> io::Result<Self> {
        let localisation = Arc::new(installed_content_localisation::localisation::read(
            &layers, language,
        )?);
        let content = Arc::new(ResolvedContent {
            layers,
            stats: Default::default(),
        });
        Self::from_content(
            content,
            localisation,
            Vec::new(),
            Vec::new(),
            language.to_owned(),
            Ok(Default::default()),
        )
    }

    /// Recorded-country text, with recovery notes from the same localiser walk.
    pub fn localise_in(
        &self,
        key: &str,
        scripted: &crate::scripted_text::ScriptedText<'_>,
    ) -> Option<String> {
        if !self.has_presentation_key(key) {
            return None;
        }
        let (text, findings) = localised_in_details(&self.localisation, key, &[], scripted);
        for finding in findings {
            self.note(finding);
        }
        text.inspect_err(|finding| self.refuse_finding((**finding).clone()))
            .ok()
    }

    pub fn localised_with(
        &self,
        key: &str,
        arguments: &[(&str, Argument<'_>)],
        view: View,
    ) -> Option<String> {
        self.localised_for(key, arguments, view, Kind::Unsupported, None)
    }

    /// The same native text walk, retaining this caller's failures separately
    /// from refusals already accumulated by other products.
    pub fn localised_with_refusals(
        &self,
        key: &str,
        arguments: &[(&str, Argument<'_>)],
        view: View,
        refusals: &Mutex<Refusals>,
    ) -> Option<String> {
        self.localised_for(key, arguments, view, Kind::Unsupported, Some(refusals))
    }

    /// Stability and special projects evaluate this text with the recorded
    /// country on their live paths.
    pub fn localised_offline_country(&self, key: &str) -> Option<String> {
        self.localised_for(key, &[], View::Scripted, Kind::Offline, None)
    }

    fn localised_for(
        &self,
        key: &str,
        arguments: &[(&str, Argument<'_>)],
        view: View,
        country_kind: Kind,
        refusals: Option<&Mutex<Refusals>>,
    ) -> Option<String> {
        if !self.has_presentation_key(key) {
            if let Some(refusals) = refusals {
                let key = key.split('\0').next().unwrap_or_default();
                refusals
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .note(
                        format!("localisation {key} is missing; title is unavailable"),
                        key,
                    );
            }
            return None;
        }
        let (text, findings) =
            localised_with_reason(&self.localisation, key, arguments, view, country_kind);
        for finding in findings {
            self.note(finding);
        }
        text.inspect_err(|finding| {
            if let Some(refusals) = refusals {
                refusals
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .note(&finding.message, &finding.subject);
            }
            self.refuse_finding((**finding).clone());
        })
        .ok()
    }

    /// Keep missing presentation keys distinct from authored empty strings.
    pub fn has_presentation_key(&self, key: &str) -> bool {
        let key = key.split('\0').next().unwrap_or_default();
        if key.is_empty() || self.localisation.get(key).is_some() {
            return true;
        }
        self.note(Finding::new(
            Kind::Content,
            key,
            format!("localisation {key} is missing; title is unavailable"),
        ));
        false
    }

    /// [`Catalogue::localised_with`] with no arguments. Presentation preserves
    /// absent versus defined empty text; the underlying native localizer retains
    /// its own missing-key behavior.
    pub fn localised(&self, key: &str, view: View) -> Option<String> {
        self.localised_with(key, &[], view)
    }

    /// The content the scripted text evaluator needs
    /// ([`crate::scripted_text::ScriptedContent::read`]), read once. Its
    /// reading's warnings join [`Catalogue::refusal_lines`].
    pub fn scripted_content(&self) -> Result<&crate::scripted_text::ScriptedContent, String> {
        used(Reader::Scripted);
        self.scripted
            .get_or_init(|| {
                #[cfg(any(test, feature = "test-support"))]
                self.content_loads
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                crate::scripted_text::ScriptedContent::read(
                    Arc::clone(&self.content),
                    self.recorded_language.clone(),
                )
                .map(|mut scripted| {
                    scripted.state_content = Arc::clone(&self.state_content);
                    scripted
                })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The defines of the recording's complete evaluated content stack.
    pub fn defines(&self) -> Result<&installed_content_core::defines::Defines, String> {
        Ok(self.scripted_content()?.environment()?.defines())
    }

    /// Participation factors from the recording's complete evaluated content stack.
    pub fn war_participation_factors(
        &self,
    ) -> Result<replay_data::war_participation::Factors, String> {
        let defines = self.scripted_content()?.environment()?.defines();
        let factor = |name| defines.fixed5("NMilitary", name).map_err(|e| e.to_string());
        Ok(replay_data::war_participation::Factors {
            industry_given: factor("WAR_SCORE_LEND_LEASE_GIVEN_IC_FACTOR")?,
            fuel_given: factor("WAR_SCORE_LEND_LEASE_GIVEN_FUEL_FACTOR")?,
            industry_received: factor("WAR_SCORE_LEND_LEASE_RECEIVED_IC_FACTOR")?,
            fuel_received: factor("WAR_SCORE_LEND_LEASE_RECEIVED_FUEL_FACTOR")?,
        })
    }

    pub fn cached_states(&self) -> Result<&crate::scripted_text::StateContent, String> {
        used(Reader::States);
        self.state_content
            .get_or_init(|| {
                #[cfg(any(test, feature = "test-support"))]
                self.content_loads
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                crate::scripted_text::read_state_content(&self.content.layers)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn content_loads(&self) -> usize {
        self.content_loads
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Notes a text of `identity` not shown for `reason`.
    pub fn refuse(&self, reason: &str, identity: &str) {
        self.refuse_finding(Finding::new(Kind::Refused, identity, reason));
    }

    pub fn refuse_as(&self, kind: Kind, reason: &str, identity: &str) {
        self.refuse_finding(Finding::new(kind, identity, reason));
    }

    pub fn refuse_finding(&self, finding: Finding) {
        note_own(&finding.message, &finding.subject);
        self.refused
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .note(&finding.message, &finding.subject);
        self.note(finding);
    }

    /// The texts refused so far, one warning per reason, and the warnings
    /// of scripted localisation and shared content reads, including lazy reads.
    /// Inside a consumer scope ([`own_refusals`]) these are the consumer's own
    /// refusals and the warnings of the readers it used, in the same form.
    pub fn refusal_lines(&self) -> Vec<String> {
        self.refusal_entries()
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }

    /// Text summaries retain their actual identity count; reader warnings have
    /// no measured affected-item count. Consumers never parse warning prose.
    pub fn refusal_entries(&self) -> Vec<(String, Option<usize>)> {
        let texts = |refused: &Refusals| -> Vec<(String, Option<usize>)> {
            refused
                .counted_diagnostic_lines("texts are not shown")
                .into_iter()
                .map(|(line, count)| (line, Some(count)))
                .collect()
        };
        let mut lines = reporting(|own| texts(&own.texts)).unwrap_or_else(|| {
            texts(
                &self
                    .refused
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()),
            )
        });
        lines.extend(
            self.reader_warning_lines()
                .into_iter()
                .map(|line| (line, None)),
        );
        lines
    }

    /// Shared lazy-reader and uncovered-read warnings without another
    /// product's accumulated text/artwork refusals. Inside a consumer scope,
    /// only the lazy readers the consumer used report, without the stack's
    /// live artwork-effect and uncovered-read diagnostics.
    pub fn reader_warning_lines(&self) -> Vec<String> {
        if let Some(readers) = reporting(|own| own.readers.clone()) {
            return self.used_reader_warning_lines(&readers);
        }
        let mut lines = Vec::new();
        let environment_warnings = if let Some(Ok(scripted)) = self.scripted.get() {
            lines.extend(
                scripted
                    .names
                    .warnings
                    .iter()
                    .map(|(_, message)| message.clone()),
            );
            let environment_warnings = scripted
                .environment
                .get()
                .and_then(|environment| environment.as_ref().ok())
                .map_or(&[][..], |environment| environment.warnings());
            lines.extend(environment_warnings.iter().cloned());
            environment_warnings
        } else {
            &[]
        };
        if let Some(Ok(states)) = self.state_content.get() {
            lines.extend(
                crate::scripted_text::live_state_warnings(&states.warnings)
                    .filter(|warning| {
                        !environment_warnings
                            .iter()
                            .any(|line| line.as_str() == *warning)
                    })
                    .map(str::to_owned),
            );
        }
        lines.extend(self.content.layers.faction_template_diagnostics());
        lines.extend(self.content.layers.shader_warning_lines());
        self.content.layers.read_warnings().append_to(&mut lines);
        lines
    }

    /// [`Self::reader_warning_lines`] of the lazy readers one consumer used,
    /// read now when this process has not read them yet, in the same order.
    /// The stack's artwork-effect and uncovered-read diagnostics are left
    /// out: any consumer's read adds to them at its own pace, so they are
    /// reported where a reply is served
    /// ([`crate::Source::append_stack_warnings`]), never in a consumer's
    /// published lines.
    fn used_reader_warning_lines(&self, readers: &BTreeSet<Reader>) -> Vec<String> {
        let mut lines = Vec::new();
        if readers.contains(&Reader::Scripted)
            && let Ok(scripted) = self.scripted_content()
        {
            lines.extend(
                scripted
                    .names
                    .warnings
                    .iter()
                    .map(|(_, message)| message.clone()),
            );
        }
        if readers.contains(&Reader::States)
            && let Ok(states) = self.cached_states()
        {
            // The scripted environment's state warnings, once it has prepared
            // them, are these same warnings.
            let environment_warnings: &[String] = match self.scripted.get() {
                Some(Ok(scripted)) => scripted
                    .environment
                    .get()
                    .and_then(|environment| environment.as_ref().ok())
                    .map_or(&[][..], |environment| environment.warnings()),
                _ => &[],
            };
            lines.extend(environment_warnings.iter().cloned());
            lines.extend(
                crate::scripted_text::live_state_warnings(&states.warnings)
                    .filter(|warning| {
                        !environment_warnings
                            .iter()
                            .any(|line| line.as_str() == *warning)
                    })
                    .map(str::to_owned),
            );
        }
        if readers.contains(&Reader::FactionTemplates) {
            // Diagnostics report an already-read template set.
            let _ = self.content.layers.faction_templates();
            lines.extend(self.content.layers.faction_template_diagnostics());
        }
        lines
    }

    pub fn note(&self, finding: Finding) {
        if let Some(findings) = &self.findings {
            findings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(finding);
        }
    }

    pub fn take_findings(&self) -> Vec<Finding> {
        self.findings
            .as_ref()
            .map(|findings| {
                std::mem::take(&mut *findings.lock().unwrap_or_else(|e| e.into_inner()))
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A collector owned exclusively by one offline preparation, never by a live source.
    pub fn of_layers_for_check(layers: ResolvedLayers, language: &str) -> io::Result<Self> {
        let mut catalogue = Self::of_layers(layers, language)?;
        catalogue.findings = Some(Mutex::default());
        Ok(catalogue)
    }
}

/// Loads a texture through the shared native GUI resource path. Textures sit
/// outside content hash coverage.
pub fn read_texture(layers: &ResolvedLayers, path: &str, decode: Decode) -> io::Result<Rgba8Image> {
    gui_texture(layers, path.as_bytes(), decode)
}

/// A bare texture's first frame of a horizontal strip (the normal state; a
/// sprite sheet is never stretched), point-sampled down to `max_edge`, as a PNG
/// data URL. Sprite artwork goes through [`sprite_frame_png`].
pub fn texture_png(
    layers: &ResolvedLayers,
    path: &str,
    decode: Decode,
    frames: u32,
    max_edge: u32,
) -> io::Result<String> {
    texture_frame_png(layers, path, decode, frames, 0, max_edge)
}

/// One native frame (zero-based) of a horizontal sprite sheet, as
/// [`texture_png`] reads the first.
pub fn texture_frame_png(
    layers: &ResolvedLayers,
    path: &str,
    decode: Decode,
    frames: u32,
    frame: u32,
    max_edge: u32,
) -> io::Result<String> {
    gui_texture_frame_png(layers, path.as_bytes(), decode, frames, frame, max_edge)
}

/// Native GUI texture loading: `resources::stack_texture` (the resolver, its
/// 16 MiB `load_file` limit, the client decode state and the cube-map refusal),
/// then RGBA8. Raw texture paths and shields use this path; decoded edges are
/// bounded only by the decoder's device limit.
pub fn gui_texture(layers: &ResolvedLayers, name: &[u8], decode: Decode) -> io::Result<Rgba8Image> {
    let texture = resources::stack_texture(layers, name, decode)?.map_err(|why| {
        let extra = if why.native_failure() {
            "; the game draws no texture either"
        } else {
            ""
        };
        invalid(format!(
            "GUI texture {} is unavailable: {why}{extra}",
            String::from_utf8_lossy(name)
        ))
    })?;
    installed_content_image::rgba::dds_base(texture.image)
}

/// One frame of a bare texture's horizontal strip, as [`texture_frame_png`]
/// reads it. A sprite's frame is drawn by [`sprite_frame_png`] instead.
pub fn gui_texture_frame_png(
    layers: &ResolvedLayers,
    name: &[u8],
    decode: Decode,
    frames: u32,
    frame: u32,
    max_edge: u32,
) -> io::Result<String> {
    image_frame_png(&gui_texture(layers, name, decode)?, frames, frame, max_edge)
}

fn image_frame_png(
    image: &Rgba8Image,
    frames: u32,
    frame: u32,
    max_edge: u32,
) -> io::Result<String> {
    if frames == 0 || frame >= frames {
        return Err(invalid("Invalid sprite frames"));
    }
    let plain = Plain {
        frames: frames
            .try_into()
            .map_err(|_| invalid("Invalid sprite frames"))?,
        ..Plain::default()
    };
    png_data_url(&native_frame(image, &plain, frame, max_edge)?)
}

/// A sprite's authored frame at time zero, using native strip geometry and
/// the content's resolved Up program, including overrides of the default effect.
pub fn sprite_frame_png(
    layers: &ResolvedLayers,
    sprite: &installed_content_gfx::gfx::SpriteType,
    decode: Decode,
    frame: u32,
    max_edge: u32,
) -> io::Result<String> {
    let cache = layers.derived::<SpriteImages>();
    cache.get(sprite_image_key(sprite, decode, frame, max_edge), || {
        render_sprite_frame_png(layers, sprite, decode, frame, max_edge)
    })
}

pub fn sprite_image_key(
    sprite: &installed_content_gfx::gfx::SpriteType,
    decode: Decode,
    frame: u32,
    max_edge: u32,
) -> String {
    format!(
        "{:?}/{:?}/{:?}/{frame}/{max_edge}",
        sprite.name, sprite.facts, decode
    )
}

/// The zero-based frame an authored icon `frame` draws in the Up state: `F - 1`
/// for `F > 0`, unclamped (past the strip the effect's sampler reads on), and
/// the first frame for zero or a negative frame.
pub fn authored_frame(frame: i32) -> u32 {
    u32::try_from(frame).map_or(0, |frame| frame.saturating_sub(1))
}

/// Render requested (sprite, frame) draws together into the session's sprite
/// images. Callers retry a refused or oversized batch through
/// [`sprite_frame_png`], preserving each draw's refusal.
pub fn sprite_frames_png(
    layers: &ResolvedLayers,
    requests: &[(&installed_content_gfx::gfx::SpriteType, u32)],
    decode: Decode,
    max_edge: u32,
) -> io::Result<()> {
    use installed_content_core::gui::native::sprites::SpriteKind;
    let mut draws = Vec::new();
    let mut transient_bytes = 0_u64;
    for &(sprite, frame) in requests {
        let (plain, animated) = match &sprite.facts.kind {
            SpriteKind::Plain(plain) => (plain, None),
            SpriteKind::FrameAnimated { plain, animation } => (plain, Some(animation)),
            _ => {
                return Err(invalid(
                    "sprite batch requires plain or frame-animated draws",
                ));
            }
        };
        let draw = sprite::draw(layers, plain, animated, decode, frame, max_edge)?;
        transient_bytes += u64::from(draw.size[0]) * u64::from(draw.size[1]) * 4;
        transient_bytes += draw
            .textures
            .iter()
            .filter_map(|(_, texture)| texture.as_ref())
            .flat_map(|texture| texture.mips.iter().flatten())
            .map(|mip| mip.data.len() as u64)
            .sum::<u64>();
        // Bound temporary batching work; individual draws retain their normal limits.
        if transient_bytes > 8 * 1024 * 1024 {
            return Err(invalid("sprite batch exceeds its temporary byte budget"));
        }
        draws.push(draw);
    }
    let images = shader_pipeline::artwork::render_batch(layers, draws, decode)
        .map_err(invalid)?
        .iter()
        .map(|image| {
            native_frame(image, &Plain::default(), 0, max_edge)
                .and_then(|image| png_data_url(&image))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let cache = layers.derived::<SpriteImages>();
    for (&(sprite, frame), image) in requests.iter().zip(images) {
        cache.get(sprite_image_key(sprite, decode, frame, max_edge), || {
            Ok(image)
        })?;
    }
    Ok(())
}

#[derive(Default)]
struct ImageEntry {
    result: OnceLock<Result<String, String>>,
    charged: std::sync::atomic::AtomicBool,
}

#[derive(Default)]
struct SpriteImageEntries {
    entries: BTreeMap<String, Arc<ImageEntry>>,
    bytes: usize,
}

#[derive(Default)]
pub struct SpriteImages {
    entries: Mutex<SpriteImageEntries>,
}

impl SpriteImages {
    pub fn contains(&self, key: &str) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .contains_key(key)
    }

    fn get(&self, key: String, render: impl FnOnce() -> io::Result<String>) -> io::Result<String> {
        let slot = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            Arc::clone(entries.entries.entry(key.clone()).or_default())
        };
        let result = slot
            .result
            .get_or_init(|| render().map_err(|error| error.to_string()))
            .clone();
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if entries
            .entries
            .get(&key)
            .is_some_and(|entry| Arc::ptr_eq(entry, &slot))
            && !slot
                .charged
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            entries.bytes += key.len()
                + match &result {
                    Ok(image) => image.len(),
                    Err(reason) => reason.len(),
                };
        }
        if entries.bytes > MAX_RETAINED_ARTWORK_BYTES {
            entries
                .entries
                .retain(|_, entry| entry.result.get().is_none());
            entries.bytes = 0;
        }
        result.map_err(invalid)
    }
}

fn render_sprite_frame_png(
    layers: &ResolvedLayers,
    sprite: &installed_content_gfx::gfx::SpriteType,
    decode: Decode,
    frame: u32,
    max_edge: u32,
) -> io::Result<String> {
    use installed_content_core::gui::native::sprites::SpriteKind;
    let (plain, animated) = match &sprite.facts.kind {
        SpriteKind::Plain(plain) => (plain, None),
        // Frame-animated sprites share the plain sprite facts and add
        // their authored frame animation.
        SpriteKind::FrameAnimated { plain, animation } => (plain, Some(animation)),
        _ => {
            return Err(invalid(format!(
                "sprite {}: {} needs native sprite rendering",
                sprite.name,
                sprite.facts.kind.keyword()
            )));
        }
    };
    // Animation initialization supplies the default effect path;
    // drawing binds that stack-resolved program and its samplers.
    sprite::frame(layers, plain, animated, decode, frame, max_edge)
        .and_then(|image| png_data_url(&image))
        .map_err(|error| invalid(format!("sprite {}: {error}", sprite.name)))
}

mod shield;
mod sprite;
pub use shield::MaskedShield;

/// One frame of a plain sprite's texture, point-sampled down to `max_edge`.
/// The query uses the integer extent while geometry steps frames by the
/// fractional pitch. Both divide the padded texture extent.
fn native_frame(
    image: &Rgba8Image,
    plain: &Plain,
    frame: u32,
    max_edge: u32,
) -> io::Result<Rgba8Image> {
    debug_assert!(frame < plain.frames.max(0) as u32);
    let texture = [image.width, image.height];
    let size = resources::frame_extent(plain, texture);
    let pitch = resources::frame_geometry(plain, texture);
    if max_edge == 0 || size.iter().any(|edge| *edge <= 0) {
        return Err(invalid(
            "Invalid sprite frames: the queried frame has no pixels",
        ));
    }
    let offset = if plain.frame_direction_y() {
        [0.0, frame as f32 * pitch[1]]
    } else {
        [frame as f32 * pitch[0], 0.0]
    };
    let size = size.map(|edge| edge as u32);
    let scale = (f64::from(size[0].max(size[1])) / f64::from(max_edge)).max(1.0);
    let width = (f64::from(size[0]) / scale).round().max(1.0) as u32;
    let height = (f64::from(size[1]) / scale).round().max(1.0) as u32;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let px = (offset[0] + (x as f32 + 0.5) * pitch[0] / width as f32) as u32;
            let py = (offset[1] + (y as f32 + 0.5) * pitch[1] / height as f32) as u32;
            let at =
                ((py.min(image.height - 1) * image.width + px.min(image.width - 1)) * 4) as usize;
            pixels.extend_from_slice(&image.pixels[at..at + 4]);
        }
    }
    Ok(Rgba8Image {
        width,
        height,
        pixels,
    })
}

/// Straight-alpha RGBA as a PNG data URL, the form every image reply uses.
pub fn png_data_url(image: &Rgba8Image) -> io::Result<String> {
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&image.pixels)?;
    }
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(encoded)
    ))
}

#[cfg(any(test, feature = "test-support"))]
pub mod texture_tests;

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;

    #[test]
    fn keyed_results_resolve_new_keys_once_and_share_success() {
        let cache = KeyedResults::default();
        for key in ["authored", "later"] {
            let key = key.to_owned();
            assert_eq!(cache.loads(&key), 0);
            let first = cache
                .get_or_init(&key, |spelling| Ok(format!("class/{spelling}")))
                .unwrap();
            for _ in 0..10 {
                let held = cache
                    .get_or_init(&key, |_| panic!("classified the same operand twice"))
                    .unwrap();
                assert!(Arc::ptr_eq(&first, &held));
                assert_eq!(*held, format!("class/{key}"));
            }
            assert_eq!(cache.loads(&key), 1);
        }
    }

    #[test]
    fn keyed_results_retain_named_failures_and_native_os_errors() {
        let cache = KeyedResults::<String, String>::default();
        let key = "mod/refused".to_owned();
        let error = cache
            .get_or_init(&key, |spelling| {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{spelling}: unsupported authored projection"),
                ))
            })
            .unwrap_err();
        for _ in 0..10 {
            let held = cache
                .get_or_init(&key, |_| panic!("retried a fixed refusal"))
                .unwrap_err();
            assert_eq!(held.kind(), error.kind());
            assert_eq!(held.to_string(), error.to_string());
        }
        assert_eq!(cache.loads(&key), 1);

        let key = "unreadable".to_owned();
        let error = cache
            .get_or_init(&key, |_| Err(io::Error::from_raw_os_error(2)))
            .unwrap_err();
        let held = cache
            .get_or_init(&key, |_| panic!("retried an OS refusal"))
            .unwrap_err();
        assert_eq!(held.raw_os_error(), error.raw_os_error());
        assert_eq!(held.kind(), error.kind());
        assert_eq!(held.to_string(), error.to_string());
        assert_eq!(cache.loads(&key), 1);
    }

    #[test]
    fn keyed_results_retry_interrupted_reads_without_evicting_other_keys() {
        let cache = KeyedResults::default();
        let held_key = "held".to_owned();
        let held = cache.get_or_init(&held_key, |_| Ok(7)).unwrap();
        let key = "cancelled".to_owned();
        for _ in 0..2 {
            assert_eq!(
                cache
                    .get_or_init(&key, |_| Err(io::Error::from(io::ErrorKind::Interrupted)))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::Interrupted
            );
        }
        assert_eq!(*cache.get_or_init(&key, |_| Ok(11)).unwrap(), 11);
        assert_eq!(
            *cache.get_or_init(&key, |_| panic!("reloaded")).unwrap(),
            11
        );
        assert_eq!(cache.loads(&key), 3);
        assert!(Arc::ptr_eq(
            &held,
            &cache
                .get_or_init(&held_key, |_| panic!("evicted another key"))
                .unwrap()
        ));
        assert_eq!(cache.loads(&held_key), 1);
    }

    #[test]
    fn keyed_results_serialise_concurrent_first_access_per_key() {
        let cache = KeyedResults::default();
        let barrier = std::sync::Barrier::new(16);
        let rows = std::thread::scope(|threads| {
            let pending: Vec<_> = (0..16)
                .map(|index| {
                    let cache = &cache;
                    let barrier = &barrier;
                    threads.spawn(move || {
                        let key = if index % 2 == 0 { "known" } else { "refused" }.to_owned();
                        barrier.wait();
                        let result = cache.get_or_init(&key, |spelling| {
                            if spelling == "known" {
                                Ok("authored class".to_owned())
                            } else {
                                Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "refused by name",
                                ))
                            }
                        });
                        (key, result)
                    })
                })
                .collect();
            pending
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        let first = Arc::clone(
            rows.iter()
                .find_map(|(_, result)| result.as_ref().ok())
                .unwrap(),
        );
        for (key, result) in rows {
            match result {
                Ok(row) => assert!(Arc::ptr_eq(&first, &row)),
                Err(error) => {
                    assert_eq!(key, "refused");
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                    assert_eq!(error.to_string(), "refused by name");
                }
            }
        }
        assert_eq!(cache.loads(&"known".to_owned()), 1);
        assert_eq!(cache.loads(&"refused".to_owned()), 1);
    }

    #[test]
    fn keyed_results_share_a_recorded_stack_and_reset_on_a_new_binding() {
        let (first, _first_folder) = folder_catalogue(&[]);
        let (second, _second_folder) = folder_catalogue(&[]);
        let cache = first
            .content
            .layers
            .derived::<KeyedResults<String, String>>();
        let cloned = first
            .content
            .layers
            .clone()
            .derived::<KeyedResults<String, String>>();
        let other = second
            .content
            .layers
            .derived::<KeyedResults<String, String>>();
        let key = "same recorded spelling".to_owned();
        let original = cache.get_or_init(&key, |_| Ok("first".into())).unwrap();
        assert!(Arc::ptr_eq(
            &original,
            &cloned
                .get_or_init(&key, |_| panic!("reclassified within the same stack"))
                .unwrap()
        ));
        assert_eq!(
            *other.get_or_init(&key, |_| Ok("second".into())).unwrap(),
            "second"
        );
        assert_eq!(
            (cache.loads(&key), cloned.loads(&key), other.loads(&key)),
            (1, 1, 1)
        );
    }

    #[test]
    fn content_results_share_success_and_failure_but_retry_cancellation() {
        let cache = Arc::new(CachedResult::default());
        let rows = std::thread::scope(|threads| {
            let pending: Vec<_> = (0..16)
                .map(|_| {
                    let cache = Arc::clone(&cache);
                    threads.spawn(move || {
                        cache
                            .get_or_init(|| Ok(vec!["mod/token".to_owned()]))
                            .unwrap()
                    })
                })
                .collect();
            pending
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(cache.loads(), 1);
        assert!(rows.iter().all(|row| Arc::ptr_eq(row, &rows[0])));

        let refused = CachedResult::<Vec<String>>::default();
        for _ in 0..20 {
            let error = refused
                .get_or_init(|| {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "mod definition refused by name",
                    ))
                })
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), "mod definition refused by name");
        }
        assert_eq!(refused.loads(), 1);
        let cancelled = CachedResult::default();
        assert_eq!(
            cancelled
                .get_or_init(|| Err::<Vec<String>, _>(io::Error::from(io::ErrorKind::Interrupted)))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        assert_eq!(
            *cancelled
                .get_or_init(|| Ok(vec!["after cancellation".to_owned()]))
                .unwrap(),
            ["after cancellation"]
        );
        assert_eq!(cancelled.loads(), 2);
    }

    #[test]
    fn content_result_binding_shares_clones_and_resets_for_a_new_stack() {
        let (first, _first_folder) = folder_catalogue(&[]);
        let (second, _second_folder) = folder_catalogue(&[]);
        let first_cache = first.content.layers.derived::<CachedResult<Vec<String>>>();
        let clone_cache = first
            .content
            .layers
            .clone()
            .derived::<CachedResult<Vec<String>>>();
        let second_cache = second.content.layers.derived::<CachedResult<Vec<String>>>();
        let original = first_cache
            .get_or_init(|| Ok(vec!["first".into()]))
            .unwrap();
        assert!(Arc::ptr_eq(
            &original,
            &clone_cache
                .get_or_init(|| panic!("shared stack reparsed"))
                .unwrap()
        ));
        assert_eq!(
            *second_cache
                .get_or_init(|| Ok(vec!["second".into()]))
                .unwrap(),
            ["second"]
        );
        assert_eq!(
            (
                first_cache.loads(),
                clone_cache.loads(),
                second_cache.loads()
            ),
            (1, 1, 1)
        );
    }

    #[test]
    fn presentation_titles_distinguish_missing_empty_and_authored_key_text() {
        let (catalogue, _folder) = folder_catalogue(&[(
            "localisation/titles_l_english.yml",
            "\u{feff}l_english:\n NAMED:0 \"Authored title\"\n EMPTY:0 \"\"\n SAME:0 \"SAME\"\n",
        )]);
        assert_eq!(
            catalogue.localised("NAMED", View::Plain).as_deref(),
            Some("Authored title")
        );
        assert_eq!(
            catalogue.localised("EMPTY", View::Plain).as_deref(),
            Some("")
        );
        assert_eq!(
            catalogue.localised("SAME", View::Plain).as_deref(),
            Some("SAME")
        );
        assert_eq!(catalogue.localised("MISSING", View::Plain), None);
        assert_eq!(catalogue.localised("", View::Plain).as_deref(), Some(""));
        assert_eq!(
            catalogue.localised("NAMED\0suffix", View::Plain).as_deref(),
            Some("Authored title")
        );
        assert_eq!(
            localised(&catalogue.localisation, "MISSING", View::Plain).unwrap(),
            "MISSING"
        );
    }

    #[test]
    fn missing_titles_name_their_refusal_in_the_callers_reply() {
        let (catalogue, _folder) = folder_catalogue(&[]);
        let refusals = Mutex::new(Refusals::default());
        let before = catalogue.refusal_lines();
        for _ in 0..2 {
            assert_eq!(
                catalogue.localised_with_refusals(
                    "mod_ideology\0suffix",
                    &[],
                    View::Plain,
                    &refusals,
                ),
                None
            );
        }
        let expected = "localisation mod_ideology is missing; title is unavailable";
        let caller = refusals.lock().unwrap().counted_diagnostic_lines("titles");
        assert_eq!(caller.len(), 1);
        assert!(caller[0].0.contains(expected));
        assert_eq!(caller[0].1, 1);
        // The reply owns the reason; the shared catalogue does not grow with each missing key.
        assert_eq!(catalogue.refusal_lines(), before);
    }

    #[test]
    fn saved_name_bytes_follow_native_nul_and_preserve_empty_authored_text() {
        let localisation = loaded("l_english:\n KEY:0 \"Native name\"\n EMPTY:0 \"\"\n");
        assert_eq!(
            localised_bytes(&localisation, b"KEY\0\xff", View::Plain).unwrap(),
            "Native name"
        );
        assert_eq!(
            localised_bytes(&localisation, b"EMPTY", View::Plain).unwrap(),
            ""
        );
        assert_eq!(
            localised_bytes(&localisation, b"\0\xff", View::Plain).unwrap(),
            ""
        );
        assert!(localised_bytes(&localisation, b"\xff", View::Plain).is_err());
        assert_eq!(
            localised_bytes(&localisation, b"[ROOT.GetName]", View::Plain).unwrap(),
            "[ROOT.GetName]"
        );
        assert!(localised_bytes(&localisation, b"[ROOT.GetName]", View::Scripted).is_err());
    }

    #[test]
    fn gui_sprite_binding_uses_native_inheritance_and_kind_local_names() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use installed_content_core::gui::native;
        let gui = native::read(
            br#"guiTypes = { containerWindowType = {
            name = topbar
            buttonType = { name = base spriteType = "GFX_inherited" }
            containerWindowType = { name = nested
                buttonType = { name = player_flag spriteType = "GFX_wrong_window" }
            }
            buttonType = { name = player_flag parent = base }
        } }"#,
        );
        let guis = native::Guis::new(vec![("interface/topbar.gui".into(), gui)]);
        let mut warnings = Vec::new();
        assert_eq!(
            installed_content_core::gui::native::window_button_sprite(
                &guis,
                "TOPBAR",
                "PLAYER_FLAG",
                &mut warnings
            )
            .unwrap(),
            "GFX_inherited"
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_missing_gui_sprite_uses_the_native_fallback_but_a_refused_one_does_not() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let facts = installed_content_core::gui::native::sprites::read_sprites(
            br#"spriteTypes = {
            spriteType = { name = GFX_default_fallback_texture texturefile = fallback.dds }
            spriteType = { name = refused noOfFrames = -1 }
        }"#,
        );
        let sprites = SpriteSet::new(&[("interface/test.gfx".to_owned(), facts)]);
        assert_eq!(
            created_gui_sprite(&sprites, "absent").unwrap().name,
            "GFX_default_fallback_texture"
        );
        assert!(
            created_gui_sprite(&sprites, "refused")
                .unwrap_err()
                .to_string()
                .contains("noOfFrames")
        );
        let why = created_gui_sprite(&SpriteSet::default(), "missing")
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("missing") && why.contains("GFX_default_fallback_texture"),
            "{why}"
        );
    }

    #[test]
    fn text_icon_sprite_selects_native_prefix_then_bare_and_keeps_named_refusal() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let facts = installed_content_core::gui::native::sprites::read_sprites(
            br#"spriteTypes = {
                spriteType = { name = GFX_mod_alias texturefile = prefixed.dds }
                spriteType = { name = mod_alias texturefile = bare.dds }
                spriteType = { name = bare_only texturefile = bare_only.dds }
                spriteType = { name = GFX_refused noOfFrames = -1 }
                spriteType = { name = refused texturefile = must_not_stand_in.dds }
            }"#,
        );
        let sprites = SpriteSet::new(&[("interface/synthetic.gfx".to_owned(), facts)]);
        assert_eq!(
            text_icon_sprite(&sprites, "mod_alias").unwrap().name,
            "GFX_mod_alias"
        );
        assert_eq!(
            text_icon_sprite(&sprites, "bare_only").unwrap().name,
            "bare_only"
        );
        assert!(
            text_icon_sprite(&sprites, "refused")
                .unwrap_err()
                .to_string()
                .contains("noOfFrames")
        );
        assert!(text_icon_sprite(&sprites, "missing").is_err());
    }

    #[test]
    fn gui_artwork_uses_the_texture_resolver_and_client_power_of_two_setting() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (_, folder) = folder_catalogue(&[("gfx/interface/present.txt", "")]);
        let mut tga = vec![0; 18];
        tga[2] = 2;
        tga[12] = 3;
        tga[14] = 1;
        tga[16] = 32;
        tga[17] = 0x28;
        tga.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255]);
        std::fs::write(folder.0.join("gfx/interface/icon.tga"), tga).unwrap();
        let layers = ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
        // The resolver tries icon.dds, icon.tga, then the supplied name.
        let image = gui_texture(
            &layers,
            b"gfx/interface/icon.png",
            installed_content_image::native::Decode {
                force_pow2_textures: true,
            },
        )
        .unwrap();
        assert_eq!((image.width, image.height), (4, 1));
        // Power-of-two padding fills the FreeImage RGB format with opaque black.
        assert_eq!(&image.pixels[12..16], &[0, 0, 0, 255]);
    }

    #[cfg(test)]
    fn fixture() -> Localisation {
        loaded(
            &(String::from(concat!(
                "l_english:\n",
                " NAME:0 \"§YReal name§!\\nSecond line\"\n",
                " NESTED:0 \"Leader of $NAME$\"\n",
                " SCOPED:0 \"[ROOT.GetName]\"\n",
                " BROKEN:0 \"a$MISSING$b\"\n",
                " ICON:0 \"£pol_power 100§§ @GER£x\"\n",
                " GAP:0 \"a¤b\"\n",
                " BRACKET:0 \"A]B\"\n",
                " OPEN:0 \"A[B\"\n",
                " PRICE:0 \"Price $$5\"\n",
                " LONG:0 \"",
            )) + &"é".repeat(0xC00)
                + "x\"\n"),
        )
    }

    /// Localisation as the game content loads it: `yml` is one file of a
    /// folder layer, read by `installed_content_localisation::localisation::read`.
    pub fn loaded(yml: &str) -> Localisation {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "hoi4_dashboard_localisation_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _folder = Folder(root.clone());
        let file = root.join("localisation/test_l_english.yml");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, format!("\u{feff}{yml}")).unwrap();
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        installed_content_localisation::localisation::read(&layers, "english").unwrap()
    }

    /// A faction catalogue through the production reader, without a second parser.
    pub fn loaded_templates(text: &str) -> Arc<installed_content_core::faction_templates::Catalog> {
        let (catalogue, _folder) =
            folder_catalogue(&[("common/factions/templates/test.txt", text)]);
        catalogue.content.layers.faction_templates().unwrap()
    }

    /// A content folder written for a test, removed when dropped.
    pub struct Folder(pub std::path::PathBuf);

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A catalogue over one content folder holding `files` (path, text), read
    /// by the same readers the dashboard's catalogue loader uses, in English. The folder
    /// lives as long as the returned guard.
    pub fn folder_catalogue(files: &[(&str, &str)]) -> (Catalogue, Folder) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "hoi4_dashboard_catalogue_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let folder = Folder(root.clone());
        // A content folder must exist even when it holds no files.
        std::fs::create_dir_all(&root).unwrap();
        if files.iter().any(|(path, _)| path.ends_with(".gfx")) {
            super::texture_tests::write_sprite_program(&root);
        }
        for (path, text) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        (layers_catalogue(layers), folder)
    }

    /// The same catalogue seam for fixtures with several content layers.
    pub fn layers_catalogue(layers: ResolvedLayers) -> Catalogue {
        let localisation = installed_content_localisation::localisation::read(&layers, "english").unwrap();
        let sprites = installed_content_gfx::gfx::read(&layers).unwrap();
        Catalogue {
            content: Arc::new(ResolvedContent {
                layers,
                stats: Default::default(),
            }),
            localisation: Arc::new(localisation),
            sprites,
            guis: OnceLock::new(),
            gui_decode: Ok(Default::default()),
            verification: Vec::new(),
            warnings: Vec::new(),
            recorded_language: "english".into(),
            refused: Mutex::default(),
            findings: None,
            scripted: OnceLock::new(),
            war_names: OnceLock::new(),
            state_content: Arc::new(OnceLock::new()),
            content_loads: Default::default(),
        }
    }

    #[test]
    fn participation_factors_follow_the_evaluated_military_define_stack() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (_, base) = folder_catalogue(&[(
            "common/defines/00_base.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NMilitary={
                WAR_SCORE_LEND_LEASE_GIVEN_IC_FACTOR=1.25,
                WAR_SCORE_LEND_LEASE_GIVEN_FUEL_FACTOR=0.5,
                WAR_SCORE_LEND_LEASE_RECEIVED_IC_FACTOR=2.5,
                WAR_SCORE_LEND_LEASE_RECEIVED_FUEL_FACTOR=0.25}}",
        )]);
        let (_, overlay) = folder_catalogue(&[(
            "common/defines/99_override.lua",
            "NDefines.NMilitary.WAR_SCORE_LEND_LEASE_GIVEN_IC_FACTOR =
                NDefines.NMilitary.WAR_SCORE_LEND_LEASE_GIVEN_IC_FACTOR * 2
             NDefines.NMilitary.WAR_SCORE_LEND_LEASE_RECEIVED_FUEL_FACTOR = -0.125",
        )]);
        let layers =
            ResolvedLayers::from_folders(&[(&base.0, &[][..]), (&overlay.0, &[][..])]).unwrap();
        let catalogue = layers_catalogue(layers);
        assert_eq!(
            catalogue.war_participation_factors().unwrap(),
            replay_data::war_participation::Factors {
                industry_given: 250_000,
                fuel_given: 50_000,
                industry_received: 250_000,
                fuel_received: -12_500,
            }
        );
    }

    #[test]
    fn faction_templates_share_one_cache_and_report_each_warning_once() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (catalogue, _folder) = folder_catalogue(&[
            (
                "common/factions/templates/test.txt",
                "bad={name=token:missing_name visible=yes}",
            ),
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
            ),
        ]);
        assert!(catalogue.refusal_lines().is_empty());
        let templates = catalogue.content.layers.faction_templates().unwrap();
        let scripted = catalogue.scripted_content().unwrap();
        let mut frame = crate::test_fixtures::source(1, 100, Some(1));
        *frame.tables[replay_data::rows::TableId::Factions.index()] =
            replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Factions(vec![
                replay_data::rows::FactionRow {
                    id: 5,
                    template_token: Some(17),
                    ..Default::default()
                },
            ]));
        *frame.tables[replay_data::rows::TableId::LexicalTokens.index()] =
            replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::LexicalTokens(
                vec![replay_data::rows::LexicalTokenRow {
                    token: 17,
                    literal: b"bad".to_vec(),
                }],
            ));
        let at =
            crate::scripted_text::ScriptedText::new(&frame, &catalogue.localisation, scripted, 0);
        assert!(
            at.faction_name_by_id(5)
                .unwrap_err()
                .contains("token missing_name")
        );
        let shared = scripted.layers.layers.faction_templates().unwrap();
        assert!(Arc::ptr_eq(&templates, &shared));
        assert!(!templates.warnings.is_empty());
        let lines = catalogue.refusal_lines();
        for warning in &templates.warnings {
            assert_eq!(lines.iter().filter(|line| *line == warning).count(), 1);
        }
    }

    /// Stored controls: colour codes and inline images leave the plain text,
    /// a doubled `§` is the character, and a spacing control is refused.
    #[test]
    fn plain_text_reads_the_stored_controls() {
        let loc = fixture();
        assert_eq!(localised(&loc, "ICON", View::Plain), Ok("100§ ".into()));
        assert_eq!(
            localised(&loc, "GAP", View::Plain),
            Err(
                "localisation GAP holds the control byte 0x14 (an authored `¤`), which plain text cannot show".into()
            )
        );
        assert_eq!(localised(&loc, "GAP", View::Plain).ok(), None);
    }

    #[test]
    fn localised_text_removes_colour_and_expands_references() {
        let loc = fixture();
        assert_eq!(
            localised(&loc, "NAME", View::Plain),
            Ok("Real name\nSecond line".into())
        );
        assert_eq!(
            localised(&loc, "NESTED", View::Plain),
            Ok("Leader of Real name\nSecond line".into())
        );
    }

    /// The shared localiser's rules, not a private engine's: `$$` is one `$`,
    /// a nested key the content lacks writes nothing, a `]` alone is a
    /// glyph, and the 0x1800-byte buffer cuts a long text.
    #[test]
    fn text_follows_the_games_localiser() {
        let loc = fixture();
        assert_eq!(localised(&loc, "PRICE", View::Plain), Ok("Price $5".into()));
        assert_eq!(localised(&loc, "BROKEN", View::Plain), Ok("ab".into()));
        assert_eq!(localised(&loc, "BRACKET", View::Plain), Ok("A]B".into()));
        // 0xC00 two-byte characters fill the buffer exactly; the `x` is cut.
        assert_eq!(localised(&loc, "LONG", View::Plain), Ok("é".repeat(0xC00)));
        // A key the content lacks is copied as its bytes.
        assert_eq!(
            localised(&loc, "Not a key", View::Plain).as_deref(),
            Ok("Not a key")
        );
        assert_eq!(
            localised(&loc, "ABSENT", View::Plain).as_deref(),
            Ok("ABSENT")
        );
    }

    /// Scripted views evaluate `[...]`: a plain view shows the
    /// brackets as glyphs; a scripted view needs a scope for a closed
    /// expression and drops the text at an unclosed `[`.
    #[test]
    fn brackets_follow_the_view() {
        let loc = fixture();
        assert_eq!(
            localised(&loc, "SCOPED", View::Plain),
            Ok("[ROOT.GetName]".into())
        );
        assert_eq!(localised(&loc, "OPEN", View::Plain), Ok("A[B".into()));
        // Without the country it is shown for, a closed expression is refused.
        assert_eq!(
            localised(&loc, "SCOPED", View::Scripted),
            Err(
                "localisation SCOPED holds the scripted expression \"[ROOT.GetName]\", which the game evaluates for the country the text is shown for, and this product does not pass that country".into()
            )
        );
        assert_eq!(localised(&loc, "OPEN", View::Scripted), Ok(String::new()));
        assert_eq!(localised(&loc, "BRACKET", View::Scripted), Ok("A]B".into()));
    }

    #[test]
    fn plain_and_recorded_text_share_recovery_and_refusal_kinds() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (catalogue, _folder) = folder_catalogue(&[
            (
                "localisation/mod_l_english.yml",
                concat!(
                    "\u{feff}l_english:\n",
                    " BROKEN:0 \"a$MISSING$b\"\n",
                    " GAP:0 \"a¤b\"\n",
                    " CUT:0 \"$VALUE$\"\n",
                    " SCOPED:0 \"§Y[ROOT.GetTag]§!\"\n",
                ),
            ),
            (
                "common/defines/mod.lua",
                "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
            ),
        ]);
        let frame = crate::scope::tests::positive_frame();
        let scripted = crate::scripted_text::ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        let cut = format!("a{}", "é".repeat(0xC00));
        let arguments = [("VALUE", Argument::Text(&cut))];
        for (key, refusal) in [
            ("ABSENT", None),
            ("BROKEN", None),
            ("GAP", Some(Kind::Unsupported)),
            ("CUT", Some(Kind::Refused)),
        ] {
            let plain = localised_with_reason(
                &catalogue.localisation,
                key,
                &arguments,
                View::Plain,
                Kind::Unsupported,
            );
            let recorded =
                localised_in_details(&catalogue.localisation, key, &arguments, &scripted);
            assert_eq!(plain, recorded, "{key}");
            assert_eq!(plain.0.as_ref().err().map(|f| f.kind), refusal, "{key}");
            assert!(plain.1.iter().all(|f| f.kind == Kind::Content), "{key}");
            if matches!(key, "ABSENT" | "BROKEN") {
                assert!(!plain.1.is_empty(), "{key}");
            }
        }
        assert_eq!(
            localised_in_details(&catalogue.localisation, "SCOPED", &[], &scripted),
            (Ok("GER".into()), Vec::new())
        );
        for kind in [Kind::Unsupported, Kind::Offline] {
            let (text, findings) =
                localised_details(&catalogue.localisation, "SCOPED", &[], View::Scripted, kind);
            assert_eq!(text, None);
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].kind, kind);
            assert_eq!(findings[0].subject, "SCOPED");
        }
        assert_eq!(catalogue.localised("GAP", View::Plain), None);
        let warnings = catalogue.refusal_lines();
        assert_eq!(warnings.len(), 1);
        assert_eq!(catalogue.localise_in("GAP", &scripted), None);
        assert_eq!(catalogue.refusal_lines(), warnings);
        assert!(catalogue.take_findings().is_empty());
    }

    #[test]
    fn a_refusal_is_counted_once_per_identity() {
        let mut refusals = Refusals::default();
        refusals.note("why", "B");
        refusals.note("why", "A");
        refusals.note("why", "A");
        assert_eq!(refusals.counts("names are"), ["2 names are not shown: why"]);
        assert_eq!(
            refusals.lines("names are"),
            ["2 names are not shown: why; A, B"]
        );
        assert_eq!(
            refusals.counted_diagnostic_lines("names are not shown"),
            [("2 names are not shown: why; A, B".into(), 2)]
        );
        assert_eq!(
            refusals.diagnostic_lines("retained values affected"),
            ["2 retained values affected: why; A, B"]
        );
        assert_eq!(
            refusals.counted_diagnostic_lines("retained values affected"),
            [("2 retained values affected: why; A, B".into(), 2)]
        );
        refusals.note("another reason", "C");
        assert_eq!(
            refusals
                .counted_diagnostic_lines("names are not shown")
                .iter()
                .map(|(_, count)| count)
                .sum::<usize>(),
            3
        );
    }

    #[test]
    fn projected_refusal_sets_merge_without_duplicate_identities() {
        let mut first = Refusals::default();
        first.note("missing operand", "A");
        let mut next = Refusals::default();
        next.note("missing operand", "A");
        next.note("missing operand", "B");
        next.note("refused content", "C");
        let next: Refusals = serde_json::from_slice(&serde_json::to_vec(&next).unwrap()).unwrap();
        first.merge(&next);
        first.merge(&next);
        assert_eq!(
            first.lines("Project texts"),
            [
                "2 Project texts not shown: missing operand; A, B",
                "1 Project texts not shown: refused content; C",
            ]
        );
    }

    #[test]
    fn bounded_refusals_deduplicate_and_bound_escaped_warning_output() {
        let mut refused = BoundedRefusals::new(1024);
        refused.note("Unsupported \"mod\" form", "operative \"one\"\nfield");
        let first = refused.check().unwrap();
        refused.note("Unsupported \"mod\" form", "operative \"one\"\nfield");
        assert_eq!(refused.check().unwrap(), first);
        refused.note("Unsupported \"mod\" form", "operative two field");
        let lines = refused.lines("operative fields");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("2 operative fields not shown:"));
        assert!(lines[0].contains("operative two field"));
        assert!(crate::transport::json_len(&lines, refused.check().unwrap()).is_some());
    }

    #[test]
    fn bounded_refusals_overflow_refuses_and_stops_storage_growth() {
        let mut refused = BoundedRefusals::new(1024);
        refused.note("Known refusal", "operative one field");
        let before = refused.bytes;
        let names = refused.refused.0.clone();
        refused.bytes = refused.budget - 1;
        refused.note("A new refusal", "operative two field");
        assert!(refused.check().is_err());
        assert_eq!(refused.refused.0, names);
        assert_eq!(refused.bytes, refused.budget - 1);
        // Into<String> is never called once the overflow flag is set.
        struct NoConversion;
        impl From<NoConversion> for String {
            fn from(_: NoConversion) -> Self {
                panic!("overflowed refusal must not allocate another note")
            }
        }
        refused.note(NoConversion, NoConversion);
        assert!(refused.check().is_err());
        assert!(before > 0);
    }

    #[test]
    fn catalogue_refusal_entries_keep_shared_read_warnings_uncounted() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (catalogue, _folder) =
            folder_catalogue(&[("outside_policy/synthetic.txt", "independent fixture")]);
        let reads = catalogue.content.layers.report_uncovered_reads();
        catalogue
            .content
            .layers
            .read("outside_policy/synthetic.txt")
            .unwrap();
        catalogue.refuse("synthetic reason", "synthetic name");
        let entries = catalogue.refusal_entries();
        assert_eq!(
            entries,
            [
                (
                    "1 texts are not shown: synthetic reason; synthetic name".into(),
                    Some(1)
                ),
                (reads.warning().unwrap(), None),
            ]
        );
        assert_eq!(
            catalogue.refusal_lines(),
            entries
                .into_iter()
                .map(|(line, _)| line)
                .collect::<Vec<_>>()
        );
        assert_eq!(catalogue.reader_warning_lines(), [reads.warning().unwrap()]);
    }

    /// A consumer's scope reports its own refusals in the shared tally's form;
    /// another consumer's refusals, before or after, stay out, and the tally a
    /// consumer saved reports the same when restored.
    #[test]
    fn own_refusal_scopes_report_only_their_own_refusals_and_survive_saving() {
        let (catalogue, _folder) = folder_catalogue(&[(
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n GAP:0 \"a¤b\"\n OTHER:0 \"c¤d\"\n",
        )]);
        assert_eq!(catalogue.localised("OTHER", View::Plain), None);
        let mut own = OwnRefusals::default();
        let lines = own_refusals(&mut own, || {
            assert_eq!(catalogue.localised("GAP", View::Plain), None);
            catalogue.refusal_lines()
        });
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("1 texts are not shown: ") && lines[0].ends_with("; GAP"),
            "{lines:?}"
        );
        catalogue.refuse("later reason", "LATER");
        // The shared tally holds every consumer's refusals.
        let shared = catalogue.refusal_lines();
        assert_eq!(shared.len(), 3, "{shared:?}");
        let saved: OwnRefusals =
            serde_json::from_slice(&serde_json::to_vec(&own).unwrap()).unwrap();
        for mut restored in [own, saved] {
            assert_eq!(
                own_refusals(&mut restored, || catalogue.refusal_lines()),
                lines
            );
        }
        assert_eq!(catalogue.refusal_lines(), shared);
    }

    /// A cached result's read refused once, for whichever consumer read first;
    /// every consumer that uses the result reports the refusal.
    #[test]
    fn cached_results_credit_their_read_refusals_to_every_consumer() {
        let (catalogue, _folder) = folder_catalogue(&[(
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n GAP:0 \"a¤b\"\n",
        )]);
        let cache = KeyedResults::<String, Option<String>>::default();
        let key = "GAP".to_owned();
        let read = |consumer: &mut OwnRefusals| {
            own_refusals(consumer, || {
                cache
                    .get_or_init(&key, |key| Ok(catalogue.localised(key, View::Plain)))
                    .unwrap();
                catalogue.refusal_lines()
            })
        };
        let (mut first, mut second) = (OwnRefusals::default(), OwnRefusals::default());
        let lines = read(&mut first);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(read(&mut second), lines);
        assert_eq!(
            cache.loads(&key),
            1,
            "the second consumer used the cached result"
        );
        assert_eq!(first, second);
    }

    /// Inside a scope, lazy reader warnings report only for readers the
    /// consumer used, read again on a stack that has not read them yet.
    #[test]
    fn own_scopes_report_the_warnings_of_the_readers_they_used() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let files = [(
            "common/factions/templates/test.txt",
            "bad={name=token:missing_name visible=yes}",
        )];
        let (catalogue, _folder) = folder_catalogue(&files);
        // Another consumer reads the templates.
        let templates = catalogue.content.layers.faction_templates().unwrap();
        assert!(!templates.warnings.is_empty());
        assert_eq!(catalogue.reader_warning_lines(), templates.warnings);
        let mut unused = OwnRefusals::default();
        assert!(own_refusals(&mut unused, || catalogue.refusal_lines()).is_empty());
        let mut user = OwnRefusals::default();
        let lines = own_refusals(&mut user, || {
            used(Reader::FactionTemplates);
            catalogue.refusal_lines()
        });
        assert_eq!(lines, templates.warnings);
        let (fresh, _fresh_folder) = folder_catalogue(&files);
        let mut restored: OwnRefusals =
            serde_json::from_slice(&serde_json::to_vec(&user).unwrap()).unwrap();
        assert_eq!(
            own_refusals(&mut restored, || fresh.reader_warning_lines()),
            templates.warnings
        );
    }

    #[test]
    fn an_unwinding_scope_keeps_its_refusals_and_leaves_no_scope_behind() {
        let (catalogue, _folder) = folder_catalogue(&[]);
        let mut own = OwnRefusals::default();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            own_refusals::<()>(&mut own, || {
                catalogue.refuse("before the panic", "UNWOUND");
                panic!("synthetic observer panic");
            })
        }));
        assert!(unwound.is_err());
        assert!(SCOPES.with(|scopes| scopes.borrow().is_empty()));
        let mut later = OwnRefusals::default();
        assert!(own_refusals(&mut later, || catalogue.refusal_lines()).is_empty());
        assert_eq!(
            own_refusals(&mut own, || catalogue.refusal_lines()),
            ["1 texts are not shown: before the panic; UNWOUND"]
        );
    }

    /// Another consumer's render and uncovered read add to the stack's live
    /// diagnostics; a scope's lines are unchanged by them, and the shared
    /// lines, as the reply boundary reports them, still carry both.
    #[test]
    fn own_scopes_leave_out_the_stacks_live_artwork_and_uncovered_read_diagnostics() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (catalogue, _folder) =
            folder_catalogue(&[("outside_policy/synthetic.txt", "another consumer's file")]);
        let reads = catalogue.content.layers.report_uncovered_reads();
        let mut own = OwnRefusals::default();
        let before = own_refusals(&mut own, || {
            catalogue.refuse("own reason", "OWN");
            catalogue.refusal_lines()
        });
        catalogue
            .content
            .layers
            .note_shader_warning("synthetic artwork-effect note".into());
        catalogue
            .content
            .layers
            .read("outside_policy/synthetic.txt")
            .unwrap();
        let after = own_refusals(&mut own, || catalogue.refusal_lines());
        assert_eq!(after, before);
        assert_eq!(after, ["1 texts are not shown: own reason; OWN"]);
        let shared = catalogue.refusal_lines();
        assert!(
            shared.contains(&"synthetic artwork-effect note".to_owned()),
            "{shared:?}"
        );
        assert!(shared.contains(&reads.warning().unwrap()), "{shared:?}");
    }

    #[test]
    fn native_frames_share_horizontal_vertical_fractional_and_downsampled_offsets() {
        let image = Rgba8Image {
            width: 4,
            height: 4,
            pixels: (0u8..16).flat_map(|value| [value, 0, 0, 255]).collect(),
        };
        let values = |sample: Rgba8Image| {
            sample
                .pixels
                .chunks_exact(4)
                .map(|pixel| pixel[0])
                .collect::<Vec<_>>()
        };
        let strip = |frames, frame_direction| Plain {
            frames,
            frame_direction,
            ..Plain::default()
        };
        let sample = |plain: Plain, frame, max_edge| native_frame(&image, &plain, frame, max_edge);
        assert_eq!(
            values(sample(strip(2, 0), 1, 4).unwrap()),
            [2, 3, 6, 7, 10, 11, 14, 15]
        );
        assert_eq!(
            values(sample(strip(2, 1), 1, 4).unwrap()),
            [8, 9, 10, 11, 12, 13, 14, 15]
        );
        // Downsampled at pixel centres.
        assert_eq!(values(sample(strip(1, 0), 0, 2).unwrap()), [5, 7, 13, 15]);
        // Three frames over four texels: one queried texel, a 4/3 pitch.
        assert_eq!(values(sample(strip(3, 0), 1, 4).unwrap()), [2, 6, 10, 14]);
        assert!(sample(strip(5, 0), 0, 4).is_err());
        assert!(sample(strip(1, 0), 0, 0).is_err());
    }

    #[test]
    fn png_data_urls_round_trip_their_size() {
        let url = png_data_url(&Rgba8Image {
            width: 3,
            height: 2,
            pixels: vec![255; 24],
        })
        .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(url.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let reader = decoder.read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (3, 2));
    }
}
