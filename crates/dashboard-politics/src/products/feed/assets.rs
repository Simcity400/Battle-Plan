use super::{EventFeedKind, FeedEvent, index::{Locator, Scanned}, text, window};
use content_reader_events::events::{Event, LazyEvents};
use content_reader_layers::content::diagnostics::Observed;
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::{ContentOptions, LoadContext, read_ahead::Preparation}};
use dashboard_core::presentation::{Binding, Cancel};
use replay_data::rows::Recorded;
use std::{
    collections::HashMap,
    io,
    sync::{Arc, OnceLock, PoisonError, RwLock, Weak},
};

#[derive(PartialEq, Eq, Hash)]
pub(super) enum Asset {
    Event(String),
    Picture(String, String),
    Decision(String),
}

pub(super) fn needed(kind: EventFeedKind, event: &FeedEvent) -> impl Iterator<Item = Asset> {
    let (definition, art) = if kind == EventFeedKind::EventWindowPresented {
        event.presentation.as_ref().and_then(|presentation| {
            let token = window::token(event, presentation)?;
            let art = match &presentation.picture {
                Recorded::Value(Some(picture)) => Some(Asset::Picture(token.clone(), String::from_utf8_lossy(picture).into_owned())),
                _ => None,
            };
            Some((Some(Asset::Event(token)), art))
        }).unwrap_or_default()
    } else {
        let token = event.token.as_deref().and_then(|bytes| {
            if bytes.is_empty() { event.native_event.map(|(_, id)| id.to_string()) }
            else { std::str::from_utf8(bytes).ok().map(str::to_owned) }
        });
        if text::presents_event(kind) {
            (token.map(Asset::Event), None)
        } else if text::names_decision(kind) {
            (None, Some(Asset::Decision(token.unwrap_or_else(|| "unidentified decision".into()))))
        } else {
            (None, None)
        }
    };
    definition.into_iter().chain(art)
}

pub(super) type Requests = Vec<(Locator, Asset)>;

pub(super) fn ordered(requests: HashMap<Asset, Locator>) -> Requests {
    let mut ordered: Requests = requests.into_iter().map(|(asset, at)| (at, asset)).collect();
    ordered.sort_by(|(left, a), (right, b)| {
        (left.hour, left.run, left.key, matches!(a, Asset::Picture(..)))
            .cmp(&(right.hour, right.run, right.key, matches!(b, Asset::Picture(..))))
    });
    ordered
}

type Definition = Observed<Result<Arc<Event>, String>>;
type Drawing = Observed<catalogue::Drawn<Result<String, String>>>;

#[derive(Default)]
struct Ready {
    events: HashMap<String, Definition>,
    drawings: HashMap<String, HashMap<String, Drawing>>,
}

#[derive(Default)]
pub(super) struct Warming {
    hour: Option<i32>,
    done: Arc<Preparation>,
    content: OnceLock<Weak<Catalogue>>,
    ready: RwLock<Ready>,
}

impl Warming {
    pub(super) fn matches(&self, content: &Arc<Catalogue>) -> bool {
        self.content.get().is_some_and(|made| made.ptr_eq(&Arc::downgrade(content)))
    }

    pub(super) fn event(&self, token: &str) -> Option<Result<Arc<Event>, String>> {
        Some(self.ready.read().unwrap_or_else(PoisonError::into_inner).events.get(token)?.get().clone())
    }

    pub(super) fn art(&self, key: (&str, &str)) -> Option<Result<String, String>> {
        Some(self.ready.read().unwrap_or_else(PoisonError::into_inner).drawings.get(key.0)?.get(key.1)?.get().get().clone())
    }

    fn start(self: &Arc<Self>, catalogue: &Arc<Catalogue>, scanned: &Arc<Scanned>, binding: &Arc<Binding>) {
        if self.content.set(Arc::downgrade(catalogue)).is_err() { return; }
        let (job, catalogue, scanned, binding) = (Arc::clone(self), Arc::clone(catalogue), Arc::clone(scanned), Arc::clone(binding));
        self.done.start("Event feed content read-ahead", move || job.build(&catalogue, &scanned.asset_requests, &binding));
    }

    #[cfg(test)]
    pub(super) fn wait(&self) -> io::Result<()> {
        assert!(self.content.get().is_some(), "preparation was reserved but not started");
        self.done.wait()
    }

    fn build(self: &Arc<Self>, catalogue: &Catalogue, requests: &Requests, binding: &Binding) -> io::Result<()> {
        let cancelled = binding.flag();
        let reader = LazyEvents::from_content(Arc::clone(&catalogue.content));
        let start = self.hour.map_or(0, |hour| requests.partition_point(|(at, _)| at.hour < hour));
        for batch in [&requests[start..], &requests[..start]].into_iter().flat_map(|span| span.chunks(64)) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if Arc::strong_count(self) == 1 { return Ok(()); }
            reader.prefetch(batch.iter().filter_map(|(_, asset)| match asset {
                Asset::Event(token) => Some(token.as_bytes()),
                _ => None,
            }), cancelled);
            for (_, asset) in batch {
                dashboard_text::metadata::check_cancel(cancelled)?;
                match asset {
                    Asset::Event(token) => {
                        let event = Observed::new(|| reader.get(token.as_bytes()));
                        let Ok(found) = event.peek() else { continue; };
                        if found.refused.is_some() { continue; }
                        if !found.picture.is_empty() && let Ok(picture) = std::str::from_utf8(&found.picture) {
                            self.draw(catalogue, (token, picture), false);
                        }
                        self.ready.write().unwrap_or_else(PoisonError::into_inner).events.insert(token.clone(), event);
                    }
                    Asset::Picture(token, picture) => self.draw(catalogue, (token, picture), false),
                    Asset::Decision(token) => self.draw(catalogue, ("decision", token), true),
                }
            }
        }
        Ok(())
    }

    fn draw(&self, catalogue: &Catalogue, key: (&str, &str), decision: bool) {
        if self.ready.read().unwrap_or_else(PoisonError::into_inner).drawings.get(key.0).is_some_and(|drawings| drawings.contains_key(key.1)) {
            return;
        }
        let drawn = Observed::new(|| if decision {
            text::draw_decision(catalogue, key.1)
        } else {
            text::draw_event(catalogue, key.0, key.1)
        });
        self.ready.write().unwrap_or_else(PoisonError::into_inner).drawings
            .entry(key.0.to_owned()).or_default().insert(key.1.to_owned(), drawn);
    }
}

#[derive(Default)]
struct Jobs(HashMap<(u64, u64, bool), Arc<Warming>>);

fn key(branch: u64, options: &ContentOptions) -> (u64, u64, bool) {
    (branch, options.presentation.binding().generation, catalogue::reads_recorded(options))
}

#[cfg(test)]
pub(super) fn held(source: &Source, branch: u64, load: &LoadContext) -> Option<Arc<Warming>> {
    source.state::<Jobs>().lock().unwrap_or_else(PoisonError::into_inner).0.get(&key(branch, load.content.as_ref()?)).cloned()
}

pub(super) fn reserve(source: &Source, branch: u64, hour: Option<i32>, load: &LoadContext) {
    let Some(options) = load.content.as_ref() else { return; };
    let key = key(branch, options);
    let state = source.state::<Jobs>();
    let mut jobs = state.lock().unwrap_or_else(PoisonError::into_inner);
    jobs.0.retain(|&(_, binding, _), job| binding >= key.1 && !matches!(job.done.result(), Some(Err(_))));
    jobs.0.entry(key).or_insert_with(|| Arc::new(Warming { hour, ..Warming::default() }));
}


pub(super) fn launch(source: &Source, branch: u64, load: &LoadContext, catalogue: &Arc<Catalogue>, scanned: &Arc<Scanned>) -> Option<Arc<Warming>> {
    let options = load.content.as_ref()?;
    let state = source.state::<Jobs>();
    let mut jobs = state.lock().unwrap_or_else(PoisonError::into_inner);
    let job = jobs.0.get_mut(&key(branch, options))?;
    if job.content.get().is_some() && !job.matches(catalogue) {
        *job = Arc::new(Warming { hour: job.hour, ..Warming::default() });
    }
    job.start(catalogue, scanned, options.presentation.binding());
    Some(Arc::clone(job))
}
