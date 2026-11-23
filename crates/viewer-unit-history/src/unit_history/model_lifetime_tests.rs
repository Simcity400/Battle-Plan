#[test]
fn unit_lifetime_and_hidden_animation_clock() {
    use crate::unit_history::{History, Kind, test_support::{GATES, target}};
    use viewer_models::test_support::{extend_lifetime_palette, lifetime_fixture, lifetime_observation};
    use viewer_models::entity_animation;
    use std::sync::Arc;

    let (mut library, mut pass) = lifetime_fixture();
    let mut target = target(40, Kind::Land);
    target.instance.animation = entity_animation::MOVE;
    let mut history = History::default();
    history.set(vec![target], Arc::default(), false);
    {
        let mut frame = |time, shown, in_view| {
            if time == 10.75 {
                extend_lifetime_palette(&mut library, &mut pass);
            }
            let placed = history.step(time, GATES, [shown; 2], true, |_| in_view);
            pass.update_placed(placed, 128.0);
            pass.animate(time, None, &[0.0]);
            lifetime_observation(&pass)
        };
        let (first, events) = frame(10.0, true, true);
        assert!(
            events.is_empty(),
            "creation clock starts now, not at global time zero"
        );
        let epoch = first[0].animation_epoch;
        let (_, events) = frame(10.5, true, true);
        assert!((events[0] - 0.3).abs() < 1e-5);
        let (hidden, events) = frame(10.75, true, false);
        assert_eq!(hidden[0].animation_epoch, epoch);
        assert!((events[0] - 0.55).abs() < 1e-5, "hidden entity advances");
        let (back, events) = frame(11.0, true, true);
        assert_eq!(back[0].animation_epoch, epoch);
        assert!((events[0] - 0.8).abs() < 1e-5);
        for _ in 0..100 {
            frame(11.0, true, false);
        }
        let (fresh, events) = frame(12.0, true, true);
        assert_ne!(fresh[0].animation_epoch, epoch);
        assert!(events.is_empty());
        frame(12.5, true, true);
        let (faded, _) = frame(12.75, false, true);
        assert_eq!(faded[0].scale, 0.5);
    }
}
