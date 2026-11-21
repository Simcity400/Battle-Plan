use super::{PanelData, dated};
use std::sync::{Arc, Weak};
use std::collections::BTreeSet;
use replay_data::Hour;

pub fn take_due(queue: &mut BTreeSet<(Hour, u32)>, hour: Hour) -> Option<u32> {
    if queue.first()?.0 > hour { return None; }
    queue.pop_first().map(|(_, id)| id)
}

#[derive(Default)]
pub struct Delivery {
    value: Weak<PanelData>,
    last: Option<(u8, Arc<[u8]>)>,
    pending: Option<(Weak<PanelData>, Option<(u8, Arc<[u8]>)>)>,
}

pub struct Record {
    pub bytes: Vec<u8>,
    pub changed: bool,
    pub refused: bool,
}

impl Delivery {
    pub fn body(&self) -> Option<&[u8]> {
        self.last.as_ref().map(|(_, body)| body.as_ref())
    }

    pub fn update(&mut self, seed: bool, value: &Arc<PanelData>, encoded: Option<Arc<[u8]>>) -> Option<(u8, &[u8])> {
        let current = Arc::downgrade(value);
        let refused = value.is_err();
        let mut changed = false;
        let mut last = self.last.clone();
        if seed || !Weak::ptr_eq(&self.value, &current) {
            let kind = if refused { dated::REFUSED } else { dated::DATA };
            let body = encoded.unwrap_or_else(|| match &**value {
                Ok(data) => rmp_serde::to_vec(data).expect("a panel value").into(),
                Err(reason) => Arc::from(reason.as_bytes()),
            });
            changed = seed || last.as_ref().is_none_or(|(k, b)| *k != kind || **b != *body);
            if changed { last = Some((kind, body)); }
        }
        let (_, last) = self.pending.insert((current, last));
        changed.then(|| {
            let (kind, body) = last.as_ref().expect("a complete record");
            (*kind, body.as_ref())
        })
    }

    pub fn commit(&mut self) {
        if let Some((value, last)) = self.pending.take() {
            self.value = value;
            self.last = last;
        }
    }

    pub fn packet(&mut self, (clock, route, seek): (i32, u32, u32), id: u32, hour: i32, seed: bool, value: &Arc<PanelData>, encoded: Option<Arc<[u8]>>) -> Record {
        let change = self.update(seed, value, encoded);
        let (kind, body) = change.unwrap_or((dated::STANDS, &[]));
        let record = Record { bytes: dated::encoded(clock, route, seek, id, hour, kind, body), changed: change.is_some(), refused: value.is_err() };
        self.commit();
        record
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_whose_step_was_dropped_is_sent_again() {
        let value = |n: i64| Arc::new(Ok(serde_json::json!({ "states": n })));
        let mut wire = Delivery::default();
        assert!(wire.update(false, &value(1), None).is_some());
        wire.commit();
        assert!(wire.update(false, &value(2), None).is_some(), "a change");
        assert!(wire.update(false, &value(2), None).is_some(), "its step was dropped: the page never had it");
        wire.commit();
        assert!(wire.update(false, &value(2), None).is_none(), "delivered once");
    }
}
