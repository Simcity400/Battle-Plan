//! A run's lexicon: every literal LexicalTokens held, with the tokens that held it and the hours
//! each did, read once per run ([`crate::answer::lexicon`]) so a reply mapping text to tokens looks
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

#[cfg(test)]
mod tests {
    use super::RunLexicon;
    use crate::{Source, test_fixtures::{CAMPAIGN, records, source}};
    use replay::campaign::Replay;
    use replay_data::rows::{LexicalTokenRow, TableId, TableRows};
    use replay_frame::frame::{DecodedFrame, TableView};
    use std::sync::{Arc, atomic::AtomicBool};

    /// Token 7 is `spy_steal` throughout; 8 is `spy_steal` until 109, then `spy_other`; 9 appears
    /// at 120 as `spy_steal`; 10 holds no text.
    fn frame(hour: i32) -> DecodedFrame {
        let mut frame = source(1, hour, Some(1_000));
        let row = |token: u32, text: &str| LexicalTokenRow { token, literal: text.as_bytes().to_vec() };
        let mut rows = vec![row(7, "spy_steal"), row(8, if hour < 110 { "spy_steal" } else { "spy_other" }), row(10, "")];
        if hour >= 120 {
            rows.push(row(9, "spy_steal"));
        }
        rows.sort_by_key(|row| row.token);
        frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(rows)).into();
        frame.metrics.table_ns[TableId::LexicalTokens.index()] = 1;
        frame
    }

    fn recording() -> Source {
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, (100..=130).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(crate::SourceKey { source_id: "lexicon".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn a_literal_is_every_token_holding_it_within_the_hours_asked_and_an_empty_one_is_none() {
        let source = recording();
        let run = crate::answer::query(&source, 1).unwrap().timeline.stretches[0].run;
        let lexicon = RunLexicon::read(source.replay_ref().recording(), run).unwrap();
        let tokens = |literal: &str, from: i32, to: i32| lexicon.tokens(literal.as_bytes(), from, to).collect::<Vec<_>>();
        assert_eq!(tokens("spy_steal", 100, 130), [7, 8, 9]);
        assert_eq!(tokens("spy_steal", 110, 119), [7], "8 left it at 110 and 9 holds it from 120");
        assert_eq!(tokens("spy_steal", 109, 109), [7, 8]);
        assert_eq!(tokens("spy_steal", 120, 120), [7, 9]);
        assert_eq!(tokens("spy_other", 100, 109), Vec::<u32>::new());
        assert_eq!(tokens("spy_other", 100, 130), [8]);
        assert_eq!(tokens("", 100, 130), Vec::<u32>::new(), "a token holding no text names nothing");
        assert_eq!(tokens("unheld", 100, 130), Vec::<u32>::new());
    }

    #[test]
    fn a_runs_lexicon_is_read_once_for_every_reply_on_a_source() {
        let source = recording();
        crate::answer::warm_lexicons(&source).unwrap();
        let run = crate::answer::query(&source, 1).unwrap().timeline.stretches[0].run;
        let first = crate::answer::lexicon(&source, run).unwrap();
        assert!(Arc::ptr_eq(&first, &crate::answer::lexicon(&source, run).unwrap()), "the open's warm-up is the reply's read");
        crate::answer::forget(&source);
        assert!(!Arc::ptr_eq(&first, &crate::answer::lexicon(&source, run).unwrap()), "a forgotten index reads it again");
    }

    /// A token's text at an hour is read once for every reply on a source: asked again, alone or with others already
    /// read, nothing is read from storage; another hour reads its own texts.
    #[test]
    fn a_tokens_spelling_at_an_hour_is_read_once_for_every_reply_on_a_source() {
        let source = recording();
        let run = crate::answer::query(&source, 1).unwrap().timeline.stretches[0].run;
        let spelled = |hour: i32, tokens: &[u32]| {
            let mut found: Vec<(u32, String)> = crate::answer::spellings(&source, run, hour, &tokens.iter().copied().collect()).unwrap().into_iter().collect();
            found.sort();
            (found, dashboard_query::cost::take())
        };
        let spy = |token: u32, text: &str| (token, text.to_owned());
        assert_eq!(spelled(105, &[7, 8]).0, [spy(7, "spy_steal"), spy(8, "spy_steal")]);
        assert_eq!(spelled(105, &[8, 7]), (vec![spy(7, "spy_steal"), spy(8, "spy_steal")], 0), "read once, nothing read again");
        assert_eq!(spelled(105, &[7, 9, 10]).0, [spy(7, "spy_steal")], "9 held nothing at 105, 10 holds no text");
        assert_eq!(spelled(105, &[9, 10, 7, 8]), (vec![spy(7, "spy_steal"), spy(8, "spy_steal")], 0), "a token without text is not read again");
        assert_eq!(spelled(125, &[8, 9]).0, [spy(8, "spy_other"), spy(9, "spy_steal")], "another hour, its own texts");
        crate::answer::forget(&source);
        assert_eq!(spelled(125, &[8]).0, [spy(8, "spy_other")]);
    }
}
