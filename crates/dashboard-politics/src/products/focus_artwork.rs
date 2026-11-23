use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, Product, artwork};
use dashboard_core::dto::{Artwork, FocusDatabase};
use dashboard_base::Source;
use dashboard_text::metadata::check_cancel;
use replay::campaign::{Replay, SeekResult};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct FocusArtwork;
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusArtworkParams {
    pub branch_id: String,
    pub hour: i32,
    pub country: Country,
    #[doc = " The database every token is looked up in."]
    pub database: FocusDatabase,
    pub tokens: Vec<String>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub text_only: Option<bool>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub include_descriptions: Option<bool>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub icons_only: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DatedFocusArtwork {
    pub artwork: Artwork,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusArtworkReply {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub database: FocusDatabase,
    pub items: Vec<DatedFocusArtwork>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct Contexts {
    artwork: Arc<Mutex<Option<artwork::Context>>>,
    text: Arc<Mutex<Option<artwork::Context>>>,
    titles: Arc<Mutex<Option<artwork::Context>>>,
}

#[derive(Default)]
struct State {
    national: Contexts,
    continuous: Contexts,
    frame: Arc<SharedFrames>,
}

const BASIC_TABLES: [TableId; 3] = [TableId::Campaign, TableId::Countries, TableId::CountryLinks];

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

#[derive(Default)]
struct FrameCache {
    frames: VecDeque<CachedFrame>,
    #[cfg(test)]
    reads: usize,
}

struct CachedFrame {
    frame: DecodedFrame,
    scoped: bool,
}

impl FrameCache {
    fn retain(
        &mut self,
        frame: DecodedFrame,
        scoped: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        self.frames.retain(|entry| entry.frame.branch != frame.branch);
        self.frames.push_back(CachedFrame { frame, scoped });
        Ok(())
    }

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

fn read_frame(
    replay: &Replay,
    branch: u64,
    hour: i32,
    scoped: bool,
    cancelled: &AtomicBool,
) -> io::Result<DecodedFrame> {
    check_cancel(cancelled)?;
    if replay.block_at(branch, hour).is_none() {
        return Err(dashboard_core::refused("Focus artwork hour was not recorded"));
    }
    let mut selected = BTreeSet::from(BASIC_TABLES);
    if scoped {
        selected.extend(dashboard_text::scripted_text::TABLES.iter().copied());
        selected.insert(TableId::Clock);
    }
    let selected: Vec<_> = selected.into_iter().collect();
    let decoded = match replay
        .seek_selected_with(branch, hour, Some(&selected), || check_cancel(cancelled))?
    {
        SeekResult::Frame(decoded) => decoded,
        SeekResult::Gap { reason, .. } => {
            return Err(dashboard_base::gap_error(
                &reason,
                "Focus artwork hour was not recorded",
            ));
        }
    };
    held_campaign(replay, branch, hour)?;
    check_cancel(cancelled)?;
    Ok(*decoded)
}

fn held_campaign(replay: &Replay, branch: u64, hour: i32) -> io::Result<()> {
    let Some(block) = replay.block_at(branch, hour) else {
        return Ok(());
    };
    let recording = replay.recording();
    let ordinal = recording
        .run(block.run)?
        .ordinal(hour)
        .ok_or_else(|| dashboard_core::invalid("A block's hour lies outside its run"))?;
    let mut campaign = replay_recording::cells::TableReader::new(recording, block.run, TableId::Campaign, None)?;
    campaign.seek(recording, ordinal)?;
    let held = campaign.view();
    let mut rows = held.rows();
    let named = match (rows.next(), rows.next()) {
        (None, _) => return Ok(()),
        (Some(slot), None) => held.text(0, slot).and_then(|id| held.string(id)),
        _ => None,
    };
    if named != Some(replay.campaign_id.as_slice()) {
        return Err(dashboard_core::invalid(
            "the run's campaign identity at this hour differs from the replay header",
        ));
    }
    Ok(())
}

impl Product for FocusArtwork {
    const KEY: &'static str = "focus-artwork";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = FocusArtworkParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, country, database, tokens, textOnly, includeDescriptions, iconsOnly];
    type Reply = FocusArtworkReply;
    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            FocusArtworkParams::decl(),
            DatedFocusArtwork::decl(),
            FocusArtworkReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_country(&params.country)?;
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
            let contexts = match params.database {
                FocusDatabase::National => &state.national,
                FocusDatabase::Continuous => &state.continuous,
            };
            (
                Arc::clone(
                    if params.text_only == Some(true) && params.include_descriptions == Some(false)
                    {
                        &contexts.titles
                    } else if params.text_only == Some(true) {
                        &contexts.text
                    } else {
                        &contexts.artwork
                    },
                ),
                Arc::clone(&state.frame),
            )
        };
        let mut slot = slot.lock().unwrap_or_else(|error| error.into_inner());
        let art = artwork::ensure(
            &mut slot,
            source,
            context.content()?,
            artwork::Kind::Focus(params.database),
        )?;
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
        let decoded = frame.at(&replay, branch, params.hour, needs_frame, cancelled)?;
        check_cancel(cancelled)?;
        let tag = dashboard_base::queries::query(source, branch)?.holders.name(&params.country, params.hour);
        present_decoded(art, params, &tag, &decoded, cancelled)
    }
}

pub(crate) fn present_decoded(
    art: &mut artwork::Context,
    params: &FocusArtworkParams,
    tag: &str,
    decoded: &DecodedFrame,
    cancelled: &AtomicBool,
) -> io::Result<FocusArtworkReply> {
    check_cancel(cancelled)?;
    let tokens: BTreeSet<_> = params.tokens.iter().map(String::as_str).collect();
    let requests: Vec<_> = tokens
        .into_iter()
        .map(|token| (token, tag))
        .collect();
    let warnings = art.standing_warnings();
    let collect = |items: &mut dyn Iterator<Item = artwork::ResolvedArt>| {
        reply_within(
            params,
            tag,
            warnings,
            items,
            cancelled,
            dashboard_base::products::MAX_PRODUCT_REPLY_BYTES,
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

fn reply_within(
    params: &FocusArtworkParams,
    tag: &str,
    warnings: Vec<String>,
    mut items: impl Iterator<Item = artwork::ResolvedArt>,
    cancelled: &AtomicBool,
    budget: usize,
) -> io::Result<FocusArtworkReply> {
    let mut reply = FocusArtworkReply {
        branch_id: params.branch_id.clone(),
        hour: params.hour,
        tag: tag.to_owned(),
        database: params.database,
        items: Vec::new(),
        warnings,
    };
    let refused = || {
        io::Error::other(format!(
            "Focus artwork exceeds the transport reply budget of {budget} bytes; no tokens were omitted"
        ))
    };
    let mut bytes = dashboard_core::transport::json_len(&reply, budget).ok_or_else(refused)?;
    loop {
        check_cancel(cancelled)?;
        let Some((artwork, reason)) = items.next() else {
            break;
        };
        let item = DatedFocusArtwork { artwork, reason };
        let separator = usize::from(!reply.items.is_empty());
        let remaining = budget.saturating_sub(bytes).saturating_sub(separator);
        let size = dashboard_core::transport::json_len(&item, remaining).ok_or_else(refused)?;
        bytes += separator + size;
        reply.items.push(item);
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_text::metadata::validate_campaign;
    use dashboard_base::test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag, source};
    use replay_data::{rows::TableRows, rows::tables};
    use replay_frame::{frame::SharedView, frame::TableView};
    use std::cell::Cell;

    fn shared_frames(source: &Source) -> Arc<Mutex<FrameCache>> {
        Arc::clone(&source.state::<State>().lock().unwrap().frame.cache)
    }

    fn product_source_with_catalogue(
        replay: Replay,
        catalogue: dashboard_base::catalogue::Catalogue,
        root: &std::path::Path,
    ) -> (Source, LoadContext) {
        let source = Source::from_replay(
            dashboard_base::SourceKey {
                source_id: "focus-frame-fixture".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let content = dashboard_base::products::ContentOptions {
            options: content_reader_layers::ResolveOptions {
                game_dir: root.to_owned(),
                user_dir: root.join("user"),
                cache_dir: root.join("cache"),
            },
            allow_unverified: true,
            presentation: Default::default(),
        };
        dashboard_base::catalogue::set_test_catalogue(&source, &content, Arc::new(catalogue));
        (
            source,
            LoadContext {
                content: Some(content),
            },
        )
    }

    fn icon_product_source(
        replay: Replay,
    ) -> (Source, LoadContext, [dashboard_base::catalogue::tests::Folder; 2]) {
        use dashboard_base::catalogue::texture_tests::{png_bytes, texture_catalogue};
        use content_reader_image::Rgba8Image;
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
            "branchId": "1", "hour": 100, "country": "synthetic", "database": "national", "tokens": ["static"]
        }))
        .unwrap();
        assert_eq!(old.database, FocusDatabase::National);
        assert!(
            serde_json::from_value::<FocusArtworkParams>(serde_json::json!({
                "branchId": "1", "hour": 100, "country": "synthetic", "tokens": ["static"]
            }))
            .is_err()
        );
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
    fn static_icon_only_product_skips_scoped_titles() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
        use dashboard_base::catalogue::texture_tests::from_url;
        let mut frame = focus_scale_frame(100, 24, 12, &["static"]);
        for table in [TableId::States, TableId::Provinces] {
            frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
        }
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
        let (source, context, _folders) = icon_product_source(replay);
        let cancelled = AtomicBool::new(false);
        let basic = read_frame(source.replay_ref(), 1, 100, false, &cancelled).unwrap();
        shared_frames(&source).lock().unwrap().retain(basic, false, &cancelled).unwrap();
        let params = FocusArtworkParams {
            branch_id: "1".into(),
            hour: 100,
            country: Country::from(String::from_utf8(scale_tag(12)).unwrap()),
            database: FocusDatabase::National,
            tokens: vec!["static".into()],
            text_only: Some(false),
            include_descriptions: Some(false),
            icons_only: Some(true),
        };
        let reply = FocusArtwork::load(&source, &params, &context, &cancelled).unwrap();
        assert_eq!(
            (reply.branch_id.as_str(), reply.hour, &reply.tag),
            ("1", 100, &params.country.to_string())
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
                24
            );
        }
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
            .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
        let frame = focus_scale_frame(100, 4, 2, &["static"]);
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
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
                country: Country::from(String::from_utf8(scale_tag(2)).unwrap()),
                database: FocusDatabase::National,
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

    fn database_files(refused: bool) -> Vec<(&'static str, &'static str)> {
        let mut files = vec![
            (
                "common/national_focus/synthetic.txt",
                "focus_tree={focus={id=shared text=national_title cost=2} focus={id=national_only text=national_title}}",
            ),
            (
                "common/continuous_focus/synthetic.txt",
                "continuous_focus_palette={focus={id=shared text=continuous_title}}",
            ),
            (
                "common/defines/synthetic.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}",
            ),
            (
                "localisation/synthetic_l_english.yml",
                "\u{feff}l_english:\n national_title:0 \"National title\"\n continuous_title:0 \"Continuous title\"\n",
            ),
        ];
        if refused {
            files.push((
                "common/continuous_focus/zz_refused.txt",
                "continuous_focus_palette={focus={text=continuous_title}}",
            ));
        }
        files
    }

    #[test]
    fn each_focus_database_answers_its_own_art_and_names() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
        for refused in [false, true] {
            let (catalogue, folder) =
                dashboard_base::catalogue::tests::folder_catalogue(&database_files(refused));
            let frame = focus_scale_frame(100, 4, 2, &["shared"]);
            let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
            let (source, context) = product_source_with_catalogue(replay, catalogue, &folder.0);
            let cancelled = AtomicBool::new(false);
            let tag = String::from_utf8(scale_tag(2)).unwrap();
            let load = |database, text_only| {
                FocusArtwork::load(
                    &source,
                    &FocusArtworkParams {
                        branch_id: "1".into(),
                        hour: 100,
                        country: Country::from(tag.clone()),
                        database,
                        tokens: vec!["national_only".into(), "shared".into()],
                        text_only: Some(text_only),
                        include_descriptions: Some(false),
                        icons_only: None,
                    },
                    &context,
                    &cancelled,
                )
                .unwrap()
            };
            let item = |reply: &FocusArtworkReply, token: &str| {
                reply
                    .items
                    .iter()
                    .find(|item| item.artwork.token == token)
                    .unwrap()
                    .clone()
            };
            for text_only in [true, false] {
                let national = load(FocusDatabase::National, text_only);
                assert_eq!(national.database, FocusDatabase::National);
                for token in ["national_only", "shared"] {
                    let art = item(&national, token);
                    assert_eq!(
                        art.artwork.name.as_deref(),
                        Some("National title"),
                        "{token}"
                    );
                    if text_only {
                        assert_eq!(art.reason, None, "{token}");
                    }
                }
                assert_eq!(
                    item(&national, "shared").artwork.base_duration_days,
                    Some(14.0)
                );
                let continuous = load(FocusDatabase::Continuous, text_only);
                assert_eq!(continuous.database, FocusDatabase::Continuous);
                let only = item(&continuous, "national_only");
                assert_eq!(only.artwork.name, None);
                let shared = item(&continuous, "shared");
                assert_eq!(shared.artwork.base_duration_days, None);
                if refused {
                    assert_eq!(shared.artwork.name, None);
                    for art in [only, shared] {
                        let reason = art.reason.unwrap();
                        assert!(
                            reason.contains("common/continuous_focus/zz_refused.txt"),
                            "{reason}"
                        );
                    }
                } else {
                    assert_eq!(shared.artwork.name.as_deref(), Some("Continuous title"));
                    assert!(
                        only.reason.as_deref().unwrap().contains(
                            "focus national_only: definition absent from recorded content"
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn focus_history_text_keeps_each_database_apart() {
        use dashboard_base::products::CountryParams;
        use crate::products::national_focus::NationalFocus;
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
        for refused in [false, true] {
            let (catalogue, folder) =
                dashboard_base::catalogue::tests::folder_catalogue(&database_files(refused));
            let mut frame = focus_scale_frame(100, 4, 2, &["shared"]);
            if let TableView::Complete(TableRows::Focus(rows)) =
                &mut *frame.tables[TableId::Focus.index()]
            {
                rows[2].continuous = Some(b"shared".to_vec());
            }
            let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![frame])])
                .unwrap()
                .into_timelines(&Default::default())
                .unwrap();
            let (source, context) = product_source_with_catalogue(replay, catalogue, &folder.0);
            let params = CountryParams {
                branch_id: "1".into(),
                country: Country::from(String::from_utf8(scale_tag(2)).unwrap()),
            };
            let reply =
                NationalFocus::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
            let order = &reply.history.segments[0].order;
            assert_eq!(order.completed[0].token, "shared");
            assert_eq!(order.continuous.as_deref(), Some("shared"));
            let texts = |items: &[Artwork]| {
                items
                    .iter()
                    .map(|item| {
                        (
                            item.token.clone(),
                            item.name.clone(),
                            item.base_duration_days,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                texts(reply.static_text.as_deref().unwrap()),
                [(
                    "shared".to_owned(),
                    Some("National title".to_owned()),
                    Some(14.0)
                )]
            );
            assert_eq!(
                reply
                    .artwork
                    .iter()
                    .map(|item| (item.token.as_str(), item.base_duration_days))
                    .collect::<Vec<_>>(),
                [("shared", Some(14.0))]
            );
            assert_eq!(
                texts(&reply.continuous_static_text),
                [(
                    "shared".to_owned(),
                    (!refused).then(|| "Continuous title".to_owned()),
                    None
                )]
            );
        }
    }

    #[test]
    fn conditional_icon_only_product_uses_dated_operands_and_preserves_refusals() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
        use dashboard_base::catalogue::texture_tests::from_url;
        use replay_data::rows::PoliticalPartyRow;
        let mut frames: Vec<_> = (100..103)
            .map(|hour| focus_scale_frame(hour, 24, 12, &["conditional"]))
            .collect();
        for (frame, ruling_id) in frames.iter_mut().zip([20, 30]) {
            frame.tables[TableId::PoliticalParties.index()] =
                TableView::Complete(TableRows::PoliticalParties(
                    [(20, "mod_group"), (30, "other_group")]
                        .into_iter()
                        .map(|(id, token)| PoliticalPartyRow {
                            country: 100_012,
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
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let (source, context, _folders) = icon_product_source(replay);
        let cancelled = AtomicBool::new(false);
        let shared = shared_frames(&source);
        for hour in 100..103 {
            let basic = read_frame(source.replay_ref(), 1, hour, false, &cancelled).unwrap();
            shared.lock().unwrap().retain(basic, false, &cancelled).unwrap();
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
                country: Country::from(String::from_utf8(scale_tag(12)).unwrap()),
                database: FocusDatabase::National,
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
                24
            );
        }
        assert_eq!(shared.lock().unwrap().reads, 3);
    }

    #[test]
    fn concurrent_focus_misses_coalesce_and_a_cancelled_waiter_does_not_cancel_another_lease() {
        use std::{
            sync::{Barrier, mpsc},
            thread,
            time::Duration,
        };
        let replay = Replay::from_records(
            recorder_derive::writers(),
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
            recorder_derive::writers(),
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
        let corrupt = Replay::open(replay.damaged_block(&replay.index[0]).unwrap()).unwrap();
        let invalid = SharedFrames::default();
        assert!(invalid.at(&corrupt, 1, 10, true, &live).is_err());
        assert!(
            invalid.cache.lock().unwrap().frames.is_empty(),
            "unverified blocks never enter the cache"
        );
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
        *rows = (0..24)
            .map(|index| {
                let mut country = template.clone();
                country.id = 9001 + index;
                country.canonical_slot = index;
                country.tag = format!("country-{index}").into_bytes();
                country
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
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
        assert_eq!(rows.rows::<tables::Countries>().unwrap().len(), 24);
    }

    #[test]
    fn a_branch_keeps_its_latest_focus_hour_and_reads_an_earlier_one_again() {
        let replay = Replay::from_records(
            recorder_derive::writers(),
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
        let reused = cache.at(&replay, 1, 12, false, &cancelled).unwrap();
        assert!(SharedView::ptr_eq(
            &second.tables[TableId::Countries.index()],
            &reused.tables[TableId::Countries.index()]
        ));
        assert_eq!(cache.reads, 2);
        let again = cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        assert_eq!(again, first);
        assert_eq!(cache.reads, 3);
        assert_eq!(cache.frames.len(), 1);
    }

    #[test]
    fn a_scoped_upgrade_answers_plain_reads_of_its_hour() {
        let mut first = source(1, 10, Some(100));
        first.tables[TableId::ActiveIdeas.index()] =
            TableView::Complete(TableRows::ActiveIdeas(Vec::new())).into();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![first])]).unwrap();
        let mut cache = FrameCache::default();
        let cancelled = AtomicBool::new(false);
        let narrow = cache.at(&replay, 1, 10, false, &cancelled).unwrap();
        assert!(matches!(
            &*narrow.tables[TableId::ActiveIdeas.index()],
            TableView::Unavailable { .. }
        ));
        let broad = cache.at(&replay, 1, 10, true, &cancelled).unwrap();
        assert!(matches!(
            &*broad.tables[TableId::ActiveIdeas.index()],
            TableView::Complete(_)
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
        assert_eq!(cache.frames.len(), 1);
    }

    fn power(frame: &DecodedFrame) -> Option<i64> {
        let TableView::Complete(rows) = &*frame.tables[TableId::Countries.index()] else {
            panic!("country fixture must be complete");
        };
        rows.rows::<tables::Countries>().unwrap()[0].political_power_raw
    }

    #[test]
    fn focus_frame_reuse_keeps_branch_hour_gaps_and_cancellation_distinct() {
        let replay = Replay::from_records(
            recorder_derive::writers(),
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
            recorder_derive::writers(),
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 10, Some(100)), target.clone()])],
        )
        .unwrap();
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
    }

    fn params() -> FocusArtworkParams {
        FocusArtworkParams {
            branch_id: "1".into(),
            hour: 10,
            country: "synthetic".into(),
            database: FocusDatabase::National,
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
            let mut resolved = item(Some(format!("artwork:{}.png", "a".repeat(32))));
            resolved.0.token.clone_from(token);
            resolved
        });
        let error =
            reply_within(&params, "synthetic", Vec::new(), items, &AtomicBool::new(false), 1024).unwrap_err();
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
            "synthetic",
            vec!["A \"quoted\" warning".into()],
            items(),
            &cancelled,
            dashboard_base::products::MAX_PRODUCT_REPLY_BYTES,
        )
        .unwrap();
        let size = dashboard_core::transport::json_len(&reply, usize::MAX).unwrap();
        assert!(reply_within(&params, "synthetic", reply.warnings.clone(), items(), &cancelled, size).is_ok());
        assert!(
            reply_within(
                &params,
                "synthetic",
                reply.warnings.clone(),
                items(),
                &cancelled,
                size - 1
            )
            .is_err()
        );
    }
}
