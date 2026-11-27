use crate::{
    Source,
    catalogue::{self, Catalogue, plain_text},
    findings::{Finding, Kind},
    products::{
        ContentOptions,
        at_hour::{self, ReadDates},
    },
};
use content_reader_localisation::localisation::{Argument, Localisation};
use dashboard_text::{
    scripted_text::{
        EventObject, EventScope, EventScopes, Link, ScriptedText, TABLES as SCRIPTED_TABLES,
        probe_unselected_tables,
    },
    triggers::{
        NULL_STATE,
        inputs::{Inputs, TableRead},
    },
};
use replay::campaign::Replay;
use replay_data::rows::TableId;
use replay_frame::frame::{DecodedFrame, Metrics, TableView, not_selected};
use std::{
    collections::{HashMap, VecDeque},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

pub const MAX_CACHED_FRAMES: usize = 4;

const UNRECORDED_RANDOM_SEED: &str = "random seed is not recorded for this text";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Empty,
    Country(u32),
    State(u32),
}

impl Scope {
    fn root_and_scopes(self) -> (u32, EventScopes) {
        let (root_id, object, root_link) = match self {
            Self::Empty => (0, EventObject::State(NULL_STATE), Link::Unknown),
            Self::Country(slot) => (slot, EventObject::Country(slot), Link::Itself),
            Self::State(state) => (0, EventObject::State(state), Link::Itself),
        };
        let mut scope = EventScope::of(object);
        scope.root = root_link;
        scope.from = Link::Unknown;
        scope.prev = Link::Unknown;
        scope.random_seed_refusal = Some(UNRECORDED_RANDOM_SEED);
        (root_id, EventScopes::new(vec![scope]))
    }
}

#[derive(Default)]
pub struct Validity {
    reads: Vec<TableRead>,
    clock_dependent: bool,
}

impl Validity {
    pub fn record(&mut self, inputs: &Inputs) {
        match inputs.footprint() {
            Some(reads) => self.reads.extend_from_slice(reads),
            None => self.clock_dependent = true,
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.clock_dependent && self.reads.is_empty()
    }

    pub fn through(
        &self,
        dates: &mut ReadDates,
        replay: &Replay,
        run: u32,
        first: i32,
        hour: i32,
        last: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<i32> {
        if self.clock_dependent {
            return Ok(hour);
        }
        if self.reads.is_empty() {
            return Ok(i32::MAX);
        }
        if !at_hour::reading_dates() {
            dashboard_text::metadata::check_cancel(cancelled)?;
            return Ok(hour);
        }
        dates.through(replay, run, &self.reads, first, hour, last, cancelled)
    }
}

pub fn has_scripted_expression(text: &str) -> bool {
    text.find('[').is_some_and(|open| text[open + 1..].contains(']'))
}

pub fn key_needs_scope(
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
) -> bool {
    localisation
        .localise_with_fallbacks(key.as_bytes(), arguments)
        .0
        .is_ok_and(|raw| has_scripted_expression(&raw))
}

struct BoundedFrames {
    entries: HashMap<(u32, i32), DecodedFrame>,
    order: VecDeque<(u32, i32)>,
}

impl BoundedFrames {
    fn new() -> Self {
        Self { entries: HashMap::new(), order: VecDeque::new() }
    }

    fn take_or_empty(&mut self, run: u32, hour: i32) -> DecodedFrame {
        if let Some(frame) = self.entries.remove(&(run, hour)) {
            if let Some(pos) = self.order.iter().position(|&k| k == (run, hour)) {
                self.order.remove(pos);
            }
            return frame;
        }
        DecodedFrame {
            branch: 0,
            hour,
            tables: std::array::from_fn(|_| not_selected()),
            metrics: Metrics::default(),
        }
    }

    fn put(&mut self, run: u32, hour: i32, frame: DecodedFrame) {
        let key = (run, hour);
        if self.entries.contains_key(&key) {
            if let Some(pos) = self.order.iter().position(|&k| k == key) {
                self.order.remove(pos);
            }
        } else if self.order.len() >= MAX_CACHED_FRAMES
            && let Some(oldest) = self.order.pop_front()
        {
            self.entries.remove(&oldest);
        }
        self.order.push_back(key);
        self.entries.insert(key, frame);
    }
}

enum FrameReader<'a> {
    Borrowed(&'a Replay),
    Owned(Arc<Replay>),
    #[cfg(any(test, feature = "test-support"))]
    Custom(Box<dyn Fn(u32, i32, &[TableId]) -> io::Result<DecodedFrame> + Send + Sync>),
}

impl FrameReader<'_> {
    fn replay(&self) -> Option<&Replay> {
        match self {
            Self::Borrowed(replay) => Some(replay),
            Self::Owned(replay) => Some(replay),
            #[cfg(any(test, feature = "test-support"))]
            Self::Custom(_) => None,
        }
    }

    fn read(&self, run: u32, hour: i32, tables: &[TableId]) -> io::Result<DecodedFrame> {
        match self {
            Self::Borrowed(replay) => read_replay_tables(replay, run, hour, tables),
            Self::Owned(replay) => read_replay_tables(replay, run, hour, tables),
            #[cfg(any(test, feature = "test-support"))]
            Self::Custom(read) => read(run, hour, tables),
        }
    }
}

fn read_replay_tables(
    replay: &Replay,
    run: u32,
    hour: i32,
    tables: &[TableId],
) -> io::Result<DecodedFrame> {
    let reads: Vec<_> = tables
        .iter()
        .map(|&table| replay_lanes::at::Read {
            table,
            columns: None,
            keys: replay_lanes::at::Keys::All,
        })
        .collect();
    let frame = dashboard_query::cost::storage(|| {
        replay_lanes::at::frame(replay.recording(), run, hour, &reads)
    })?;
    at_hour::count_rows(|| {
        tables
            .iter()
            .map(|table| match frame.tables[table.index()].held() {
                TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len() as u64,
                _ => 0,
            })
            .sum()
    });
    Ok(frame)
}

enum CatalogueSource<'a> {
    Borrowed(&'a Catalogue),
    Owned(Arc<Catalogue>),
    Missing(String),
}

impl CatalogueSource<'_> {
    fn get(&self) -> Result<&Catalogue, &str> {
        match self {
            Self::Borrowed(catalogue) => Ok(catalogue),
            Self::Owned(catalogue) => Ok(catalogue),
            Self::Missing(reason) => Err(reason),
        }
    }
}

pub struct ScopedText<'a> {
    reader: FrameReader<'a>,
    catalogue: CatalogueSource<'a>,
    frames: Mutex<BoundedFrames>,
}

impl<'a> ScopedText<'a> {
    pub fn new(replay: &'a Replay, catalogue: &'a Catalogue) -> Self {
        Self {
            reader: FrameReader::Borrowed(replay),
            catalogue: CatalogueSource::Borrowed(catalogue),
            frames: Mutex::new(BoundedFrames::new()),
        }
    }
}

impl ScopedText<'static> {
    pub fn owned(replay: Arc<Replay>, catalogue: Result<Arc<Catalogue>, String>) -> Self {
        let catalogue = match catalogue {
            Ok(catalogue) => CatalogueSource::Owned(catalogue),
            Err(reason) => CatalogueSource::Missing(reason),
        };
        Self {
            reader: FrameReader::Owned(replay),
            catalogue,
            frames: Mutex::new(BoundedFrames::new()),
        }
    }

    pub fn for_source(source: &Source, content: Option<&ContentOptions>) -> Self {
        let catalogue = match content {
            Some(content) => catalogue::recorded_catalogue(source, content)
                .map_err(|error| error.to_string()),
            None => Err("Game content is not configured".to_owned()),
        };
        Self::owned(source.replay(), catalogue)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_reader(
        catalogue: Result<Arc<Catalogue>, String>,
        read_tables: impl Fn(u32, i32, &[TableId]) -> io::Result<DecodedFrame> + Send + Sync + 'static,
    ) -> Self {
        let catalogue = match catalogue {
            Ok(catalogue) => CatalogueSource::Owned(catalogue),
            Err(reason) => CatalogueSource::Missing(reason),
        };
        Self {
            reader: FrameReader::Custom(Box::new(read_tables)),
            catalogue,
            frames: Mutex::new(BoundedFrames::new()),
        }
    }
}

impl ScopedText<'_> {
    pub fn eval_at<T>(
        &self,
        run: u32,
        hour: i32,
        mut run_in_frame: impl FnMut(&DecodedFrame) -> T,
    ) -> io::Result<T> {
        let mut frame = self
            .frames
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take_or_empty(run, hour);
        let mut outcome = None;
        for _ in 0..=SCRIPTED_TABLES.len() {
            let needed = probe_unselected_tables(&frame, || {
                outcome = Some(run_in_frame(&frame));
            });
            if needed.is_empty() {
                break;
            }
            let partial = match self.reader.read(run, hour, &needed) {
                Ok(partial) => partial,
                Err(error) => {
                    self.frames
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .put(run, hour, frame);
                    return Err(error);
                }
            };
            frame.branch = partial.branch;
            for table in needed {
                frame.tables[table.index()] = partial.tables[table.index()].clone();
            }
        }
        self.frames
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .put(run, hour, frame);
        Ok(outcome.expect("probe ran evaluation"))
    }

    pub fn expand_raw(
        &self,
        run: u32,
        hour: i32,
        scope: Scope,
        key: &str,
        text: &str,
    ) -> Result<String, String> {
        match self.expand_raw_checked(run, hour, scope, key, text, None) {
            Ok(result) => result,
            Err(error) => Err(format!("localisation {key}: {error}")),
        }
    }

    pub fn expand_raw_at_branch(
        &self,
        branch: u64,
        hour: i32,
        scope: Scope,
        key: &str,
        text: &str,
    ) -> io::Result<Result<String, String>> {
        let Some(block) = self.reader.replay().and_then(|replay| replay.block_at(branch, hour)) else {
            return Ok(Err(format!("localisation {key}: hour {hour} is not recorded")));
        };
        self.expand_raw_checked(block.run, hour, scope, key, text, None)
    }

    pub fn localise(
        &self,
        run: u32,
        hour: i32,
        scope: Scope,
        key: &str,
        arguments: &[(&str, Argument<'_>)],
        validity: Option<&mut Validity>,
    ) -> Result<String, String> {
        let catalogue = self
            .catalogue
            .get()
            .map_err(|why| format!("localisation {key}: {why}"))?;
        if !catalogue.has_presentation_key(key) {
            let clean = key.split('\0').next().unwrap_or_default();
            return Err(format!("localisation {clean} is missing"));
        }
        let (raw, fallbacks) = catalogue
            .localisation
            .localise_with_fallbacks(key.as_bytes(), arguments);
        for reason in fallbacks {
            catalogue.note(Finding::new(Kind::Content, key, reason));
        }
        let raw = raw.map_err(|why| {
            let message = why.to_string();
            catalogue.refuse_finding(Finding::grouped(
                Kind::Refused,
                key,
                "native localisation text refused",
                message.clone(),
            ));
            message
        })?;
        let Some(open) = raw.find('[') else {
            return plain_text(&raw).map_err(|why| {
                let message = format!("localisation {key} holds {why}");
                catalogue.refuse_finding(Finding::grouped(
                    Kind::Unsupported,
                    key,
                    &format!("localisation holds {why}"),
                    message.clone(),
                ));
                message
            });
        };
        if !raw[open + 1..].contains(']') {
            catalogue.note(Finding::new(
                Kind::Content,
                key,
                format!("localisation {key} holds an unclosed scripted expression; the game omits the text"),
            ));
            return Ok(String::new());
        }
        let expanded = match self.expand_raw_checked(run, hour, scope, key, &raw, validity) {
            Ok(result) => result,
            Err(error) => Err(format!("localisation {key}: {error}")),
        };
        expanded.inspect_err(|message| {
            catalogue.refuse_finding(Finding::grouped(
                Kind::Unsupported,
                key,
                "recorded-country scripted text unavailable",
                message.clone(),
            ));
        })
    }

    fn expand_raw_checked(
        &self,
        run: u32,
        hour: i32,
        scope: Scope,
        key: &str,
        text: &str,
        mut validity: Option<&mut Validity>,
    ) -> io::Result<Result<String, String>> {
        let Some(open) = text.find('[') else {
            return Ok(plain_text(text)
                .map_err(|reason| format!("localisation {key} holds {reason}")));
        };
        if !text[open + 1..].contains(']') {
            return Ok(Ok(String::new()));
        }
        let catalogue = match self.catalogue.get() {
            Ok(catalogue) => catalogue,
            Err(why) => return Ok(Err(format!("localisation {key}: {why}"))),
        };
        let content = match catalogue.scripted_content() {
            Ok(content) => content,
            Err(why) => return Ok(Err(format!("localisation {key}: {why}"))),
        };
        let (root, scopes) = scope.root_and_scopes();
        let mut frame = self
            .frames
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take_or_empty(run, hour);
        let mut last_result: Result<String, String> =
            Err("scripted localisation did not resolve".to_owned());
        for _ in 0..=SCRIPTED_TABLES.len() {
            let scripted = dashboard_text::triggers::inputs::unrecorded(|| {
                ScriptedText::new(&frame, &catalogue.localisation, content, root)
            });
            let (needed, result, captured) = if validity.is_some() {
                let ((needed, result), inputs) =
                    dashboard_text::triggers::inputs::capture(&frame, || {
                        let mut result = None;
                        let needed = probe_unselected_tables(&frame, || {
                            result = Some(scripted.expand_in(text, &scopes));
                        });
                        (needed, result.expect("probe ran expansion"))
                    });
                (needed, result, Some(inputs))
            } else {
                let mut result = None;
                let needed = probe_unselected_tables(&frame, || {
                    result = Some(scripted.expand_in(text, &scopes));
                });
                (needed, result.expect("probe ran expansion"), None)
            };
            last_result = result;
            if needed.is_empty() {
                if let (Some(validity), Some(inputs)) = (validity.as_deref_mut(), captured.as_ref()) {
                    validity.record(inputs);
                }
                break;
            }
            let partial = match self.reader.read(run, hour, &needed) {
                Ok(partial) => partial,
                Err(error) => {
                    self.frames
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .put(run, hour, frame);
                    if engine_allocation::is_allocation_error(&error) {
                        return Err(error);
                    }
                    return Ok(Err(format!("localisation {key}: {error}")));
                }
            };
            frame.branch = partial.branch;
            for table in needed {
                frame.tables[table.index()] = partial.tables[table.index()].clone();
            }
        }
        self.frames
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .put(run, hour, frame);
        let expanded = match last_result {
            Ok(expanded) => expanded,
            Err(why) => return Ok(Err(format!("localisation {key} holds {why}"))),
        };
        Ok(plain_text(&expanded)
            .map_err(|reason| format!("localisation {key}: its name holds {reason}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{CAMPAIGN, records, source};
    use replay_data::rows::{
        ActiveIdeaRow, CompletedFocusRow, CountryAppearanceRow, CountryLinkRow, CountryRow,
        PoliticsRow, Recorded, ScriptVariableEntry, ScriptVariablesRow, StateRow, TableRows,
    };

    fn with_script_game<T>(work: impl FnOnce() -> T) -> T {
        let tables = dashboard_text::catalogue::tests::script_game::executable().build();
        engine_executable::executable_tables::scope(Some(tables), work)
    }

    fn synthetic_replay(hours: std::ops::RangeInclusive<i32>) -> Arc<Replay> {
        let frames: Vec<_> = hours
            .map(|hour| {
                let mut frame = source(1, hour, Some(1_000));
                let set = |frame: &mut DecodedFrame, table: TableId, rows: TableRows| {
                    frame.tables[table.index()] = TableView::Complete(rows).into();
                    frame.metrics.table_ns[table.index()] = 1;
                };
                set(
                    &mut frame,
                    TableId::Countries,
                    TableRows::Countries(vec![
                        CountryRow { id: 9001, canonical_slot: 7, tag: b"AAA".to_vec(), ..Default::default() },
                        CountryRow { id: 9002, canonical_slot: 8, tag: b"BBB".to_vec(), ..Default::default() },
                    ]),
                );
                set(
                    &mut frame,
                    TableId::CountryLinks,
                    TableRows::CountryLinks(vec![
                        CountryLinkRow { id: 9001, canonical_slot: 7 },
                        CountryLinkRow { id: 9002, canonical_slot: 8 },
                    ]),
                );
                set(
                    &mut frame,
                    TableId::CountryAppearance,
                    TableRows::CountryAppearance(vec![
                        CountryAppearanceRow { country: 9001, ..Default::default() },
                        CountryAppearanceRow { country: 9002, ..Default::default() },
                    ]),
                );
                set(
                    &mut frame,
                    TableId::Politics,
                    TableRows::Politics(vec![
                        PoliticsRow { country: 9001, ruling_party: None, ..Default::default() },
                        PoliticsRow { country: 9002, ruling_party: None, ..Default::default() },
                    ]),
                );
                set(
                    &mut frame,
                    TableId::States,
                    TableRows::States(vec![StateRow {
                        id: 1,
                        owner: if hour < 110 { 9001 } else { 9002 },
                        controller: if hour < 110 { 9001 } else { 9002 },
                        ..Default::default()
                    }]),
                );
                set(
                    &mut frame,
                    TableId::ActiveIdeas,
                    TableRows::ActiveIdeas(vec![ActiveIdeaRow {
                        country: 9001,
                        ordinal: 0,
                        token: b"synth_idea".to_vec(),
                    }]),
                );
                let mut focuses = Vec::new();
                if hour >= 103 {
                    focuses.push(CompletedFocusRow {
                        country: 9002,
                        ordinal: 0,
                        token: b"other_focus".to_vec(),
                    });
                }
                if hour >= 106 {
                    focuses.push(CompletedFocusRow {
                        country: 9999,
                        ordinal: 0,
                        token: b"unlinked_focus".to_vec(),
                    });
                }
                set(
                    &mut frame,
                    TableId::CompletedFocuses,
                    TableRows::CompletedFocuses(focuses),
                );
                set(
                    &mut frame,
                    TableId::ScriptVariables,
                    TableRows::ScriptVariables(vec![
                        ScriptVariablesRow {
                            scope_kind: 1,
                            scope_id: 9001,
                            seed: Recorded::Value(None),
                            entries: vec![ScriptVariableEntry {
                                name: b"synth_tier".to_vec(),
                                value_raw: if hour < 110 { 100_000 } else { 200_000 },
                            }],
                        },
                        ScriptVariablesRow {
                            scope_kind: 1,
                            scope_id: 9002,
                            seed: Recorded::Value(None),
                            entries: vec![ScriptVariableEntry {
                                name: b"synth_tier".to_vec(),
                                value_raw: i64::from(hour) * 1_000,
                            }],
                        },
                    ]),
                );
                frame
            })
            .collect();
        Arc::new(
            Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
                .unwrap()
                .into_timelines(&Default::default())
                .unwrap(),
        )
    }

    fn synthetic_catalogue() -> (Arc<Catalogue>, crate::catalogue::tests::Folder) {
        let (cat, dir) = crate::catalogue::tests::folder_catalogue(&[
            ("common/defines/synth.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=7}}"),
            (
                "common/scripted_localisation/synth.txt",
                concat!(
                    "defined_text = {\n",
                    "  name = synth_tier_loc\n",
                    "  text = { trigger = { check_variable = { synth_tier > 1 } } localization_key = SYNTH_TIER_HIGH }\n",
                    "  text = { localization_key = SYNTH_TIER_LOW }\n",
                    "}\n",
                    "defined_text = {\n",
                    "  name = synth_random_loc\n",
                    "  text = {\n",
                    "    random_list = {\n",
                    "      50 = { localization_key = SYNTH_TIER_LOW }\n",
                    "      50 = { localization_key = SYNTH_TIER_HIGH }\n",
                    "    }\n",
                    "  }\n",
                    "}\n",
                ),
            ),
            (
                "localisation/english/synth_l_english.yml",
                concat!(
                    "\u{feff}l_english:\n",
                    " AAA:0 \"Alpha\"\n",
                    " AAA_ADJ:0 \"Alphan\"\n",
                    " BBB:0 \"Beta\"\n",
                    " BBB_ADJ:0 \"Betan\"\n",
                    " PLAIN_KEY:0 \"Plain Label\"\n",
                    " ROOT_KEY:0 \"[ROOT.GetName] Pact\"\n",
                    " OWNER_KEY:0 \"[1.owner.GetAdjective] Coin\"\n",
                    " TIER_KEY:0 \"[synth_tier_loc]\"\n",
                    " SYNTH_TIER_LOW:0 \"Tier Low\"\n",
                    " SYNTH_TIER_HIGH:0 \"Tier High\"\n",
                    " COND_KEY:0 \"[(yes)A|B]\"\n",
                    " DAYS_KEY:0 \"[?synth_flag:days_left]\"\n",
                    " TEMP_KEY:0 \"[?temp_synth]\"\n",
                    " RAND_KEY:0 \"[synth_random_loc]\"\n",
                    " FROM_KEY:0 \"[FROM.GetName]\"\n",
                    " PREV_KEY:0 \"[PREV.GetName]\"\n",
                    " THIS_KEY:0 \"[THIS.GetName]\"\n",
                ),
            ),
        ]);
        (Arc::new(cat), dir)
    }

    #[test]
    fn plain_key_reads_zero_tables_zero_rows_and_never_loads_scripted_content() {
        let (cat, _dir) = synthetic_catalogue();
        let reads = Arc::new(Mutex::new(Vec::<Vec<TableId>>::new()));
        let recorded_reads = Arc::clone(&reads);
        let scoped = ScopedText::with_reader(Ok(Arc::clone(&cat)), move |_, _, tables| {
            recorded_reads.lock().unwrap().push(tables.to_vec());
            panic!("plain localisation must not read any replay tables: {tables:?}");
        });
        let loads_before = cat.content_loads();
        let (result, rows) = at_hour::counted(|| {
            scoped.localise(0, 100, Scope::Country(7), "PLAIN_KEY", &[], None)
        });
        assert_eq!(result.as_deref(), Ok("Plain Label"));
        assert_eq!(rows, 0);
        assert!(reads.lock().unwrap().is_empty());
        assert_eq!(cat.content_loads(), loads_before);
    }

    #[test]
    fn scripted_key_reads_only_referenced_tables_and_bounds_frame_cache_to_four() {
        with_script_game(|| {
            let replay = synthetic_replay(100..=115);
            let (cat, _dir) = synthetic_catalogue();
            let reads = Arc::new(Mutex::new(Vec::<(i32, Vec<TableId>)>::new()));
            let log = Arc::clone(&reads);
            let replay_for_reader = Arc::clone(&replay);
            let scoped = ScopedText::with_reader(Ok(Arc::clone(&cat)), move |run, hour, tables| {
                log.lock().unwrap().push((hour, tables.to_vec()));
                read_replay_tables(&replay_for_reader, run, hour, tables)
            });

            let name = scoped.localise(0, 100, Scope::Country(7), "OWNER_KEY", &[], None).unwrap();
            assert_eq!(name, "Alphan Coin");
            let opened: Vec<TableId> = reads.lock().unwrap().iter().flat_map(|(_, t)| t.iter().copied()).collect();
            assert!(opened.contains(&TableId::States), "{opened:?}");
            assert!(!opened.contains(&TableId::ActiveIdeas), "unrelated tables must stay unread: {opened:?}");
            assert!(!opened.contains(&TableId::ScriptVariables), "unrelated tables must stay unread: {opened:?}");

            let reads_after_first = reads.lock().unwrap().len();
            let again = scoped.localise(0, 100, Scope::Country(7), "OWNER_KEY", &[], None).unwrap();
            assert_eq!(again, "Alphan Coin");
            assert_eq!(reads.lock().unwrap().len(), reads_after_first, "same (run, hour) hits request frame cache");

            for hour in 101..=104 {
                assert_eq!(
                    scoped.localise(0, hour, Scope::Country(7), "OWNER_KEY", &[], None).unwrap(),
                    "Alphan Coin"
                );
            }
            assert_eq!(scoped.frames.lock().unwrap().entries.len(), MAX_CACHED_FRAMES);

            let reads_before_revisit = reads.lock().unwrap().len();
            assert_eq!(
                scoped.localise(0, 100, Scope::Country(7), "OWNER_KEY", &[], None).unwrap(),
                "Alphan Coin"
            );
            assert!(
                reads.lock().unwrap().len() > reads_before_revisit,
                "hour 100 was evicted once 4 newer frames filled the cache"
            );
        });
    }

    #[test]
    fn named_refusals_preserve_key_and_reason_for_every_unsupported_form() {
        with_script_game(|| {
            let replay = synthetic_replay(100..=100);
            let (cat, _dir) = synthetic_catalogue();
            let scoped = ScopedText::new(&replay, &cat);

            for (key, scope) in [
                ("COND_KEY", Scope::Country(7)),
                ("DAYS_KEY", Scope::Country(7)),
                ("TEMP_KEY", Scope::Country(7)),
                ("RAND_KEY", Scope::Country(7)),
                ("FROM_KEY", Scope::Country(7)),
                ("PREV_KEY", Scope::Country(7)),
                ("ROOT_KEY", Scope::Empty),
                ("THIS_KEY", Scope::Empty),
            ] {
                let reason = scoped
                    .localise(0, 100, scope, key, &[], None)
                    .expect_err(&format!("{key} under {scope:?} must be refused"));
                assert!(reason.contains(key), "{key}: refusal must name the key: {reason}");
                assert!(reason.len() > key.len() + 15, "{key}: refusal must carry a reason: {reason}");
                if key == "RAND_KEY" {
                    assert!(reason.contains("random seed"), "{reason}");
                }
            }

            let older_scoped = ScopedText::with_reader(Ok(Arc::clone(&cat)), |_, hour, tables| {
                let mut frame = DecodedFrame {
                    branch: 1,
                    hour,
                    tables: std::array::from_fn(|_| not_selected()),
                    metrics: Metrics::default(),
                };
                for &table in tables {
                    frame.tables[table.index()] = TableView::NotRecorded {
                        mode: replay_data::scope::CaptureMode::Full,
                    }
                    .into();
                }
                Ok(frame)
            });
            let older_reason = older_scoped
                .localise(0, 100, Scope::Country(7), "TIER_KEY", &[], None)
                .expect_err("missing table in older format must be refused");
            assert!(older_reason.contains("TIER_KEY"), "{older_reason}");
        });
    }

    #[test]
    fn validity_follows_captured_inputs_across_mid_recording_change() {
        with_script_game(|| {
            let replay = synthetic_replay(100..=120);
            let (cat, _dir) = synthetic_catalogue();
            let scoped = ScopedText::new(&replay, &cat);
            let mut dates = ReadDates::default();
            let cancelled = AtomicBool::new(false);

            let mut at_100 = Validity::default();
            let low = scoped
                .localise(0, 100, Scope::Country(7), "TIER_KEY", &[], Some(&mut at_100))
                .unwrap();
            assert_eq!(low, "Tier Low");
            let through_100 = at_100
                .through(&mut dates, &replay, 0, 100, 100, 120, &cancelled)
                .unwrap();
            assert_eq!(through_100, 109, "valid through 109 before the variable changes at 110");

            let mut at_110 = Validity::default();
            let high = scoped
                .localise(0, 110, Scope::Country(7), "TIER_KEY", &[], Some(&mut at_110))
                .unwrap();
            assert_eq!(high, "Tier High");
            let through_110 = at_110
                .through(&mut dates, &replay, 0, 100, 110, 120, &cancelled)
                .unwrap();
            assert_eq!(through_110, 120, "stays valid through the end of the run once settled");

            let mut owner_validity = Validity::default();
            assert_eq!(
                scoped
                    .localise(0, 100, Scope::Country(7), "OWNER_KEY", &[], Some(&mut owner_validity))
                    .unwrap(),
                "Alphan Coin"
            );
            assert_eq!(
                owner_validity
                    .through(&mut dates, &replay, 0, 100, 100, 120, &cancelled)
                    .unwrap(),
                109,
                "state owner change at 110 bounds validity to 109"
            );
            assert_eq!(
                scoped.localise(0, 110, Scope::Country(7), "OWNER_KEY", &[], None).unwrap(),
                "Betan Coin"
            );

            let focus_read = TableRead::fields(TableId::CompletedFocuses, &["token"]).owned_by(7, 32);
            let focus_through = dates
                .through(&replay, 0, &[focus_read], 100, 100, 120, &cancelled)
                .unwrap();
            assert_eq!(
                focus_through, 105,
                "linked other-country focus at 103 is ignored, while unlinked focus row at 106 bounds validity to 105"
            );
        });
    }
}
