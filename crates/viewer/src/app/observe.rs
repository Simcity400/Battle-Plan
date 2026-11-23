use dashboard_core::protocol::{MapOptions, ObservedCountry};
use content_reader_map::map::ProvinceKind;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use viewer_foundation::perspective::Country;
use viewer_map_modes::map_mode::{MapMode, Stock};

pub(crate) fn ctrl_click_target(
    frame: &DecodedFrame,
    mode: &MapMode,
    province: u32,
    kind: Option<ProvinceKind>,
    ours: u32,
) -> Result<Option<ObservedCountry>, String> {
    if matches!(mode, MapMode::Stock(Stock::StrategicAir | Stock::StrategicNavy)) {
        return Ok(None);
    }
    match kind {
        Some(ProvinceKind::Sea) => return Ok(None),
        Some(ProvinceKind::Land | ProvinceKind::Lake) => {}
        None => return Err(format!("province {province} has no class in the content")),
    }
    let provinces = frame.complete::<tables::Provinces>().map_err(|why| format!("Provinces: {why}"))?;
    let state = provinces
        .binary_search_by_key(&province, |row| row.id)
        .map(|at| provinces[at].state)
        .map_err(|_| format!("Provinces province {province} is absent"))?;
    if state == 0 {
        return Ok(None);
    }
    let states = frame.complete::<tables::States>().map_err(|why| format!("States: {why}"))?;
    let owner = states
        .binary_search_by_key(&state, |row| row.id)
        .map(|at| states[at].owner)
        .map_err(|_| format!("States state {state} is absent"))?;
    if owner as i32 <= 0 {
        return Ok(None);
    }
    let links = frame.complete::<tables::CountryLinks>().map_err(|why| format!("CountryLinks: {why}"))?;
    let slot = links
        .binary_search_by_key(&owner, |link| link.id)
        .map(|at| links[at].canonical_slot)
        .map_err(|_| format!("CountryLinks holds no link {owner}, state {state}'s owner"))?;
    if slot == ours {
        return Ok(None);
    }
    let countries = frame.complete::<tables::Countries>().map_err(|why| format!("Countries: {why}"))?;
    let tag = countries
        .iter()
        .rev()
        .find(|country| country.canonical_slot == slot)
        .ok_or_else(|| format!("Countries holds no country in slot {slot}, state {state}'s owner"))?;
    Ok(Some(ObservedCountry { slot, tag: String::from_utf8_lossy(&tag.tag).into_owned(), sequence: 0 }))
}

pub(crate) fn host_country(options: &MapOptions) -> Country {
    match (options.country_slot, &options.country) {
        (Some(slot), tag) => Country::Slot { slot, tag: tag.clone().unwrap_or_default() },
        (None, Some(tag)) => Country::Tag(tag.clone()),
        (None, None) => Country::Recorded,
    }
}

impl super::Viewer {
    pub(crate) fn ctrl_click(
        &mut self,
        document: &crate::document::Document,
        frame: Option<&DecodedFrame>,
        ours: Option<u32>,
        province: u32,
        shift: bool,
        ctx: &eframe::egui::Context,
    ) -> bool {
        if self.picker.country_pinned {
            return false;
        }
        let Some(ours) = ours else {
            return false;
        };
        let mode = self.shown_mode().clone();
        let kind = document.content.counters.kinds.get(&province).copied();
        let head = super::head_rows::head(super::live_of(&self.playhead_map));
        if self.held_clicks.is_empty()
            && let Some(read) = self.head_rows.clicks(head, frame, |frame| ctrl_click_target(frame, &mode, province, kind, ours))
        {
            match read {
                Err(why) => self.overlay_loading.push(format!("the Ctrl+click's country: {why}")),
                Ok(None) => return false,
                Ok(Some(target)) => self.observe(target),
            }
            return true;
        }
        let ground = !(shift && !(self.unit_selection.units().is_empty() && self.fleet_selection.fleets().is_empty() && self.air_selection.wings().is_empty()));
        self.click(document, super::held_clicks::Click::Observe { viewer: ours, province, mode, ground, shift }, frame, ctx);
        true
    }

    pub(super) fn observe(&mut self, mut target: ObservedCountry) {
        self.trace_switch(target.slot, &target.tag, "switch_click");
        let to = Country::Slot { slot: target.slot, tag: target.tag.clone() };
        if self.embedded.is_some() || self.dashboard_map {
            target.sequence = self.observing.ask(to.clone(), now_micros());
            self.picker.events.push(super::picker::ViewEvent::Observe(target));
        }
        self.perspective.country = to;
    }

    pub(crate) fn host_perspective(&mut self, options: &MapOptions) {
        let country = self.observing.shown(host_country(options), options.observed);
        if country != self.perspective.country
            && let Country::Slot { slot, tag } = &country
        {
            self.trace_switch(*slot, tag, "switch_host");
        }
        self.perspective.country = country;
        self.perspective.fog = options.fog;
    }
}

#[derive(Default)]
pub(crate) struct Observing {
    sequence: u64,
    asked: Option<(u64, Country)>,
}

impl Observing {
    fn ask(&mut self, to: Country, now: u64) -> u64 {
        self.sequence = (self.sequence + 1).max(now);
        self.asked = Some((self.sequence, to));
        self.sequence
    }

    fn shown(&mut self, host: Country, echoed: Option<u64>) -> Country {
        match &self.asked {
            Some((sequence, latest)) if echoed.is_none_or(|echoed| echoed < *sequence) => latest.clone(),
            _ => {
                self.asked = None;
                host
            }
        }
    }
}

fn now_micros() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_micros() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{CountryLinkRow, CountryRow, ProvinceRow, StateRow, TableRows};
    use viewer_foundation::test_support::{test_frame, test_put};

    fn frame() -> DecodedFrame {
        let mut frame = test_frame(7);
        let province = |id, state, controller| ProvinceRow { id, state, owner: 0, controller, ..Default::default() };
        test_put(&mut frame, TableRows::Provinces(vec![province(1, 10, 102), province(2, 0, 0), province(3, 0, 0), province(4, 11, 0), province(5, 12, 202)]));
        let state = |id, owner, controller| StateRow { id, owner, controller, ..Default::default() };
        test_put(&mut frame, TableRows::States(vec![state(10, 101, 102), state(11, 0, 0), state(12, 202, 202)]));
        let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
        test_put(&mut frame, TableRows::CountryLinks(vec![link(101, 1), link(102, 2), link(202, 2)]));
        let country = |id, canonical_slot, tag: &str| CountryRow { id, canonical_slot, tag: tag.into(), ..Default::default() };
        test_put(&mut frame, TableRows::Countries(vec![country(101, 1, "GER"), country(102, 2, "SOV")]));
        frame
    }

    fn switch(slot: u32, tag: &str) -> Result<Option<ObservedCountry>, String> {
        Ok(Some(ObservedCountry { slot, tag: tag.into(), sequence: 0 }))
    }

    #[test]
    fn a_ctrl_click_switches_to_the_state_owner_as_the_observer_does() {
        use ProvinceKind::*;
        let frame = frame();
        let political = MapMode::default();
        let cases: [(&str, &MapMode, u32, Option<ProvinceKind>, u32, Result<Option<ObservedCountry>, String>); 9] = [
            ("an occupied province switches to its owner, not its occupier", &political, 1, Some(Land), 2, switch(1, "GER")),
            ("the owner already viewed is a plain click", &political, 1, Some(Land), 1, Ok(None)),
            ("a sea province picks its region", &political, 2, Some(Sea), 2, Ok(None)),
            ("stateless land", &political, 3, Some(Land), 2, Ok(None)),
            ("a lake has no state", &political, 3, Some(Lake), 2, Ok(None)),
            ("a state owned by link zero", &political, 4, Some(Land), 2, Ok(None)),
            ("another link of the country viewed is the same country", &political, 5, Some(Land), 2, Ok(None)),
            ("the strategic air map picks the region", &MapMode::Stock(Stock::StrategicAir), 1, Some(Land), 2, Ok(None)),
            ("the strategic navy map picks the region", &MapMode::Stock(Stock::StrategicNavy), 1, Some(Land), 2, Ok(None)),
        ];
        for (case, mode, province, kind, ours, expected) in cases {
            assert_eq!(ctrl_click_target(&frame, mode, province, kind, ours), expected, "{case}");
        }
        for stock in Stock::ALL {
            let expected = if matches!(stock, Stock::StrategicAir | Stock::StrategicNavy) { Ok(None) } else { switch(1, "GER") };
            assert_eq!(ctrl_click_target(&frame, &MapMode::Stock(stock), 1, Some(Land), 2), expected, "{stock:?}");
        }
        assert_eq!(ctrl_click_target(&frame, &MapMode::Scripted("custom".into()), 1, Some(Land), 2), switch(1, "GER"));
    }

    #[test]
    fn an_undecided_target_is_named_never_guessed() {
        let land = Some(ProvinceKind::Land);
        let political = MapMode::default();
        assert!(ctrl_click_target(&frame(), &political, 1, None, 2).unwrap_err().contains("no class"));
        assert!(ctrl_click_target(&frame(), &political, 9, land, 2).unwrap_err().contains("province 9 is absent"));
        for (table, name) in [
            (replay_data::rows::TableId::Provinces, "Provinces"),
            (replay_data::rows::TableId::States, "States"),
            (replay_data::rows::TableId::CountryLinks, "CountryLinks"),
            (replay_data::rows::TableId::Countries, "Countries"),
        ] {
            let mut frame = frame();
            frame.tables[table.index()] = replay_frame::frame::TableView::Unavailable { reason: "read fault".into() }.into();
            let why = ctrl_click_target(&frame, &political, 1, land, 2).unwrap_err();
            assert!(why.starts_with(name), "{name}: {why}");
        }
        let mut frame = frame();
        test_put(&mut frame, TableRows::CountryLinks(vec![CountryLinkRow { id: 102, canonical_slot: 2 }]));
        assert!(ctrl_click_target(&frame, &political, 1, land, 2).unwrap_err().contains("no link 101"));
    }

    #[test]
    fn a_switch_made_here_holds_until_the_host_echoes_it() {
        let country = |slot: u32, tag: &str| Country::Slot { slot, tag: tag.into() };
        let (ita, nzl, ger) = (country(1, "ITA"), country(2, "NZL"), country(4, "GER"));
        let mut observing = Observing::default();
        assert_eq!(observing.shown(ita.clone(), None), ita, "nothing asked: the host's");
        let first = observing.ask(nzl.clone(), 0);
        assert_eq!(observing.shown(ita.clone(), None), nzl, "the host's options from before the click never bounce it back");
        assert_eq!(observing.shown(ger.clone(), None), nzl, "nor does a choice it made before it saw the click");
        assert_eq!(observing.shown(nzl.clone(), Some(first)), nzl, "the host's echo is the same country, so nothing restarts");
        assert_eq!(observing.shown(ger.clone(), Some(first)), ger, "once echoed, the host's next choice is its own");

        let mut observing = Observing::default();
        let to_nzl = observing.ask(nzl.clone(), 0);
        let to_ita = observing.ask(ita.clone(), 0);
        assert!(to_ita > to_nzl, "each switch's sequence is past the one before");
        assert_eq!(observing.shown(ita.clone(), None), ita, "the cached A shows the latest switch, A");
        assert_eq!(observing.shown(ita.clone(), None), ita, "every frame");
        assert_eq!(observing.shown(nzl.clone(), Some(to_nzl)), ita, "a delayed echo of B keeps A");
        assert_eq!(observing.shown(ita.clone(), Some(to_ita)), ita, "the echo of A");
        assert_eq!(observing.shown(nzl.clone(), Some(to_ita)), nzl, "followed, the host leads again");

        let mut observing = Observing::default();
        assert_eq!(observing.ask(ita, 1_000), 1_000, "a switch counts on from the clock");
        assert_eq!(observing.ask(nzl, 5), 1_001, "and never back");
    }

    #[test]
    fn a_ctrl_click_switches_at_once_and_the_host_follows() {
        use crate::app::picker::ViewEvent;
        use dashboard_core::protocol::MapAction;
        let mut viewer = super::super::Viewer::create(crate::Config::default(), crate::Options::default(), None, std::sync::Arc::new(std::sync::Mutex::new(None)), None, None);
        viewer.dashboard_map = true;
        let mut frame = eframe::Frame::_new_kittest();
        let configure = |viewer: &mut super::super::Viewer, frame: &mut eframe::Frame, country: &str, slot, observed| {
            let options = MapOptions { country: Some(country.into()), country_slot: Some(slot), observed, ..Default::default() };
            viewer.host_map_action(MapAction::Configure { options }, &eframe::egui::Context::default(), frame);
        };
        let (ita, nzl) = (Country::Slot { slot: 1, tag: "ITA".into() }, Country::Slot { slot: 2, tag: "NZL".into() });
        configure(&mut viewer, &mut frame, "ITA", 1, None);
        assert_eq!(viewer.perspective.country, ita);
        viewer.observe(ObservedCountry { slot: 2, tag: "NZL".into(), sequence: 0 });
        assert_eq!(viewer.perspective.country, nzl, "the view switches before the host answers");
        let asked: Vec<_> = viewer.take_view_events().into_iter().filter_map(|event| match event {
            ViewEvent::Observe(country) => Some(country),
            _ => None,
        }).collect();
        assert_eq!(asked.len(), 1, "the host is told once: {asked:?}");
        let sequence = asked[0].sequence;
        assert_eq!((asked[0].slot, asked[0].tag.as_str()), (2, "NZL"), "and the host is told");
        assert!(sequence > 0, "with the switch's sequence");
        configure(&mut viewer, &mut frame, "ITA", 1, None);
        assert_eq!(viewer.perspective.country, nzl, "options sent before the host saw the click change nothing");
        configure(&mut viewer, &mut frame, "NZL", 2, Some(sequence));
        assert_eq!(viewer.perspective.country, nzl);
        configure(&mut viewer, &mut frame, "ITA", 1, Some(sequence));
        assert_eq!(viewer.perspective.country, ita, "the host's own later choice shows");
    }

    #[test]
    fn the_host_options_follow_a_slot_before_a_tag() {
        let options = |country: Option<&str>, country_slot| MapOptions { country: country.map(str::to_owned), country_slot, ..Default::default() };
        assert_eq!(host_country(&options(None, None)), Country::Recorded);
        assert_eq!(host_country(&options(Some("GER"), None)), Country::Tag("GER".into()));
        assert_eq!(host_country(&options(Some("GER"), Some(1))), Country::Slot { slot: 1, tag: "GER".into() });
    }
}
