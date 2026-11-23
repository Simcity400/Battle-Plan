
use dashboard_query::{identity::{Holder, Holders}, read};
use replay_recording::columnar_frames::Recording;
use replay_columnar::summary::Source as Column;
use replay_data::rows::TableId;
use std::{collections::HashMap, io};

#[derive(Default)]
pub struct RunLinks {
    pub slots: HashMap<u32, read::Runs>,
}

impl RunLinks {
    pub fn within(&self, raw: u32, first: i32, last: i32) -> impl Iterator<Item = (Option<u32>, i32, i32)> + '_ {
        let runs = self.slots.get(&raw).map(Vec::as_slice).unwrap_or_default();
        let start = runs.partition_point(|run| run.1 < first);
        let covered = runs[start..].iter().take_while(move |run| run.0 <= last).map(move |&(a, b, slot)| (slot.map(|slot| slot as u32), a.max(first), b.min(last)));
        let mut at = i64::from(first);
        let mut out = Vec::new();
        for (slot, a, b) in covered {
            if i64::from(a) > at {
                out.push((None, at as i32, a - 1));
            }
            out.push((slot, a, b));
            at = i64::from(b) + 1;
        }
        if at <= i64::from(last) {
            out.push((None, at as i32, last));
        }
        out.into_iter()
    }

    pub fn slot_at(&self, raw: u32, hour: i32) -> Option<u32> {
        self.within(raw, hour, hour).next().and_then(|(slot, ..)| slot)
    }
}

pub fn run_links(recording: &Recording, run: u32) -> io::Result<RunLinks> {
    let held = recording.run(run)?;
    let (first, last) = (held.first_hour, held.first_hour + held.ordinals as i32 - 1);
    let column = dashboard_declare::field(TableId::CountryLinks, "canonical_slot")?;
    let runs = read::runs(recording, run, TableId::CountryLinks, Column::Cell(column), None, first, last)?;
    let mut slots: HashMap<u32, read::Runs> = runs.into_iter().map(|(key, runs)| (key as u32, runs)).collect();
    for runs in slots.values_mut() {
        runs.sort_unstable_by_key(|run| run.0);
    }
    Ok(RunLinks { slots })
}

pub struct Names {
    pub slots: HashMap<u32, Vec<(String, i32, i32)>>,
}

impl Names {
    pub fn new(holders: &Holders) -> Self {
        let mut slots: HashMap<u32, Vec<(String, i32, i32)>> = HashMap::new();
        for (tag, runs) in &holders.tags {
            for run in runs {
                if let Holder::Found { slot, .. } = run.holder {
                    slots.entry(slot).or_default().push((tag.clone(), run.first, run.last));
                }
            }
        }
        for runs in slots.values_mut() {
            runs.sort_unstable_by_key(|run| run.1);
        }
        Self { slots }
    }

    pub fn slot_tag(&self, slot: u32, hour: i32) -> Option<&str> {
        let runs = self.slots.get(&slot)?;
        let at = runs.partition_point(|run| run.2 < hour);
        runs.get(at).filter(|run| run.1 <= hour).map(|run| run.0.as_str())
    }

    pub fn slot_tags(&self, slot: u32, first: i32, last: i32) -> Vec<(Option<&str>, i32, i32)> {
        let runs = self.slots.get(&slot).map(Vec::as_slice).unwrap_or_default();
        let start = runs.partition_point(|run| run.2 < first);
        let mut out = Vec::new();
        let mut at = i64::from(first);
        for (tag, a, b) in runs[start..].iter().take_while(|run| run.1 <= last) {
            let (a, b) = ((*a).max(first), (*b).min(last));
            if i64::from(a) > at {
                out.push((None, at as i32, a - 1));
            }
            out.push((Some(tag.as_str()), a, b));
            at = i64::from(b) + 1;
        }
        if at <= i64::from(last) {
            out.push((None, at as i32, last));
        }
        out
    }

    pub fn tag_at(&self, links: &RunLinks, raw: u32, hour: i32) -> Option<&str> {
        self.slot_tag(links.slot_at(raw, hour)?, hour)
    }

    pub fn tags_over(&self, links: &RunLinks, raw: u32, first: i32, last: i32) -> Vec<(Option<&str>, i32, i32)> {
        let mut out: Vec<(Option<&str>, i32, i32)> = Vec::new();
        for (slot, a, b) in links.within(raw, first, last) {
            let pieces = match slot {
                Some(slot) => self.slot_tags(slot, a, b),
                None => vec![(None, a, b)],
            };
            for piece in pieces {
                match out.last_mut() {
                    Some(held) if held.0 == piece.0 && i64::from(held.2) + 1 == i64::from(piece.1) => held.2 = piece.2,
                    _ => out.push(piece),
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_named_by_its_slots_holder_and_unnamed_where_no_link_or_holder_is() {
        let links = RunLinks { slots: HashMap::from([(7, vec![(100, 149, Some(3)), (150, 199, Some(4))])]) };
        let names = Names { slots: HashMap::from([(3, vec![("AAA".to_owned(), 100, 139), ("BBB".to_owned(), 140, 199)]), (4, vec![("CCC".to_owned(), 160, 199)])]) };
        assert_eq!(names.tag_at(&links, 7, 120), Some("AAA"));
        assert_eq!(names.tag_at(&links, 7, 155), None);
        assert_eq!(names.tag_at(&links, 8, 120), None);
        assert_eq!(
            names.tags_over(&links, 7, 90, 210),
            vec![(None, 90, 99), (Some("AAA"), 100, 139), (Some("BBB"), 140, 149), (None, 150, 159), (Some("CCC"), 160, 199), (None, 200, 210)]
        );
    }
}
