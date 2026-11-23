use collector_division_templates::extended::division_templates::{
    Captured,
    tests::{Fixture, capacity, fixture, large_content, set_name, template_outcomes},
};
use engine_native::creferenced_division_template as native;
use engine_native::ccountry;
use engine_native_memory::{memory::SliceMemory, walk::Outcome};
use replay_data::rows::Recorded;
use replay_frame::frame::TableView;
use std::sync::Arc;

#[test]
fn hourly_template_job_publishes_direct_walk_rows() {
    use crate::acquisition_countries::TemplatesRequest;
    use engine_native_intelligence::country_memory::Cache;

    let mut source = fixture();
    let mut capture = Captured::vacant();
    for (hour, literal, custom_icon) in [(7, &b"Alpha"[..], 11_i32), (8, &b"Bravo"[..], 22)] {
        set_name(&mut source, literal, b"fallback");
        source.bytes[0x5000 + native::CUSTOM_ICON..][..4]
            .copy_from_slice(&custom_icon.to_le_bytes());
        let mut countries =
            engine_allocation::shared(Cache::try_with_capacity(256).unwrap()).unwrap();
        let countries = Cache::prepare_shared(&mut countries, &source, 0, 0x1000).unwrap();
        let mut request = TemplatesRequest::take(&countries, 0, 0x1000, &mut capture, hour, false);
        request.run(&source);
        request.restore(&mut capture, None);
        // SAFETY: Isolated template test owns its synthetic fixture.
        unsafe {
            capture.capture(
                &SliceMemory {
                    base: 0,
                    bytes: &[],
                },
                0,
                0x1000,
                hour,
                false,
            );
        }
        assert_eq!(template_outcomes(&capture), [Outcome::Rows { len: 1 }; 2]);
        let [
            TableView::Complete(replay_data::rows::TableRows::DivisionTemplates(dictionary)),
            TableView::Complete(replay_data::rows::TableRows::DivisionTemplateIcons(icons)),
        ] = capture.tables()
        else {
            panic!("complete template job tables at {hour}");
        };
        assert_eq!(dictionary.len(), 1);
        assert_eq!(dictionary[0].name, literal);
        assert_eq!(
            dictionary[0].name_key,
            Recorded::Value(b"fallback".to_vec())
        );
        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].template, dictionary[0].template);
        assert_eq!(icons[0].custom_icon, custom_icon);
        assert_eq!(icons[0].priority_unit_name, b"infantry");
        assert_eq!(
            icons[0]
                .display_name_inputs(hour, Some(&dictionary[0]))
                .unwrap()
                .literal,
            literal
        );
    }
}

#[test]
fn promoted_template_worker_matches_direct_rows_and_faults() {
    use crate::acquisition_jobs::{Batch, Job, Pool, Report, TemplatesRequest};
    use collector_workers::roles::Role;
    use engine_native_intelligence::country_memory::Cache;

    fn word(source: &Fixture, at: usize) -> usize {
        u64::from_le_bytes(source.bytes[at..at + 8].try_into().unwrap()) as usize
    }

    assert_eq!(Role::Templates.priority(), Role::Countries.priority());
    assert_eq!(Role::Templates.priority(), Role::IntelResidual.priority());
    for countries in [1, 40] {
        for fault in 0..3 {
            let mut source = large_content(countries, 1);
            let owner = word(&source, 0x10000 + countries * 8);
            let roster = word(&source, owner + ccountry::DIVISION_TEMPLATES);
            let template = word(&source, roster);
            match fault {
                1 => source.bytes[template + native::SUBUNIT_COUNTS..][..8]
                    .copy_from_slice(&u64::MAX.to_le_bytes()),
                2 => source.bytes[owner + ccountry::DIVISION_TEMPLATES..][..8]
                    .copy_from_slice(&u64::MAX.to_le_bytes()),
                _ => {}
            }
            let source = Arc::new(source);
            let mut direct = Captured::with_capacities(&capacity());
            let mut captured = Captured::with_capacities(&capacity());
            let pool = {
                let source = Arc::clone(&source);
                Pool::with_test_runner(1, move |job| match job {
                    Job::Templates(request) => Report::Templates(request.run(source.as_ref())),
                    _ => panic!("only Templates was submitted"),
                })
            };
            for hour in [7, 8] {
                // SAFETY: Isolated template test owns its synthetic fixture.
                unsafe { direct.capture(source.as_ref(), 0, 0x1000, hour, false) };
                let mut cache =
                    engine_allocation::shared(Cache::try_with_capacity(countries + 1).unwrap())
                        .unwrap();
                let cache = Cache::prepare_shared(&mut cache, source.as_ref(), 0, 0x1000).unwrap();
                let mut request = Some(Job::Templates(TemplatesRequest::take(
                    &cache,
                    0,
                    0x1000,
                    &mut captured,
                    hour,
                    false,
                )));
                let mut batch = Batch::new(&pool);
                let ticket = unsafe { batch.submit(&mut request) }.unwrap();
                let completed = batch.finish(ticket).unwrap();
                assert!(matches!(completed.report, Ok(Report::Templates(_))));
                let Job::Templates(request) = completed.request else {
                    panic!("template request must be returned");
                };
                request.restore(&mut captured, None);
                // SAFETY: Isolated template test owns its synthetic fixture.
                unsafe {
                    captured.capture(
                        &SliceMemory {
                            base: 0,
                            bytes: &[],
                        },
                        0,
                        0x1000,
                        hour,
                        false,
                    );
                }
                assert_eq!(
                    captured.outcomes(),
                    direct.outcomes(),
                    "{countries}/{fault}/{hour}"
                );
                assert_eq!(
                    captured.tables(),
                    direct.tables(),
                    "{countries}/{fault}/{hour}"
                );
                assert_eq!(captured.country_state(), direct.country_state());
                assert_eq!(captured.model_failures(), direct.model_failures());
                if fault == 2 {
                    assert!(matches!(
                        captured.outcomes()[0],
                        Outcome::Unavailable { .. }
                    ));
                } else {
                    assert_eq!(
                        template_outcomes(&captured),
                        [Outcome::Rows { len: countries }; 2]
                    );
                }
            }
        }
    }
}
