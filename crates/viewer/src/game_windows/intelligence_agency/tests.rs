use super::*;
use content_reader_military::intelligence::AgencyUpgrade;

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

fn level(branch: &str, upgrade: &str, level: i32) -> RecordedLevel {
    RecordedLevel { branch: Some(branch.to_owned()), upgrade: Some(upgrade.to_owned()), level, authenticated: true }
}

#[test]
fn a_pip_takes_the_level_icon_frame_for_its_level_count_and_none_beyond_the_drawn_counts() {
    assert_eq!(pip_frame(1, 0), Some(1));
    assert_eq!(pip_frame(1, 1), Some(2));
    assert_eq!(pip_frame(2, 0), Some(3));
    assert_eq!(pip_frame(2, 2), Some(5));
    assert_eq!(pip_frame(3, 1), Some(7));
    assert_eq!(pip_frame(4, 4), Some(14));
    assert_eq!(pip_frame(4, 9), Some(14), "a level above the count fills the icon");
    assert_eq!(pip_frame(2, -1), Some(3));
    assert_eq!(pip_frame(5, 1), None);
    assert_eq!(pip_frame(0, 0), None);
}

#[test]
fn branches_follow_the_content_order_and_absence_is_zero_only_when_every_acquired_upgrade_is_identified() {
    let content = upgrades(&[("branch_a", &[("one", 1), ("two", 2)]), ("branch_b", &[("three", 4)])]);
    let recorded = [level("BRANCH_A", "Two", 2), level("branch_b", "three", 3)];
    let laid = branches(&content, Ok(&recorded));
    assert_eq!(laid.iter().map(|branch| branch.token.as_str()).collect::<Vec<_>>(), ["branch_a", "branch_b"]);
    assert_eq!(laid[0].pips[0], Pip { upgrade: "one".into(), levels: 1, state: PipState::Level(0) });
    assert_eq!(laid[0].pips[1], Pip { upgrade: "two".into(), levels: 2, state: PipState::Level(2) });
    assert_eq!(laid[1].pips[0].state, PipState::Level(3));

    let unidentified = [level("branch_a", "two", 1), RecordedLevel { branch: None, upgrade: Some("x".into()), level: 1, authenticated: true }];
    let laid = branches(&content, Ok(&unidentified));
    assert_eq!(laid[0].pips[1].state, PipState::Level(1), "a witnessed level is kept");
    assert!(matches!(&laid[0].pips[0].state, PipState::Unknown(why) if why.contains("could not be identified")));

    let refused = [RecordedLevel { authenticated: false, ..level("branch_a", "one", 1) }];
    assert!(matches!(&branches(&content, Ok(&refused))[0].pips[0].state, PipState::Unknown(_)), "a refused identity is not taken as this upgrade");

    let missing = branches(&content, Err("table truncated".into()));
    assert!(missing.iter().flat_map(|branch| &branch.pips).all(|pip| matches!(&pip.state, PipState::Unknown(why) if why.contains("table truncated"))));
}
