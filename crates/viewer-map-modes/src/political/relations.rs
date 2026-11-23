use super::*;
use viewer_scene_base::projection_fold::{RowFold, fields};
use map_display_data::{retained::Retained, value_index::ValueIndex};
use replay_data::rows::TableId;

type Subject = (u32, u32, i32, i32, u8);
#[derive(Clone)]
pub(crate) struct Relations {
    hour: i32,
    subjects: RowFold<Subject>,
    autonomy: RowFold<Retained<Vec<u8>>>,
    subject_by_link: ValueIndex<(u32, u128), ()>,
    autonomy_by_link: ValueIndex<(u32, u128), ()>,
    ends: ValueIndex<(i32, u128), ()>,
    subject_slots: ValueIndex<u128, (u32, u32)>,
    autonomy_slots: ValueIndex<u128, u32>,
    by_subject: ValueIndex<(u32, u128), u32>,
    by_status: ValueIndex<(u32, u128), Retained<Vec<u8>>>,
    subject_errors: ValueIndex<u128, Retained<String>>,
    autonomy_errors: ValueIndex<u128, Retained<String>>,
    status_conflicts: ValueIndex<u32, u128>,
    status_errors: ValueIndex<u128, u32>,
    overlords: ValueIndex<u32, u32>,
    conflicting: ValueIndex<u32, ()>,
    states: ValueIndex<u32, Retained<Vec<u8>>>,
    inherited: ValueIndex<u32, u32>,
    by_overlord: ValueIndex<(u32, u32), ()>,
    undecided: ValueIndex<u32, Retained<String>>,
    inherit_conflicts: ValueIndex<u32, ()>,
    autonomy_gap: Retained<Option<String>>,
    subject_gap: Retained<Option<String>>,
    pub(crate) changed: ValueIndex<u32, ()>,
    pub(crate) warnings: Retained<Vec<String>>,
}
fn gap<T: replay_data::rows::TableSchema>(
    table: &Available<'_, T>,
    name: &str,
) -> io::Result<Retained<Option<String>>> {
    Ok(Retained::new(match table {
        Available::Complete(_) => None,
        Available::Truncated { missing, .. } => {
            Some(format!("{name} is truncated ({missing} missing rows)"))
        }
        Available::Unavailable { reason } => Some(format!("{name} is unavailable: {reason}")),
        Available::NotRecorded { mode } => {
            Some(format!("{name} is not recorded in a {mode} recording"))
        }
    }))
}
pub(crate) fn autonomy_gap_warning(gap: &str) -> String {
    format!(
        "{gap}, so a country without a recorded current autonomy has an unknown colour source; \
         its colour is withheld"
    )
}

pub(crate) fn undecided_warning(slot: u32, why: &str) -> String {
    format!("Country slot {slot}'s colour source is unknown ({why}); its colour is withheld")
}

fn missing_link(reference: u32, role: &str) -> io::Result<Retained<String>> {
    Ok(Retained::new(format!(
        "political fill references missing country link {reference} for {role}"
    )))
}
fn active(row: &Subject, hour: i32) -> bool {
    row.4 == 0 && (row.3 <= row.2 || hour < row.3)
}
impl Relations {
    pub(crate) fn read(
        map: &MapData,
        frame: &DecodedFrame,
        links: &RowFold<u32>,
        previous: Option<&Self>,
    ) -> io::Result<Self> {
        let autonomy_rows = rows::<tables::Autonomy>(frame)?;
        let autonomy = RowFold::read(
            frame,
            TableId::Autonomy,
            autonomy_rows.rows(),
            fields(TableId::Autonomy, &["state"]),
            previous.map(|old| &old.autonomy),
            |row| Ok(Retained::new(row.state.clone())),
        )?;
        let subject_rows = rows::<tables::Subjects>(frame)?;
        let subjects = RowFold::read(
            frame,
            TableId::Subjects,
            subject_rows.rows(),
            fields(TableId::Subjects, &["start_hour", "end_hour", "cancel"]),
            previous.map(|old| &old.subjects),
            |row| {
                Ok((
                    row.subject,
                    row.overlord,
                    row.start_hour,
                    row.end_hour,
                    row.cancel,
                ))
            },
        )?;
        let subject_gap = gap(&subject_rows, "Subjects")?;
        let autonomy_gap = gap(&autonomy_rows, "Autonomy")?;
        let mut next = match previous {
            Some(old) => old.clone(),
            None => Self {
                hour: frame.hour,
                subjects: subjects.clone(),
                autonomy: autonomy.clone(),
                subject_by_link: Default::default(),
                autonomy_by_link: Default::default(),
                ends: Default::default(),
                subject_slots: Default::default(),
                autonomy_slots: Default::default(),
                by_subject: Default::default(),
                by_status: Default::default(),
                subject_errors: Default::default(),
                autonomy_errors: Default::default(),
                status_conflicts: Default::default(),
                status_errors: Default::default(),
                overlords: Default::default(),
                conflicting: Default::default(),
                states: Default::default(),
                inherited: Default::default(),
                by_overlord: Default::default(),
                undecided: Default::default(),
                inherit_conflicts: Default::default(),
                autonomy_gap: autonomy_gap.clone(),
                subject_gap: subject_gap.clone(),
                changed: Default::default(),
                warnings: Retained::new(Vec::new()),
            },
        };
        let mut dirty_subjects = ValueIndex::default();
        let mut dirty_autonomy = ValueIndex::default();
        for &key in subjects.edits.iter() {
            dirty_subjects = dirty_subjects.inserted(key, ())?;
        }
        for &key in autonomy.edits.iter() {
            dirty_autonomy = dirty_autonomy.inserted(key, ())?;
        }
        for &key in links.edits.iter() {
            for (&(_, row), _) in next
                .subject_by_link
                .iter_from(&(key as u32, 0))
                .take_while(|(pair, _)| pair.0 == key as u32)
            {
                dirty_subjects = dirty_subjects.inserted(row, ())?;
            }
            for (&(_, row), _) in next
                .autonomy_by_link
                .iter_from(&(key as u32, 0))
                .take_while(|(pair, _)| pair.0 == key as u32)
            {
                dirty_autonomy = dirty_autonomy.inserted(row, ())?;
            }
        }
        if let Some(old) = previous {
            let (from, until) = (old.hour.min(frame.hour), old.hour.max(frame.hour));
            for (&(hour, key), _) in next
                .ends
                .iter_from(&(from, u128::MAX))
                .take_while(|(pair, _)| pair.0 <= until)
            {
                if hour > from {
                    dirty_subjects = dirty_subjects.inserted(key, ())?;
                }
            }
        }
        let mut dirty_slots = ValueIndex::default();
        for (&key, _) in dirty_autonomy.iter() {
            if let Some(&slot) = next.autonomy_slots.get(&key) {
                next.by_status = next.by_status.removed(&(slot, key))?;
                next.autonomy_slots = next.autonomy_slots.removed(&key)?;
                dirty_slots = dirty_slots.inserted(slot, ())?;
            }
            next.autonomy_by_link = next.autonomy_by_link.removed(&(key as u32, key))?;
            next.autonomy_errors = next.autonomy_errors.removed(&key)?;
            if let Some(state) = autonomy.values.get(&key) {
                next.autonomy_by_link = next.autonomy_by_link.inserted((key as u32, key), ())?;
                match links.values.get(&key) {
                    Some(&slot) => {
                        next.autonomy_slots = next.autonomy_slots.inserted(key, slot)?;
                        next.by_status = next.by_status.inserted((slot, key), state.clone())?;
                        dirty_slots = dirty_slots.inserted(slot, ())?;
                    }
                    None => {
                        next.autonomy_errors = next
                            .autonomy_errors
                            .inserted(key, missing_link(key as u32, "autonomy colour")?)?
                    }
                }
            }
        }
        for (&key, _) in dirty_subjects.iter() {
            if let Some(&(slot, _)) = next.subject_slots.get(&key) {
                next.by_subject = next.by_subject.removed(&(slot, key))?;
                next.subject_slots = next.subject_slots.removed(&key)?;
                dirty_slots = dirty_slots.inserted(slot, ())?;
            }
            if let Some(old) = previous.and_then(|old| old.subjects.values.get(&key)) {
                next.subject_by_link = next.subject_by_link.removed(&(old.0, key))?;
                next.subject_by_link = next.subject_by_link.removed(&(old.1, key))?;
                next.ends = next.ends.removed(&(old.3, key))?;
            }
            next.subject_errors = next.subject_errors.removed(&key)?;
            if let Some(row) = subjects.values.get(&key) {
                for raw in [row.0, row.1] {
                    next.subject_by_link = next.subject_by_link.inserted((raw, key), ())?;
                }
                if row.4 == 0 && row.3 > row.2 {
                    next.ends = next.ends.inserted((row.3, key), ())?;
                }
                if active(row, frame.hour) {
                    match (
                        links.values.get(&(row.0 as u128)),
                        links.values.get(&(row.1 as u128)),
                    ) {
                        (Some(&slot), Some(&overlord)) => {
                            next.subject_slots =
                                next.subject_slots.inserted(key, (slot, overlord))?;
                            next.by_subject = next.by_subject.inserted((slot, key), overlord)?;
                            dirty_slots = dirty_slots.inserted(slot, ())?;
                        }
                        (None, _) => {
                            next.subject_errors = next
                                .subject_errors
                                .inserted(key, missing_link(row.0, "subject colour")?)?
                        }
                        (_, None) => {
                            next.subject_errors = next
                                .subject_errors
                                .inserted(key, missing_link(row.1, "subject colour")?)?
                        }
                    }
                }
            }
        }
        if next.subject_gap != subject_gap {
            for &slot in next.states.keys() {
                dirty_slots = dirty_slots.inserted(slot, ())?;
            }
        }
        for (&slot, _) in dirty_slots.iter() {
            if let Some(&raw) = next.status_conflicts.get(&slot) {
                next.status_errors = next.status_errors.removed(&raw)?;
                next.status_conflicts = next.status_conflicts.removed(&slot)?;
            }
            let mut states = next
                .by_status
                .iter_from(&(slot, 0))
                .take_while(|(pair, _)| pair.0 == slot);
            let state = states.next().map(|(_, state)| state.clone());
            if let Some(first) = &state {
                if let Some((&(_, raw), _)) = states.find(|(_, state)| *state != first) {
                    next.status_conflicts = next.status_conflicts.inserted(slot, raw)?;
                    next.status_errors = next.status_errors.inserted(raw, slot)?;
                }
                next.states = next.states.inserted(slot, first.clone())?;
            } else {
                next.states = next.states.removed(&slot)?;
            }
            let mut overlords = next
                .by_subject
                .iter_from(&(slot, 0))
                .take_while(|(pair, _)| pair.0 == slot);
            let mut overlord = overlords.next().map(|(_, value)| *value);
            next.conflicting = next.conflicting.removed(&slot)?;
            if let Some(first) = overlord {
                for (_, &value) in overlords {
                    if value != first {
                        next.conflicting = next.conflicting.inserted(slot, ())?;
                    }
                    overlord = Some(value);
                }
            }
            next.overlords = match overlord {
                Some(value) => next.overlords.inserted(slot, value)?,
                None => next.overlords.removed(&slot)?,
            };
        }
        let link_error = next.autonomy_errors.iter().next();
        let status_error = next.status_errors.iter().next();
        if let Some((&raw, error)) = link_error {
            if status_error.is_none_or(|(&key, _)| raw < key) {
                return Err(invalid(error.to_string()));
            }
        }
        if let Some((_, &slot)) = status_error {
            return Err(invalid(format!(
                "country slot {slot} has conflicting current autonomy definitions"
            )));
        }
        if let Some((_, error)) = next.subject_errors.iter().next() {
            return Err(invalid(error.to_string()));
        }
        next.changed = ValueIndex::default();
        let mut warnings_changed = next.autonomy_gap != autonomy_gap;
        for (&slot, _) in dirty_slots.iter() {
            next.changed = next.changed.inserted(slot, ())?;
            let before_edge = next.inherited.get(&slot).copied();
            let before_warning = next.undecided.get(&slot).cloned();
            next.inherited = next.inherited.removed(&slot)?;
            next.undecided = next.undecided.removed(&slot)?;
            next.inherit_conflicts = next.inherit_conflicts.removed(&slot)?;
            if let Some(state) = next.states.get(&slot).filter(|state| !state.is_empty()) {
                match map.country_colors.use_overlord_color.get(state.as_slice()) {
                    None => {
                        next.undecided = next.undecided.inserted(slot, Retained::new(format!("its current autonomy definition {} is not an autonomy state the content defines, so its use_overlord_color flag is unknown", String::from_utf8_lossy(state))))?;
                    }
                    Some(false) => {}
                    Some(true) if next.conflicting.get(&slot).is_some() => {
                        next.inherit_conflicts = next.inherit_conflicts.inserted(slot, ())?;
                    }
                    Some(true) => match next.overlords.get(&slot) {
                        Some(&overlord) => {
                            next.inherited = next.inherited.inserted(slot, overlord)?;
                        }
                        None => {
                            next.undecided = next.undecided.inserted(slot, Retained::new(match subject_gap.as_ref() {
                                Some(gap) => format!("its current autonomy inherits, and {gap}"),
                                None => "its current autonomy inherits, but no active Subjects row names its overlord".to_owned(),
                            }))?;
                        }
                    },
                }
            }
            let after_edge = next.inherited.get(&slot).copied();
            if before_edge != after_edge {
                if let Some(overlord) = before_edge {
                    next.by_overlord = next.by_overlord.removed(&(overlord, slot))?;
                }
                if let Some(overlord) = after_edge {
                    next.by_overlord = next.by_overlord.inserted((overlord, slot), ())?;
                }
            }
            warnings_changed |= before_warning.as_ref() != next.undecided.get(&slot);
        }
        if let Some(&slot) = next.inherit_conflicts.keys().next() {
            return Err(invalid(format!(
                "country slot {slot} has conflicting colour overlords"
            )));
        }
        if previous.is_none() || warnings_changed {
            let gap = autonomy_gap.as_ref();
            let count = next
                .undecided
                .len()
                .checked_add(usize::from(gap.is_some()))
                .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
            next.warnings = Retained::new({
                let mut warnings = Vec::with_capacity(count);
                warnings.extend(gap.map(|gap| autonomy_gap_warning(gap)));
                warnings.extend(
                    next.undecided
                        .iter()
                        .map(|(slot, why)| undecided_warning(*slot, why)),
                );
                warnings
            });
        }
        next.autonomy_gap = autonomy_gap;
        next.subject_gap = subject_gap;
        next.subjects = subjects;
        next.autonomy = autonomy;
        next.hour = frame.hour;
        Ok(next)
    }
    pub(crate) fn source(&self, slot: u32) -> io::Result<Option<u32>> {
        let mut source = slot;
        for _ in 0..=self.inherited.len() {
            if self.undecided.get(&source).is_some()
                || (self.autonomy_gap.is_some() && self.states.get(&source).is_none())
            {
                return Ok(None);
            }
            match self.inherited.get(&source) {
                Some(&next) => source = next,
                None => return Ok(Some(source)),
            }
        }
        Err(invalid("cycle in subject colour inheritance"))
    }

    pub(crate) fn recolors_all(&self, previous: Option<&Self>) -> bool {
        previous.is_some_and(|old| old.autonomy_gap.is_some() != self.autonomy_gap.is_some())
    }
    pub(crate) fn affected(&self, previous: Option<&Self>) -> io::Result<ValueIndex<u32, ()>> {
        let mut pending = self.changed.clone();
        let mut affected = ValueIndex::default();
        loop {
            let Some(slot) = pending.keys().next().copied() else {
                break;
            };
            pending = pending.removed(&slot)?;
            if affected.get(&slot).is_some() {
                continue;
            }
            affected = affected.inserted(slot, ())?;
            for index in
                std::iter::once(&self.by_overlord).chain(previous.map(|old| &old.by_overlord))
            {
                for (&(_, child), _) in index
                    .iter_from(&(slot, 0))
                    .take_while(|(pair, _)| pair.0 == slot)
                {
                    if affected.get(&child).is_none() {
                        pending = pending.inserted(child, ())?;
                    }
                }
            }
        }
        Ok(affected)
    }
}
