use crate::Source;
use std::{collections::HashMap, io, sync::{Arc, PoisonError}};

pub mod country_ids;
pub mod lexicon;

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
struct Lexicons(std::collections::HashMap<u32, Arc<std::sync::Mutex<Option<Arc<lexicon::RunLexicon>>>>>);

pub fn lexicon(source: &Source, run: u32) -> io::Result<Arc<lexicon::RunLexicon>> {
    let slot = Arc::clone(source.state::<Lexicons>().lock().unwrap_or_else(std::sync::PoisonError::into_inner).0.entry(run).or_default());
    let mut held = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(found) = held.as_ref() {
        return Ok(Arc::clone(found));
    }
    let made = Arc::new(lexicon::RunLexicon::read(source.replay_ref().recording(), run)?);
    *held = Some(Arc::clone(&made));
    Ok(made)
}

pub fn warm_lexicons(source: &Source) -> io::Result<()> {
    let Some(timelines) = source.replay_ref().timelines() else { return Ok(()) };
    let mut branches: Vec<u64> = timelines.list.iter().map(|timeline| timeline.id).collect();
    branches.sort_by_key(|branch| *branch != timelines.main);
    let mut runs: Vec<u32> = Vec::new();
    for branch in branches {
        for stretch in &query(source, branch)?.timeline.stretches {
            if !runs.contains(&stretch.run) {
                runs.push(stretch.run);
            }
        }
    }
    runs.into_iter().try_for_each(|run| lexicon(source, run).map(drop))
}

#[derive(Default)]
struct Histories {
    lifetimes: HashMap<(u32, usize), Arc<Vec<(u128, i64, i64)>>>,
    links: HashMap<u32, Arc<country_ids::RunLinks>>,
}

pub fn forget(source: &Source) {
    *source.state::<Held>().lock().unwrap_or_else(PoisonError::into_inner) = Held::default();
    *source.state::<Histories>().lock().unwrap_or_else(PoisonError::into_inner) = Histories::default();
    *source.state::<Lexicons>().lock().unwrap_or_else(PoisonError::into_inner) = Lexicons::default();
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
    let state = source.state::<Histories>();
    if let Some(found) = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).lifetimes.get(&(run, table.index())) {
        return Ok(Arc::clone(found));
    }
    let made = Arc::new(dashboard_query::cost::lifetimes(source.replay_ref().recording(), run, table)?);
    state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).lifetimes.insert((run, table.index()), Arc::clone(&made));
    Ok(made)
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
