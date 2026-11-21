use super::{PartyNaming, party_key, party_key_with_selected_tag};

#[test]
fn a_party_is_named_as_the_game_names_it() {
    let loc = crate::catalogue::tests::loaded(concat!(
        "l_english:\n",
        " GER_fascism_party:0 \"Copper Circle\"\n",
        " GER_fascism_party_long:0 \"Copper Circle Assembly\"\n",
        " SOV_x_party:0 \"Quartz League\"\n",
        " fascism:0 \"Fixture ideology\"\n",
        " scoped_party:0 \"[ROOT.GetAdjective] Party\"\n",
        " gap_party:0 \"Bloc¤Ribbon\"\n",
    ));
    let refused = std::cell::RefCell::new(crate::catalogue::Refusals::default());
    let name = |naming: &PartyNaming| {
        let key = party_key(naming, |key| loc.get(key).is_some())?;
        crate::catalogue::localised(&loc, &key, crate::catalogue::View::Plain)
            .inspect_err(|reason| refused.borrow_mut().note(reason.clone(), &key))
            .ok()
    };
    let naming =
        |token: &str, tag: Option<&str>, original: Option<&str>, key: Option<&str>| PartyNaming {
            token: token.to_owned(),
            tag: tag.map(str::to_owned),
            original_tag: original.map(str::to_owned),
            key: key.map(str::to_owned),
        };
    assert_eq!(
        name(&naming("fascism", Some("GER"), None, None)).as_deref(),
        Some("Copper Circle")
    );
    assert_eq!(
        name(&naming("fascism", Some("D01"), Some("GER"), None)).as_deref(),
        Some("Copper Circle")
    );
    assert_eq!(
        name(&naming("fascism", Some("ITA"), Some(""), None)).as_deref(),
        Some("Fixture ideology")
    );
    assert_eq!(
        name(&naming("modism", Some("ITA"), Some(""), None)).as_deref(),
        Some("modism")
    );
    assert_eq!(name(&naming("fascism", Some("D01"), None, None)), None);
    assert_eq!(name(&naming("fascism", None, Some(""), None)), None);
    assert_eq!(
        name(&naming("x", Some("GER"), None, Some("SOV_x_party"))).as_deref(),
        Some("Quartz League")
    );
    assert_eq!(
        name(&naming("x", Some("GER"), None, Some("MOD_key"))).as_deref(),
        Some("MOD_key")
    );
    assert_eq!(
        name(&naming("x", Some("GER"), None, Some("§YCustom Party§!"))).as_deref(),
        Some("§YCustom Party§!")
    );
    assert!(refused.borrow().lines("x").is_empty());
    assert_eq!(
        name(&naming("x", Some("GER"), None, Some("scoped_party"))).as_deref(),
        Some("[ROOT.GetAdjective] Party")
    );
    assert_eq!(
        name(&naming("x", Some("GER"), None, Some("gap_party"))),
        None
    );
    assert_eq!(
        refused.borrow().lines("party names are"),
        [
            "1 party names are not shown: localisation gap_party holds the control byte 0x14 (an authored `¤`), which plain text cannot show; gap_party",
        ]
    );
}

#[test]
fn exact_selected_empty_party_tag_keeps_the_legacy_absence_contract() {
    let naming = PartyNaming {
        token: "mod_group".into(),
        tag: Some("MOD_own".into()),
        original_tag: Some(String::new()),
        key: None,
    };
    let defines = |key: &str| key == "_mod_group_party";
    assert_eq!(party_key(&naming, defines).as_deref(), Some("mod_group"));
    assert_eq!(
        party_key_with_selected_tag(&naming, "", defines).as_deref(),
        Some("_mod_group_party")
    );
    assert_eq!(
        party_key_with_selected_tag(&naming, "MOD_original", defines).as_deref(),
        Some("mod_group")
    );
}
