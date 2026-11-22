use super::*;

fn key(kind: u8, identity: &[u8]) -> StabilityPartKeyRow {
    StabilityPartKeyRow {
        handle: 7,
        kind,
        identity: identity.to_vec(),
        identity_id: 0,
        secondary: 0,
        script: Recorded::NotRecorded,
        saved: Recorded::NotRecorded,
        template_present: Recorded::NotRecorded,
        template_tag: Recorded::NotRecorded,
        template_file: Recorded::NotRecorded,
        template_line: Recorded::NotRecorded,
        template_generated: Recorded::NotRecorded,
    }
}

const COUNTRY: IdeaCountry<'static> = IdeaCountry { tag: "AAA", slot: 1, look: None };

#[test]
fn a_part_is_asked_by_its_measure_and_handle_and_nothing_past_war_support() {
    assert_eq!(part_of("7"), Some((StabilityPartRow::STABILITY, 7)));
    assert_eq!(part_of(&((1u64 << 32) | 9).to_string()), Some((StabilityPartRow::WAR_SUPPORT, 9)));
    assert_eq!(part_of(&(2u64 << 32).to_string()), None);
    assert_eq!(part_of("-1"), None);
    assert_eq!(part_of("x"), None);
}

#[test]
fn a_parts_id_is_its_source_as_main_named_it_whatever_its_handle() {
    let term = named_part("7", 0, &key(StabilityPartKeyRow::TERM, b"base"), None, COUNTRY);
    assert_eq!((term.kind.as_str(), term.id.as_deref(), term.term.as_deref()), ("term", Some("term:base"), Some("base")));
    for (kind, id) in [(StabilityPartKeyRow::REMAINDER, "remainder"), (StabilityPartKeyRow::CLAMP, "clamp"), (StabilityPartKeyRow::TOTAL, "total")] {
        let part = named_part("7", 1, &key(kind, b""), None, COUNTRY);
        assert_eq!((part.id.as_deref(), part.measure.as_str(), part.line.is_none()), (Some(id), "warSupport", true));
    }
    let unpaired = named_part("7", 0, &key(CountryStabilityLineRow::UNPAIRED, b"Territorial Integrity"), None, COUNTRY);
    assert_eq!(unpaired.id.as_deref(), Some("unpaired:Territorial Integrity"));
    let line = unpaired.line.unwrap();
    assert_eq!((line.kind.as_str(), line.text.as_deref(), line.identity.as_str()), ("unpaired", Some("Territorial Integrity"), ""));
    let refused = named_part("7", 0, &key(StabilityPartKeyRow::REFUSED_LABEL, b"a\tb"), None, COUNTRY);
    assert_eq!(refused.id.as_deref(), Some("unpaired-refused:a\\tb"));
    assert_eq!(refused.line.unwrap().text, None);
    let mut idea = key(CountryStabilityLineRow::IDEA, b"some_idea");
    idea.secondary = -3;
    let idea = named_part("7", 0, &idea, None, COUNTRY);
    assert_eq!(idea.id.as_deref(), Some("line:idea:some_idea:0:-3"));
    assert_eq!(idea.line.unwrap().label, None, "no content names nothing, never a guess");
}

#[test]
fn a_character_line_carries_its_recorded_character_and_a_character_not_known_its_base() {
    let mut advisor = key(CountryStabilityLineRow::ADVISOR, b"adv_idea");
    advisor.identity_id = 41;
    assert_eq!(named_part("7", 0, &advisor, None, COUNTRY).id.as_deref(), Some("line:advisor:adv_idea:41:0"));
    advisor.saved = Recorded::Value(b"SAVED".to_vec());
    assert_eq!(named_part("7", 0, &advisor, None, COUNTRY).id.as_deref(), Some("line:advisor:adv_idea:41:0::SAVED"));
    advisor.script = Recorded::Value(b"script_token".to_vec());
    advisor.template_present = Recorded::Value(1);
    advisor.template_tag = Recorded::Value(b"TAG".to_vec());
    advisor.template_file = Recorded::Value(b"file.txt".to_vec());
    advisor.template_line = Recorded::Value(12);
    advisor.template_generated = Recorded::Value(0);
    assert_eq!(named_part("7", 0, &advisor, None, COUNTRY).id.as_deref(), Some("line:advisor:adv_idea:41:0:script_token:SAVED"));
    let character = part_character(&advisor).unwrap();
    let template = character.template.unwrap();
    assert_eq!((template.tag.as_str(), template.file.as_slice(), template.line, template.generated), ("TAG", b"file.txt".as_slice(), 12, false));
    advisor.template_present = Recorded::Value(0);
    assert!(part_character(&advisor).unwrap().template.is_err(), "a character with no template");
    advisor.template_present = Recorded::NotRecorded;
    assert!(part_character(&advisor).unwrap().template.is_err(), "a template not recorded is not known");
}

#[test]
fn a_keys_lines_add_to_its_total_and_what_they_leave_is_the_remainder() {
    let line = |line: u32, key: u32, value: i64| CountryStabilityLineRow { country: 5, line, key, kind: CountryStabilityLineRow::STATIC, identity: b"s".to_vec(), value, ..Default::default() };
    let lines = [line(0, 3, 10), line(1, 3, 15), line(2, 4, 99)];
    let mut refused = dashboard_text::catalogue::Refusals::default();
    let good = |_: u32| Some(true);
    let modifier = modifier((b"stability_factor", 3), Some(40), Some(&lines), &good, &mut refused, "AAA");
    assert_eq!(modifier.lines.len(), 2);
    assert_eq!(modifier.remainder_raw.as_deref(), Some("15"));
    let whole = super::modifier((b"stability_factor", 3), Some(25), Some(&lines), &good, &mut refused, "AAA");
    assert_eq!(whole.remainder_raw, None, "lines that add up leave no remainder");
    let unread = super::modifier((b"stability_factor", 3), Some(25), None, &good, &mut refused, "AAA");
    assert_eq!((unread.lines.len(), unread.remainder_raw), (0, None), "lines not read with the inputs explain nothing");
    let absent = super::modifier((b"stability_factor", u32::MAX), None, Some(&lines), &good, &mut refused, "AAA");
    assert_eq!((absent.index, absent.total_raw, absent.lines.len()), (None, None, 0));
}
