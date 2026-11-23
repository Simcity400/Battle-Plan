use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DelayedEvent {
    pub instance: u32,
    pub definition: ScriptIdentity,
    pub originator: EventFeedCountry,
    pub remaining: i32,
    pub scopes: Vec<EventFeedScope>,
}
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Owner {
    pub country: EventFeedCountry,
    timers: Vec<DelayedEvent>,
    ready: bool,
    fault: bool,
    building: bool,
    cursor: Option<usize>,
    baseline_hour: Option<i32>,
    #[serde(skip)]
    pub(super) version: Version,
}
impl Owner {
    pub fn rows(&self) -> Result<&[DelayedEvent], &'static str> {
        (self.ready && self.cursor.is_none()).then_some(self.timers.as_slice()).ok_or("country delayed-event timers lack a complete baseline, visit prefix or writer coverage")
    }
    pub fn gap(&mut self) {
        let mut version = self.version;
        if !self.timers.is_empty() {
            version.change();
        }
        *self = Self {
            country: self.country,
            version,
            ..Self::default()
        };
    }
    pub fn partial(&mut self, hour: i32) {
        if self.baseline_hour != Some(hour) {
            self.gap();
        }
    }
    fn tracking(&self) -> bool {
        self.ready || self.building
    }
    fn refuse(&mut self) {
        self.ready = false;
        self.fault = true;
    }
    #[cfg(test)]
    pub fn observe(
        &mut self,
        row: &EventFeedHistoryRow,
        event: &EventFeedEvent,
        script: EventFeedScript,
        change: EventFeedScriptChange,
    ) -> io::Result<()> {
        self.apply(row.read_hour, Change::of(row, event), script, change)
    }
    pub fn apply(
        &mut self,
        read_hour: i32,
        occurrence: Change<'_>,
        script: EventFeedScript,
        change: EventFeedScriptChange,
    ) -> io::Result<()> {
        let event = occurrence.event;
        if self.country != event.target {
            self.country = event.target;
            self.version.change();
        }
        use EventFeedScriptChange::*;
        match change {
            DelayedEmpty => {
                *self = Self {
                    country: event.target,
                    ready: true,
                    baseline_hour: Some(read_hour),
                    ..Self::default()
                };
            }
            DelayedBaseline => {
                self.timers.clear();
                self.version.change();
                self.ready = false;
                self.fault = false;
                self.building = true;
                self.cursor = None;
                self.baseline_hour = None;
            }
            DelayedReady => {
                if !self.building {
                    return Err(invalid("delayed-event completion has no country baseline"));
                }
                self.building = false;
                self.ready = !self.fault;
                self.baseline_hour = Some(read_hour);
            }
            DelayedStored { remaining } => {
                let instance = script
                    .instance
                    .ok_or_else(|| invalid("delayed-event compact identity absent"))?;
                if !self.tracking() {
                    return Ok(());
                }
                if script.token_unrecorded || script.scope_unrecorded || script.scope_len == 0 {
                    self.refuse();
                    return Ok(());
                }
                let scopes = occurrence.scopes(script);
                if self.timers.iter().any(|timer| timer.instance == instance) {
                    return Err(invalid("duplicate delayed-event compact identity"));
                }
                allocation::reserve_exact(&mut self.timers, 1)?;
                self.timers.push(DelayedEvent {
                    instance,
                    definition: occurrence.identity(script)?,
                    originator: event.actor,
                    remaining,
                    scopes: allocation::copy_vec(scopes)?,
                });
                self.version.change();
            }
            DelayedAdvance { count, finished } => {
                if !self.ready {
                    return Ok(());
                }
                let first = self.cursor.unwrap_or(0);
                if count == 0 && finished && first >= self.timers.len() {
                    self.cursor = None;
                    return Ok(());
                }
                let Some(end) = first
                    .checked_add(count as usize)
                    .filter(|end| *end <= self.timers.len())
                else {
                    self.refuse();
                    return Ok(());
                };
                for timer in &mut self.timers[first..end] {
                    timer.remaining = timer.remaining.wrapping_sub(1);
                }
                self.cursor = Some(end);
                if finished {
                    if end < self.timers.len() {
                        self.refuse();
                    }
                    self.cursor = None;
                }
            }
            DelayedRemoved { iteration, visited } => {
                let id = script
                    .instance
                    .ok_or_else(|| invalid("delayed-event retirement identity absent"))?;
                if !self.tracking() {
                    return Ok(());
                }
                if let Some(index) = self.timers.iter().position(|timer| timer.instance == id) {
                    if iteration && self.ready {
                        let cursor = self.cursor.get_or_insert(0);
                        if visited {
                            if cursor.checked_sub(1) != Some(index) {
                                self.refuse();
                                return Ok(());
                            }
                            *cursor -= 1;
                        } else if *cursor != index {
                            self.refuse();
                            return Ok(());
                        }
                    }
                    self.timers.swap_remove(index);
                    self.version.change();
                } else if self.ready {
                    self.refuse();
                }
            }
            DelayedRefused => self.gap(),
            _ => return Err(invalid("non-country change reached delayed-event reducer")),
        }
        Ok(())
    }
}
