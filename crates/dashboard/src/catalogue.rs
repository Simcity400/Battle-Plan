//! Content facts every product shares: the recording's localisation in the
//! user's game language and its interface sprites, read once per source and
//! folders, plus the texture decoding every image reply uses. Everything is
//! resolved through the recorded content stack; nothing names a vanilla asset.

use crate::content_check::{Finding, Kind};
use crate::{Source, content, invalid, products::ContentOptions};
use base64::Engine;
use installed_content::{
    ResolvedLayers,
    content::ResolvedContent,
    gfx::SpriteSet,
    gui::native::{resources, sprites::Plain},
    image::Rgba8Image,
    image::native::Decode,
    localisation::{Argument, Localisation},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, OnceLock},
};

/// Retry the fallback sprite only when the requested name is absent.
/// A declared but refused sprite stays refused.
pub(crate) fn created_gui_sprite<'a>(
    sprites: &'a SpriteSet,
    name: &str,
) -> io::Result<&'a installed_content::gfx::SpriteType> {
    if let Some(sprite) = sprites.find(name).map_err(invalid)? {
        return Ok(sprite);
    }
    let fallback = std::str::from_utf8(installed_content::gui::native::resources::FALLBACK_SPRITE)
        .expect("the executable's fallback sprite literal is ASCII");
    sprites
        .lookup(fallback)
        .map_err(|why| invalid(format!("{name} is not declared, and its fallback: {why}")))
}

/// Text icons search the GFX_ prefix before the bare authored alias.
/// A declared but unsupported sprite is a refusal; only an absent declaration
/// takes the second lookup.
pub(crate) fn text_icon_sprite<'a>(
    sprites: &'a SpriteSet,
    alias: &str,
) -> io::Result<&'a installed_content::gfx::SpriteType> {
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

/// `key` through the game's localiser (`installed_content::localisation`), with its
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
pub(crate) fn localised_details(
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

pub(crate) fn localised_in_details(
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

/// Stored game text (`installed_content::localisation`) as plain text, read as the
/// game's text layout reads it (`installed_content::localisation::pieces`): colour
/// codes are removed, inline images (a text icon or an inline flag) are left
/// out of the plain text, and the 0x14 and 0x1A spacing controls, which have
/// no plain-text form, are refused by name.
pub fn plain_text(text: &str) -> Result<String, String> {
    use installed_content::localisation::{Piece, pieces};
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refusals(BTreeMap<String, BTreeSet<String>>);

impl Refusals {
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

    pub(crate) fn counted_diagnostic_lines(&self, what: &str) -> Vec<(String, usize)> {
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
}

/// Named, counted refusals whose storage is bounded with a product's reply.
/// Check after each scan step and before turning them into reply warnings.
/// Reaching the bound refuses the whole product; it never drops individual
/// warnings to make an otherwise successful reply fit.
pub(crate) struct BoundedRefusals {
    refused: Refusals,
    bytes: usize,
    overflow: bool,
    budget: usize,
}

impl BoundedRefusals {
    pub(crate) fn new(budget: usize) -> Self {
        Self {
            refused: Refusals::default(),
            bytes: 0,
            overflow: false,
            budget,
        }
    }

    pub(crate) fn note(&mut self, reason: impl Into<String>, identity: impl Into<String>) {
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

    pub(crate) fn check(&self) -> io::Result<usize> {
        if self.overflow {
            Err(io::Error::other(
                "Product refusals exceed the complete-response budget",
            ))
        } else {
            Ok(self.bytes)
        }
    }

    pub(crate) fn lines(&self, what: &str) -> Vec<String> {
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
    /// Shared native template registry, read once with this content catalogue.
    pub guis: installed_content::gui::native::Guis,
    /// The current client's texture decode settings, used by native GUI artwork.
    pub gui_decode: Result<installed_content::image::native::Decode, String>,
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
    war_names: OnceLock<Result<installed_content::wargoals::Names, String>>,
    /// State templates are usable without the scripted-localisation inputs.
    pub(crate) state_content: Arc<OnceLock<Result<crate::scripted_text::StateContent, String>>>,
}

impl Catalogue {
    pub(crate) fn war_names(&self) -> Result<&installed_content::wargoals::Names, String> {
        self.war_names
            .get_or_init(|| {
                installed_content::wargoals::read_names(&self.content.layers).map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
    fn load(source: &Source, options: &ContentOptions) -> io::Result<Self> {
        let mut verification = Vec::new();
        let replay = source.replay();
        content::check_replay(&replay, options.allow_unverified, &mut verification)?;
        let language = &replay.identity.language;
        let recorded_language = language.strip_prefix("l_").unwrap_or(language).to_owned();
        let content = source.content(&options.options)?;
        let mut warnings = Vec::new();
        let settings = installed_content::settings::read_keys(&options.options.user_dir);
        let language = match &settings {
            Ok(settings) => settings.language()?,
            Err(why) => {
                warnings.push(format!(
                    "Localisation uses the recorded language {recorded_language}: settings.txt is unavailable: {why}"
                ));
                recorded_language.clone()
            }
        };
        let localisation = content::localisation_with_key(
            source,
            options,
            (
                content::content_key(Some(options)).expect("content options given"),
                options.options.cache_dir.clone(),
                language,
            ),
        )?;
        let gui_decode = settings
            .map(|settings| settings.texture_decode())
            .map_err(|why| format!("settings.txt is unavailable: {why}"));
        Self::from_content(
            content,
            localisation,
            verification,
            warnings,
            recorded_language,
            gui_decode,
        )
    }

    fn from_content(
        content: Arc<ResolvedContent>,
        localisation: Arc<Localisation>,
        verification: Vec<String>,
        mut warnings: Vec<String>,
        recorded_language: String,
        gui_decode: Result<installed_content::image::native::Decode, String>,
    ) -> io::Result<Self> {
        let sprites = installed_content::gfx::read(&content.layers)?;
        let guis = installed_content::gui::native::read_layers(&content.layers)?;
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
            guis,
            gui_decode,
            verification,
            warnings,
            recorded_language,
            refused: Mutex::default(),
            findings: None,
            scripted: OnceLock::new(),
            war_names: OnceLock::new(),
            state_content: Arc::new(OnceLock::new()),
        })
    }

    /// A content-only catalogue, without a recording or identity verification.
    pub(crate) fn of_layers(layers: ResolvedLayers, language: &str) -> io::Result<Self> {
        let localisation = Arc::new(installed_content::localisation::read(&layers, language)?);
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
    pub(crate) fn localised_with_refusals(
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
    pub(crate) fn localised_offline_country(&self, key: &str) -> Option<String> {
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

    /// [`Catalogue::localised_with`] with no arguments.
    pub fn localised(&self, key: &str, view: View) -> Option<String> {
        self.localised_with(key, &[], view)
    }

    /// The content the scripted text evaluator needs
    /// ([`crate::scripted_text::ScriptedContent::read`]), read once. Its
    /// reading's warnings join [`Catalogue::refusal_lines`].
    pub fn scripted_content(&self) -> Result<&crate::scripted_text::ScriptedContent, String> {
        self.scripted
            .get_or_init(|| {
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

    pub(crate) fn cached_states(&self) -> Result<&crate::scripted_text::StateContent, String> {
        self.state_content
            .get_or_init(|| {
                crate::scripted_text::read_state_content(&self.content.layers)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Notes a text of `identity` not shown for `reason`.
    pub fn refuse(&self, reason: &str, identity: &str) {
        self.refuse_finding(Finding::new(Kind::Refused, identity, reason));
    }

    pub(crate) fn refuse_as(&self, kind: Kind, reason: &str, identity: &str) {
        self.refuse_finding(Finding::new(kind, identity, reason));
    }

    pub(crate) fn refuse_finding(&self, finding: Finding) {
        self.refused
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .note(&finding.message, &finding.subject);
        self.note(finding);
    }

    /// The texts refused so far, one warning per reason, and the warnings
    /// of scripted localisation and shared content reads, including lazy reads.
    pub fn refusal_lines(&self) -> Vec<String> {
        self.refusal_entries()
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }

    /// Text summaries retain their actual identity count; reader warnings have
    /// no measured affected-item count. Consumers never parse warning prose.
    pub(crate) fn refusal_entries(&self) -> Vec<(String, Option<usize>)> {
        let mut lines: Vec<_> = self
            .refused
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .counted_diagnostic_lines("texts are not shown")
            .into_iter()
            .map(|(line, count)| (line, Some(count)))
            .collect();
        lines.extend(
            self.reader_warning_lines()
                .into_iter()
                .map(|line| (line, None)),
        );
        lines
    }

    /// Shared lazy-reader and uncovered-read warnings without another
    /// product's accumulated text/artwork refusals.
    pub(crate) fn reader_warning_lines(&self) -> Vec<String> {
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

    pub(crate) fn note(&self, finding: Finding) {
        if let Some(findings) = &self.findings {
            findings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(finding);
        }
    }

    pub(crate) fn take_findings(&self) -> Vec<Finding> {
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
    pub(crate) fn of_layers_for_check(layers: ResolvedLayers, language: &str) -> io::Result<Self> {
        let mut catalogue = Self::of_layers(layers, language)?;
        catalogue.findings = Some(Mutex::default());
        Ok(catalogue)
    }
}

type Cached = Option<((content::ContentKey, std::path::PathBuf), Arc<Catalogue>)>;

/// The shared catalogue for these folders, loaded on first use and replaced when
/// the folders, the cache folder or the unverified-content choice change.
pub fn catalogue(source: &Source, options: &ContentOptions) -> io::Result<Arc<Catalogue>> {
    let key = (
        content::content_key(Some(options)).expect("content options given"),
        options.options.cache_dir.clone(),
    );
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached, catalogue)) = state.as_ref()
        && *cached == key
    {
        return Ok(Arc::clone(catalogue));
    }
    let catalogue = Arc::new(Catalogue::load(source, options)?);
    *state = Some((key, Arc::clone(&catalogue)));
    Ok(catalogue)
}

#[cfg(test)]
pub(crate) fn set_test_catalogue(
    source: &Source,
    options: &ContentOptions,
    catalogue: Arc<Catalogue>,
) {
    let key = (
        content::content_key(Some(options)).expect("content options given"),
        options.options.cache_dir.clone(),
    );
    *source.state::<Cached>().lock().unwrap() = Some((key, catalogue));
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
/// then RGBA8. Sprite artwork, shields and resource icons all use this path;
/// decoded edges are bounded only by the decoder's device limit.
pub(crate) fn gui_texture(
    layers: &ResolvedLayers,
    name: &[u8],
    decode: Decode,
) -> io::Result<Rgba8Image> {
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
    installed_content::image::rgba::dds_base(texture.image)
}

/// A GUI sprite frame returned as dashboard artwork.
pub(crate) fn gui_texture_frame_png(
    layers: &ResolvedLayers,
    name: &[u8],
    decode: Decode,
    frames: u32,
    frame: u32,
    max_edge: u32,
) -> io::Result<String> {
    image_frame_png(&gui_texture(layers, name, decode)?, frames, frame, max_edge)
}

pub(crate) fn image_frame_png(
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
    sprite: &installed_content::gfx::SpriteType,
    decode: Decode,
    frame: u32,
    max_edge: u32,
) -> io::Result<String> {
    use installed_content::gui::native::sprites::SpriteKind;
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

mod sprite;

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

#[cfg(test)]
pub(crate) mod texture_tests;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

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
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use installed_content::gui::native;
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
        let guis = native::Guis {
            files: vec![("interface/topbar.gui".into(), gui)],
        };
        let mut warnings = Vec::new();
        assert_eq!(
            installed_content::gui::native::window_button_sprite(
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
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let facts = installed_content::gui::native::sprites::read_sprites(
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
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let facts = installed_content::gui::native::sprites::read_sprites(
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
        engine::executable_tables::initialize_for_tests()
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
            installed_content::image::native::Decode {
                force_pow2_textures: true,
            },
        )
        .unwrap();
        assert_eq!((image.width, image.height), (4, 1));
        // Power-of-two padding fills the FreeImage RGB format with opaque black.
        assert_eq!(&image.pixels[12..16], &[0, 0, 0, 255]);
    }

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
    /// folder layer, read by `installed_content::localisation::read`.
    pub(crate) fn loaded(yml: &str) -> Localisation {
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
        installed_content::localisation::read(&layers, "english").unwrap()
    }

    /// A faction catalogue through the production reader, without a second parser.
    pub(crate) fn loaded_templates(text: &str) -> Arc<installed_content::faction_templates::Catalog> {
        let (catalogue, _folder) =
            folder_catalogue(&[("common/factions/templates/test.txt", text)]);
        catalogue.content.layers.faction_templates().unwrap()
    }

    /// A content folder written for a test, removed when dropped.
    pub(crate) struct Folder(pub std::path::PathBuf);

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A catalogue over one content folder holding `files` (path, text), read
    /// by the same readers [`Catalogue::load`] uses, in English. The folder
    /// lives as long as the returned guard.
    pub(crate) fn folder_catalogue(files: &[(&str, &str)]) -> (Catalogue, Folder) {
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
    pub(crate) fn layers_catalogue(layers: ResolvedLayers) -> Catalogue {
        let localisation = installed_content::localisation::read(&layers, "english").unwrap();
        let sprites = installed_content::gfx::read(&layers).unwrap();
        let guis = installed_content::gui::native::read_layers(&layers).unwrap();
        Catalogue {
            content: Arc::new(ResolvedContent {
                layers,
                stats: Default::default(),
            }),
            localisation: Arc::new(localisation),
            sprites,
            guis,
            gui_decode: Ok(Default::default()),
            verification: Vec::new(),
            warnings: Vec::new(),
            recorded_language: "english".into(),
            refused: Mutex::default(),
            findings: None,
            scripted: OnceLock::new(),
            war_names: OnceLock::new(),
            state_content: Arc::new(OnceLock::new()),
        }
    }

    #[test]
    fn faction_templates_share_one_cache_and_report_each_warning_once() {
        engine::executable_tables::initialize_for_tests()
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
        *frame.tables[replay::rows::TableId::Factions.index()] =
            replay::frame::TableView::Complete(replay::rows::TableRows::Factions(vec![
                replay::rows::FactionRow {
                    id: 5,
                    template_token: Some(17),
                    ..Default::default()
                },
            ]));
        *frame.tables[replay::rows::TableId::LexicalTokens.index()] =
            replay::frame::TableView::Complete(replay::rows::TableRows::LexicalTokens(vec![
                replay::rows::LexicalTokenRow {
                    token: 17,
                    literal: b"bad".to_vec(),
                },
            ]));
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
        engine::executable_tables::initialize_for_tests()
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
        engine::executable_tables::initialize_for_tests()
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
    #[test]
    fn missing_or_unreadable_settings_only_refuse_catalogue_artwork() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{container::Replay, spool::Record};
        use std::sync::atomic::AtomicBool;
        let (_, folder) = crate::catalogue::tests::folder_catalogue(&[
            ("hoi4.exe", "test"),
            (
                "user/settings.txt",
                "language=l_english force_pow2_textures=no",
            ),
            (
                "user/dlc_load.json",
                r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
            ),
            (
                "localisation/a_l_english.yml",
                "\u{feff}l_english:\n NAME:0 \"Recorded name\"\n",
            ),
        ]);
        let (options, identity) = crate::test_fixtures::content_identity(&folder.0);
        let settings = options.options.user_dir.join("settings.txt");
        std::fs::remove_file(&settings).unwrap();
        for unreadable in [false, true] {
            if unreadable {
                std::fs::create_dir(&settings).unwrap();
            }
            let mut input = records(1, vec![source(1, 100, Some(1))]);
            let Record::SessionStart(session) = &mut input[0] else {
                unreachable!()
            };
            session.identity = identity.clone();
            let source = crate::Source::from_replay(
                crate::SourceKey {
                    source_id: "settings-isolation".into(),
                    revision: unreadable.to_string(),
                },
                "memory.battleplan".into(),
                Arc::new(Replay::from_records(CAMPAIGN.into(), &[input]).unwrap()),
                &AtomicBool::new(false),
            )
            .unwrap();
            let catalogue = crate::catalogue::catalogue(&source, &options).unwrap();
            assert_eq!(
                catalogue
                    .localised("NAME", crate::catalogue::View::Plain)
                    .unwrap(),
                "Recorded name"
            );
            assert!(
                catalogue
                    .gui_decode
                    .as_ref()
                    .unwrap_err()
                    .contains("settings.txt")
            );
            assert!(
                catalogue
                    .warnings
                    .iter()
                    .any(|why| why.contains("recorded language"))
            );
            if unreadable {
                std::fs::remove_dir(&settings).unwrap();
            }
            std::fs::write(&settings, "language=l_english force_pow2_textures=yes").unwrap();
            let cached = crate::catalogue::catalogue(&source, &options).unwrap();
            assert!(Arc::ptr_eq(&catalogue, &cached));
            assert!(cached.gui_decode.is_err());
            std::fs::remove_file(&settings).unwrap();
        }
    }
}
