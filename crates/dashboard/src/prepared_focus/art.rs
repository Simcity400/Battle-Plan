//! Focus art at an hour, evaluated when a reply asks for it: titles, icons and
//! descriptions as the focus tree shows them (`focus-art`), the content's
//! static text (`focus-static-text`) and the event feed's completion art.
//! A request whose tokens read no recorded state reads no recording; one
//! that does reads that hour's frame (shared with `focus-artwork`'s frames).
//! Results that read no recorded state are kept per content binding, within
//! a byte budget, least recently used evicted first.
use super::*;
use crate::products::{Product, validate_tag};
use replay::frame::SharedView;
use std::sync::{Arc, PoisonError};

/// Most tokens one `focus-art` request evaluates: one page of a country's
/// completed focuses.
const MAX_TOKENS: usize = PAGE_ROWS;

/// Bytes of art kept across requests (icons included). A budget on reuse
/// only: evicted art is evaluated again, never refused or shortened.
const KEPT_ART_BYTES: usize = 64 * 1024 * 1024;

type ArtKey = (Presentation, String, String);

/// Art that reads no recorded state, kept within a byte budget: the least
/// recently used goes first.
pub(crate) struct Kept {
    entries: BTreeMap<ArtKey, (artwork::ResolvedArt, u64, usize)>,
    used: BTreeMap<u64, ArtKey>,
    next: u64,
    bytes: usize,
    budget: usize,
}

impl Default for Kept {
    fn default() -> Self {
        Self::with_budget(KEPT_ART_BYTES)
    }
}

/// A whole map node for one entry of `T`: a node holds at least one entry and
/// reserves room for eleven, its metadata and an internal node's twelve edges.
const fn node<T>() -> usize {
    11 * std::mem::size_of::<T>() + 16 * std::mem::size_of::<usize>()
}

impl Kept {
    pub(crate) fn with_budget(budget: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            used: BTreeMap::new(),
            next: 0,
            bytes: 0,
            budget,
        }
    }

    /// An entry's retained bytes: its key, held twice (the entry and its
    /// place in the recency order), both maps' nodes for it, its art (icon
    /// included) and reason.
    fn size(key: &ArtKey, (art, reason): &artwork::ResolvedArt) -> usize {
        let text = |text: &Option<String>| text.as_ref().map_or(0, String::capacity);
        // A whole node of each map bounds its place there.
        node::<(ArtKey, (artwork::ResolvedArt, u64, usize))>()
            + node::<(u64, ArtKey)>()
            + 2 * (key.1.capacity() + key.2.capacity())
            + art.token.capacity()
            + text(&art.name)
            + text(&art.description)
            + text(&art.icon)
            + text(reason)
    }

    /// The kept art of `key`, now the most recently used.
    pub(crate) fn get(&mut self, key: &ArtKey) -> Option<artwork::ResolvedArt> {
        let (item, used, _) = self.entries.get_mut(key)?;
        self.used.remove(used);
        *used = self.next;
        self.used.insert(self.next, key.clone());
        self.next += 1;
        Some(item.clone())
    }

    /// Keep `item`, evicting the least recently used until it fits; an item
    /// larger than the whole budget is not kept.
    pub(crate) fn insert(&mut self, key: ArtKey, item: artwork::ResolvedArt) {
        let size = Self::size(&key, &item);
        if let Some((_, used, old)) = self.entries.remove(&key) {
            self.used.remove(&used);
            self.bytes -= old;
        }
        if size > self.budget {
            return;
        }
        while self.bytes + size > self.budget {
            let Some((_, oldest)) = self.used.pop_first() else {
                break;
            };
            if let Some((_, _, old)) = self.entries.remove(&oldest) {
                self.bytes -= old;
            }
        }
        self.used.insert(self.next, key.clone());
        self.entries.insert(key, (item, self.next, size));
        self.next += 1;
        self.bytes += size;
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
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
}

/// Focus tree presentation over one content binding. Results that read no
/// recorded state are kept; the others are evaluated on each hour's frame.
#[derive(Default)]
pub(crate) struct Evaluator {
    binding: Option<Binding>,
    context: Option<artwork::Context>,
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
            self.context = None;
            self.catalogue = None;
            self.warnings.clear();
            self.kept = Kept::default();
            let result = (|| {
                let catalogue = crate::catalogue::catalogue(source, content)?;
                self.catalogue = Some(Arc::clone(&catalogue));
                let context = artwork::Context::load(catalogue, artwork::Kind::Focus)?;
                self.warnings = context.standing_warnings();
                self.warnings.sort();
                self.warnings.dedup();
                self.context = Some(context);
                Ok(())
            })();
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
            });
        }
        self.binding
            .as_ref()
            .expect("focus content bound")
            .result
            .as_ref()
            .map(|_| ())
            .map_err(crate::catalogue::ContentError::error)
    }

    /// Whether any of `tokens` reads recorded state for `tag` in `mode`.
    fn needs_frame(&self, tag: &str, tokens: &[String], mode: Presentation) -> bool {
        let context = self.context.as_ref().expect("focus context loaded");
        tokens
            .iter()
            .any(|token| context.requested_tree_focus_needs_frame(token, tag, mode))
    }

    /// `tokens`' presentation for `tag` on `inputs`, in token order, as the
    /// focus tree shows it.
    fn present(
        &mut self,
        inputs: &artwork::FocusFrame<'_>,
        tag: &str,
        tokens: &[String],
        mode: Presentation,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<PresentedFocus>> {
        check_cancel(cancelled)?;
        let context = self.context.as_mut().expect("focus context loaded");
        let mut requests = Vec::new();
        // Kept art is taken now, so keeping new art cannot evict a row this
        // request counts on.
        let mut planned = Vec::with_capacity(tokens.len());
        for token in tokens {
            // A country's own sprite makes otherwise shared art its own.
            let scoped = matches!(
                mode,
                Presentation::Display | Presentation::Icons | Presentation::Full
            ) && context.focus_country_icon_override(token, tag);
            let key = (
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
                items
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
                    let item = evaluated
                        .next()
                        .ok_or_else(|| io::Error::other("Focus presentation omitted token"))?;
                    if !dynamic {
                        self.kept.insert(key, item.clone());
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

/// `tokens`' focus tree art for `tag` at `hour`, as hourly preparation
/// presented it at that hour, in token order. A content binding that failed,
/// or an evaluation that failed, is the reply's named, counted failure.
#[allow(clippy::too_many_arguments)]
pub fn art_at(
    source: &Source,
    content: &ContentOptions,
    branch: u64,
    tag: &str,
    hour: i32,
    tokens: &[String],
    mode: Presentation,
    cancelled: &AtomicBool,
) -> io::Result<FocusArtReply> {
    check_cancel(cancelled)?;
    let state = source.state::<ArtState>();
    let failed = |reason: String, warnings: Vec<String>| FocusArtReply {
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
            if frame.is_some() || !evaluator.needs_frame(tag, tokens, mode) {
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
                return match evaluator.present(&inputs, tag, tokens, mode, cancelled) {
                    Ok(art) => Ok(FocusArtReply {
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

/// The content's text for each of `tokens`, in request order (repeats
/// included): titles and descriptions that read no recorded state.
pub fn static_text(
    source: &Source,
    content: Option<&ContentOptions>,
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
    let context = state.0.context.as_ref().expect("focus context loaded");
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

/// The event feed's completion art context, which names a definition's icon
/// and full text as an event shows them.
#[derive(Default)]
struct EventArtState(Option<artwork::Context>);

/// Each completion's art at its own hour, for `requests` of (hour, tag,
/// token), as the event feed shows it, with the content's standing warnings;
/// the content's refusal when it does not load.
pub(crate) fn event_art(
    source: &Source,
    content: &ContentOptions,
    branch: u64,
    requests: &[(i32, &str, &str)],
    cancelled: &AtomicBool,
) -> io::Result<Result<(Vec<artwork::ResolvedArt>, Vec<String>), String>> {
    check_cancel(cancelled)?;
    let state = source.state::<EventArtState>();
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    let art = match artwork::ensure(&mut state.0, source, content, artwork::Kind::Focus) {
        Ok(art) => art,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => return Ok(Err(error.to_string())),
    };
    let mut by_hour: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
    for (index, (hour, _, _)) in requests.iter().enumerate() {
        by_hour.entry(*hour).or_default().push(index);
    }
    let mut results: Vec<Option<artwork::ResolvedArt>> = vec![None; requests.len()];
    for (hour, indices) in by_hour {
        check_cancel(cancelled)?;
        let tokens: Vec<String> = indices
            .iter()
            .map(|&index| requests[index].2.to_owned())
            .collect();
        let frame = if art.requested_focus_needs_frame(&tokens) {
            #[cfg(test)]
            FRAME_READS.with(|reads| reads.set(reads.get() + 1));
            crate::products::focus_artwork::scoped_frame(source, branch, hour, cancelled)?
        } else {
            unread_frame(branch, hour)
        };
        let asked: Vec<(&str, &str)> = indices
            .iter()
            .map(|&index| (requests[index].2, requests[index].1))
            .collect();
        for (index, item) in indices.iter().zip(art.focuses_at(&asked, &frame)) {
            results[*index] = Some(item);
        }
    }
    let art: Vec<artwork::ResolvedArt> = results
        .into_iter()
        .map(|item| item.ok_or_else(|| io::Error::other("Focus completion art omitted a row")))
        .collect::<io::Result<_>>()?;
    let warnings = state
        .0
        .as_ref()
        .map(artwork::Context::standing_warnings)
        .unwrap_or_default();
    Ok(Ok((art, warnings)))
}

/// `focus-art`: a country's focus art at one hour.
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusArtParams {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    /// Distinct tokens in ascending order, at most one page.
    pub tokens: Vec<String>,
    pub mode: FocusArtMode,
}

/// A failure of the whole request, and how many tokens it left without art.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusArtFailure {
    pub reason: String,
    pub tokens: u32,
}

/// The requested tokens' art in token order (none when the request failed),
/// the content's standing warnings and the request's failures.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusArtReply {
    pub art: Vec<PresentedFocus>,
    pub warnings: Vec<String>,
    pub failures: Vec<FocusArtFailure>,
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
            FocusArtParams::decl(),
            FocusArtMode::decl(),
            FocusArtFailure::decl(),
            FocusArtReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_tag(&params.tag)?;
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid focus art branch")?;
        if params.tokens.is_empty()
            || params.tokens.len() > MAX_TOKENS
            || params
                .tokens
                .iter()
                .any(|token| token.is_empty() || token.contains('\0'))
            || params.tokens.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err("Focus art tokens must be distinct, ascending and at most one page");
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
        if !recorded(source, branch, params.hour) {
            return Err(io::Error::other("Focus art hour was not recorded"));
        }
        art_at(
            source,
            context.content()?,
            branch,
            &params.tag,
            params.hour,
            &params.tokens,
            params.mode.presentation(),
            cancelled,
        )
    }
}
