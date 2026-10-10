use super::*;
use replay::rows::TransportNavalEdge;
use std::cell::RefCell;

fn graph() -> TransportNavalSearch {
    let links = [
        [1, 2].as_slice(),
        [3].as_slice(),
        [3].as_slice(),
        [].as_slice(),
    ];
    TransportNavalSearch {
        width: Some(100),
        rules: vec![b"SYNTHETIC".to_vec()],
        regions: Some(
            links
                .iter()
                .map(|links| {
                    Some(TransportNavalRegion {
                        position: [0, 0],
                        edges: links
                            .iter()
                            .map(|&to| TransportNavalEdge {
                                to,
                                flags: 2,
                                unrestricted: 0,
                                rules: vec![Some(0)],
                            })
                            .collect(),
                    })
                })
                .collect(),
        ),
    }
}

#[test]
fn strategic_search_keeps_native_heap_ties_and_source_inclusive_path() {
    let paths = RefCell::new(Vec::new());
    let result = search(
        &graph(),
        0,
        3,
        false,
        false,
        &mut |_| panic!("unfiltered search has no access demand"),
        &mut |name| {
            paths.borrow_mut().push(name.to_vec());
            Ok(true)
        },
    )
    .unwrap();
    assert_eq!(result, [0, 2, 3]);
    // Both source edges and both remaining edges are visited before the
    // destination is popped; heap ties still choose the route through 2.
    assert_eq!(paths.into_inner(), vec![b"SYNTHETIC".to_vec(); 4]);
}

#[test]
fn nullable_rules_unrestricted_edges_and_first_record_keep_native_lazy_or() {
    let mut graph = graph();
    {
        let regions = graph.regions.as_mut().unwrap();
        regions[0].as_mut().unwrap().edges[0].rules = vec![None, Some(u32::MAX)];
        regions[0].as_mut().unwrap().edges[1].unrestricted = 17;
        regions[1].as_mut().unwrap().edges[0].flags = 1;
        regions[2].as_mut().unwrap().edges[0].rules = vec![Some(0), None];
    }
    assert!(
        admit(
            &graph,
            0,
            3,
            false,
            |_| panic!("unreached"),
            |_| panic!("unreached"),
            |_| Ok(false)
        )
        .unwrap()
    );
    let regions = graph.regions.as_mut().unwrap();
    let first = regions[0].as_mut().unwrap().edges[0].clone();
    let mut blocked = first.clone();
    blocked.flags = 0;
    regions[0].as_mut().unwrap().edges = vec![blocked, first];
    assert!(
        !admit(
            &graph,
            0,
            3,
            false,
            |_| panic!("unreached"),
            |_| panic!("unreached"),
            |_| panic!("first matching adjacency blocked")
        )
        .unwrap()
    );
}

#[test]
fn restricted_first_path_retries_mode_one_once_then_keeps_mode_two_blocked() {
    let graph = graph();
    let scanned = RefCell::new(Vec::new());
    assert!(
        admit(
            &graph,
            0,
            3,
            true,
            |region| Ok(if region == 2 { 1 } else { 0 }),
            |mode| {
                scanned.borrow_mut().push(mode);
                Ok(mode == 1)
            },
            |_| Ok(true)
        )
        .unwrap()
    );
    assert_eq!(scanned.into_inner(), [1, 2]);
    // Both available paths end in a mode1 destination: first retry is empty,
    // then only the mode1 block is removed. High-bit bytes are not mode1/2.
    assert!(
        admit(
            &graph,
            0,
            3,
            true,
            |region| Ok(if region == 3 { 1 } else { 255 }),
            |mode| Ok(mode == 1),
            |_| Ok(true)
        )
        .unwrap()
    );
    assert!(
        !admit(
            &graph,
            0,
            3,
            true,
            |region| Ok(if region == 3 { 2 } else { 0 }),
            |mode| Ok(mode == 2),
            |_| Ok(true)
        )
        .unwrap()
    );
    assert!(
        admit(
            &graph,
            0,
            3,
            true,
            |_| Err("original access read refused".into()),
            |_| panic!("mode scan follows path byte"),
            |_| Ok(true)
        )
        .is_err()
    );
}

#[test]
fn failed_search_returns_false_without_access_demand_and_signed_centroids_wrap() {
    let mut graph = graph();
    graph.regions.as_mut().unwrap()[0]
        .as_mut()
        .unwrap()
        .edges
        .clear();
    assert!(
        !admit(
            &graph,
            0,
            3,
            true,
            |_| panic!("unreached"),
            |_| panic!("unreached"),
            |_| panic!("unreached")
        )
        .unwrap()
    );
    assert_eq!(heuristic([98, 0], [1, 4], 100), 500_000);
    assert_eq!(
        heuristic([0, 0], [50_000, 0], 100_000),
        i64::from(50_000_i32.wrapping_mul(50_000)) * 100_000
    );
    graph.width = None;
    assert!(
        admit(
            &graph,
            0,
            0,
            false,
            |_| panic!("unreached"),
            |_| panic!("unreached"),
            |_| panic!("unreached")
        )
        .is_err()
    );
}

#[test]
fn source_inclusive_retry_scan_reaches_source_and_avoids_modes_for_unrestricted_path() {
    let graph = graph();
    let mut scanned = Vec::new();
    assert!(
        admit(
            &graph,
            0,
            0,
            true,
            |region| {
                scanned.push(region);
                Ok(0)
            },
            |_| panic!("mode presence is reached only after a restricted path region"),
            |_| panic!("same source and target requires no edges")
        )
        .unwrap()
    );
    assert_eq!(scanned, [0]);
    let calls = RefCell::new(Vec::new());
    assert!(
        admit(
            &graph,
            0,
            3,
            true,
            |region| {
                calls.borrow_mut().push(region);
                Ok(255)
            },
            |_| panic!("only exact native bytes1/2 trigger retry"),
            |_| Ok(true)
        )
        .unwrap()
    );
    assert_eq!(calls.into_inner(), [0, 2, 3]);
}

#[test]
fn reached_second_mode_scan_failure_cannot_be_hidden_by_first_true_mode() {
    let mut calls = Vec::new();
    let result = admit(
        &graph(),
        0,
        3,
        true,
        |_| Ok(1),
        |mode| {
            calls.push(mode);
            if mode == 1 {
                Ok(true)
            } else {
                Err("native second mode scan refused".into())
            }
        },
        |_| Ok(true),
    );
    assert_eq!(calls, [1, 2]);
    assert_eq!(result, Err("native second mode scan refused".into()));
}
