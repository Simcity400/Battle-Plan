use collector_air::extended::air::carrier::tests::*;
use collector_air::sizing;
use engine_native::cair_base::CarrierDemand;
use replay_data::rows;

#[test]
fn two_queued_carrier_slots_recover_after_real_retirement_at_the_same_paused_hour() {
    let _hint = sizing::hint_guard();
    use collector_air::extended::air::air_bases_test_kit as kit;
    const HOUR: i32 = 201;
    let bytes = coherent_carrier_fixture();
    let mut jobs = [
        collector_air::extended::air::AirBasesJob::fixture(),
        collector_air::extended::air::AirBasesJob::fixture(),
    ];
    let mut frames = [coherent_carrier_parts(), coherent_carrier_parts()];
    let mut memo = collector_air::extended::air::AirBasesMemo::default();

    for job in &mut jobs {
        assert_eq!(job.carrier.bases.capacity()[1..], [0, 0]);
        assert_eq!(job.carrier.registrations.capacity()[1], 0);
        assert_eq!(job.carrier.scratch.capacity(), [0; 5]);
        coherent_carrier_refresh(job, &bytes, HOUR, false);
        assert_eq!(
            (
                job.carrier.demand.countries,
                job.carrier.demand.name_bytes,
                job.carrier.demand.pools
            ),
            (3, COHERENT_NAME_BYTES.len(), 1)
        );
        assert!(
            job.carrier
                .log
                .borrow()
                .spans
                .iter()
                .any(|span| (COHERENT_DENSE..COHERENT_DENSE + 3 * 8).contains(&span.addr))
        );
        assert!(!job.grow_for_next());
    }
    for (index, ((air, fog), job)) in frames.iter_mut().zip(&jobs).enumerate() {
        assert_eq!(air.carrier.capacity()[1..], [0, 0]);
        assert_eq!(fog.carrier.capacity()[1], 0);
        assert_eq!(fog.carrier_scratch.capacity(), [0; 5]);
        let report = memo.decode(
            air,
            fog,
            job,
            collector_air::extended::air::AirBasesMemoContext {
                owner: 17,
                base: kit::BASE,
                seq: index as u64 + 1,
                hour: HOUR,
                force_full: false,
            },
            &mut [0; 3],
        );
        assert!(!report.hit);
        let rows::Recorded::Value(Some(_)) = air.carrier.packet(COHERENT_HOST_HANDLE) else {
            panic!("pressure base row missing")
        };
        assert!(air.carrier.capacity()[1] >= 3);
        assert!(air.carrier.capacity()[2] >= COHERENT_NAME_BYTES.len());
        assert!(fog.carrier.capacity()[1] >= 1);
    }

    jobs = crate::decode::retire_and_reacquire_two_carrier_jobs_for_test(jobs);
    for job in &jobs {
        assert!(job.log.spans.is_empty());
        assert!(job.carrier.log.borrow().spans.is_empty());
        assert_eq!(job.carrier.demand, CarrierDemand::default());
        assert!(job.carrier.seed.is_err());
    }
    for job in &mut jobs {
        coherent_carrier_refresh(job, &bytes, HOUR, true);
        assert!(!job.grow_for_next());
    }
    for (index, ((air, fog), job)) in frames.iter_mut().zip(&jobs).enumerate() {
        air.bases_raw_mode = true;
        fog.registrations_raw_mode = true;
        let demand = job.carrier.demand;
        let [_, countries, names] = air.carrier.capacity();
        assert!(countries >= demand.countries && names >= demand.name_bytes);
        assert!(fog.carrier.capacity()[1] >= demand.pools);
        let [indexed, resolved, country_terms, aces, forecasts] = fog.carrier_scratch.capacity();
        assert!(
            indexed >= demand.indexed
                && resolved >= demand.resolved
                && country_terms >= demand.country_terms
                && aces >= demand.aces
                && forecasts >= demand.forecasts
        );
        let report = memo.decode(
            air,
            fog,
            job,
            collector_air::extended::air::AirBasesMemoContext {
                owner: 17,
                base: kit::BASE,
                seq: index as u64 + 3,
                hour: HOUR,
                force_full: true,
            },
            &mut [0; 3],
        );
        assert!(report.eligible && report.installed && !report.hit && !report.compared);
        coherent_assert_complete_carrier_packets(air, fog);
    }
}
