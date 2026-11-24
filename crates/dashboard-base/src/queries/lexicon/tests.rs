use super::RunLexicon;
use crate::{Source, test_fixtures::{CAMPAIGN, records, source}};
use replay::campaign::Replay;
use replay_data::rows::{LexicalTokenRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::{Arc, atomic::AtomicBool};

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
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (100..=130).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(crate::SourceKey { source_id: "lexicon".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

#[test]
fn a_literal_is_every_token_holding_it_within_the_hours_asked_and_an_empty_one_is_none() {
    let source = recording();
    let run = crate::queries::query(&source, 1).unwrap().timeline.stretches[0].run;
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
fn a_tokens_spelling_at_an_hour_is_read_once_for_every_reply_on_a_source() {
    let source = recording();
    let run = crate::queries::query(&source, 1).unwrap().timeline.stretches[0].run;
    let spelled = |hour: i32, tokens: &[u32]| {
        let mut found: Vec<(u32, String)> = crate::queries::spellings(&source, run, hour, &tokens.iter().copied().collect()).unwrap().into_iter().collect();
        found.sort();
        (found, dashboard_query::cost::take())
    };
    let spy = |token: u32, text: &str| (token, text.to_owned());
    assert_eq!(spelled(105, &[7, 8]).0, [spy(7, "spy_steal"), spy(8, "spy_steal")]);
    assert_eq!(spelled(105, &[8, 7]), (vec![spy(7, "spy_steal"), spy(8, "spy_steal")], 0), "read once, nothing read again");
    assert_eq!(spelled(105, &[7, 9, 10]).0, [spy(7, "spy_steal")], "9 held nothing at 105, 10 holds no text");
    assert_eq!(spelled(105, &[9, 10, 7, 8]), (vec![spy(7, "spy_steal"), spy(8, "spy_steal")], 0), "a token without text is not read again");
    assert_eq!(spelled(125, &[8, 9]).0, [spy(8, "spy_other"), spy(9, "spy_steal")], "another hour, its own texts");
    crate::queries::forget(&source);
    assert_eq!(spelled(125, &[8]).0, [spy(8, "spy_other")]);
}
