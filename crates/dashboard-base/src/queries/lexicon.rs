//! A run's lexicon: every literal LexicalTokens held, with the tokens that held it and the hours
//! each did, read once per run ([`super::lexicon`]) so a reply mapping text to tokens looks
//! a literal up instead of reading the table's whole history.

use replay::{columnar_frames::Recording, lanes::Keys, points};
use replay_data::rows::TableId;
use std::{collections::HashMap, io};

/// One token holding a literal from `first` through `last`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub token: u32,
    pub first: i32,
    pub last: i32,
}

#[derive(Debug, Default)]
pub struct RunLexicon {
    by_literal: HashMap<Box<[u8]>, Vec<Held>>,
}

impl RunLexicon {
    /// Run `run`'s lexicon over its every hour: one pass over LexicalTokens' literal column.
    pub fn read(recording: &Recording, run: u32) -> io::Result<Self> {
        let held = recording.run(run)?;
        let (first, last) = (held.first_hour, held.first_hour + held.ordinals as i32 - 1);
        let mut open: HashMap<u32, (u32, i32)> = HashMap::default();
        let mut spans: Vec<(u32, Held)> = Vec::new();
        let (_, texts) = dashboard_query::cost::storage(|| {
            points::cells(recording, run, TableId::LexicalTokens, &[0], Keys::All, first, last, &mut |hour, key, cells| {
                let token = key as u32;
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
        Ok(Self { by_literal })
    }

    /// The tokens holding `literal` at some hour from `from` through `to`, ascending, once each.
    pub fn tokens(&self, literal: &[u8], from: i32, to: i32) -> impl Iterator<Item = u32> + '_ {
        let spans = self.by_literal.get(literal).map_or(&[][..], Vec::as_slice);
        let mut last = None;
        spans.iter().filter(move |span| span.first <= to && span.last >= from).map(|span| span.token).filter(move |token| last.replace(*token) != Some(*token))
    }
}
