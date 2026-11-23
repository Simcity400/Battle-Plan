use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use viewer_runtime::retirement::Dropper;
use viewer_borders::{borders::{BorderClass, BorderMesh, BorderMeshes}, demilitarized::Zone};
use map_display_data::landscape::Regions;

fn key() -> Key {
    Key {
        branch: 1,
        hour: 2,
        perspective: Default::default(),
        borders: 3,
        regions: vec![7].into(),
        adjacency: false,
        selection: None,
    }
}

fn plan() -> FactionHighlight {
    FactionHighlight::new(
        &BorderMeshes {
            meshes: BorderClass::ALL
                .iter()
                .map(|&class| BorderMesh {
                    class,
                    chunks: vec![],
                    coarse_chunks: vec![],
                    draws: vec![],
                })
                .collect(),
            hidden: BorderMesh {
                class: BorderClass::Province,
                chunks: vec![],
                coarse_chunks: vec![],
                draws: vec![],
            },
            indices: vec![],
            one_pixel_provinces: vec![],
        },
        &Regions {
            country_of: vec![].into(),
            owner_of: vec![].into(),
            state_of: vec![].into(),
            country_colors: Default::default(),
            warnings: vec![],
            strategic_attributes: Default::default(),
            mode: None,
        },
        &Zone::default(),
        &Default::default(),
    )
}

#[test]
fn hover_preparation_is_deferred_and_queue_admission_retries_without_frame_work() {
    let dropper = Dropper::new().unwrap();
    let mut lane = Lane::default();
    assert!(
        lane.update(
            key(),
            |_| false,
            || panic!("frame cannot build borders"),
            &dropper
        )
        .is_none()
    );
    let mut queued = None;
    let built = Arc::new(AtomicUsize::new(0));
    let worker_built = Arc::clone(&built);
    let frame_thread = std::thread::current().id();
    assert!(
        lane.update(
            key(),
            |job| {
                queued = Some(job);
                true
            },
            move || {
                assert_ne!(std::thread::current().id(), frame_thread);
                worker_built.fetch_add(1, Ordering::Relaxed);
                plan()
            },
            &dropper
        )
        .is_none()
    );
    assert_eq!(built.load(Ordering::Relaxed), 0);
    assert!(
        lane.update(
            key(),
            |_| panic!("duplicate job"),
            || panic!("duplicate build"),
            &dropper
        )
        .is_none()
    );
    std::thread::spawn(queued.take().unwrap()).join().unwrap();
    assert_eq!(built.load(Ordering::Relaxed), 1);
    assert!(
        lane.update(
            key(),
            |_| panic!("completed job"),
            || panic!("completed build"),
            &dropper
        )
        .is_some()
    );
    lane.installed();
    assert!(
        lane.update(
            key(),
            |_| panic!("installed job"),
            || panic!("installed build"),
            &dropper
        )
        .is_none()
    );
}

#[test]
fn a_failed_upload_does_not_mark_the_hover_installed() {
    let dropper = Dropper::new().unwrap();
    let mut lane = Lane::default();
    let mut job = None;
    assert!(
        lane.update(
            key(),
            |work| {
                job = Some(work);
                true
            },
            plan,
            &dropper
        )
        .is_none()
    );
    job.take().unwrap()();
    assert!(
        lane.update(key(), |_| panic!("completed job"), plan, &dropper)
            .is_some()
    );
    assert!(
        lane.update(
            key(),
            |work| {
                job = Some(work);
                true
            },
            plan,
            &dropper
        )
        .is_none()
    );
    assert!(job.is_some());
}

#[test]
fn hover_results_are_rejected_after_hour_perspective_border_or_selection_changes() {
    let dropper = Dropper::new().unwrap();
    for changed in [
        Key { branch: 9, ..key() },
        Key { hour: 9, ..key() },
        Key {
            perspective: perspective::Choice {
                country: perspective::Country::Tag("OTHER".into()),
                fog: true,
            },
            ..key()
        },
        Key {
            borders: 9,
            ..key()
        },
        Key {
            regions: vec![9].into(),
            ..key()
        },
        Key {
            regions: Arc::default(),
            ..key()
        },
    ] {
        let mut lane = Lane::default();
        let mut job = None;
        assert!(
            lane.update(
                key(),
                |work| {
                    job = Some(work);
                    true
                },
                plan,
                &dropper
            )
            .is_none()
        );
        assert!(
            lane.update(
                changed.clone(),
                |_| panic!("only one in-flight job"),
                plan,
                &dropper
            )
            .is_none()
        );
        job.take().unwrap()();
        let mut next = None;
        assert!(
            lane.update(
                changed.clone(),
                |work| {
                    next = Some(work);
                    true
                },
                plan,
                &dropper
            )
            .is_none()
        );
        if changed.regions.is_empty() {
            assert!(
                next.is_none(),
                "clearing must restore the prepared ordinary plans"
            );
        } else {
            next.unwrap()();
            assert!(
                lane.update(changed, |_| panic!("already prepared"), plan, &dropper)
                    .is_some()
            );
        }
    }
}

#[test]
fn a_ground_selection_alone_builds_a_plan_and_nothing_builds_none() {
    let dropper = Dropper::new().unwrap();
    let outline = crate::app::ground_click::Outline { provinces: vec![4, 5].into(), state: true };
    let selected = Key { regions: Arc::default(), selection: Some(outline), ..key() };
    let mut lane = Lane::default();
    let mut submitted = false;
    lane.update(selected, |_| { submitted = true; true }, plan, &dropper);
    assert!(submitted);
    let mut lane = Lane::default();
    let nothing = Key { regions: Arc::default(), ..key() };
    lane.update(nothing, |_| panic!("nothing to outline"), plan, &dropper);
}

#[test]
fn a_ground_selection_alone_wants_the_picture_its_outline_is_built_over() {
    let outline = crate::app::ground_click::Outline { provinces: vec![4, 5].into(), state: true };
    assert!(wanted(&[], Some(&outline), false), "a selected state with nothing hovered");
    assert!(wanted(&[7], None, false));
    assert!(wanted(&[], None, true), "a highlight on screen is taken off over it");
    assert!(!wanted(&[], None, false));
}
