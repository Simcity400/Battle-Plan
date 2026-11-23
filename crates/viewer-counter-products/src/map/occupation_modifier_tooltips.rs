use dashboard_effects::ScriptedEffects;
use super::counter_content;
use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link, ScriptedText};
use content_reader_politics::occupation_modifiers::{Definition, Kind};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use viewer_foundation::tooltip_text::{self, Tooltip};

pub fn build(
    content: &counter_content::Content,
    frame: &DecodedFrame,
    state: u32,
    definition: &Definition,
    viewer: u32,
) -> Tooltip {
    let mut out = Tooltip::default();
    let result = (|| {
        let mut scopes = recorded_scopes(frame, state, definition.kind)?;
        let controller_scope = usize::from(!definition.kind.is_core());
        let perspective = match scopes.scopes[controller_scope].this {
            EventObject::Country(country) => country,
            _ => return Err("occupation controller scope is not a country".into()),
        };
        let scripted = content.scripted.as_ref().as_ref().map_err(Clone::clone)?;
        let text = ScriptedText::new(frame, &content.localisation, scripted, perspective).viewing(viewer);
        let effects = scripted.effects()?;
        let context = dashboard_effects::effect_description::Context::new(
            &text,
            &content.localisation,
            perspective,
            "common/resistance_compliance_modifiers",
            effects.view(None),
        );
        Ok::<_, String>(
            dashboard_effects::effect_description::occupation_modifier_description(
                definition,
                &context,
                &mut scopes,
            ),
        )
    })();
    match result {
        Ok(description) => {
            out.line(
                false,
                0,
                "occupation modifier text",
                tooltip_text::displayable_flags(content.text_icons.as_ref(), description.text),
            );
            for (why, count) in description.refusals {
                *out.refused
                    .entry(format!("occupation modifier {}: {why}", definition.token))
                    .or_default() += count;
            }
        }
        Err(why) => out.refuse(&format!("occupation modifier {}", definition.token), why),
    }
    out
}

fn recorded_scopes(frame: &DecodedFrame, state: u32, kind: Kind) -> Result<EventScopes, String> {
    if kind == Kind::Invalid {
        return Err("occupation modifier has invalid type (1 definition)".into());
    }
    let state_row = frame
        .complete::<tables::States>()
        .map_err(|why| why.to_string())?
        .iter()
        .find(|row| row.id == state)
        .ok_or_else(|| format!("States lacks state {state}"))?;
    let resistance = frame
        .complete::<tables::StateResistance>()
        .map_err(|why| why.to_string())?
        .iter()
        .find(|row| row.state == state)
        .ok_or_else(|| format!("StateResistance lacks state {state}"))?;
    let occupied = if kind.is_core() {
        resistance
            .map
            .require("StateResistance.map")?
            .as_ref()
            .ok_or("StateResistance.map was unreadable when recorded")?
            .core_country
            .ok_or("occupation relation's occupied-country scope was not recorded")?
    } else {
        resistance.country
    };
    let links = dashboard_text::table_access::rows::<tables::CountryLinks>(frame)
        .map_err(|why| why.to_string())?;
    let slot = |raw| {
        dashboard_text::table_access::country_slot(&links, raw).map(|slot| slot.unwrap_or(0))
    };
    Ok(scopes(
        slot(state_row.controller)?,
        state,
        slot(occupied)?,
        kind.is_core(),
    ))
}

fn scopes(controller: u32, state: u32, occupied: u32, core: bool) -> EventScopes {
    let mut root = EventScope::of(if core {
        EventObject::Country(controller)
    } else {
        EventObject::State(state)
    });
    root.from = Link::Scope(1);
    let mut from = EventScope::of(EventObject::Country(if core {
        occupied
    } else {
        controller
    }));
    from.root = Link::Scope(0);
    if core {
        EventScopes::new(vec![root, from])
    } else {
        from.from = Link::Scope(2);
        let mut occupied = EventScope::of(EventObject::Country(occupied));
        occupied.root = Link::Scope(0);
        EventScopes::new(vec![root, from, occupied])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::CountryLinkRow, rows::OccupationMap, rows::Recorded, rows::StateResistanceRow,
        rows::StateRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;

    fn frame(map: Recorded<Option<OccupationMap>>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Default::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "unused".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 83,
                controller: 700,
                ..Default::default()
            }]));
        *frame.tables[TableId::StateResistance.index()] =
            TableView::Complete(TableRows::StateResistance(vec![StateResistanceRow {
                state: 83,
                country: 900,
                map,
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 700,
                    canonical_slot: 7,
                },
                CountryLinkRow {
                    id: 900,
                    canonical_slot: 9,
                },
                CountryLinkRow {
                    id: 1100,
                    canonical_slot: 11,
                },
            ]));
        frame
    }

    #[test]
    fn occupation_core_hover_uses_hourly_retained_pointer_instead_of_state_reference() {
        let frame = frame(Recorded::Value(Some(OccupationMap {
            core_country: Some(1100),
            ..Default::default()
        })));
        let core = recorded_scopes(&frame, 83, Kind::CoreCompliance).unwrap();
        assert_eq!(core.scopes[0].this, EventObject::Country(7));
        assert_eq!(core.scopes[1].this, EventObject::Country(11));
        let state = recorded_scopes(&frame, 83, Kind::StateCompliance).unwrap();
        assert_eq!(state.scopes[2].this, EventObject::Country(9));
    }

    #[test]
    fn occupation_missing_relation_scope_refuses_core_without_demanding_it_for_state() {
        let recorded = frame(Recorded::Value(Some(OccupationMap::default())));
        let why = recorded_scopes(&recorded, 83, Kind::CoreResistance).unwrap_err();
        assert!(why.contains("relation's occupied-country scope"));
        assert!(recorded_scopes(&recorded, 83, Kind::StateResistance).is_ok());
        let missing = frame(Recorded::Value(None));
        assert!(
            recorded_scopes(&missing, 83, Kind::CoreResistance)
                .unwrap_err()
                .contains("unreadable when recorded")
        );
        assert!(recorded_scopes(&missing, 83, Kind::StateResistance).is_ok());
    }

    #[test]
    fn occupation_badge_core_and_state_scopes_keep_fresh_links() {
        let state = scopes(7, 83, 9, false);
        assert_eq!(state.scopes.len(), 3);
        assert_eq!(state.scopes[0].this, EventObject::State(83));
        assert_eq!(state.scopes[0].root, Link::Itself);
        assert_eq!(state.scopes[0].from, Link::Scope(1));
        assert_eq!(state.scopes[1].this, EventObject::Country(7));
        assert_eq!(state.scopes[1].root, Link::Scope(0));
        assert_eq!(state.scopes[1].from, Link::Scope(2));
        assert_eq!(state.scopes[2].this, EventObject::Country(9));
        assert_eq!(state.scopes[2].root, Link::Scope(0));
        assert_eq!(state.scopes[2].from, Link::Itself);
        assert!(state.scopes.iter().all(|scope| scope.prev == Link::Itself));
        let core = scopes(7, 83, 11, true);
        assert_eq!(core.scopes.len(), 2);
        assert_eq!(core.scopes[1].this, EventObject::Country(11));
        assert_eq!(core.scopes[1].root, Link::Scope(0));
        assert_eq!(core.scopes[1].from, Link::Itself);
    }
}
