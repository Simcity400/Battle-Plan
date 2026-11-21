use crate::{Dated, Hour, Interval, Lookup, Missing, MissingKind, Reason};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Absence {
    Unknown { name: String, detail: String },
    NotRecorded,
    Refused { name: String, detail: String },
    Lost { name: String, detail: String },
}

impl Absence {
    pub fn unknown(name: &str, detail: &str) -> Self {
        Self::Unknown {
            name: name.into(),
            detail: detail.into(),
        }
    }

    pub fn kind(&self) -> MissingKind<'_> {
        fn reason<'a>(name: &'a str, detail: &'a str) -> Reason<'a> {
            Reason { name, detail }
        }
        match self {
            Self::Unknown { name, detail } => MissingKind::Unknown(reason(name, detail)),
            Self::NotRecorded => MissingKind::NotRecorded,
            Self::Refused { name, detail } => MissingKind::Refused(reason(name, detail)),
            Self::Lost { name, detail } => MissingKind::Lost(reason(name, detail)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct History<K, V> {
    versions: BTreeMap<K, Vec<(Hour, Result<V, Absence>)>>,
    changes: Vec<(Hour, K)>,
}

impl<K, V> Default for History<K, V> {
    fn default() -> Self {
        Self {
            versions: BTreeMap::new(),
            changes: Vec::new(),
        }
    }
}

impl<K: Ord + Clone, V: PartialEq> History<K, V> {
    pub fn set(&mut self, hour: Hour, key: K, value: Result<V, Absence>) -> io::Result<bool> {
        let versions = self.versions.entry(key.clone()).or_default();
        match versions.last_mut() {
            Some((from, _)) if *from > hour => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "history set before its last version",
                ));
            }
            Some((_, held)) if *held == value => return Ok(false),
            Some((from, held)) if *from == hour => *held = value,
            _ => versions.push((hour, value)),
        }
        if self.changes.last() != Some(&(hour, key.clone())) {
            self.changes.push((hour, key));
        }
        Ok(true)
    }

    pub fn at(&self, key: &K, hour: Hour, watermark: Hour) -> Lookup<'_, Dated<&V>> {
        let versions = self.versions.get(key).map_or(&[][..], Vec::as_slice);
        let index = versions.partition_point(|(from, _)| *from <= hour);
        let Some((from, value)) = index.checked_sub(1).map(|at| &versions[at]) else {
            let last = versions.first().map_or(hour, |(first, _)| first - 1);
            return Lookup::Missing(Missing {
                hours: Interval::new(hour, last).unwrap_or(Interval::hour(hour)),
                kind: MissingKind::NotRecorded,
            });
        };
        match value {
            Ok(value) => Lookup::Known(Dated {
                since: *from,
                value,
            }),
            Err(absence) => {
                let last = versions.get(index).map_or(watermark, |(next, _)| next - 1);
                Lookup::Missing(Missing {
                    hours: Interval::new(*from, last).unwrap_or(Interval::hour(*from)),
                    kind: absence.kind(),
                })
            }
        }
    }

    pub fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Hour, &K) -> io::Result<()>,
    ) -> io::Result<()> {
        let start = self
            .changes
            .partition_point(|(hour, _)| *hour < hours.first());
        for (hour, key) in &self.changes[start..] {
            if *hour > hours.last() {
                break;
            }
            visit(*hour, key)?;
        }
        Ok(())
    }

    pub fn first(&self, key: &K) -> Option<Hour> {
        self.versions.get(key).and_then(|versions| versions.first()).map(|(hour, _)| *hour)
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.versions.keys()
    }
}
