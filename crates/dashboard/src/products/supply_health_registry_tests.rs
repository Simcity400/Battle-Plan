//! Supply health as the registry and the TypeScript contract publish it.

use super::{LoadContext, Product, find};
use crate::products::supply_health::{SupplyHealth, SupplyHealthParams};
use crate::test_fixtures::{CAMPAIGN, records, source};
use crate::{Source, SourceKey};
use replay::container::Replay;
use std::{
    io,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

#[test]
fn supply_health_registry_no_country_cap_and_old_recording_unavailable_without_content() {
    assert!(find("supply-health").unwrap().content_dependent);
    let params = SupplyHealthParams {
        branch_id: "1".into(),
        tags: (0..5_000).map(|i| format!("opaque {i}")).collect(),
    };
    assert!(SupplyHealth::validate(&params).is_ok());
    for name in [
        "SupplyHealthParams",
        "SupplyHealthRun",
        "SupplyHealthSeries",
        "SupplyHealthReply",
    ] {
        assert!(crate::typescript_contract().contains(&format!("export type {name} =")));
    }
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![source(1, 100, None)])]).unwrap();
    let src = Source::from_replay(
        SourceKey {
            source_id: "fixture".into(),
            revision: "1".into(),
        },
        PathBuf::from("synthetic.battleplan"),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let got = SupplyHealth::load(
        &src,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!got.available);
    assert_eq!(got.missing_inputs, ["DivisionLandInputs"]);
    assert!(got.series.iter().all(|s| s.runs.is_empty()));
    assert_eq!(
        SupplyHealth::load(
            &src,
            &params,
            &LoadContext { content: None },
            &AtomicBool::new(true)
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
}
