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
    pub fn new(first: i32) -> Self {
        Self { facts: Facts::new(first), holding: Holding::default(), through: None, links: HashMap::new() }
    }

    pub fn to(&mut self, query: &Query<'_>, country: &Country, hour: i32, cancelled: &AtomicBool) -> io::Result<()> {
        if self.through.is_some_and(|through| through > hour) {
            *self = Self::new(query.timeline.first);
        }
        if self.through == Some(hour) {
            return Ok(());
        }
        let from = self.through.map_or(i32::MIN, |through| through + 1);
        let (pieces, holding) = pieces(query, country, query.timeline.first, hour);
        for piece in pieces.into_iter().filter(|piece| piece.last >= from) {
            let piece = Piece { first: piece.first.max(from), ..piece };
            let read = read_piece(query, links_of(query, &mut self.links, piece.run)?, &piece)?;
            see_piece(&mut self.facts, &piece, &read, cancelled)?;
        }
        self.holding = holding;
        self.through = Some(hour);
        Ok(())
    }
}

struct Followed {
    follow: Follow,
    spelling: Option<Spelling>,
    data: Arc<dashboard_follow::panel::PanelData>,
}

pub(super) fn reader(replay: Arc<Replay>, timeline: Arc<Timeline>, holders: Arc<Holders>, params: Arc<SpecialProjectsParams>, content: Arc<ProjectContent>, executable: Option<engine_executable::executable_tables::Tables>) -> ProductReader {
    let state = Mutex::new(None::<Followed>);
    Arc::new(move |hour, reset, cancelled, _step| {
        let query = Query { replay: &replay, holders: holders.clone(), timeline: timeline.clone() };
        let mut held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let answer = engine_executable::executable_tables::scope(executable, || -> io::Result<_> {
            if timeline.at(hour).is_none() {
                return Err(dashboard_core::refused("Special projects hour is not recorded"));
            }
            if reset || held.is_none() {
                *held = Some(Followed { follow: Follow::new(timeline.first), spelling: None, data: Arc::new(Err(String::new())) });
            }
            let s = held.as_mut().expect("a followed state");
            if s.follow.through != Some(hour) || s.data.is_err() {
                s.follow.to(&query, &params.country, hour, cancelled)?;
                let tag = query.holders.name(&params.country, hour);
                let reply = present(&query, &params, &content, &s.follow, &tag, cancelled, &mut s.spelling)?;
                s.data = Arc::new(Ok(serde_json::to_value(reply)?));
            }
            Ok(s.data.clone())
        });
        let value = answer.unwrap_or_else(|e| {
            *held = None;
            Arc::new(Err(e.to_string()))
        });
        dashboard_follow::panel::ProductRead { value, rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new() }
    })
}
