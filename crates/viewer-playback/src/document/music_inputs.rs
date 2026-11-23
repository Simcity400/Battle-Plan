use dashboard_text::triggers::{Evaluator, Scope, Scopes, World, inputs};
use content_reader_parser::parser::Item;
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{HashMap, VecDeque},
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
struct Entry {
    content: Arc<viewer_audio::Content>,
    branch: replay_data::BranchId,
    country: u32,
    songs: Vec<Option<Arc<Evaluation>>>,
}

pub struct Memo {
    entries: Mutex<VecDeque<Arc<Entry>>>,
    capacity: usize,
    #[cfg(any(test, feature = "test-support"))]
    evaluations: std::sync::atomic::AtomicUsize,
}

impl Memo {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
            #[cfg(any(test, feature = "test-support"))]
            evaluations: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub(super) fn chances(
        &self,
        frame: &DecodedFrame,
        content: &Arc<viewer_audio::Content>,
        country: u32,
        world: &World<'_>,
    ) -> viewer_audio::Chances {
        let previous = self.entries.try_lock().ok().and_then(|entries| {
            entries
                .iter()
                .rev()
                .find(|entry| {
                    entry.branch == frame.branch
                        && entry.country == country
                        && Arc::ptr_eq(&entry.content, content)
                })
                .cloned()
        });
        let mut rereads = HashMap::new();
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
                inputs.confirm(frame, |key| {
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
                #[cfg(any(test, feature = "test-support"))]
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
                        #[cfg(any(test, feature = "test-support"))]
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
        let entry = Arc::new(Entry {
            content: Arc::clone(content),
            branch: frame.branch,
            country,
            songs,
        });
        if let Ok(mut entries) = self.entries.try_lock() {
            let replaced = entries
                .iter()
                .position(|last| {
                    last.branch == frame.branch
                        && last.country == country
                        && Arc::ptr_eq(&last.content, content)
                })
                .and_then(|at| entries.remove(at));
            let evicted = (entries.len() >= self.capacity).then(|| entries.pop_front());
            entries.push_back(entry);
            drop(entries);
            drop((replaced, evicted));
        }
        output
    }
}

#[cfg(test)]
mod tests;
