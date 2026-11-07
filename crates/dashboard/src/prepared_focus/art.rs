//! Focus art at an hour, evaluated when a reply asks for it: titles, icons and
//! descriptions as the focus tree shows them (`focus-art`) and the content's
//! static text (`focus-static-text`); and the event feed's completion art,
//! its text and icon choice made in preparation at each completion's hour,
//! its icon drawn as the reply shows it.
//! A request whose tokens read no recorded state reads no recording; one
//! that does reads that hour's frame (shared with `focus-artwork`'s frames).
//! Results that read no recorded state are content, resolved once per content
//! binding and kept with it.
use super::*;
use crate::FocusDatabase;
use crate::products::{Product, validate_tag};
use replay_frame::frame::SharedView;
use std::sync::{Arc, PoisonError};

/// Most tokens one `focus-art` request evaluates, over all its hours: one
/// page of a country's completed focuses and its active focus.
const MAX_TOKENS: usize = PAGE_ROWS + 1;

/// The database, presentation, token and (for country-specific art) tag.
type ArtKey = (FocusDatabase, Presentation, String, String);

/// Art that reads no recorded state: content, one entry per focus, presentation and country
/// whose own sprite it shows, dropped with the content binding.
#[derive(Default)]
pub(crate) struct Kept {
    entries: BTreeMap<ArtKey, crate::catalogue::Drawn<artwork::ResolvedArt>>,
}

impl Kept {
    /// The kept art of `key`.
    pub(crate) fn get(&self, key: &ArtKey) -> Option<artwork::ResolvedArt> {
        Some(self.entries.get(key)?.get().clone())
    }

    pub(crate) fn insert(
        &mut self,
        key: ArtKey,
        item: crate::catalogue::Drawn<artwork::ResolvedArt>,
    ) {
        self.entries.insert(key, item);
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.entries.len()
    }
}

/// The content binding an evaluator holds, and whether it loaded.
struct Binding {
    source: crate::SourceKey,
    key: crate::catalogue::Binding,
    result: Result<(), crate::catalogue::ContentError>,
    diagnostics: crate::catalogue::OwnRefusals,
}

/// Each focus database's context: a token is looked up in its own alone.
struct Contexts {
    national: artwork::Context,
    continuous: artwork::Context,
}

impl Contexts {
    fn of(&self, database: FocusDatabase) -> &artwork::Context {
        match database {
            FocusDatabase::National => &self.national,
            FocusDatabase::Continuous => &self.continuous,
        }
    }

    fn of_mut(&mut self, database: FocusDatabase) -> &mut artwork::Context {
        match database {
            FocusDatabase::National => &mut self.national,
            FocusDatabase::Continuous => &mut self.continuous,
        }
    }
}

/// Focus tree presentation over one content binding. Results that read no
/// recorded state are kept; the others are evaluated on each hour's frame.
#[derive(Default)]
pub(crate) struct Evaluator {
    binding: Option<Binding>,
    contexts: Option<Contexts>,
    catalogue: Option<Arc<crate::catalogue::Catalogue>>,
    warnings: Vec<String>,
    kept: Kept,
}

impl Evaluator {
    /// Bind the content; a binding that failed fails again without loading.
    fn configure(&mut self, source: &Source, content: &ContentOptions) -> io::Result<()> {
        if !self.binding.as_ref().is_some_and(|bound| {
            bound.source == *source.key() && crate::catalogue::binding_matches(&bound.key, content)
        }) {
            self.binding = None;
            self.contexts = None;
            self.catalogue = None;
            self.warnings.clear();
            self.kept = Kept::default();
            let (result, diagnostics) = crate::catalogue::capture(|| {
                let catalogue = crate::catalogue::catalogue(source, content)?;
                self.catalogue = Some(Arc::clone(&catalogue));
                let load = |database| {
                    artwork::Context::load(Arc::clone(&catalogue), artwork::Kind::Focus(database))
                };
                let contexts = Contexts {
                    national: load(FocusDatabase::National)?,
                    continuous: load(FocusDatabase::Continuous)?,
                };
                // Both databases share one content reading and its warnings.
                self.warnings = contexts.national.standing_warnings();
                self.warnings.sort();
                self.warnings.dedup();
                self.contexts = Some(contexts);
                Ok(())
            });
            if result
                .as_ref()
                .is_err_and(|error: &io::Error| error.kind() == io::ErrorKind::Interrupted)
            {
                return result;
            }
            self.binding = Some(Binding {
                source: source.key().clone(),
                key: crate::catalogue::binding(content),
                result: result.map_err(crate::catalogue::ContentError::from),
                diagnostics,
            });
        }
        crate::catalogue::credit(
            &self
                .binding
                .as_ref()
                .expect("focus content bound")
                .diagnostics,
        );
        self.binding
            .as_ref()
            .expect("focus content bound")
            .result
            .as_ref()
            .map(|_| ())
            .map_err(crate::catalogue::ContentError::error)
    }

    /// The loaded context of `database`.
    fn context(&self, database: FocusDatabase) -> &artwork::Context {
        self.contexts
            .as_ref()
            .expect("focus contexts loaded")
            .of(database)
    }

    /// Whether any of `tokens` of `database` reads recorded state for `tag` in `mode`.
    fn needs_frame(
        &self,
        database: FocusDatabase,
        tag: &str,
        tokens: &[String],
        mode: Presentation,
    ) -> bool {
        let context = self.context(database);
        tokens
            .iter()
            .any(|token| context.requested_tree_focus_needs_frame(token, tag, mode))
    }

    /// `tokens`' presentation for `tag` on `inputs`, each looked up in
    /// `database`, in token order, as the focus tree shows it.
    fn present(
        &mut self,
        inputs: &artwork::FocusFrame<'_>,
        database: FocusDatabase,
        tag: &str,
        tokens: &[String],
        mode: Presentation,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<PresentedFocus>> {
        check_cancel(cancelled)?;
        let context = self
            .contexts
            .as_mut()
            .expect("focus contexts loaded")
            .of_mut(database);
        let mut requests = Vec::new();
        // Kept art is taken first; what is evaluated is kept after.
        let mut planned = Vec::with_capacity(tokens.len());
        for token in tokens {
            // A country's own sprite makes otherwise shared art its own.
            let scoped = matches!(
                mode,
                Presentation::Display | Presentation::Icons | Presentation::Full
            ) && context.focus_country_icon_override(token, tag);
            let key = (
                database,
                mode,
                token.clone(),
                if scoped {
                    tag.to_owned()
                } else {
                    String::new()
                },
            );
            let dynamic = context.requested_tree_focus_needs_frame(token, tag, mode);
            let kept = if dynamic { None } else { self.kept.get(&key) };
            if kept.is_none() {
                requests.push((token.as_str(), tag));
            }
            planned.push((key, dynamic, kept));
        }
        let evaluated = if requests.is_empty() {
            Vec::new()
        } else {
            context.focuses_in(&requests, inputs, true, mode, |items| {
                // Each item keeps what its own draw fell back on.
                std::iter::from_fn(|| {
                    crate::catalogue::Drawn::try_new(|| items.next().ok_or(())).ok()
                })
                .map(|item| {
                    check_cancel(cancelled)?;
                    Ok(item)
                })
                .collect::<io::Result<Vec<_>>>()
            })?
        };
        let mut evaluated = evaluated.into_iter();
        let mut presented = Vec::with_capacity(planned.len());
        for (key, dynamic, kept) in planned {
            let item = match kept {
                Some(item) => item,
                None => {
                    let drawn = evaluated
                        .next()
                        .ok_or_else(|| io::Error::other("Focus presentation omitted token"))?;
                    let item = drawn.get().clone();
                    if !dynamic {
                        self.kept.insert(key, drawn);
                    }
                    item
                }
            };
            presented.push(PresentedFocus {
                artwork: item.0,
                reason: item.1,
            });
        }
        Ok(presented)
    }
}

/// The source's focus art evaluator.
#[derive(Default)]
struct ArtState(Evaluator);

// Recorded frames art requests read on this thread.
#[cfg(test)]
thread_local! {
    pub(super) static FRAME_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A frame of `hour` holding no table: what a request that reads no
/// recorded state is evaluated on.
fn unread_frame(branch: u64, hour: i32) -> DecodedFrame {
    let omitted = SharedView::from(TableView::Unavailable {
        reason: "Not read: the requested focus art reads no recorded state".into(),
    });
    DecodedFrame {
        branch,
        hour,
        tables: std::array::from_fn(|_| omitted.clone()),
        metrics: Default::default(),
    }
}

/// Whether `branch` has a block holding `hour` (from the index alone).
fn recorded(source: &Source, branch: u64, hour: i32) -> bool {
    source
        .replay_ref()
        .index
        .iter()
        .any(|block| block.branch == branch && block.first_hour <= hour && hour <= block.last_hour)
}

/// One hour's focus art: in token order (none when it failed), with the
/// content's standing warnings and the evaluation's failures.
#[derive(Clone, Debug, Serialize)]
pub struct HourArt {
    pub art: Vec<PresentedFocus>,
    pub warnings: Vec<String>,
    pub failures: Vec<FocusArtFailure>,
}

/// `tokens`' focus tree art for `tag` at `hour`, each looked up in
/// `database`, as hourly preparation presented it at that hour, in token
/// order. A content binding that failed, or an evaluation that failed, is the
/// reply's named, counted failure.
#[allow(clippy::too_many_arguments)]
pub fn art_at(
    source: &Source,
    content: &ContentOptions,
    branch: u64,
    tag: &str,
    hour: i32,
    database: FocusDatabase,
    tokens: &[String],
    mode: Presentation,
    cancelled: &AtomicBool,
) -> io::Result<HourArt> {
    check_cancel(cancelled)?;
    let state = source.state::<ArtState>();
    let failed = |reason: String, warnings: Vec<String>| HourArt {
        art: Vec::new(),
        warnings,
        failures: vec![FocusArtFailure {
            reason,
            tokens: tokens.len() as u32,
        }],
    };
    let mut frame: Option<DecodedFrame> = None;
    loop {
        {
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            let evaluator = &mut state.0;
            match evaluator.configure(source, content) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(error) => return Ok(failed(error.to_string(), Vec::new())),
                Ok(()) => {}
            }
            if frame.is_some() || !evaluator.needs_frame(database, tag, tokens, mode) {
                let unread;
                let decoded = match &frame {
                    Some(frame) => frame,
                    None => {
                        unread = unread_frame(branch, hour);
                        &unread
                    }
                };
                let catalogue = Arc::clone(
                    evaluator
                        .catalogue
                        .as_ref()
                        .expect("focus catalogue loaded"),
                );
                let inputs = artwork::FocusFrame::new(decoded, &catalogue);
                let warnings = evaluator.warnings.clone();
                return match evaluator.present(&inputs, database, tag, tokens, mode, cancelled) {
                    Ok(art) => Ok(HourArt {
                        art,
                        warnings,
                        failures: Vec::new(),
                    }),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
                    Err(error) => Ok(failed(error.to_string(), warnings)),
                };
            }
        }
        // Read outside the evaluator, so cached art never waits on a seek.
        #[cfg(test)]
        FRAME_READS.with(|reads| reads.set(reads.get() + 1));
        frame = Some(crate::products::focus_artwork::scoped_frame(
            source, branch, hour, cancelled,
        )?);
    }
}

/// The content's text for each of `tokens`, each looked up in `database`, in
/// request order (repeats included): titles and descriptions that read no
/// recorded state.
pub fn static_text(
    source: &Source,
    content: Option<&ContentOptions>,
    database: FocusDatabase,
    tokens: &[String],
    cancelled: &AtomicBool,
) -> io::Result<Vec<Artwork>> {
    check_cancel(cancelled)?;
    let content = content.ok_or_else(|| {
        io::Error::other(
            "Choose the game and user folders for focus titles, icons and descriptions",
        )
    })?;
    let state = source.state::<ArtState>();
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    state.0.configure(source, content)?;
    let context = state.0.context(database);
    let unique: BTreeSet<&str> = tokens.iter().map(String::as_str).collect();
    let (items, _) = context.text_catalogue(unique, cancelled)?;
    let by_token: BTreeMap<String, Artwork> = items
        .into_iter()
        .map(|item| (item.token.clone(), item))
        .collect();
    tokens
        .iter()
        .map(|token| {
            by_token
                .get(token)
                .cloned()
                .ok_or_else(|| io::Error::other(format!("Focus static text omitted {token}")))
        })
        .collect()
}

/// The focus content's standing warnings, as the national focus history's reply names them.
pub fn standing_warnings(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<String>> {
    check_cancel(cancelled)?;
    let content = content.ok_or_else(|| {
        io::Error::other(
            "Choose the game and user folders for focus titles, icons and descriptions",
        )
    })?;
    let state = source.state::<ArtState>();
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    state.0.configure(source, content)?;
    Ok(state.0.warnings.clone())
}

/// A completion's art as the event feed shows it, chosen in preparation at its hour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub enum CompletionArt {
    /// Its text and icon choice, and what choosing them noted, which a reply showing it names as
    /// its own.
    Shown {
        art: artwork::ChosenArt,
        noted: crate::catalogue::OwnRefusals,
    },
    /// Why the event feed's focus context did not load.
    Refused(String),
    /// Why the recording held no frame at its hour.
    Gap(String),
}

impl CompletionArt {
    /// The bytes it holds, for the cache keeping it.
    pub(crate) fn bytes(&self) -> usize {
        let text = |text: &Option<String>| text.as_ref().map_or(0, String::capacity);
        64 + match self {
            Self::Shown { art, .. } => {
                let icon = match &art.icon {
                    Ok(choice) | Err(choice) => choice.capacity(),
                };
                art.art.token.capacity()
                    + text(&art.art.name)
                    + text(&art.art.description)
                    + icon
                    + art.text.iter().map(String::capacity).sum::<usize>()
            }
            Self::Refused(reason) | Self::Gap(reason) => reason.capacity(),
        }
    }

    fn gap(&self) -> Option<&String> {
        match self {
            Self::Gap(reason) => Some(reason),
            _ => None,
        }
    }

    fn refusal(&self) -> Option<&String> {
        match self {
            Self::Refused(reason) => Some(reason),
            _ => None,
        }
    }
}

/// The tables completion art reads of its hour's frame.
pub(crate) fn tables() -> &'static [TableId] {
    static TABLES: std::sync::OnceLock<Vec<TableId>> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let mut tables: BTreeSet<TableId> = crate::scripted_text::TABLES.iter().copied().collect();
        tables.extend([
            TableId::Campaign,
            TableId::Clock,
            TableId::Countries,
            TableId::CountryLinks,
        ]);
        tables.into_iter().collect()
    })
}

/// Never set: preparation seeks a frame whole.
static WHOLE: AtomicBool = AtomicBool::new(false);

/// The event feed's completion art context over the recorded content, as preparation holds it:
/// the national focus context, which names a definition's icon and full text as an event shows
/// them, loaded on first use.
#[derive(Default)]
pub struct Presenter(Option<artwork::Context>);

impl Presenter {
    /// Each of `asked` (token, tag), completed at `hour` of `branch`, as the event feed shows it,
    /// chosen as one batch: on `frame`'s tables where any of them reads recorded state, on none
    /// otherwise. `frame` names why the recording holds none; `lexicon` is its walk's lexer
    /// folds ([`artwork::FocusFrame::with_lexicon`]).
    pub(crate) fn present<'f>(
        &mut self,
        catalogue: &Arc<crate::catalogue::Catalogue>,
        (branch, hour): (u64, i32),
        asked: &[(&str, &str)],
        lexicon: Option<&'f dashboard_world::Lexicon>,
        frame: impl FnOnce() -> io::Result<Result<replay_frame::frame::HourFrame<'f>, String>>,
    ) -> io::Result<Vec<CompletionArt>> {
        let every =
            |art: CompletionArt| -> io::Result<Vec<CompletionArt>> { Ok(vec![art; asked.len()]) };
        // Completions are of national focuses.
        let national = artwork::Kind::Focus(FocusDatabase::National);
        let context = match artwork::ensure_in(&mut self.0, Arc::clone(catalogue), national) {
            Ok(context) => context,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => return every(CompletionArt::Refused(error.to_string())),
        };
        let tokens: Vec<String> = asked.iter().map(|(token, _)| (*token).to_owned()).collect();
        let unread;
        let (decoded, lexicon) = if context.requested_focus_needs_frame(&tokens) {
            match frame()? {
                Ok(frame) => (frame, lexicon),
                Err(reason) => return every(CompletionArt::Gap(reason)),
            }
        } else {
            unread = unread_frame(branch, hour);
            (replay_frame::frame::HourFrame::from(&unread), None)
        };
        let shown = context.event_focus_choices_at(asked, decoded, lexicon, |items| {
            std::iter::from_fn(|| {
                let (item, noted) = crate::catalogue::capture(|| items.next());
                item.map(|art| CompletionArt::Shown { art, noted })
            })
            .collect()
        });
        Ok(shown)
    }

    /// [`Self::present`] on the frame sought from `source`'s recording at `hour`.
    pub(crate) fn present_sought(
        &mut self,
        source: &Source,
        catalogue: &Arc<crate::catalogue::Catalogue>,
        (branch, hour): (u64, i32),
        asked: &[(&str, &str)],
    ) -> io::Result<Vec<CompletionArt>> {
        let mut sought: Option<DecodedFrame> = None;
        let slot = &mut sought;
        self.present(catalogue, (branch, hour), asked, None, move || {
            #[cfg(test)]
            FRAME_READS.with(|reads| reads.set(reads.get() + 1));
            Ok(
                match dashboard_base::prepared::query::frames::seek(
                    source, branch, hour, tables(), &WHOLE,
                )? {
                    replay::campaign::SeekResult::Frame(frame) => {
                        Ok(replay_frame::frame::HourFrame::from(&*slot.insert(*frame)))
                    }
                    replay::campaign::SeekResult::Gap { reason, .. } => Err(reason),
                },
            )
        })
    }
}

/// `completions`' art as preparation presents what its walk did not: each hour's completions on
/// the frame sought at that hour, with the recorded content `content` names; none without it.
#[cfg(test)]
pub(crate) fn present_completions(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    completions: &[crate::products::national_focus::FocusCompletion],
) -> io::Result<Vec<Option<CompletionArt>>> {
    let Some(catalogue) =
        content.and_then(|content| crate::catalogue::catalogue(source, content).ok())
    else {
        return Ok(vec![None; completions.len()]);
    };
    let mut presenter = Presenter::default();
    let mut art = Vec::with_capacity(completions.len());
    for hour in completions.chunk_by(|left, right| left.hour == right.hour) {
        let at = hour[0].hour;
        let asked: Vec<(String, String)> = hour
            .iter()
            .map(|completion| (completion.token.clone(), completion.tag.clone()))
            .collect();
        let asked: Vec<(&str, &str)> = asked
            .iter()
            .map(|(token, tag)| (token.as_str(), tag.as_str()))
            .collect();
        let shown = presenter.present_sought(source, &catalogue, (branch, at), &asked)?;
        art.extend(shown.into_iter().map(Some));
    }
    Ok(art)
}

/// The event feed's completion art context, held for the content's standing warnings.
#[derive(Default)]
struct EventArtState(Option<artwork::Context>);

/// The art the event feed shows, with the content's standing warnings.
type EventArtReply = (Vec<artwork::ResolvedArt>, Vec<String>);

/// `work` on the completion art context over `catalogue`, held only while it runs; the content's
/// refusal when it does not load.
fn with_event_art<T>(
    state: &std::sync::Mutex<EventArtState>,
    catalogue: &Arc<crate::catalogue::Catalogue>,
    work: impl FnOnce(&mut artwork::Context) -> T,
) -> io::Result<Result<T, String>> {
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    // Completions are of national focuses.
    let national = artwork::Kind::Focus(FocusDatabase::National);
    match artwork::ensure_in(&mut state.0, Arc::clone(catalogue), national) {
        Ok(art) => Ok(Ok(work(art))),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
        Err(error) => Ok(Err(error.to_string())),
    }
}

/// Why a completion holds no art while the content is chosen.
const UNPREPARED: &str =
    "Focus completion art was prepared without the game's content; prepare the recording again";

/// The art of `completions` (each one's hour, and the art preparation kept with it) as the event
/// feed shows it, with the content's standing warnings, crediting what choosing it noted: its
/// text and icon choice as preparation made them at its hour, each hour's icons drawn as one
/// batch as this request shows them. The content's refusal when its focus context does not
/// load, or the earliest refusal preparation met. The earliest completion whose hour the
/// recording held no frame for fails the reply.
// I/O failure and content refusal are separate; successful art also carries standing warnings.
#[allow(clippy::type_complexity)]
pub(crate) fn event_art(
    source: &Source,
    content: &ContentOptions,
    completions: &[(i32, Option<CompletionArt>)],
) -> io::Result<Result<EventArtReply, String>> {
    let catalogue = match crate::catalogue::catalogue(source, content) {
        Ok(catalogue) => catalogue,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => return Ok(Err(error.to_string())),
    };
    let state = source.state::<EventArtState>();
    let warnings = match with_event_art(&state, &catalogue, |art| art.standing_warnings())? {
        Ok(warnings) => warnings,
        Err(reason) => return Ok(Err(reason)),
    };
    let earliest = |failed: fn(&CompletionArt) -> Option<&String>| {
        completions
            .iter()
            .filter_map(|(hour, art)| Some((*hour, failed(art.as_ref()?)?)))
            .min_by_key(|(hour, _)| *hour)
            .map(|(_, reason)| reason.clone())
    };
    if let Some(reason) = earliest(CompletionArt::gap) {
        return Err(io::Error::other(reason));
    }
    if let Some(reason) = earliest(CompletionArt::refusal) {
        return Ok(Err(reason));
    }
    let mut chosen = Vec::with_capacity(completions.len());
    let mut by_hour: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
    for (hour, art) in completions {
        let Some(CompletionArt::Shown { art, .. }) = art else {
            return Ok(Err(UNPREPARED.to_owned()));
        };
        by_hour.entry(*hour).or_default().push(chosen.len());
        chosen.push(art);
    }
    for (_, art) in completions {
        if let Some(CompletionArt::Shown { noted, .. }) = art {
            crate::catalogue::credit(noted);
        }
    }
    let drawn = with_event_art(&state, &catalogue, |context| {
        let mut drawn: Vec<Option<Result<String, String>>> = vec![None; chosen.len()];
        for indices in by_hour.values() {
            let asked: Vec<(&str, &Result<String, String>)> = indices
                .iter()
                .map(|&index| (chosen[index].art.token.as_str(), &chosen[index].icon))
                .collect();
            for (&index, icon) in indices.iter().zip(context.event_focus_icons(&asked)) {
                drawn[index] = Some(icon);
            }
        }
        drawn
    })?;
    let drawn = match drawn {
        Ok(drawn) => drawn,
        Err(reason) => return Ok(Err(reason)),
    };
    let art = chosen
        .into_iter()
        .zip(drawn)
        .map(|(chosen, icon)| -> io::Result<artwork::ResolvedArt> {
            let icon =
                icon.ok_or_else(|| io::Error::other("Focus completion art omitted a row"))?;
            let mut art = chosen.art.clone();
            let refused = match icon {
                Ok(icon) => {
                    art.icon = Some(icon);
                    None
                }
                Err(reason) => Some(reason),
            };
            Ok((art, artwork::refusal(refused, &chosen.text)))
        })
        .collect::<io::Result<_>>()?;
    Ok(Ok((art, warnings)))
}

/// `focus-art`: a country's focus art at several hours, each hour's tokens
/// looked up in one database: a page of completions, each at the hour that
/// confirmed it, and the active focus, in one request.
pub struct FocusArt;

/// What `focus-art` presents: tree titles and icons, or titles and
/// descriptions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FocusArtMode {
    Display,
    Text,
}

impl FocusArtMode {
    fn presentation(self) -> Presentation {
        match self {
            Self::Display => Presentation::Display,
            Self::Text => Presentation::Text,
        }
    }
}

/// Tokens at one hour, each looked up in one database.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusArtGroup {
    pub hour: i32,
    /// The database every token is looked up in.
    pub database: FocusDatabase,
    /// Distinct tokens in ascending order.
    pub tokens: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusArtParams {
    pub branch_id: String,
    pub tag: String,
    /// Ascending by hour, then database, each pair once; at most a page of
    /// tokens and the active focus over all of them.
    pub groups: Vec<FocusArtGroup>,
    pub mode: FocusArtMode,
}

/// A failure of a whole group's evaluation, and how many tokens it left
/// without art.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusArtFailure {
    pub reason: String,
    pub tokens: u32,
}

/// One group's art in token order (none when it failed): its evaluation's
/// failures, or the error that refused or failed that group alone.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusArtGroupReply {
    pub art: Vec<PresentedFocus>,
    pub failures: Vec<FocusArtFailure>,
    pub error: Option<crate::QueryError>,
}

/// Each group's art, in request order, and the content's standing warnings
/// once.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusArtReply {
    pub groups: Vec<FocusArtGroupReply>,
    pub warnings: Vec<String>,
}

impl crate::products::ReplyWarnings for FocusArtReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

impl Product for FocusArt {
    const KEY: &'static str = "focus-art";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = FocusArtParams;
    type Reply = FocusArtReply;

    fn types() -> Vec<String> {
        vec![
            FocusArtGroup::decl(),
            FocusArtParams::decl(),
            FocusArtMode::decl(),
            FocusArtFailure::decl(),
            FocusArtGroupReply::decl(),
            FocusArtReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_tag(&params.tag)?;
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid focus art branch")?;
        if params.groups.is_empty()
            || params.groups.windows(2).any(|pair| {
                (pair[0].hour, pair[0].database) >= (pair[1].hour, pair[1].database)
            })
        {
            return Err("Focus art hours must be ascending, each with a database once");
        }
        let mut total = 0_usize;
        for group in &params.groups {
            total += group.tokens.len();
            if group.tokens.is_empty()
                || group
                    .tokens
                    .iter()
                    .any(|token| token.is_empty() || token.contains('\0'))
                || group.tokens.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err("Focus art tokens must be distinct and ascending at each hour");
            }
        }
        if total > MAX_TOKENS {
            return Err("Focus art asks for at most a page and the active focus");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let mut warnings: Vec<String> = Vec::new();
        let mut groups = Vec::with_capacity(params.groups.len());
        for group in &params.groups {
            let evaluated = if recorded(source, branch, group.hour) {
                context.content().and_then(|content| {
                    art_at(
                        source,
                        content,
                        branch,
                        &params.tag,
                        group.hour,
                        group.database,
                        &group.tokens,
                        params.mode.presentation(),
                        cancelled,
                    )
                })
            } else {
                Err(dashboard_core::refused("Focus art hour was not recorded"))
            };
            groups.push(match evaluated {
                Ok(hour) => {
                    // Every hour names the same content's standing warnings.
                    for warning in hour.warnings {
                        if !warnings.contains(&warning) {
                            warnings.push(warning);
                        }
                    }
                    FocusArtGroupReply {
                        art: hour.art,
                        failures: hour.failures,
                        error: None,
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(error) => FocusArtGroupReply {
                    art: Vec::new(),
                    failures: Vec::new(),
                    error: Some(crate::QueryError::from(&error)),
                },
            });
        }
        Ok(FocusArtReply { groups, warnings })
    }
}
