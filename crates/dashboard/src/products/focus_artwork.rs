//! Hour-specific focus presentation; history consumers share the native selector.
use super::{LoadContext, Product, artwork, validate_tag};
use crate::{
    Artwork, Source,
    metadata::{check_cancel, validate_campaign},
};
use replay::container::Replay;
use replay_data::rows::TableId;
use replay_frame::{frame::DecodedFrame, frame::FrameDecoder, frame::TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct FocusArtwork;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusArtworkParams {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub tokens: Vec<String>,
    #[serde(default)]
    #[ts(optional)]
    pub text_only: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub include_descriptions: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub icons_only: Option<bool>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DatedFocusArtwork {
    pub artwork: Artwork,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusArtworkReply {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub items: Vec<DatedFocusArtwork>,
    pub warnings: Vec<String>,
}
#[derive(Default)]
struct State {
    artwork: Arc<Mutex<Option<artwork::Context>>>,
    text: Arc<Mutex<Option<artwork::Context>>>,
    titles: Arc<Mutex<Option<artwork::Context>>>,
    frame: Arc<SharedFrames>,
}

/// The history scan has checked the campaign and the complete block checksum.
/// Keep only the inputs a basic presentation read would reconstruct at that hour.
pub(crate) fn retain_basic(
    source: &Source,
    decoded: &DecodedFrame,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    check_cancel(cancelled)?;
    let state = source.state::<State>();
    let frames = Arc::clone(
        &state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .frame,
    );
    frames
        .cache
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .retain_basic(decoded, cancelled)
}

/// One recorded hour's frame of the tables focus presentation reads (the
/// focus tables, scripted-text inputs and Clock), from the frames this
/// product keeps, so `focus-art` and the event feed share its reads.
pub(crate) fn scoped_frame(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<DecodedFrame> {
    check_cancel(cancelled)?;
    let frames = Arc::clone(
        &source
            .state::<State>()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .frame,
    );
    frames.at(&source.replay(), branch, hour, true, cancelled)
}

/// Retained row allocations for recent focus presentation hours. Nested strings
/// and lists owned by rows are outside this measure, as in the viewer frame cache.
/// Frame/table ownership and unavailable reasons also count; shared allocations
/// are conservatively charged once per entry. This never limits returned data.
const FRAME_CACHE_ROW_BYTES: usize = 64 * 1024 * 1024;
const BASIC_TABLES: [TableId; 3] = [TableId::Campaign, TableId::Countries, TableId::CountryLinks];

/// Cached hours remain available while another request reconstructs a miss.
/// Misses serialize and recheck the cache so identical reads still share rows.
#[derive(Default)]
struct SharedFrames {
    cache: Arc<Mutex<FrameCache>>,
    read: Mutex<()>,
}

impl SharedFrames {
    fn at(
        &self,
        replay: &Replay,
        branch: u64,
        hour: i32,
        scoped: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<DecodedFrame> {
        self.at_with(branch, hour, scoped, cancelled, || {
            read_frame(replay, branch, hour, scoped, cancelled)
        })
    }

    fn at_with(
        &self,
        branch: u64,
        hour: i32,
        scoped: bool,
        cancelled: &AtomicBool,
        read: impl FnOnce() -> io::Result<DecodedFrame>,
    ) -> io::Result<DecodedFrame> {
        check_cancel(cancelled)?;
        if let Some(decoded) = self
            .cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .cached(branch, hour, scoped, cancelled)?
        {
            return Ok(decoded);
        }
        let _read = self.read.lock().unwrap_or_else(|error| error.into_inner());
        check_cancel(cancelled)?;
        {
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(decoded) = cache.cached(branch, hour, scoped, cancelled)? {
                return Ok(decoded);
            }
            #[cfg(test)]
            {
                cache.reads += 1;
            }
        }
        let decoded = read()?;
        check_cancel(cancelled)?;
        self.cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(decoded.clone(), scoped, cancelled)?;
        Ok(decoded)
    }
}

struct FrameCache {
    frames: VecDeque<CachedFrame>,
    bytes: usize,
    budget: usize,
    #[cfg(test)]
    reads: usize,
}

struct CachedFrame {
    frame: DecodedFrame,
    scoped: bool,
    bytes: usize,
}

impl Default for FrameCache {
    fn default() -> Self {
        Self::with_budget(FRAME_CACHE_ROW_BYTES)
    }
}

impl FrameCache {
    fn with_budget(budget: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            bytes: 0,
            budget,
            #[cfg(test)]
            reads: 0,
        }
    }

    fn retain_basic(&mut self, decoded: &DecodedFrame, cancelled: &AtomicBool) -> io::Result<()> {
        check_cancel(cancelled)?;
        if let Some(index) = self.frames.iter().position(|entry| {
            entry.frame.branch == decoded.branch && entry.frame.hour == decoded.hour
        }) {
            // A previously scoped read is more capable; history never downgrades it.
            let entry = self.frames.remove(index).expect("cached frame index");
            self.frames.push_back(entry);
            return Ok(());
        }
        let omitted = replay_frame::frame::SharedView::from(TableView::Unavailable {
            reason: "Table not selected for reconstruction".into(),
        });
        let basic = DecodedFrame {
            branch: decoded.branch,
            hour: decoded.hour,
            tables: std::array::from_fn(|index| {
                if BASIC_TABLES.contains(&TableId::ALL[index]) {
                    decoded.tables[index].clone()
                } else {
                    omitted.clone()
                }
            }),
            metrics: decoded.metrics.clone(),
        };
        self.retain(basic, false, cancelled)
    }

    fn frame_bytes(frame: &DecodedFrame) -> usize {
        frame
            .tables
            .iter()
            .fold(size_of::<CachedFrame>(), |bytes, table| {
                let rows = match &**table {
                    TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
                        rows.allocated_bytes()
                    }
                    TableView::Unavailable { reason } => reason.capacity(),
                };
                bytes
                    .saturating_add(size_of::<TableView>() + 2 * size_of::<usize>())
                    .saturating_add(rows)
            })
    }

    fn retain(
        &mut self,
        frame: DecodedFrame,
        scoped: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let bytes = Self::frame_bytes(&frame);
        check_cancel(cancelled)?;
        // Larger frames remain complete replies without entering the cache.
        if bytes > self.budget {
            return Ok(());
        }
        if let Some(index) = self
            .frames
            .iter()
            .position(|entry| entry.frame.branch == frame.branch && entry.frame.hour == frame.hour)
        {
            let previous = self.frames.remove(index).expect("cached frame index");
            self.bytes -= previous.bytes;
        }
        while self.bytes > self.budget - bytes {
            let previous = self.frames.pop_front().expect("retained frame owns bytes");
            self.bytes -= previous.bytes;
        }
        check_cancel(cancelled)?;
        self.frames.push_back(CachedFrame {
            frame,
            scoped,
            bytes,
        });
        self.bytes += bytes;
        Ok(())
    }

    /// Cloning shares row allocations; scoped frames also satisfy basic reads.
    fn cached(
        &mut self,
        branch: u64,
        hour: i32,
        scoped: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<DecodedFrame>> {
        check_cancel(cancelled)?;
        if let Some(index) = self.frames.iter().position(|entry| {
            entry.frame.branch == branch && entry.frame.hour == hour && (!scoped || entry.scoped)
        }) {
            let entry = self.frames.remove(index).expect("cached frame index");
            let decoded = entry.frame.clone();
            self.frames.push_back(entry);
            check_cancel(cancelled)?;
            return Ok(Some(decoded));
        }
        Ok(None)
    }

    #[cfg(test)]
    fn at(
        &mut self,
        replay: &Replay,
        branch: u64,
        hour: i32,
        scoped: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<DecodedFrame> {
        if let Some(decoded) = self.cached(branch, hour, scoped, cancelled)? {
            return Ok(decoded);
        }
        self.reads += 1;
        let decoded = read_frame(replay, branch, hour, scoped, cancelled)?;
        self.retain(decoded.clone(), scoped, cancelled)?;
        Ok(decoded)
    }
}

/// Reconstruct one exact hour without holding the shared cache mutex.
fn read_frame(
    replay: &Replay,
    branch: u64,
    hour: i32,
    scoped: bool,
    cancelled: &AtomicBool,
) -> io::Result<DecodedFrame> {
    check_cancel(cancelled)?;
    let block = replay
        .index
        .iter()
        .find(|block| block.branch == branch && block.first_hour <= hour && hour <= block.last_hour)
        .ok_or_else(|| io::Error::other("Focus artwork hour was not recorded"))?;
    let mut selected = BTreeSet::from(BASIC_TABLES);
    if scoped {
        selected.extend(crate::scripted_text::TABLES.iter().copied());
        selected.insert(TableId::Clock);
    }
    let selected: Vec<_> = selected.into_iter().collect();
    let mut decoder = FrameDecoder::for_tables(&selected);
    let mut found = None;
    replay.visit_block(block, Some(&selected), |frame| {
        check_cancel(cancelled)?;
        if frame.hour > hour {
            return Ok(false);
        }
        let decoded = decoder.advance(&frame)?;
        validate_campaign(decoded, replay)?;
        if frame.hour == hour {
            check_cancel(cancelled)?;
            found = Some(decoded.clone());
            return Ok(false);
        }
        Ok(true)
    })?;
    // Only retain a frame after the replay reader has verified its block.
    let decoded = found.ok_or_else(|| io::Error::other("Focus artwork hour was not recorded"))?;
    check_cancel(cancelled)?;
    Ok(decoded)
}

impl Product for FocusArtwork {
    const KEY: &'static str = "focus-artwork";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = FocusArtworkParams;
    type Reply = FocusArtworkReply;
    fn types() -> Vec<String> {
        vec![
            FocusArtworkParams::decl(),
            DatedFocusArtwork::decl(),
            FocusArtworkReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_tag(&params.tag)?;
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid focus artwork branch")?;
        if params.tokens.is_empty()
            || params
                .tokens
                .iter()
                .any(|token| token.is_empty() || token.contains('\0'))
        {
            return Err("Invalid focus artwork tokens");
        }
        if params.icons_only == Some(true)
            && (params.text_only == Some(true) || params.include_descriptions != Some(false))
        {
            return Err("Icon-only focus artwork requires icons and excludes descriptions");
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
        let replay = source.replay();
        let state = source.state::<State>();
        let (slot, frame) = {
            let state = state.lock().unwrap_or_else(|error| error.into_inner());
            (
                Arc::clone(
                    if params.text_only == Some(true) && params.include_descriptions == Some(false)
                    {
                        &state.titles
                    } else if params.text_only == Some(true) {
                        &state.text
                    } else {
                        &state.artwork
                    },
                ),
                Arc::clone(&state.frame),
            )
        };
        let mut slot = slot.lock().unwrap_or_else(|error| error.into_inner());
        let art = artwork::ensure(&mut slot, source, context.content()?, artwork::Kind::Focus)?;
        let needs_frame = if params.icons_only == Some(true) {
            art.requested_focus_icons_need_frame(&params.tokens)
        } else if params.include_descriptions == Some(false) {
            if params.text_only == Some(true) {
                art.requested_focus_title_needs_frame(&params.tokens)
            } else {
                art.requested_focus_display_needs_frame(&params.tokens)
            }
        } else if params.text_only == Some(true) {
            art.requested_focus_text_needs_frame(&params.tokens)
        } else {
            art.requested_focus_needs_frame(&params.tokens)
        };
        // Reconstruction misses never hold the cache lock needed by cached icons.
        let decoded = frame.at(&replay, branch, params.hour, needs_frame, cancelled)?;
        check_cancel(cancelled)?;
        present_decoded(art, params, &decoded, cancelled)
    }
}

/// The same presentation boundary for already verified preparation frames.
pub(crate) fn present_decoded(
    art: &mut artwork::Context,
    params: &FocusArtworkParams,
    decoded: &DecodedFrame,
    cancelled: &AtomicBool,
) -> io::Result<FocusArtworkReply> {
    check_cancel(cancelled)?;
    let tokens: BTreeSet<_> = params.tokens.iter().map(String::as_str).collect();
    let requests: Vec<_> = tokens
        .into_iter()
        .map(|token| (token, params.tag.as_str()))
        .collect();
    let warnings = art.standing_warnings();
    let collect = |items: &mut dyn Iterator<Item = artwork::ResolvedArt>| {
        reply_within(
            params,
            warnings,
            items,
            cancelled,
            super::MAX_PRODUCT_REPLY_BYTES,
        )
    };
    if params.icons_only == Some(true) {
        art.focus_icons_at(&requests, decoded, collect)
    } else if params.include_descriptions == Some(false) {
        if params.text_only == Some(true) {
            art.focus_titles_at(&requests, decoded, collect)
        } else {
            art.focus_display_at(&requests, decoded, collect)
        }
    } else if params.text_only == Some(true) {
        art.focus_text_at(&requests, decoded, collect)
    } else {
        art.focus_tree_at(&requests, decoded, collect)
    }
}

/// Measure the actual escaped JSON before retaining each lazy artwork item.
fn reply_within(
    params: &FocusArtworkParams,
    warnings: Vec<String>,
    mut items: impl Iterator<Item = artwork::ResolvedArt>,
    cancelled: &AtomicBool,
    budget: usize,
) -> io::Result<FocusArtworkReply> {
    let mut reply = FocusArtworkReply {
        branch_id: params.branch_id.clone(),
        hour: params.hour,
        tag: params.tag.clone(),
        items: Vec::new(),
        warnings,
    };
    let refused = || {
        io::Error::other(format!(
            "Focus artwork exceeds the transport reply budget of {budget} bytes; no tokens were omitted"
        ))
    };
    let mut bytes = crate::transport::json_len(&reply, budget).ok_or_else(refused)?;
    loop {
        check_cancel(cancelled)?;
        let Some((artwork, reason)) = items.next() else {
            break;
        };
        let item = DatedFocusArtwork { artwork, reason };
        let separator = usize::from(!reply.items.is_empty());
        let remaining = budget.saturating_sub(bytes).saturating_sub(separator);
        let size = crate::transport::json_len(&item, remaining).ok_or_else(refused)?;
        bytes += separator + size;
        reply.items.push(item);
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag, source};
    use replay_data::{rows::TableRows, rows::tables};
    use replay_frame::{frame::SharedView, frame::TableView};
    use std::cell::Cell;

    #[test]
    fn event_completion_hours_share_one_trigger_kit_decode_and_reuse_earlier_pages() {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                (10..=15)
                    .map(|hour| source(1, hour, Some(i64::from(hour))))
                    .collect(),
            )],
        )
        .unwrap();
        let (source, context, _folder) = product_source(replay);
        let cancelled = AtomicBool::new(false);
        let requests = [
            (10, "GER", "focus_scoped"),
            (12, "GER", "focus_scoped"),
            (14, "GER", "focus_scoped"),
        ];
        let (first, decodes) = replay::container::test_work::table_decodes(|| {
            crate::prepared_focus::art::event_art(
                &source,
                context.content().unwrap(),
                1,
                &requests,
                &cancelled,
            )
            .unwrap()
            .unwrap()
        });
        for table in [
            TableId::ResearchTechnologies,
            TableId::LexicalTokens,
            TableId::Provinces,
            TableId::States,
            TableId::ArmyOrderPlans,
            TableId::Characters,
            TableId::ScriptVariables,
        ] {
            assert_eq!(
                decodes
                    .iter()
                    .filter(|(_, _, decoded, _)| *decoded == table)
                    .count(),
                1,
                "{table:?}"
            );
        }
        assert_eq!(
            dashboard_base::prepared::query::frames::reads(&source),
            (1, 0)
        );
        let (next, decodes) = replay::container::test_work::table_decodes(|| {
            crate::prepared_focus::art::event_art(
                &source,
                context.content().unwrap(),
                1,
                &requests[1..],
                &cancelled,
            )
            .unwrap()
            .unwrap()
        });
        assert!(decodes.is_empty());
        assert_eq!(
            serde_json::to_value(&next.0).unwrap(),
            serde_json::to_value(&first.0[1..]).unwrap()
        );
        let catalogue = crate::catalogue::catalogue(&source, context.content().unwrap()).unwrap();
        let mut art = artwork::Context::load(catalogue, artwork::Kind::Focus).unwrap();
        for ((hour, tag, token), actual) in requests.iter().zip(&first.0) {
            let frame = read_frame(source.replay_ref(), 1, *hour, true, &cancelled).unwrap();
            let expected = art.focuses_at(&[(*token, *tag)], &frame).remove(0);
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
    }

    fn shared_frames(source: &Source) -> Arc<Mutex<FrameCache>> {
        Arc::clone(&source.state::<State>().lock().unwrap().frame.cache)
    }

    fn product_source(replay: Replay) -> (Source, LoadContext, crate::catalogue::tests::Folder) {
        let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/national_focus/synthetic.txt",
                "focus_tree={focus={id=focus_a text=title_a icon=GFX_missing_a} focus={id=focus_b text=title_b icon=GFX_missing_b} focus={id=focus_scoped text=title_scoped}}",
            ),
            (
                "common/defines/synthetic.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}",
            ),
            (
                "localisation/synthetic_l_english.yml",
                "\u{feff}l_english:\n title_a:0 \"First title\"\n title_b:0 \"Second title\"\n title_scoped:0 \"[ROOT.GetTag] scoped\"\n",
            ),
        ]);
        let (source, context) = product_source_with_catalogue(replay, catalogue, &folder.0);
        (source, context, folder)
    }

    fn product_source_with_catalogue(
        replay: Replay,
        catalogue: crate::catalogue::Catalogue,
        root: &std::path::Path,
    ) -> (Source, LoadContext) {
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "focus-frame-fixture".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let content = super::super::ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: root.to_owned(),
                user_dir: root.join("user"),
                cache_dir: root.join("cache"),
            },
            allow_unverified: true,
            presentation: Default::default(),
        };
        crate::catalogue::set_test_catalogue(&source, &content, Arc::new(catalogue));
        (
            source,
            LoadContext {
                content: Some(content),
            },
        )
    }

    fn icon_product_source(
        replay: Replay,
    ) -> (Source, LoadContext, [crate::catalogue::tests::Folder; 2]) {
        use crate::catalogue::texture_tests::{png_bytes, texture_catalogue};
        use installed_content_image::Rgba8Image;
        let image = |value| {
            png_bytes(&Rgba8Image {
                width: 1,
                height: 1,
                pixels: vec![value, 0, 0, 255],
            })
        };
        let (catalogue, folders) = texture_catalogue(
            &[
                (
                    "common/national_focus/synthetic.txt",
                    "focus_tree={focus={id=static text=scoped_title icon=GFX_mod_static cost=2} focus={id=conditional text=scoped_title icon={GFX_mod_conditional={has_government=mod_group} GFX_mod_static=yes} cost=2}}",
                ),
                (
                    "interface/art.gfx",
                    "spriteTypes={spriteType={name=GFX_mod_static textureFile=gfx/mod/static.png} spriteType={name=GFX_mod_conditional textureFile=gfx/mod/conditional.png}}",
                ),
                (
                    "common/defines/synthetic.lua",
                    "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}",
                ),
                (
                    "common/ideologies/synthetic.txt",
                    "ideologies={mod_group={} other_group={}}",
                ),
                (
                    "common/scripted_localisation/synthetic.txt",
                    "defined_text={name=NeedsParty text={trigger={has_government=mod_group} localization_key=known_title}}",
                ),
                (
                    "localisation/synthetic_l_english.yml",
                    "\u{feff}l_english:\n scoped_title:0 \"[NeedsParty]\"\n scoped_title_desc:0 \"[NeedsParty]\"\n known_title:0 \"Scoped title\"\n",
                ),
            ],
            &[
                ("gfx/mod/static.png", image(23)),
                ("gfx/mod/conditional.png", image(47)),
            ],
        );
        let (source, context) = product_source_with_catalogue(replay, catalogue, &folders[1].0);
        (source, context, folders)
    }

    fn focus_scale_frame(hour: i32, count: u32, selected: u32, tokens: &[&str]) -> DecodedFrame {
        use replay_data::{rows::CompletedFocusRow, rows::FocusRow};
        let mut frame = scale_frame(1, hour, count);
        frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
            (0..count)
                .map(|index| FocusRow {
                    country: 100_000 + index,
                    completed_count: if index == selected {
                        tokens.len() as i32
                    } else {
                        0
                    },
                    ..FocusRow::default()
                })
                .collect(),
        ))
        .into();
        frame.tables[TableId::CompletedFocuses.index()] =
            TableView::Complete(TableRows::CompletedFocuses(
                tokens
                    .iter()
                    .enumerate()
                    .map(|(ordinal, token)| CompletedFocusRow {
                        country: 100_000 + selected,
                        ordinal: ordinal as u32,
                        token: token.as_bytes().to_vec(),
                    })
                    .collect(),
            ))
            .into();
        frame.tables[TableId::ActiveIdeas.index()] =
            TableView::Complete(TableRows::ActiveIdeas(Vec::new())).into();
        if let TableView::Complete(TableRows::Countries(countries)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            countries[selected as usize].political_power_raw = Some(i64::from(hour));
        }
        for table in BASIC_TABLES {
            frame.metrics.table_ns[table.index()] = 1;
        }
        frame
    }

    #[test]
    fn icon_only_requests_preserve_defaults_and_reject_conflicting_fields() {
        let old: FocusArtworkParams = serde_json::from_value(serde_json::json!({
            "branchId": "1", "hour": 100, "tag": "synthetic", "tokens": ["static"]
        }))
        .unwrap();
        assert_eq!(old.icons_only, None);
        assert_eq!(old.text_only, None);
        assert_eq!(old.include_descriptions, None);
        assert!(FocusArtwork::validate(&old).is_ok());
        for icons_only in [None, Some(false), Some(true)] {
            for text_only in [None, Some(false), Some(true)] {
                for include_descriptions in [None, Some(false), Some(true)] {
                    let params = FocusArtworkParams {
                        icons_only,
                        text_only,
                        include_descriptions,
                        ..old.clone()
                    };
                    let valid = icons_only != Some(true)
                        || (text_only != Some(true) && include_descriptions == Some(false));
                    assert_eq!(FocusArtwork::validate(&params).is_ok(), valid, "{params:?}");
                }
            }
        }
    }

    #[test]
    fn static_icon_only_product_skips_scoped_titles_at_five_thousand_countries() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::catalogue::texture_tests::from_url;
        let mut frame = focus_scale_frame(100, 5_000, 2_500, &["static"]);
        for table in [TableId::States, TableId::Provinces] {
            frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
        }
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
        let (source, context, _folders) = icon_product_source(replay);
        let cancelled = AtomicBool::new(false);
        let basic = read_frame(source.replay_ref(), 1, 100, false, &cancelled).unwrap();
        retain_basic(&source, &basic, &cancelled).unwrap();
        let params = FocusArtworkParams {
            branch_id: "1".into(),
            hour: 100,
            tag: String::from_utf8(scale_tag(2_500)).unwrap(),
            tokens: vec!["static".into()],
            text_only: Some(false),
            include_descriptions: Some(false),
            icons_only: Some(true),
        };
        let reply = FocusArtwork::load(&source, &params, &context, &cancelled).unwrap();
        assert_eq!(
            (reply.branch_id.as_str(), reply.hour, &reply.tag),
            ("1", 100, &params.tag)
        );
        let item = &reply.items[0];
        assert_eq!(item.artwork.token, "static");
        assert_eq!(item.artwork.base_duration_days, Some(14.0));
        assert_eq!(
            from_url(item.artwork.icon.as_deref().unwrap()).pixels[0],
            23
        );
        assert!(item.artwork.name.is_none());
        assert!(item.artwork.description.is_none());
        assert!(item.reason.is_none(), "{:?}", item.reason);
        assert!(
            reply
                .warnings
                .iter()
                .all(|warning| !warning.contains("focus localisation"))
        );
        let frames = shared_frames(&source);
        {
            let cache = frames.lock().unwrap();
            assert_eq!(
                cache.reads, 0,
                "scoped text must not upgrade a static icon frame"
            );
            assert_eq!(cache.frames.len(), 1);
            assert!(!cache.frames[0].scoped);
            assert_eq!(
                cache.frames[0]
                    .frame
                    .complete::<tables::Countries>()
                    .unwrap()
                    .len(),
                5_000
            );
        }
        // The same authored title, when requested, still needs the dated kit
        // and keeps its missing-party refusal rather than inventing a value.
        let title = FocusArtwork::load(
            &source,
            &FocusArtworkParams {
                text_only: Some(true),
                icons_only: None,
                ..params
            },
            &context,
            &cancelled,
        )
        .unwrap();
        assert!(title.items[0].artwork.name.is_none());
        assert!(title.items[0].artwork.icon.is_none());
        let reason = title.items[0].reason.as_deref().unwrap();
        assert!(reason.contains("PoliticalParties"), "{reason}");
        assert_eq!(frames.lock().unwrap().reads, 1);
        assert!(frames.lock().unwrap().frames[0].scoped);
    }

    #[test]
    fn every_projection_names_a_focus_missing_from_recorded_content() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let frame = focus_scale_frame(100, 4, 2, &["static"]);
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
        let (source, context, _folders) = icon_product_source(replay);
        let cancelled = AtomicBool::new(false);
        for (text_only, include_descriptions, icons_only) in [
            (Some(false), Some(false), Some(true)),
            (Some(true), Some(false), None),
            (Some(true), Some(true), None),
            (Some(false), None, None),
        ] {
            let params = FocusArtworkParams {
                branch_id: "1".into(),
                hour: 100,
                tag: String::from_utf8(scale_tag(2)).unwrap(),
                tokens: vec!["unauthored".into()],
                text_only,
                include_descriptions,
                icons_only,
            };
            let reply = FocusArtwork::load(&source, &params, &context, &cancelled).unwrap();
            let item = &reply.items[0];
            assert!(item.artwork.name.is_none() && item.artwork.icon.is_none());
            let reason = item.reason.as_deref().unwrap_or_default();
            assert!(
                reason.contains("focus unauthored: definition absent from recorded content"),
                "{text_only:?}/{include_descriptions:?}/{icons_only:?}: {reason}"
            );
        }
    }

    #[test]
    fn conditional_icon_only_product_uses_dated_operands_and_preserves_refusals() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::catalogue::texture_tests::from_url;
        use replay_data::rows::PoliticalPartyRow;
        let mut frames: Vec<_> = (100..103)
            .map(|hour| focus_scale_frame(hour, 5_000, 2_500, &["conditional"]))
            .collect();
        for (frame, ruling_id) in frames.iter_mut().zip([20, 30]) {
            frame.tables[TableId::PoliticalParties.index()] =
                TableView::Complete(TableRows::PoliticalParties(
                    [(20, "mod_group"), (30, "other_group")]
                        .into_iter()
                        .map(|(id, token)| PoliticalPartyRow {
                            country: 102_500,
                            id,
                            is_ruling: u8::from(id == ruling_id),
                            token: token.as_bytes().to_vec(),
                            ..Default::default()
                        })
                        .collect(),
                ))
                .into();
        }
        frames[2].tables[TableId::PoliticalParties.index()] = TableView::Unavailable {
            reason: "synthetic party read failed".into(),
        }
        .into();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let (source, context, _folders) = icon_product_source(replay);
        let cancelled = AtomicBool::new(false);
        let shared = shared_frames(&source);
        for hour in 100..103 {
            let basic = read_frame(source.replay_ref(), 1, hour, false, &cancelled).unwrap();
            retain_basic(&source, &basic, &cancelled).unwrap();
        }
        assert!(
            shared
                .lock()
                .unwrap()
                .frames
                .iter()
                .all(|entry| !entry.scoped)
        );
        for (hour, pixel) in [(100, Some(47)), (101, Some(23)), (102, None)] {
            let params = FocusArtworkParams {
                branch_id: "1".into(),
                hour,
                tag: String::from_utf8(scale_tag(2_500)).unwrap(),
                tokens: vec!["conditional".into()],
                text_only: Some(false),
                include_descriptions: Some(false),
                icons_only: Some(true),
            };
            let reply = FocusArtwork::load(&source, &params, &context, &cancelled).unwrap();
            assert_eq!(reply.items.len(), 1);
            let item = &reply.items[0];
            assert_eq!(item.artwork.base_duration_days, Some(14.0));
            assert!(item.artwork.name.is_none());
            assert!(item.artwork.description.is_none());
            if let Some(pixel) = pixel {
                assert_eq!(
                    from_url(item.artwork.icon.as_deref().unwrap()).pixels[0],
                    pixel
                );
                assert!(item.reason.is_none(), "{:?}", item.reason);
            } else {
                assert!(item.artwork.icon.is_none());
                let reason = item.reason.as_deref().unwrap();
                assert!(reason.contains("PoliticalParties"), "{reason}");
                assert!(!reason.contains("focus localisation"), "{reason}");
                assert!(
                    !reason.contains("selected sprite GFX_mod_static"),
                    "{reason}"
                );
            }
            let cache = shared.lock().unwrap();
            let entry = cache
                .frames
                .iter()
                .find(|entry| entry.frame.hour == hour)
                .unwrap();
            assert!(
                entry.scoped,
                "conditional icons still upgrade their recorded operands"
            );
            assert_eq!(
                entry.frame.complete::<tables::Countries>().unwrap().len(),
                5_000
            );
        }
        assert_eq!(shared.lock().unwrap().reads, 3);
    }

    #[test]
    fn seeded_focus_images_finish_while_an_unrelated_scoped_frame_is_reconstructed() {
        use crate::products::{CountryParams, national_focus::NationalFocus};
        use std::{sync::mpsc, thread, time::Duration};
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    focus_scale_frame(100, 5_000, 2_500, &["focus_a"]),
                    focus_scale_frame(102, 5_000, 2_500, &["focus_a", "focus_b"]),
                ],
            )],
        )
        .unwrap();
        let (source, context, _folder) = product_source(replay);
        let cancelled = AtomicBool::new(false);
        let tag = String::from_utf8(scale_tag(2_500)).unwrap();
        NationalFocus::load(
            &source,
            &CountryParams {
                branch_id: "1".into(),
                tag: tag.clone(),
            },
            &context,
            &cancelled,
        )
        .unwrap();
        let frames = Arc::clone(&source.state::<State>().lock().unwrap().frame);
        let (admitted_tx, admitted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (image_tx, image_rx) = mpsc::channel();
        let image_params = FocusArtworkParams {
            branch_id: "1".into(),
            hour: 100,
            tag,
            tokens: vec!["focus_a".into()],
            text_only: Some(false),
            include_descriptions: Some(false),
            icons_only: None,
        };
        thread::scope(|scope| {
            let reader_frames = &frames;
            let reader_source = &source;
            let reader_cancelled = &cancelled;
            let reader = scope.spawn(move || {
                reader_frames.at_with(1, 102, true, reader_cancelled, || {
                    admitted_tx.send(()).unwrap();
                    release_rx
                        .recv_timeout(Duration::from_secs(10))
                        .map_err(io::Error::other)?;
                    read_frame(reader_source.replay_ref(), 1, 102, true, reader_cancelled)
                })
            });
            admitted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let images = scope.spawn(|| {
                image_tx
                    .send(FocusArtwork::load(
                        &source,
                        &image_params,
                        &context,
                        &cancelled,
                    ))
                    .unwrap()
            });
            // The scoped reconstruction cannot finish before this reply. The
            // timeout only prevents a deadlock from hanging a failing regression.
            let early_image = image_rx.recv_timeout(Duration::from_secs(5));
            release_tx.send(()).unwrap();
            let scoped = reader.join().unwrap().unwrap();
            images.join().unwrap();
            let image = early_image
                .expect("a verified cached image hour must not wait for another scoped read")
                .unwrap();
            assert_eq!(image.items[0].artwork.name.as_deref(), Some("First title"));
            assert!(
                image.items[0]
                    .reason
                    .as_deref()
                    .unwrap()
                    .contains("selected sprite")
            );
            assert_eq!(scoped.hour, 102);
            assert!(matches!(
                &*scoped.tables[TableId::ActiveIdeas.index()],
                TableView::Complete(_)
            ));
        });
        let cache = frames.cache.lock().unwrap();
        assert_eq!(
            cache.reads, 1,
            "the static image reply uses its verified history frame"
        );
        assert!(cache.bytes <= FRAME_CACHE_ROW_BYTES);
    }

    #[test]
    fn concurrent_focus_misses_coalesce_and_a_cancelled_waiter_does_not_cancel_another_lease() {
        use std::{
            sync::{Barrier, mpsc},
            thread,
            time::Duration,
        };
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![source(1, 10, Some(100)), source(1, 12, Some(120))],
            )],
        )
        .unwrap();
        let frames = SharedFrames::default();
        let live = AtomicBool::new(false);
        let cancelled = AtomicBool::new(false);
        let (admitted_tx, admitted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let waiting = Barrier::new(4);
        thread::scope(|scope| {
            let reader_frames = &frames;
            let reader_replay = &replay;
            let reader_live = &live;
            let first = scope.spawn(move || {
                reader_frames.at_with(1, 10, true, reader_live, || {
                    admitted_tx.send(()).unwrap();
                    release_rx
                        .recv_timeout(Duration::from_secs(10))
                        .map_err(io::Error::other)?;
                    read_frame(reader_replay, 1, 10, true, reader_live)
                })
            });
            admitted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let plain = scope.spawn(|| {
                waiting.wait();
                frames.at_with(1, 10, false, &live, || {
                    panic!("a completed scoped miss must coalesce the basic request")
                })
            });
            let scoped = scope.spawn(|| {
                waiting.wait();
                frames.at_with(1, 10, true, &live, || {
                    panic!("a completed scoped miss must coalesce the second scoped request")
                })
            });
            let stopped = scope.spawn(|| {
                waiting.wait();
                frames.at_with(1, 12, true, &cancelled, || {
                    panic!("a cancelled waiter must not begin reconstruction")
                })
            });
            waiting.wait();
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            release_tx.send(()).unwrap();
            let first = first.join().unwrap().unwrap();
            for reused in [
                plain.join().unwrap().unwrap(),
                scoped.join().unwrap().unwrap(),
            ] {
                assert!(SharedView::ptr_eq(
                    &first.tables[TableId::Countries.index()],
                    &reused.tables[TableId::Countries.index()]
                ));
            }
            assert_eq!(
                stopped.join().unwrap().unwrap_err().kind(),
                io::ErrorKind::Interrupted
            );
        });
        let cache = frames.cache.lock().unwrap();
        assert_eq!(cache.reads, 1);
        assert_eq!(cache.frames.len(), 1);
        assert!(cache.frames[0].scoped);
    }

    #[test]
    fn failed_or_cancelled_focus_misses_release_admission_without_retaining_a_frame() {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 10, Some(100))])],
        )
        .unwrap();
        let frames = SharedFrames::default();
        let live = AtomicBool::new(false);
        let error = frames
            .at_with(1, 10, true, &live, || {
                Err(io::Error::other("synthetic frame read failed"))
            })
            .unwrap_err();
        assert_eq!(error.to_string(), "synthetic frame read failed");
        assert!(frames.cache.lock().unwrap().frames.is_empty());
        let cancelled = AtomicBool::new(false);
        assert_eq!(
            frames
                .at_with(1, 10, true, &cancelled, || {
                    let decoded = read_frame(&replay, 1, 10, true, &live)?;
                    cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                    Ok(decoded)
                })
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        assert!(frames.cache.lock().unwrap().frames.is_empty());
        let verified = frames.at(&replay, 1, 10, true, &live).unwrap();
        assert_eq!(power(&verified), Some(100));
        let reused = frames.at(&replay, 1, 10, false, &live).unwrap();
        assert!(SharedView::ptr_eq(
            &verified.tables[TableId::Countries.index()],
            &reused.tables[TableId::Countries.index()]
        ));
        assert_eq!(frames.cache.lock().unwrap().reads, 3);
        let mut bytes = replay.bytes().to_vec();
        let block = &replay.index[0];
        bytes[(block.offset + block.length - 1) as usize] ^= 0x40;
        let corrupt = Replay::open(bytes).unwrap();
        let invalid = SharedFrames::default();
        assert!(invalid.at(&corrupt, 1, 10, true, &live).is_err());
        assert!(
            invalid.cache.lock().unwrap().frames.is_empty(),
            "unverified blocks never enter the cache"
        );
    }

    #[test]
    fn national_focus_product_seeds_verified_completion_frames_at_five_thousand_countries() {
        use crate::products::{CountryParams, national_focus::NationalFocus};
        const COUNTRIES: u32 = 5_000;
        let selected = COUNTRIES / 2;
        let tag = String::from_utf8(scale_tag(selected)).unwrap();
        let frames = vec![
            focus_scale_frame(100, COUNTRIES, selected, &["focus_a"]),
            focus_scale_frame(101, COUNTRIES, selected, &["focus_a"]),
            focus_scale_frame(102, COUNTRIES, selected, &["focus_a", "focus_b"]),
            focus_scale_frame(103, COUNTRIES, selected, &["focus_a", "focus_b"]),
        ];
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let (source, context, _folder) = product_source(replay);
        let cancelled = AtomicBool::new(false);
        let params = CountryParams {
            branch_id: "1".into(),
            tag: tag.clone(),
        };
        let reply = NationalFocus::load(&source, &params, &context, &cancelled).unwrap();
        assert_eq!(
            reply
                .history
                .segments
                .iter()
                .map(|segment| (segment.first_hour, segment.last_hour))
                .collect::<Vec<_>>(),
            [(100, 101), (102, 103)]
        );
        assert_eq!(
            reply.history.segments[1].order.completed[0].completed_by_hour,
            None
        );
        assert_eq!(
            reply.history.segments[1].order.completed[1].completed_by_hour,
            Some(102)
        );
        let shared = shared_frames(&source);
        {
            let mut cache = shared.lock().unwrap();
            assert_eq!(
                cache
                    .frames
                    .iter()
                    .map(|entry| entry.frame.hour)
                    .collect::<Vec<_>>(),
                [100, 102]
            );
            assert_eq!(cache.reads, 0);
            for hour in [100, 102] {
                let cached = cache
                    .at(source.replay_ref(), 1, hour, false, &cancelled)
                    .unwrap();
                let replay::container::SeekResult::Frame(direct) = source
                    .replay_ref()
                    .seek_selected(1, hour, Some(&BASIC_TABLES))
                    .unwrap()
                else {
                    panic!("synthetic completion hour must be recorded");
                };
                assert_eq!(cached, *direct);
                assert_eq!(
                    cached.complete::<tables::Countries>().unwrap().len(),
                    COUNTRIES as usize
                );
                for table in BASIC_TABLES {
                    assert_eq!(cached.tables[table.index()].sampled_at(), Some(hour));
                }
                for table in [
                    TableId::Focus,
                    TableId::CompletedFocuses,
                    TableId::ActiveIdeas,
                ] {
                    assert!(
                        matches!(&*cached.tables[table.index()], TableView::Unavailable { reason } if reason == "Table not selected for reconstruction")
                    );
                }
            }
            assert_eq!(
                cache.reads, 0,
                "seeded basic hours must avoid another block read"
            );
        }
        for (hour, token, title) in [
            (100, "focus_a", "First title"),
            (102, "focus_b", "Second title"),
        ] {
            let params = FocusArtworkParams {
                branch_id: "1".into(),
                hour,
                tag: tag.clone(),
                tokens: vec![token.into()],
                text_only: Some(false),
                include_descriptions: Some(false),
                icons_only: None,
            };
            let displayed = FocusArtwork::load(&source, &params, &context, &cancelled).unwrap();
            assert_eq!(displayed.items[0].artwork.name.as_deref(), Some(title));
            assert!(
                displayed.items[0]
                    .reason
                    .as_deref()
                    .unwrap()
                    .contains("selected sprite")
            );
        }
        assert_eq!(
            shared.lock().unwrap().reads,
            0,
            "actual static image requests must reuse history frames"
        );
        let scoped = FocusArtwork::load(
            &source,
            &FocusArtworkParams {
                branch_id: "1".into(),
                hour: 102,
                tag: tag.clone(),
                tokens: vec!["focus_scoped".into()],
                text_only: Some(true),
                include_descriptions: Some(false),
                icons_only: None,
            },
            &context,
            &cancelled,
        )
        .unwrap();
        assert_eq!(
            scoped.items[0].artwork.name.as_deref(),
            Some(format!("{tag} scoped").as_str())
        );
        assert!(scoped.items[0].reason.is_none());
        assert_eq!(
            shared.lock().unwrap().reads,
            1,
            "scoped operands still require a broad read"
        );
        let broad = shared
            .lock()
            .unwrap()
            .at(source.replay_ref(), 1, 102, true, &cancelled)
            .unwrap();
        assert!(matches!(
            &*broad.tables[TableId::ActiveIdeas.index()],
            TableView::Complete(_)
        ));
        retain_basic(&source, &broad, &cancelled).unwrap();
        let retained = shared
            .lock()
            .unwrap()
            .at(source.replay_ref(), 1, 102, true, &cancelled)
            .unwrap();
        assert!(SharedView::ptr_eq(
            &broad.tables[TableId::ActiveIdeas.index()],
            &retained.tables[TableId::ActiveIdeas.index()]
        ));
        assert_eq!(
            shared.lock().unwrap().reads,
            1,
            "history must not downgrade a scoped frame"
        );
    }

    #[test]
    fn focus_history_never_seeds_corrupt_or_cancelled_product_reads() {
        use crate::products::{CountryParams, national_focus::NationalFocus};
        let valid = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![focus_scale_frame(100, 1, 0, &["focus_a"])])],
        )
        .unwrap();
        let block = &valid.index[0];
        let mut bytes = valid.bytes().to_vec();
        bytes[(block.offset + block.length - 1) as usize] ^= 0x40;
        let corrupt = Replay::open(bytes).unwrap();
        let (source, context, _folder) = product_source(corrupt);
        let params = CountryParams {
            branch_id: "1".into(),
            tag: String::from_utf8(scale_tag(0)).unwrap(),
        };
        assert!(NationalFocus::load(&source, &params, &context, &AtomicBool::new(false)).is_err());
        assert!(shared_frames(&source).lock().unwrap().frames.is_empty());
        let valid = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![focus_scale_frame(100, 1, 0, &["focus_a"])])],
        )
        .unwrap();
        let (source, context, _folder) = product_source(valid);
        assert_eq!(
            NationalFocus::load(&source, &params, &context, &AtomicBool::new(true))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        assert!(shared_frames(&source).lock().unwrap().frames.is_empty());
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            retain_basic(
                &source,
                &focus_scale_frame(100, 1, 0, &["focus_a"]),
                &cancelled
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Interrupted
        );
        assert!(shared_frames(&source).lock().unwrap().frames.is_empty());
    }

    #[test]
    fn seeded_basic_frames_use_the_existing_budget_and_keep_scoped_entries() {
        let frames = [
            source(1, 10, Some(100)),
            source(1, 12, Some(120)),
            source(1, 14, Some(140)),
        ];
        let cancelled = AtomicBool::new(false);
        let mut measuring = FrameCache::default();
        measuring.retain_basic(&frames[0], &cancelled).unwrap();
        let size = measuring.frames[0].bytes;
        let mut cache = FrameCache::with_budget(size * 2);
        for frame in &frames {
            cache.retain_basic(frame, &cancelled).unwrap();
        }
        assert_eq!(
            cache
                .frames
                .iter()
                .map(|entry| entry.frame.hour)
                .collect::<Vec<_>>(),
            [12, 14]
        );
        assert!(cache.bytes <= cache.budget);
        let broad = frames[2].clone();
        cache.retain(broad.clone(), true, &cancelled).unwrap();
        cache.retain_basic(&broad, &cancelled).unwrap();
        assert!(cache.frames.back().unwrap().scoped);
        assert!(SharedView::ptr_eq(
            &cache.frames.back().unwrap().frame.tables[TableId::Countries.index()],
            &broad.tables[TableId::Countries.index()]
        ));
        let mut oversized = FrameCache::with_budget(size - 1);
        oversized.retain_basic(&frames[0], &cancelled).unwrap();
        assert!(oversized.frames.is_empty());
        assert_eq!(oversized.bytes, 0);
    }

    fn power(frame: &DecodedFrame) -> Option<i64> {
        let TableView::Complete(rows) = &*frame.tables[TableId::Countries.index()] else {
            panic!("country fixture must be complete");
        };
        rows.rows::<tables::Countries>().unwrap()[0].political_power_raw
    }

    #[test]
    fn repeated_focus_frames_share_all_recorded_countries() {
        let mut frame = source(1, 10, Some(100));
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        else {
            unreachable!();
        };
        let template = rows[0].clone();
        *rows = (0..5_000)
            .map(|index| {
                let mut country = template.clone();
                country.id = 9001 + index;
                country.canonical_slot = index;
                country.tag = format!("country-{index}").into_bytes();
                country
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
        let mut cache = FrameCache::default();
        let cancelled = AtomicBool::new(false);
        let first = cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        let next = cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        let countries = TableId::Countries.index();
        assert!(SharedView::ptr_eq(
            &first.tables[countries],
            &next.tables[countries]
        ));
        let TableView::Complete(rows) = &*next.tables[countries] else {
            unreachable!();
        };
        assert_eq!(rows.rows::<tables::Countries>().unwrap().len(), 5_000);
    }

    #[test]
    fn interleaved_focus_hours_reuse_their_recorded_allocations() {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![source(1, 10, Some(100)), source(1, 12, Some(120))],
            )],
        )
        .unwrap();
        let mut cache = FrameCache::default();
        let cancelled = AtomicBool::new(false);
        let first = cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        let second = cache.at(&replay, 1, 12, false, &cancelled).unwrap();
        for (hour, expected) in [(10, &first), (12, &second)] {
            let reused = cache.at(&replay, 1, hour, false, &cancelled).unwrap();
            assert_eq!(power(&reused), power(expected));
            assert!(SharedView::ptr_eq(
                &expected.tables[TableId::Countries.index()],
                &reused.tables[TableId::Countries.index()]
            ));
        }
    }

    #[test]
    fn interleaved_focus_hours_keep_scoped_upgrades_for_plain_reads() {
        let mut first = source(1, 10, Some(100));
        first.tables[TableId::ActiveIdeas.index()] =
            TableView::Complete(TableRows::ActiveIdeas(Vec::new())).into();
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![first, source(1, 12, Some(120))])],
        )
        .unwrap();
        let mut cache = FrameCache::default();
        let cancelled = AtomicBool::new(false);
        let narrow = cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        assert!(matches!(
            &*narrow.tables[TableId::ActiveIdeas.index()],
            TableView::Unavailable { .. }
        ));
        let other = cache.at(&replay, 1, 12, false, &cancelled).unwrap();
        let broad = cache.at(&replay, 1, 10, true, &cancelled).unwrap();
        assert!(matches!(
            &*broad.tables[TableId::ActiveIdeas.index()],
            TableView::Complete(_)
        ));
        let other_reused = cache.at(&replay, 1, 12, false, &cancelled).unwrap();
        assert!(SharedView::ptr_eq(
            &other.tables[TableId::Countries.index()],
            &other_reused.tables[TableId::Countries.index()]
        ));
        for scoped in [false, true] {
            let reused = cache.at(&replay, 1, 10, scoped, &cancelled).unwrap();
            for table in [TableId::Countries, TableId::ActiveIdeas] {
                assert!(SharedView::ptr_eq(
                    &broad.tables[table.index()],
                    &reused.tables[table.index()]
                ));
            }
        }
        assert_eq!(cache.frames.len(), 2);
    }

    #[test]
    fn focus_frame_budget_evicts_least_recent_hours_without_changing_replies() {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    source(1, 10, Some(100)),
                    source(1, 12, Some(120)),
                    source(1, 14, Some(140)),
                ],
            )],
        )
        .unwrap();
        let cancelled = AtomicBool::new(false);
        let sample = FrameCache::default()
            .at(&replay, 1, 10, false, &cancelled)
            .unwrap();
        let budget = 2 * FrameCache::frame_bytes(&sample);
        let mut cache = FrameCache::with_budget(budget);
        let first = cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        let second = cache.at(&replay, 1, 12, false, &cancelled).unwrap();
        assert_eq!(
            FrameCache::frame_bytes(&first),
            FrameCache::frame_bytes(&second)
        );
        cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        let third = cache.at(&replay, 1, 14, false, &cancelled).unwrap();
        assert_eq!(power(&third), Some(140));
        assert!(cache.bytes <= budget);
        assert_eq!(
            cache
                .frames
                .iter()
                .map(|entry| entry.frame.hour)
                .collect::<Vec<_>>(),
            vec![10, 14]
        );
        let first_reused = cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        assert!(SharedView::ptr_eq(
            &first.tables[TableId::Countries.index()],
            &first_reused.tables[TableId::Countries.index()]
        ));
        let second_reloaded = cache.at(&replay, 1, 12, false, &cancelled).unwrap();
        assert_eq!(power(&second_reloaded), Some(120));
        assert!(!SharedView::ptr_eq(
            &second.tables[TableId::Countries.index()],
            &second_reloaded.tables[TableId::Countries.index()]
        ));
        assert!(cache.bytes <= budget);
    }

    #[test]
    fn oversized_focus_frames_return_complete_without_retention() {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 10, Some(100))])],
        )
        .unwrap();
        let cancelled = AtomicBool::new(false);
        let mut cache = FrameCache::with_budget(1);
        let first = cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        let second = cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        assert_eq!(first, second);
        assert_eq!(power(&second), Some(100));
        assert!(!SharedView::ptr_eq(
            &first.tables[TableId::Countries.index()],
            &second.tables[TableId::Countries.index()]
        ));
        assert!(cache.frames.is_empty());
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn focus_frame_reuse_keeps_branch_hour_gaps_and_cancellation_distinct() {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[
                records(1, vec![source(1, 10, Some(100)), source(1, 12, Some(120))]),
                records(2, vec![source(2, 10, Some(200))]),
            ],
        )
        .unwrap();
        let mut cache = FrameCache::default();
        let cancelled = AtomicBool::new(false);
        assert_eq!(
            power(&cache.at(&replay, 1, 10, false, &cancelled).unwrap()),
            Some(100)
        );
        assert!(
            cache
                .at(&replay, 1, 11, false, &cancelled)
                .unwrap_err()
                .to_string()
                .contains("was not recorded")
        );
        assert_eq!(
            power(&cache.at(&replay, 1, 12, false, &cancelled).unwrap()),
            Some(120)
        );
        assert_eq!(
            power(&cache.at(&replay, 2, 10, false, &cancelled).unwrap()),
            Some(200)
        );
        assert!(cache.at(&replay, 3, 10, false, &cancelled).is_err());
        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            cache
                .at(&replay, 2, 10, false, &cancelled)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn focus_frame_reuse_validates_intermediate_campaigns_before_retaining() {
        let mut target = source(1, 11, Some(110));
        target.tables[TableId::Campaign.index()] = TableView::Unavailable {
            reason: "synthetic missing campaign read".into(),
        }
        .into();
        let mut replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 10, Some(100)), target.clone()])],
        )
        .unwrap();
        // Alter the header identity after constructing a valid recording. The
        // target's unavailable identity passes the guard, but the earlier
        // complete campaign read must still refuse this block.
        replay.campaign_id = b"different-synthetic-campaign".to_vec();
        validate_campaign(&target, &replay).unwrap();
        let mut cache = FrameCache::default();
        assert!(
            cache
                .at(&replay, 1, 11, false, &AtomicBool::new(false))
                .unwrap_err()
                .to_string()
                .contains("campaign identity")
        );
        assert!(cache.frames.is_empty());
        assert_eq!(cache.bytes, 0);
    }

    fn params() -> FocusArtworkParams {
        FocusArtworkParams {
            branch_id: "1".into(),
            hour: 10,
            tag: "synthetic".into(),
            tokens: vec!["focus".into()],
            text_only: None,
            include_descriptions: None,
            icons_only: None,
        }
    }
    fn item(icon: Option<String>) -> artwork::ResolvedArt {
        (
            Artwork {
                token: "focus".into(),
                name: Some("A \"quoted\" focus".into()),
                description: None,
                base_duration_days: None,
                icon,
            },
            None,
        )
    }

    #[test]
    fn large_token_requests_stop_artwork_generation_at_the_serialized_budget() {
        let mut params = params();
        params.tokens = (0..5_000).map(|index| format!("focus-{index}")).collect();
        assert!(FocusArtwork::validate(&params).is_ok());
        let produced = Cell::new(0);
        let items = params.tokens.iter().map(|token| {
            produced.set(produced.get() + 1);
            let mut resolved = item(Some(format!("data:image/png;base64,{}", "A".repeat(512))));
            resolved.0.token.clone_from(token);
            resolved
        });
        let error =
            reply_within(&params, Vec::new(), items, &AtomicBool::new(false), 1024).unwrap_err();
        assert!(error.to_string().contains("transport reply budget"));
        assert!(
            produced.get() < 5_000,
            "rejection must stop before cloning all requested icons"
        );
    }

    #[test]
    fn reply_admission_charges_escaped_strings_and_item_separators_exactly() {
        let params = params();
        let items = || [item(None), item(None)].into_iter();
        let cancelled = AtomicBool::new(false);
        let reply = reply_within(
            &params,
            vec!["A \"quoted\" warning".into()],
            items(),
            &cancelled,
            super::super::MAX_PRODUCT_REPLY_BYTES,
        )
        .unwrap();
        let size = crate::transport::json_len(&reply, usize::MAX).unwrap();
        assert!(reply_within(&params, reply.warnings.clone(), items(), &cancelled, size).is_ok());
        assert!(
            reply_within(
                &params,
                reply.warnings.clone(),
                items(),
                &cancelled,
                size - 1
            )
            .is_err()
        );
    }
}
