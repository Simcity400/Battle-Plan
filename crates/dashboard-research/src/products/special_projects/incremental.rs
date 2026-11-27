use super::*;
use dashboard_follow::panel::ProductReader;
use dashboard_query::{identity::Holders, timeline::Timeline};
use replay::campaign::Replay;

pub(super) struct Follow {
    pub facts: Facts,
    pub holding: Holding,
    through: Option<i32>,
    links: HashMap<u32, RunLinks>,
}

impl Follow {
    pub(super) fn through(&self) -> Option<i32> {
        self.through
    }

    pub fn new(first: i32) -> Self {
        Self { facts: Facts::new(first), holding: Holding::default(), through: None, links: HashMap::new() }
    }

    pub fn to(&mut self, query: &Query<'_>, country: &Country, hour: i32, cancelled: &AtomicBool) -> io::Result<()> {
        self.cover(query, country, hour, hour, cancelled)
    }

    fn cover(&mut self, query: &Query<'_>, country: &Country, hour: i32, limit: i32, cancelled: &AtomicBool) -> io::Result<()> {
        if self.through.is_some_and(|through| through > hour) {
            *self = Self::new(query.timeline.first);
        }
        if self.through == Some(hour) {
            return Ok(());
        }
        let from = self.through.map_or(i32::MIN, |through| through + 1);
        let (pieces, mut holding) = pieces(query, country, query.timeline.first, limit);
        let mut through = limit;
        for piece in pieces.into_iter().filter(|piece| piece.last >= from && piece.first <= hour) {
            let mut piece = Piece { first: piece.first.max(from), ..piece };
            let links = links_of(query, &mut self.links, piece.run)?;
            let read = match read_piece(query, links, &piece) {
                Err(_) if piece.last > hour => {
                    piece.last = hour;
                    read_piece(query, links, &piece)?
                }
                read => read?,
            };
            let last = see_piece(&mut self.facts, &piece, &read, hour, cancelled)?;
            if piece.last >= hour {
                through = last;
            }
        }
        if let Some(span) = holding.identities.last_mut() { span.last_hour = span.last_hour.min(through); }
        if let Some(span) = holding.unresolved.last_mut() { span.1 = span.1.min(through); }
        self.holding = holding;
        self.through = Some(through);
        Ok(())
    }
}

struct Followed {
    follow: Follow,
    first: i32,
    spelling: Option<Spelling>,
    data: Arc<dashboard_follow::panel::PanelData>,
}

pub(super) fn reader(replay: Arc<Replay>, timeline: Arc<Timeline>, holders: Arc<Holders>, params: Arc<SpecialProjectsParams>, content: Arc<ProjectContent>, executable: Option<engine_executable::executable_tables::Tables>) -> ProductReader {
    let state = Mutex::new(None::<Followed>);
    Arc::new(move |hour, reset, cancelled, _step| {
        let query = Query { replay: &replay, holders: holders.clone(), timeline: timeline.clone() };
        let mut held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (answer, rows) = at_hour::counted(|| engine_executable::executable_tables::scope(executable, || -> io::Result<_> {
            let stretch = timeline.at(hour).ok_or_else(|| dashboard_core::refused("Special projects hour is not recorded"))?;
            if reset || held.is_none() {
                *held = Some(Followed { follow: Follow::new(timeline.first), first: hour, spelling: None, data: Arc::new(Err(String::new())) });
            }
            let s = held.as_mut().expect("a followed state");
            let fresh = !s.follow.through.is_some_and(|last| (s.first..=last).contains(&hour));
            if fresh {
                let run = replay.recording().run(stretch.run)?;
                let ordinal = run.ordinal(hour).ok_or_else(|| dashboard_core::refused("Special projects hour is not recorded"))?;
                let size = replay_columnar::unit::BLOCK;
                let block_end = run.block_first_hour(ordinal / size).saturating_add(size as i32 - 1);
                let limit = block_end.min(stretch.last).min(holders.interval(hour).1);
                s.follow.cover(&query, &params.country, hour, limit, cancelled)?;
                let tag = query.holders.name(&params.country, hour);
                #[cfg(any(test, feature = "test-support"))]
                dashboard_base::test_fixtures::product_called();
                let reply = present(&query, &params, &content, &s.follow, &tag, cancelled, &mut s.spelling)?;
                s.data = Arc::new(Ok(serde_json::to_value(reply)?));
                s.first = hour;
            }
            Ok((s.data.clone(), fresh, s.first))
        }));
        let (value, fresh, from) = answer.unwrap_or_else(|e| {
            *held = None;
            (Arc::new(Err(e.to_string())), true, hour)
        });
        dashboard_follow::panel::ProductRead { value, rows, fresh, pending: false, live: None, lanes: Vec::new(), from }
    })
}
