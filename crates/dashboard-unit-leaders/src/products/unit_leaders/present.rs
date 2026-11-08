//! Leaders' display text, derived from the kept facts when a reply is built.

use super::*;

/// The content a reply's text derives from, or why it cannot be read.
pub struct Shown(Result<Read, Arc<str>>);

struct Read {
    catalogue: Arc<Catalogue>,
    politics: Arc<politics_content::PoliticsContent>,
}

impl Content {
    pub(crate) fn shown(&self) -> Shown {
        Shown(Ok(Read {
            catalogue: Arc::clone(&self.catalogue),
            politics: Arc::clone(&self.politics),
        }))
    }
}

/// The experience kept runs' progress requires: the next skill level's, and each trait's.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(crate) struct Requirements {
    /// Whether preparation read the recorded content; where it did not, no text derives.
    read: bool,
    #[serde(with = "dashboard_core::map_pairs")]
    next: HashMap<(i32, i32), Option<i64>>,
    #[serde(with = "dashboard_core::map_pairs")]
    traits: HashMap<Arc<str>, Option<i64>>,
    /// The people and roles whose text's refusals are noted, from the first.
    named: usize,
    ranked: usize,
}

impl Requirements {
    /// Notes what new `runs` and `persons` need, and the refusals their text meets.
    pub(crate) fn note<'r>(
        &mut self,
        runs: impl Iterator<Item = &'r Run>,
        roles: &[Role],
        persons: &[Person],
        content: Option<&Content>,
    ) {
        self.read |= content.is_some();
        // Text a reply derives is refused into the preparation's warnings, as when it was kept.
        if let Some(content) = content {
            for person in persons.get(self.named..).unwrap_or_default() {
                if let Some(saved) = &person.saved {
                    content.politics.character_name(person.token.as_deref(), saved);
                }
            }
            for role in roles.get(self.ranked..).unwrap_or_default() {
                if let Some(key) = rank_key(role.category) {
                    content.catalogue.localised(key, View::Plain);
                }
                attribute_names(role.kind, Some(content.catalogue.as_ref()));
            }
        }
        self.named = self.named.max(persons.len());
        self.ranked = self.ranked.max(roles.len());
        for run in runs {
            let value = &run.value;
            let Some(progress) = &value.progress else {
                continue;
            };
            if let Some(level) = value.skill {
                let category = roles[value.role as usize].category;
                self.next.entry((level, category)).or_insert_with(|| {
                    content
                        .and_then(|c| c.leaders.skills.next_required_experience(level, category))
                        .flatten()
                        .map(|v| i64::from(v) * 100_000)
                });
            }
            for token in progress
                .training
                .iter()
                .flat_map(|rows| rows.iter())
                .filter_map(|row| row.token.as_ref())
            {
                if !self.traits.contains_key(token) {
                    if let Some(content) = content {
                        content.catalogue.localised(token, View::Plain);
                    }
                    let required = content
                        .and_then(|c| c.leaders.traits.get(token))
                        .map(|t| i64::from(t.required_experience()) * 100_000);
                    self.traits.insert(Arc::clone(token), required);
                }
            }
        }
    }
}

/// The content and requirements a reply's text derives from, with what it derived so far.
pub(crate) struct Present<'c> {
    shown: Option<&'c Read>,
    requirements: &'c Requirements,
    ranks: HashMap<i32, Option<String>>,
    attributes: HashMap<u8, [Option<String>; 4]>,
    names: HashMap<Arc<str>, Option<String>>,
}

impl<'c> Present<'c> {
    /// Refuses where preparation read the recorded content and the reply cannot.
    pub(crate) fn new(
        shown: Option<&'c Shown>,
        requirements: &'c Requirements,
    ) -> io::Result<Self> {
        let shown = match (requirements.read, shown) {
            (false, _) => None,
            (true, Some(Shown(Ok(read)))) => Some(read),
            (true, Some(Shown(Err(why)))) => {
                return Err(io::Error::other(format!(
                    "Leaders' names, ranks and progress need the recorded content: {why}"
                )));
            }
            (true, None) => {
                return Err(io::Error::other(
                    "Leaders' names, ranks and progress need the recorded content, and none was given",
                ));
            }
        };
        Ok(Self {
            shown,
            requirements,
            ranks: HashMap::new(),
            attributes: HashMap::new(),
            names: HashMap::new(),
        })
    }

    pub(crate) fn person(&self, person: &Person) -> LeaderPerson {
        let name = person.saved.as_deref().and_then(|saved| {
            self.shown?
                .politics
                .character_name(person.token.as_deref(), saved)
        });
        LeaderPerson {
            name,
            portrait_hour: person.portrait_hour,
            portrait_reference: person.reference.to_string(),
        }
    }

    pub(crate) fn role(&mut self, role: &Role) -> LeaderRole {
        let catalogue = self.shown.map(|shown| shown.catalogue.as_ref());
        let rank = self
            .ranks
            .entry(role.category)
            .or_insert_with(|| {
                catalogue.and_then(|c| {
                    rank_key(role.category).and_then(|key| c.localised(key, View::Plain))
                })
            })
            .clone();
        let attribute_names = self
            .attributes
            .entry(role.kind)
            .or_insert_with(|| attribute_names(role.kind, catalogue))
            .clone();
        LeaderRole {
            owner: role.owner.clone(),
            category: role.category,
            rank,
            kind: role.kind,
            maximum_skill: role.maximum_skill,
            attribute_names,
        }
    }

    /// `progress` of a run whose skill is `skill` in a role of `category`.
    pub(crate) fn progress(
        &mut self,
        progress: &Progress,
        skill: Option<i32>,
        category: i32,
    ) -> LeaderProgress {
        let catalogue = self.shown.map(|shown| shown.catalogue.as_ref());
        let raw = |value: Option<&Option<i64>>| value.copied().flatten().map(|v| v.to_string());
        let next_required_raw = skill
            .and_then(|level| raw(self.requirements.next.get(&(level, category))));
        let (training, factors) = match &progress.training {
            None => (None, None),
            Some(rows) => {
                let mut training = Vec::new();
                let mut factors = Vec::new();
                for row in rows.iter() {
                    let (name, required_raw) = match &row.token {
                        Some(token) => (
                            self.names
                                .entry(Arc::clone(token))
                                .or_insert_with(|| {
                                    catalogue.and_then(|c| c.localised(token, View::Plain))
                                })
                                .clone(),
                            raw(self.requirements.traits.get(token)),
                        ),
                        None => (None, None),
                    };
                    let token = row.token.as_deref().map(str::to_owned);
                    if row.factor {
                        factors.push(LeaderTrainingFactor {
                            token,
                            name,
                            factor_raw: row.experience.to_string(),
                        });
                    } else {
                        training.push(LeaderTraining {
                            token,
                            name,
                            experience_raw: row.experience.to_string(),
                            required_raw,
                        });
                    }
                }
                (Some(training), Some(factors))
            }
        };
        LeaderProgress {
            experience_raw: progress.experience.to_string(),
            next_required_raw,
            training,
            factors,
        }
    }
}

/// The recorded catalogue and politics content, once per recorded catalogue of a source.
#[derive(Default)]
struct Served {
    held: Option<(Arc<Catalogue>, Arc<Shown>)>,
}

/// The recorded content, or why it cannot be read; None where no content is configured.
pub fn served_content(
    source: &Source,
    options: Option<&dashboard_core::config::ContentOptions>,
) -> io::Result<Option<Arc<Shown>>> {
    let Some(options) = options else {
        return Ok(None);
    };
    let catalogue = match catalogue::recorded_catalogue(source, options) {
        Ok(catalogue) => catalogue,
        Err(error) => return Ok(Some(Arc::new(Shown(Err(Arc::from(error.to_string())))))),
    };
    let state = source.state::<Served>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((held, shown)) = &state.held
        && Arc::ptr_eq(held, &catalogue)
    {
        return Ok(Some(Arc::clone(shown)));
    }
    let shown = Arc::new(Shown(
        match politics_content::recorded_politics_content(source, options) {
            Ok(politics) => Ok(Read {
                catalogue: Arc::clone(&catalogue),
                politics,
            }),
            Err(error) => Err(Arc::from(error.to_string())),
        },
    ));
    state.held = Some((catalogue, Arc::clone(&shown)));
    Ok(Some(shown))
}
