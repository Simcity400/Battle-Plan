use super::*;
use crate::script_test_game as game;
use crate::triggers::inputs::{self, Key};
use replay_data::rows::{LexicalTokenRow, PoliticsRow, TableId, TableRows};
use std::sync::Arc;

struct Fixture {
    catalogue: crate::catalogue::Catalogue,
    _folder: crate::catalogue::tests::Folder,
    frame: DecodedFrame,
}

impl Fixture {
    fn new() -> Self {
        game::enter();
        let (catalogue, folder) = super::tests::catalogue("", "");
        let mut frame = super::tests::complete_frame();
        *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![
            PoliticsRow {
                country: 100,
                ..Default::default()
            },
            PoliticsRow {
                country: 200,
                ..Default::default()
            },
        ]));
        Self {
            catalogue,
            _folder: folder,
            frame,
        }
    }

    fn text(&self, root: u32) -> ScriptedText<'_> {
        ScriptedText::new(
            &self.frame,
            &self.catalogue.localisation,
            self.catalogue.scripted_content().unwrap(),
            root,
        )
    }
}

#[test]
fn root_views_expand_each_root_with_unchanged_native_names() {
    let fixture = Fixture::new();
    let first = fixture.text(1);
    assert_eq!(
        first.expand("[ROOT.GetTag] / [ROOT.GetName]"),
        Ok("GER / German Reich".into())
    );
    let second = first.for_root(2);
    assert_eq!(
        second.expand("[ROOT.GetTag] / [ROOT.GetName]"),
        Ok("SOV / Soviet Union".into())
    );
    assert_eq!(
        second.expand("[ROOT.GetTag] / [ROOT.GetName]"),
        fixture.text(2).expand("[ROOT.GetTag] / [ROOT.GetName]")
    );
    assert_eq!(first.expand("[ROOT.GetTag]"), Ok("GER".into()));
}

#[test]
fn root_views_keep_lazy_worlds_and_view_settings_independent() {
    let fixture = Fixture::new();
    let first = fixture.text(1).with_nato_counters(false);
    assert!(first.world.get().is_none());
    let second = first.for_root(2);
    assert!(!second.nato_counters);
    assert!(second.world.get().is_none());
    assert!(
        first
            .world
            .set(Err("first root world refusal".into()))
            .is_ok()
    );
    assert!(second.world.get().is_none());
    let second = second.with_nato_counters(true);
    assert!(second.nato_counters);
    assert!(!first.nato_counters);
    let third = first.for_root(3);
    assert!(third.world.get().is_none());
    assert!(!third.nato_counters);
}

#[test]
fn root_views_retain_input_owners_after_the_original_root_drops() {
    let fixture = Fixture::new();
    let first = fixture.text(1);
    let second = first.for_root(2);
    let scopes = Arc::downgrade(&first.scopes);
    let names = Arc::downgrade(&first.names);
    let lexical = Arc::downgrade(&first.lexical_names);
    drop(first);
    assert!(scopes.upgrade().is_some());
    assert!(names.upgrade().is_some());
    assert!(lexical.upgrade().is_some());
    assert_eq!(second.country_tag(2), Ok("SOV"));
    drop(second);
    assert!(scopes.upgrade().is_none());
    assert!(names.upgrade().is_none());
    assert!(lexical.upgrade().is_none());
}

#[test]
fn warm_root_views_still_record_tag_and_name_dependencies() {
    let fixture = Fixture::new();
    let first = fixture.text(1);
    assert_eq!(first.country_tag(2), Ok("SOV"));
    assert_eq!(first.country_names().name(2), Ok("Soviet Union".into()));
    let second = first.for_root(2);
    let ((tag, name), mut reads) = inputs::capture(&fixture.frame, || {
        (second.country_tag(2), second.country_names().name(2))
    });
    assert_eq!(tag, Ok("SOV"));
    assert_eq!(name, Ok("Soviet Union".into()));
    assert_eq!(
        reads.keys().cloned().collect::<Vec<_>>(),
        vec![
            Key::Text(TextQuery::CountryTag(2)),
            Key::Names(recorded_names::Query::Name(2)),
        ]
    );
    assert!(reads.confirm(&fixture.frame, |key| second.reread(key)));
    let mut changed = fixture.frame.clone();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[1].tag = b"MOD".to_vec();
    }
    let changed_text = ScriptedText::new(
        &changed,
        &fixture.catalogue.localisation,
        first.content(),
        2,
    );
    assert!(!reads.confirm(&changed, |key| changed_text.reread(key)));
}

#[test]
fn root_views_preserve_missing_naming_and_reserved_zero_refusals() {
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "synthetic appearance gap".into(),
    };
    let first = fixture.text(1);
    let second = first.for_root(2);
    let native = fixture.text(2);
    let refusal = second.country_names().name(2).unwrap_err();
    assert!(refusal.contains("country appearance"), "{refusal}");
    assert_eq!(Err(refusal), native.country_names().name(2));
    assert_eq!(second.country_tag(2), Ok("SOV"));
    let reserved = first.for_root(0);
    let refusal = reserved.country_tag(0).unwrap_err();
    assert!(refusal.contains("reserved country naming"), "{refusal}");
    assert_eq!(Err(refusal), fixture.text(0).country_tag(0));
}

#[test]
fn root_views_preserve_warm_lexical_duplicate_diagnostics() {
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"same".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"SAME".to_vec(),
            },
        ]));
    let first = fixture.text(1);
    assert_eq!(
        first.lexical_names().token_of(b"same"),
        Err("SAME names several lexer tokens".into())
    );
    let second = first.for_root(2);
    assert_eq!(
        second.lexical_names().token_of(b"same"),
        Err("SAME names several lexer tokens".into())
    );
    assert_eq!(second.lexical_names().token_of(b"other"), Ok(None));
    assert_eq!(
        second.lexical_names().token_of(b"same"),
        fixture.text(2).lexical_names().token_of(b"same")
    );
}

#[test]
fn root_views_never_share_inputs_across_separately_constructed_hours() {
    let fixture = Fixture::new();
    let first = fixture.text(1);
    let mut later = fixture.frame.clone();
    later.hour += 1;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *later.tables[TableId::Countries.index()]
    {
        rows[1].tag = b"MOD".to_vec();
    }
    let later_text = ScriptedText::new(&later, &fixture.catalogue.localisation, first.content(), 2);
    assert!(!Arc::ptr_eq(&first.scopes, &later_text.scopes));
    assert!(!Arc::ptr_eq(&first.names, &later_text.names));
    assert!(!Arc::ptr_eq(
        &first.lexical_names,
        &later_text.lexical_names
    ));
    assert_eq!(first.for_root(2).country_tag(2), Ok("SOV"));
    assert_eq!(later_text.for_root(2).country_tag(2), Ok("MOD"));
}
