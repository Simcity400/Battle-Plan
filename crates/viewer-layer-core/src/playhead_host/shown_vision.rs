use replay_data::Hour;
use std::{sync::Arc, time::Instant};
use viewer_foundation::perspective::{Choice, Vision};

#[derive(Default)]
pub struct ShownVision {
    held: Option<(VisionKey, Instant, Result<Arc<Vision>, String>)>,
    making: Option<(VisionKey, Instant, u64)>,
    forgotten: u64,
    maker: Option<(std::sync::mpsc::Sender<crate::vision::SeenCopy>, std::sync::mpsc::Receiver<Arc<Vision>>)>,
}

#[derive(Clone, Debug, PartialEq)]
struct VisionKey {
    applied: Option<Hour>,
    published: crate::layers::Published,
    carried: usize,
    choice: Choice,
}

impl ShownVision {
    pub fn shown(&mut self, layer: Option<&crate::vision::Vision>, applied: Option<Hour>, parts_home: bool, exact: bool) -> Result<Arc<Vision>, String> {
        use crate::layers::Layer as _;
        self.take_made();
        let layer = match (parts_home, layer) {
            (true, Some(layer)) => layer,
            (true, None) => return Err("the map draws no vision layer".into()),
            (false, _) => return self.kept(),
        };
        if layer.restarting() {
            return if exact { Err("the vision of this perspective is still being made".into()) } else { self.kept() };
        }
        let key = VisionKey { applied, published: layer.published(), carried: layer.carried().entities, choice: layer.choice().clone() };
        let now = Instant::now();
        let due = self.held.as_ref().is_none_or(|(held, ..)| *held != key);
        if due && self.making.is_none() {
            match layer.shown_copy() {
                Err(why) => self.held = Some((key.clone(), now, Err(why))),
                Ok(copy) => self.make(key.clone(), now, copy),
            }
        }
        match &self.held {
            Some((held, _, value)) if !exact || *held == key => value.clone(),
            Some((_, _, Err(why))) => Err(why.clone()),
            _ => Err("the vision of this step is still being copied out of the layer".into()),
        }
    }

    fn make(&mut self, key: VisionKey, at: Instant, copy: crate::vision::SeenCopy) {
        if self.maker.is_none() {
            let (jobs, receive) = std::sync::mpsc::channel::<crate::vision::SeenCopy>();
            let (send, made) = std::sync::mpsc::channel();
            let started = std::thread::Builder::new().name("shown vision".into()).spawn(move || {
                viewer_runtime::worker_priority::background_priority();
                let mut before = None;
                for copy in receive {
                    let (seen, made) = copy.make_after(before.as_ref());
                    before = made;
                    if send.send(Arc::new(seen)).is_err() {
                        return;
                    }
                }
            });
            if started.is_ok() {
                self.maker = Some((jobs, made));
            }
        }
        match &self.maker {
            Some((jobs, _)) if jobs.send(copy).is_ok() => self.making = Some((key, at, self.forgotten)),
            _ => self.held = Some((key, at, Err("no thread to make the vision on".into()))),
        }
    }

    fn take_made(&mut self) {
        let Some((_, made)) = &self.maker else { return };
        while let Ok(vision) = made.try_recv() {
            if let Some((key, at, forgotten)) = self.making.take()
                && forgotten == self.forgotten
            {
                self.held = Some((key, at, Ok(vision)));
            }
        }
    }

    pub fn kept(&self) -> Result<Arc<Vision>, String> {
        self.held.as_ref().map_or_else(|| Err("the vision is being made".into()), |(_, _, held)| held.clone())
    }

    pub fn kept_for(&self, choice: &Choice) -> Result<Arc<Vision>, String> {
        match &self.held {
            Some((key, _, held)) if key.choice == *choice => held.clone(),
            Some(_) => Err("the vision of this perspective is still being copied".into()),
            None => Err("the vision is being made".into()),
        }
    }

    pub fn forget(&mut self) {
        self.held = None;
        self.forgotten += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Making {
        shown: ShownVision,
        sent: std::sync::mpsc::Receiver<crate::vision::SeenCopy>,
        made: std::sync::mpsc::Sender<Arc<Vision>>,
    }

    impl Making {
        fn new() -> Self {
            let (jobs, sent) = std::sync::mpsc::channel();
            let (made, receive) = std::sync::mpsc::channel();
            Self { shown: ShownVision { maker: Some((jobs, receive)), ..ShownVision::default() }, sent, made }
        }

        fn ask(&mut self, choice: &Choice, hour: Hour, ours: u32) {
            let key = VisionKey { applied: Some(hour), published: crate::layers::Published::Current, carried: 0, choice: choice.clone() };
            let parts = viewer_foundation::perspective::Parts { ours, ..Default::default() };
            self.shown.make(key, Instant::now(), crate::vision::SeenCopy::of_parts(parts));
        }

        fn arrive(&mut self) {
            let copy = self.sent.try_recv().expect("a copy was sent to be made");
            self.made.send(Arc::new(copy.make())).unwrap();
            self.shown.take_made();
        }
    }

    fn country(tag: &str) -> Choice {
        Choice { country: viewer_foundation::perspective::Country::Tag(tag.into()), fog: true }
    }

    #[test]
    fn a_vision_made_for_one_perspective_is_never_kept_for_another() {
        let (a, b) = (country("GER"), country("ENG"));
        let mut making = Making::new();
        assert!(making.shown.kept_for(&a).is_err(), "nothing made yet");
        making.ask(&a, 5, 1);
        making.arrive();
        assert_eq!(making.shown.kept_for(&a).map(|vision| vision.ours()), Ok(1), "the copy made for A is A's");
        assert!(making.shown.kept_for(&b).is_err(), "A's copy is no vision of B");
        making.ask(&b, 5, 2);
        assert!(making.shown.kept_for(&b).is_err(), "B's copy is still being made: A's is not shown meanwhile");
        making.arrive();
        assert_eq!(making.shown.kept_for(&b).map(|vision| vision.ours()), Ok(2), "the copy made for B, once it stands");
        assert!(making.shown.kept_for(&a).is_err(), "and B's is no vision of A");
    }

    #[test]
    fn a_forgotten_vision_is_not_kept_and_a_copy_asked_before_the_forget_is_let_go() {
        let a = country("GER");
        let mut making = Making::new();
        making.ask(&a, 90, 1);
        making.arrive();
        assert!(making.shown.kept_for(&a).is_ok(), "the copy of hour 90 stands");
        making.shown.forget();
        assert!(making.shown.kept_for(&a).is_err(), "forgotten: the vision of hour 90 is not kept");
        assert!(making.shown.kept().is_err(), "nor kept for any perspective");
        making.ask(&a, 95, 1);
        making.shown.forget();
        making.arrive();
        assert!(making.shown.kept_for(&a).is_err(), "a copy asked before the forget is let go when it arrives");
        assert!(making.shown.making.is_none(), "and nothing is still being made");
        making.ask(&a, 20, 3);
        making.arrive();
        assert_eq!(making.shown.kept_for(&a).map(|vision| vision.ours()), Ok(3), "a copy asked after the forget is kept");
    }
}
