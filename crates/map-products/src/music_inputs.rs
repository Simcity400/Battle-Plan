use dashboard_text::triggers::{Evaluator, Scope, Scopes, World, inputs};
use content_reader_parser::parser::Item;
use replay_data::rows::TableId;
use replay_frame::frame::{DecodedFrame, ViewIdentity};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Outcome {
    value: Option<i64>,
    refusal: Option<String>,
}

impl Outcome {
    fn evaluate(
        name: &str,
        chance: &[Item],
        evaluate: &mut impl FnMut(&[Item]) -> Result<i64, String>,
    ) -> Self {
        match evaluate(chance) {
            Ok(value) => Self {
                value: Some(value),
                refusal: None,
            },
            Err(why) => Self {
                value: None,
                refusal: Some(format!("song {name}: {why}")),
            },
        }
    }

    fn append(self, into: &mut viewer_audio::Chances) {
        into.weights.push(
            self.value
                .map(|value| value as f64 / super::MUSIC_CHANCE_ONE),
        );
        into.refusals.extend(self.refusal);
    }
}

struct Evaluation {
    value: Outcome,
    inputs: inputs::Inputs,
}
enum Then {
    Frame(Arc<DecodedFrame>),
    Identities(Box<[ViewIdentity]>),
}

struct Identities<'a> {
    then: &'a [ViewIdentity],
    now: &'a DecodedFrame,
}

impl inputs::TableChanges for Identities<'_> {
    fn table(&self, table: TableId) -> inputs::TableChange<'_> {
        if self.then[table.index()].is(&self.now.tables[table.index()]) { inputs::TableChange::Same } else { inputs::TableChange::Any }
    }
}

struct Entry {
    content: Arc<viewer_audio::Content>,
    then: Then,
    hour: replay_data::Hour,
    branch: replay_data::BranchId,
    country: u32,
    songs: Vec<Option<Arc<Evaluation>>>,
}

#[derive(Clone)]
pub struct Kept(Arc<Entry>);

impl Kept {
    pub fn hour(&self) -> replay_data::Hour {
        self.0.hour
    }
}

#[derive(Default)]
pub struct Memo {
    entries: Mutex<Vec<std::sync::Weak<Entry>>>,
    by_identity: bool,
    #[cfg(test)]
    evaluations: std::sync::atomic::AtomicUsize,
}

impl Memo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn by_identity() -> Self {
        Self { by_identity: true, ..Self::default() }
    }

    pub fn chances(
        &self,
        frame: &DecodedFrame,
        content: &Arc<viewer_audio::Content>,
        country: u32,
        world: &World<'_>,
    ) -> viewer_audio::Chances {
        self.chances_kept(frame, content, country, world).0
    }

    pub fn chances_kept(
        &self,
        frame: &DecodedFrame,
        content: &Arc<viewer_audio::Content>,
        country: u32,
        world: &World<'_>,
    ) -> (viewer_audio::Chances, Kept) {
        let same = |entry: &Entry| {
            entry.branch == frame.branch
                && entry.country == country
                && Arc::ptr_eq(&entry.content, content)
        };
        let previous = self.entries.try_lock().ok().and_then(|entries| {
            entries
                .iter()
                .rev()
                .filter_map(std::sync::Weak::upgrade)
                .find(|entry| same(entry))
        });
        let mut rereads = HashMap::new();
        let now = if self.by_identity {
            Then::Identities(frame.tables.iter().map(|view| view.identity()).collect())
        } else {
            Then::Frame(Arc::new(frame.clone()))
        };
        let held = previous.as_ref().filter(|entry| entry.hour <= frame.hour);
        let by_frame = held.and_then(|entry| match &entry.then {
            Then::Frame(then) => Some(inputs::Between::new(then, frame)),
            Then::Identities(_) => None,
        });
        let by_identity = held.and_then(|entry| match &entry.then {
            Then::Identities(then) => Some(Identities { then, now: frame }),
            Then::Frame(_) => None,
        });
        let changes: Option<&dyn inputs::TableChanges> = match (&by_frame, &by_identity) {
            (Some(between), _) => Some(between as &dyn inputs::TableChanges),
            (None, Some(identities)) => Some(identities as &dyn inputs::TableChanges),
            (None, None) => None,
        };
        let since = held.zip(changes).map(|(entry, changes)| {
            inputs::Since::span(frame.branch, entry.hour, frame.hour, Some(frame), changes)
        });
        let evaluator = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::Country(country));
        let mut output = viewer_audio::Chances {
            weights: Vec::with_capacity(content.music.songs.len()),
            refusals: Vec::new(),
        };
        let mut songs = Vec::with_capacity(content.music.songs.len());
        for (index, song) in content.music.songs.iter().enumerate() {
            if !content.enabled(index) {
                output.weights.push(None);
                songs.push(None);
                continue;
            }
            let mut certificate = previous
                .as_ref()
                .and_then(|entry| entry.songs[index].as_ref())
                .map(|last| last.inputs.clone());
            let confirmed = certificate.as_mut().is_some_and(|inputs| {
                inputs.confirm_since(frame, since.as_ref(), |key| {
                    *rereads.entry(key.clone()).or_insert_with(|| match key {
                        inputs::Key::World(query) => Some(world.reread(query)),
                        inputs::Key::Scopes(query) => Some(world.recorded().reread(query)),
                        inputs::Key::ClockSeed => {
                            Some(inputs::digest(&dashboard_text::scripted_text::clock_seed(frame)))
                        }
                        inputs::Key::Table(_)
                        | inputs::Key::Fields(..)
                        | inputs::Key::Names(_)
                        | inputs::Key::Text(_)
                        | inputs::Key::FocusTag(_) => None,
                    })
                })
            });
            let evaluate = || {
                #[cfg(test)]
                self.evaluations
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let (value, inputs) = inputs::capture(frame, || {
                    Outcome::evaluate(
                        &content.music.tracks[song.track].name,
                        &song.chance,
                        &mut |body| {
                            evaluator
                                .mean_time_to_happen("music chance", body, scopes)
                                .map_err(|why| why.to_string())
                        },
                    )
                });
                Arc::new(Evaluation { value, inputs })
            };
            let value = if confirmed {
                let last = previous.as_ref().unwrap().songs[index].as_ref().unwrap();
                if !inputs::check_reuse() {
                    Arc::new(Evaluation {
                        value: last.value.clone(),
                        inputs: certificate.unwrap(),
                    })
                } else {
                    let value = evaluate();
                    if value.value != last.value {
                        eprintln!(
                            "music input reuse changed song {} at branch {} hour {}",
                            index, frame.branch, frame.hour
                        );
                        #[cfg(test)]
                        assert_eq!(value.value, last.value, "certified music outcome changed");
                    }
                    value
                }
            } else {
                evaluate()
            };
            value.value.clone().append(&mut output);
            songs.push(Some(value));
        }
        drop(since);
        drop(by_frame);
        let entry = Arc::new(Entry {
            content: Arc::clone(content),
            then: now,
            hour: frame.hour,
            branch: frame.branch,
            country,
            songs,
        });
        if let Ok(mut entries) = self.entries.try_lock() {
            entries.retain(|last| last.upgrade().is_some_and(|last| !same(&last)));
            entries.push(Arc::downgrade(&entry));
        }
        (output, Kept(entry))
    }
}
