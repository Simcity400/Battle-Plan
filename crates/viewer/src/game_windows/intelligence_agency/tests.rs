use super::*;
use content_reader_military::intelligence::AgencyUpgrade;
use dashboard_intelligence::products::intelligence::agencies::UpgradeItem;

fn upgrades(branches: &[(&str, &[(&str, usize)])]) -> AgencyUpgrades {
    let mut upgrades = AgencyUpgrades::default();
    for (branch, tokens) in branches {
        upgrades.branches.push((branch.to_string(), tokens.iter().map(|(token, _)| token.to_string()).collect()));
        for (token, levels) in *tokens {
            upgrades.branch_definitions.insert(
                (branch.to_string(), token.to_string()),
                AgencyUpgrade { token: token.to_string(), name: token.to_string(), branch: branch.to_string(), sprite: String::new(), levels: *levels },
            );
        }
    }
    upgrades
}

fn item(ordinal: u32, branch: Option<&str>, token: Option<&str>, level: i32, named: bool) -> UpgradeItem {
    UpgradeItem {
        ordinal,
        branch: branch.map(str::to_owned),
        token: token.map(str::to_owned),
        level,
        name: named.then(|| token.unwrap_or_default().to_owned()),
        missing: (!named).then(|| "Acquired upgrade identity refused: unknown".to_owned()),
    }
}

fn held(items: Vec<UpgradeItem>, unavailable: Option<&str>) -> Upgrades {
    Upgrades { unavailable: unavailable.map(str::to_owned), nonnegative_level_sum: None, items }
}

fn laid(content: &AgencyUpgrades, recorded: Result<&Upgrades, &str>) -> Vec<Branch> {
    branches(content, recorded, |branch| Ok(format!("title {branch}")), |_, token| Ok(format!("name {token}")))
}

#[test]
fn a_pip_takes_the_frame_run_for_its_level_count_from_the_installed_marker_and_none_past_its_frames() {
    assert_eq!(pip_frame(14, 1, 0), Some(1));
    assert_eq!(pip_frame(14, 1, 1), Some(2));
    assert_eq!(pip_frame(14, 2, 0), Some(3));
    assert_eq!(pip_frame(14, 2, 2), Some(5));
    assert_eq!(pip_frame(14, 3, 1), Some(7));
    assert_eq!(pip_frame(14, 4, 4), Some(14));
    assert_eq!(pip_frame(14, 4, 9), Some(14), "a level above the count fills the marker");
    assert_eq!(pip_frame(14, 2, -1), Some(3));
    assert_eq!(pip_frame(14, 5, 1), None, "the installed marker has no run for five levels");
    assert_eq!(pip_frame(20, 5, 1), Some(16), "a mod's longer marker draws five levels");
    assert_eq!(pip_frame(14, 0, 0), None);
}

#[test]
fn branches_follow_the_content_order_and_absence_is_zero_only_when_every_acquired_upgrade_is_identified() {
    let content = upgrades(&[("branch_a", &[("one", 1), ("two", 2)]), ("branch_b", &[("three", 4)])]);
    let recorded = held(vec![item(0, Some("BRANCH_A"), Some("Two"), 2, true), item(1, Some("branch_b"), Some("three"), 3, true)], None);
    let shown = laid(&content, Ok(&recorded));
    assert_eq!(shown.iter().map(|branch| (branch.token.as_str(), branch.name.clone())).collect::<Vec<_>>(), [
        ("branch_a", Ok("title branch_a".to_owned())),
        ("branch_b", Ok("title branch_b".to_owned())),
    ]);
    assert_eq!(shown[0].pips[0], Pip { token: "one".into(), name: Ok("name one".into()), levels: 1, level: Ok(0) });
    assert_eq!(shown[0].pips[1].level, Ok(2));
    assert_eq!(shown[1].pips[0].level, Ok(3));

    let unidentified = held(vec![item(0, Some("branch_a"), Some("two"), 1, true), item(1, None, Some("x"), 1, false)], None);
    let shown = laid(&content, Ok(&unidentified));
    assert_eq!(shown[0].pips[1].level, Ok(1), "a witnessed level is kept");
    assert!(matches!(&shown[0].pips[0].level, Err(why) if why.contains("could not be identified")));

    let refused = held(vec![item(0, Some("branch_a"), Some("one"), 1, false)], Some("Acquired agency levels contain unauthenticated identities"));
    let shown = laid(&content, Ok(&refused));
    assert!(matches!(&shown[0].pips[0].level, Err(why) if why.contains("unauthenticated")), "a refused identity is not taken as this upgrade");
    assert!(shown.iter().flat_map(|branch| &branch.pips).all(|pip| pip.level.is_err()), "nor is any absence a zero then");

    let missing = laid(&content, Err("table truncated"));
    assert!(missing.iter().flat_map(|branch| &branch.pips).all(|pip| matches!(&pip.level, Err(why) if why.contains("table truncated"))));
}
