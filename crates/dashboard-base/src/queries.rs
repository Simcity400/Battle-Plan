use crate::Source;
use std::{collections::HashMap, io, sync::{Arc, PoisonError}};

pub mod country_ids;

#[derive(Default)]
struct Held(Arc<dashboard_query::Index>);

pub fn index(source: &Source) -> Arc<dashboard_query::Index> {
    let held = source.state::<Held>();
    let held = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(&held.0)
}

#[derive(Default)]
struct Spellings(std::collections::HashMap<(u32, i32), std::collections::HashMap<u32, Option<String>>>);

pub fn spellings(source: &Source, run: u32, hour: i32, tokens: &std::collections::BTreeSet<u32>) -> io::Result<std::collections::HashMap<u32, String>> {
    let state = source.state::<Spellings>();
    let missing: Vec<u128> = {
        let held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let known = held.0.get(&(run, hour));
        tokens.iter().filter(|token| known.is_none_or(|known| !known.contains_key(token))).map(|&token| u128::from(token)).collect()
    };
    if !missing.is_empty() {
        let read = [replay_lanes::at::Read { table: replay_data::rows::TableId::LexicalTokens, columns: None, keys: replay_lanes::lanes::Keys::These(&missing) }];
        let frame = dashboard_query::cost::storage(|| replay_lanes::at::frame(source.replay_ref().recording(), run, hour, &read))?;
        let mut spelled: std::collections::HashMap<u32, Option<String>> = missing.iter().map(|&token| (token as u32, None)).collect();
        for row in frame.complete::<replay_data::rows::tables::LexicalTokens>().unwrap_or(&[]) {
            if let Ok(text) = std::str::from_utf8(content_reader_parser::parser::c_string(&row.literal))
                && !text.is_empty()
            {
                spelled.insert(row.token, Some(text.to_owned()));
            }
        }
        state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0.entry((run, hour)).or_default().extend(spelled);
    }
    let held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(known) = held.0.get(&(run, hour)) else { return Ok(std::collections::HashMap::new()) };
    Ok(tokens.iter().filter_map(|token| Some((*token, known.get(token)?.clone()?))).collect())
}

#[derive(Default)]
struct Histories {
    links: HashMap<u32, Arc<country_ids::RunLinks>>,
}

pub fn forget(source: &Source) {
    source.replay_ref().forget_row_lifetimes();
    *source.state::<Held>().lock().unwrap_or_else(PoisonError::into_inner) = Held::default();
    *source.state::<Histories>().lock().unwrap_or_else(PoisonError::into_inner) = Histories::default();
    *source.state::<Spellings>().lock().unwrap_or_else(PoisonError::into_inner) = Spellings::default();
    source.state::<dashboard_query::log::BucketCache>().lock().unwrap_or_else(PoisonError::into_inner).clear();
    source.state::<crate::products::log_page::CoverageCache>().lock().unwrap_or_else(PoisonError::into_inner).clear();
}

pub fn log_caches(source: &Source) -> crate::products::log_page::LogCaches {
    crate::products::log_page::LogCaches {
        buckets: source.state::<dashboard_query::log::BucketCache>(),
        coverage: source.state::<crate::products::log_page::CoverageCache>(),
    }
}

pub fn links(source: &Source, run: u32) -> io::Result<Arc<country_ids::RunLinks>> {
    let state = source.state::<Histories>();
    if let Some(found) = state.lock().unwrap_or_else(PoisonError::into_inner).links.get(&run) {
        return Ok(Arc::clone(found));
    }
    let made = Arc::new(country_ids::run_links(source.replay_ref().recording(), run)?);
    state.lock().unwrap_or_else(PoisonError::into_inner).links.insert(run, Arc::clone(&made));
    Ok(made)
}

pub fn lifetimes(source: &Source, run: u32, table: replay_data::rows::TableId) -> io::Result<Arc<Vec<(u128, i64, i64)>>> {
    dashboard_query::cost::recorded_lifetimes(source.replay_ref(), run, table)
}

pub fn query(source: &Source, branch: u64) -> io::Result<dashboard_query::Query<'_>> {
    index(source).query(source.replay_ref(), branch)
}

#[cfg(feature = "test-support")]
pub fn has_cached_spellings(source: &Source) -> bool {
    !source.state::<Spellings>().lock().unwrap_or_else(PoisonError::into_inner).0.is_empty()
}

pub struct RunCache<K, V>(HashMap<u32, Arc<V>>, std::marker::PhantomData<fn() -> K>);

impl<K, V> Default for RunCache<K, V> {
    fn default() -> Self {
        Self(HashMap::new(), std::marker::PhantomData)
    }
}

impl<K: 'static, V: Send + Sync + 'static> RunCache<K, V> {
    pub fn forget(source: &Source) {
        *source.state::<Self>().lock().unwrap_or_else(PoisonError::into_inner) = Self::default();
    }

    pub fn held(state: &std::sync::Mutex<Self>, run: u32, read: impl FnOnce() -> io::Result<V>) -> io::Result<Arc<V>> {
        if let Some(found) = state.lock().unwrap_or_else(PoisonError::into_inner).0.get(&run) {
            return Ok(Arc::clone(found));
        }
        let made = Arc::new(read()?);
        state.lock().unwrap_or_else(PoisonError::into_inner).0.insert(run, Arc::clone(&made));
        Ok(made)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Source, test_fixtures::{CAMPAIGN, records, source}};
    use replay::campaign::Replay;
    use replay_data::rows::{LexicalTokenRow, TableId, TableRows};
    use replay_frame::frame::{DecodedFrame, TableView};
    use std::sync::{Arc, atomic::AtomicBool};

    fn frame(hour: i32) -> DecodedFrame {
        let mut frame = source(1, hour, Some(1_000));
        let row = |token: u32, text: &str| LexicalTokenRow { token, literal: text.as_bytes().to_vec() };
        let mut rows = vec![row(7, "spy_steal"), row(8, if hour < 110 { "spy_steal" } else { "spy_other" }), row(10, "")];
        if hour >= 120 {
            rows.push(row(9, "spy_steal"));
        }
        rows.sort_by_key(|row| row.token);
        frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(rows)).into();
        frame.metrics.table_ns[TableId::LexicalTokens.index()] = 1;
        frame
    }

    fn recording() -> Source {
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (100..=130).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(crate::SourceKey { source_id: "spellings".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn a_tokens_spelling_at_an_hour_is_read_once_for_every_reply_on_a_source() {
        let source = recording();
        let run = super::query(&source, 1).unwrap().timeline.stretches[0].run;
        let spelled = |hour: i32, tokens: &[u32]| {
            let mut found: Vec<(u32, String)> = super::spellings(&source, run, hour, &tokens.iter().copied().collect()).unwrap().into_iter().collect();
            found.sort();
            (found, dashboard_query::cost::take())
        };
        let spy = |token: u32, text: &str| (token, text.to_owned());
        assert_eq!(spelled(105, &[7, 8]).0, [spy(7, "spy_steal"), spy(8, "spy_steal")]);
        assert_eq!(spelled(105, &[8, 7]), (vec![spy(7, "spy_steal"), spy(8, "spy_steal")], 0), "read once, nothing read again");
        assert_eq!(spelled(105, &[7, 9, 10]).0, [spy(7, "spy_steal")], "9 held nothing at 105, 10 holds no text");
        assert_eq!(spelled(105, &[9, 10, 7, 8]), (vec![spy(7, "spy_steal"), spy(8, "spy_steal")], 0), "a token without text is not read again");
        assert_eq!(spelled(125, &[8, 9]).0, [spy(8, "spy_other"), spy(9, "spy_steal")], "another hour, its own texts");
        super::forget(&source);
        assert_eq!(spelled(125, &[8]).0, [spy(8, "spy_other")]);
    }
}
