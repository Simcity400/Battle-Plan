use super::tables::{Available, rows};
use viewer_runtime::errors::invalid;
use content_reader_map::map::MapData;
use replay_data::{Hour, rows::SubjectRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub fn slot_of(links: &BTreeMap<u32, u32>, reference: u32, role: &str) -> io::Result<u32> {
    links.get(&reference).copied().ok_or_else(|| {
        invalid(format!(
            "political fill references missing country link {reference} for {role}"
        ))
    })
}

pub fn active_subject(row: &SubjectRow, hour: Hour) -> bool {
    row.cancel == 0 && (row.end_hour <= row.start_hour || hour < row.end_hour)
}

#[derive(Clone, Debug, Default)]
pub struct Inheritance {
    pub inherited: BTreeMap<u32, u32>,
    pub undecided: BTreeMap<u32, String>,
    pub with_status: BTreeSet<u32>,
    pub subjects: BTreeSet<u32>,
    pub absent: Option<String>,
}

impl Inheritance {
    pub fn source(&self, slot: u32) -> Result<u32, String> {
        let mut source = slot;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(source) {
                return Err("cycle in subject colour inheritance".to_owned());
            }
            if let Some(why) = self.undecided.get(&source) {
                return Err(format!(
                    "country slot {source}'s colour source is unknown: {why}"
                ));
            }
            if let Some(why) = &self.absent
                && !self.with_status.contains(&source)
            {
                return Err(format!(
                    "country slot {source}'s current autonomy status is unknown: {why}"
                ));
            }
            match self.inherited.get(&source) {
                Some(&overlord) => source = overlord,
                None => return Ok(source),
            }
        }
    }
}

impl Inheritance {
    pub fn known_source(&self, slot: u32) -> io::Result<Option<u32>> {
        let mut source = slot;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(source) {
                return Err(invalid("cycle in subject colour inheritance"));
            }
            if self.undecided.contains_key(&source)
                || (self.absent.is_some() && !self.with_status.contains(&source))
            {
                return Ok(None);
            }
            match self.inherited.get(&source) {
                Some(&overlord) => source = overlord,
                None => return Ok(Some(source)),
            }
        }
    }
}

pub fn inheritance(
    map: &MapData,
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<Inheritance> {
    let autonomy = rows::<tables::Autonomy>(frame)?;
    let mut states = BTreeMap::new();
    for row in autonomy.rows() {
        let slot = slot_of(links, row.subject, "autonomy colour")?;
        if states
            .insert(slot, &row.state)
            .is_some_and(|old| old != &row.state)
        {
            return Err(invalid(format!(
                "country slot {slot} has conflicting current autonomy definitions"
            )));
        }
    }
    let subjects = rows::<tables::Subjects>(frame)?;
    let mut overlords: BTreeMap<u32, u32> = BTreeMap::new();
    let mut conflicting = BTreeSet::new();
    for row in subjects.rows() {
        if !active_subject(row, frame.hour) {
            continue;
        }
        let subject = slot_of(links, row.subject, "subject colour")?;
        let overlord = slot_of(links, row.overlord, "subject colour")?;
        if overlords
            .insert(subject, overlord)
            .is_some_and(|old| old != overlord)
        {
            conflicting.insert(subject);
        }
    }
    let mut result = Inheritance {
        with_status: states.keys().copied().collect(),
        subjects: overlords.keys().copied().collect(),
        absent: table_gap(&autonomy, "Autonomy"),
        ..Inheritance::default()
    };
    for (&subject, state) in &states {
        if state.is_empty() {
            continue;
        }
        let Some(&inherit) = map.country_colors.use_overlord_color.get(*state) else {
            result.undecided.insert(
                subject,
                format!(
                    "its current autonomy definition {} is not an autonomy state the content defines, so its use_overlord_color flag is unknown",
                    String::from_utf8_lossy(state)
                ),
            );
            continue;
        };
        if !inherit {
            continue;
        }
        if conflicting.contains(&subject) {
            return Err(invalid(format!(
                "country slot {subject} has conflicting colour overlords"
            )));
        }
        match overlords.get(&subject) {
            Some(&overlord) => {
                result.inherited.insert(subject, overlord);
            }
            None => {
                let why = match table_gap(&subjects, "Subjects") {
                    Some(gap) => format!("its current autonomy inherits, and {gap}"),
                    None => "its current autonomy inherits, but no active Subjects row names its overlord"
                        .to_owned(),
                };
                result.undecided.insert(subject, why);
            }
        }
    }
    Ok(result)
}

fn table_gap<T: replay_data::rows::TableSchema>(
    table: &Available<'_, T>,
    name: &str,
) -> Option<String> {
    match table {
        Available::Complete(_) => None,
        Available::Truncated { missing, .. } => {
            Some(format!("{name} is truncated ({missing} missing rows)"))
        }
        Available::Unavailable { reason } => Some(format!("{name} is unavailable: {reason}")),
        Available::NotRecorded { mode } => {
            Some(format!("{name} is not recorded in a {mode} recording"))
        }
    }
}
