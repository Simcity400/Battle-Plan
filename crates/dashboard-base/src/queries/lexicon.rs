
use replay_recording::columnar_frames::Recording;
use replay_lanes::{lanes::Keys, points};
use replay_data::rows::TableId;
use std::{collections::HashMap, io};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub token: u32,
    pub first: i32,
    pub last: i32,
}

#[derive(Debug, Default)]
pub struct RunLexicon {
    by_literal: HashMap<Box<[u8]>, Vec<Held>>,
    changes: Vec<(u32, i32)>,
}

impl RunLexicon {
    pub fn read(recording: &Recording, run: u32) -> io::Result<Self> {
        let held = recording.run(run)?;
        let (first, last) = (held.first_hour, held.first_hour + held.ordinals as i32 - 1);
        let mut open: HashMap<u32, (u32, i32)> = HashMap::default();
        let mut spans: Vec<(u32, Held)> = Vec::new();
        let mut changes = Vec::new();
        let (_, texts) = dashboard_query::cost::storage(|| {
            points::cells(recording, run, TableId::LexicalTokens, &[0], Keys::All, first, last, None, &mut |hour, key, cells| {
                let token = key as u32;
                changes.push((token, hour));
                if let Some((text, since)) = open.remove(&token) {
                    spans.push((text, Held { token, first: since, last: hour - 1 }));
                }
                if let Some([points::Cell::Text(text), ..]) = cells {
                    open.insert(token, (*text, hour));
                }
                true
            })
        })?;
        spans.extend(open.into_iter().map(|(token, (text, since))| (text, Held { token, first: since, last })));
        let mut by_literal: HashMap<Box<[u8]>, Vec<Held>> = HashMap::default();
        for (text, span) in spans {
            if let Some(literal) = texts.get(text as usize).filter(|literal| !literal.is_empty()) {
                by_literal.entry(literal.clone()).or_default().push(span);
            }
        }
        for spans in by_literal.values_mut() {
            spans.sort_unstable_by_key(|span| (span.token, span.first));
        }
        changes.sort_unstable();
        Ok(Self { by_literal, changes })
    }

    pub fn changes(&self, token: u32, after: i32, through: i32) -> impl Iterator<Item = i32> + '_ {
        let start = self.changes.partition_point(|&held| held <= (token, after));
        self.changes[start..].iter().take_while(move |&&(held, hour)| held == token && hour <= through).map(|&(_, hour)| hour)
    }

    pub fn tokens(&self, literal: &[u8], from: i32, to: i32) -> impl Iterator<Item = u32> + '_ {
        let spans = self.by_literal.get(literal).map_or(&[][..], Vec::as_slice);
        let mut last = None;
        spans.iter().filter(move |span| span.first <= to && span.last >= from).map(|span| span.token).filter(move |token| last.replace(*token) != Some(*token))
    }
}

#[cfg(test)]
mod tests;
