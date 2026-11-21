use super::*;

fn text(text: &str, tone: Option<&str>) -> GameRun {
    GameRun::Text { text: text.into(), tone: tone.map(Into::into) }
}

#[test]
fn game_text_keeps_lines_tones_icons_and_unlock_links() {
    let rendered = "\u{13}GFX_unit_icon|1 Infantry\n   Soft:\u{11}G0.17\u{11}!\nEnables \u{11}H\u{1}eequipment_a\u{2}Rifle\u{3}\u{11}! and \u{12}GER\n";
    assert_eq!(
        runs(rendered).unwrap(),
        [
            GameRun::Icon { sprite: "GFX_unit_icon|1".into() },
            text("Infantry\n   Soft:", None),
            text("0.17", Some("G")),
            text("\nEnables ", None),
            GameRun::Link { entity: EntityKind::Equipment, token: "equipment_a".into(), text: "Rifle".into(), tone: Some("H".into()) },
            text(" and ", None),
            GameRun::Flag { tag: "GER".into() },
            text("\n", None),
        ]
    );
}

#[test]
fn colour_resets_return_to_the_enclosing_tone() {
    assert_eq!(
        runs("\u{11}Ya \u{11}Gb\u{11}! c\u{11}! d").unwrap(),
        [text("a ", Some("Y")), text("b", Some("G")), text(" c", Some("Y")), text(" d", None)]
    );
}

#[test]
fn control_bytes_plain_text_cannot_show_are_refused_by_name() {
    let why = runs("a \u{14} b").unwrap_err();
    assert!(why.contains("0x14") && why.contains("plain text cannot show"), "{why}");
    assert!(runs("\u{1}eunclosed\u{2}name").unwrap_err().contains("not closed"));
}
