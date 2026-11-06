//! Naming operands of yards: ports, designs and tokens interned while observing,
//! and their names resolved through the recorded content stack when a reply asks.
use super::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub enum Operand {
    ShipCache {
        handle: u64,
        bytes: Vec<u8>,
    },
    ShipRebuilt {
        handle: u64,
        naming: rows::ShipNamingOperands,
        group: Result<Option<Vec<u8>>, String>,
    },
    Design(RawVariant),
    Class {
        token: u32,
        spelling: Result<String, String>,
        index: Option<i32>,
    },
    Task {
        handle: u64,
        bytes: Vec<u8>,
    },
    Policy {
        port: bool,
        value: i32,
    },
    Port(PortName),
    Refusal(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct PortName {
    pub province: u32,
    pub override_name: Result<Vec<u8>, String>,
    pub owner_tag: Result<String, String>,
    pub state: Result<StateName, String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct StateName {
    pub id: u32,
    pub custom: Vec<u8>,
    pub owner: Vec<u8>,
}

pub fn port_operand(frame: &DecodedFrame, countries: &Countries, province: u32) -> PortName {
    let override_name = frame
        .complete::<tables::ProvinceNames>()
        .map(|rows| {
            find(Some(rows), province, |row| row.province)
                .map(|row| row.name.clone())
                .unwrap_or_default()
        })
        .map_err(|why| why.to_string());
    let state = frame
        .complete::<tables::Provinces>()
        .map_err(|why| why.to_string())
        .and_then(|rows| {
            find(Some(rows), province, |row| row.id)
                .ok_or_else(|| format!("Province {province} not recorded"))
        })
        .and_then(|province| {
            frame
                .complete::<tables::States>()
                .map_err(|why| why.to_string())
                .and_then(|rows| {
                    find(Some(rows), province.state, |row| row.id)
                        .ok_or_else(|| format!("State {} not recorded", province.state))
                })
        });
    let owner_tag = state.as_ref().map_err(Clone::clone).and_then(|state| {
        let slot = if state.owner == 0 {
            Some(0)
        } else {
            countries.slot(state.owner)
        }
        .ok_or_else(|| "Port state owner's country not recorded".to_owned())?;
        installed_content_country_names::country_names::original_or_own_tag(frame, slot).map(str::to_owned)
    });
    let state = state.map(|state| StateName {
        id: state.id,
        custom: state.custom_name.clone(),
        owner: state.owner_name.clone(),
    });
    PortName {
        province,
        override_name,
        owner_tag,
        state,
    }
}

/// The game shows a nonempty primary name over the state's, so an unknown or unshowable one refuses.
pub(super) fn port_name(port: &PortName, cat: &Catalogue) -> Result<String, String> {
    let primary = || -> Result<Option<String>, String> {
        let override_name = port.override_name.as_ref().map_err(Clone::clone)?;
        let text = cat
            .localisation
            .localise(override_name)
            .map_err(|why| why.to_string())?;
        if !text.is_empty() {
            return catalogue::plain_text(&text).map(Some);
        }
        let tag = port.owner_tag.as_ref().map_err(Clone::clone)?;
        for key in [
            format!("{tag}_VICTORY_POINTS_{}", port.province),
            format!("VICTORY_POINTS_{}", port.province),
        ] {
            if cat.localisation.get(&key).is_some() {
                let text = cat
                    .localisation
                    .localise(key.as_bytes())
                    .map_err(|why| why.to_string())?;
                if !text.is_empty() {
                    return catalogue::plain_text(&text).map(Some);
                }
            }
        }
        Ok(None)
    };
    if let Some(text) = primary()? {
        return Ok(text);
    }
    let state = port.state.as_ref().map_err(Clone::clone)?;
    let content = cat.cached_states();
    let row = rows::StateRow {
        id: state.id,
        custom_name: state.custom.clone(),
        owner_name: state.owner.clone(),
        ..Default::default()
    };
    scripted_text::state_display_name(
        &row,
        cat,
        content
            .as_ref()
            .map(|states| &states.names)
            .map_err(String::as_str),
        scripted_text::StateNameUsage::Place,
    )
}

/// `operand`'s name through the recorded content stack, or why it has none.
pub fn resolve(operand: &Operand, cat: Option<&Arc<Catalogue>>) -> Result<YardChoice, String> {
    let cat = cat.ok_or("Naval yard names need the recorded content stack")?;
    let (id, name) = match operand {
        Operand::ShipCache { handle, bytes } => (
            handle.to_string(),
            scripted_text::stored_text(
                &cat.localisation,
                &cat.recorded_language,
                bytes,
                "ship cached name",
            )?,
        ),
        Operand::ShipRebuilt {
            handle,
            naming,
            group,
        } => (handle.to_string(), ship_names::rebuild(cat, naming, group)?),
        Operand::Design(raw) => (
            raw.id.to_string(),
            variant_names::variant_name(raw, Some(cat.as_ref() as &dyn Texts))?
                .ok_or("Design name unavailable")?,
        ),
        Operand::Class {
            spelling, index, ..
        } => {
            let token = if let Some(index) = index {
                // The authenticated index distinguishes definitions sharing token zero.
                let index =
                    u32::try_from(*index).map_err(|_| "Ship class definition index is invalid")?;
                let units = cat
                    .content
                    .layers
                    .derived::<catalogue::CachedResult<installed_content_core::unit_leader::SubUnits>>()
                    .get_or_init(|| installed_content_core::unit_leader::sub_units(&cat.content.layers))
                    .map_err(|why| why.to_string())?;
                units
                    .name(index)
                    .ok_or("Ship class definition absent from recorded content")??
                    .to_owned()
            } else {
                spelling.as_ref().map_err(Clone::clone)?.clone()
            };
            let name = cat
                .localised(&token, View::Plain)
                .filter(|_| cat.has_presentation_key(&token))
                .ok_or_else(|| format!("Ship class {token}: localisation unavailable"))?;
            (token, name)
        }
        Operand::Task { handle, bytes } => (
            handle.to_string(),
            scripted_text::stored_text(
                &cat.localisation,
                &cat.recorded_language,
                bytes,
                "task force cached name",
            )?,
        ),
        Operand::Policy { port, value } => {
            let key = format!(
                "NAVY_{}_POLICY_{value}",
                if *port { "DOCKYARD_REPAIR" } else { "REPAIR" }
            );
            let name = cat
                .localised(&key, View::Plain)
                .filter(|_| cat.has_presentation_key(&key))
                .ok_or_else(|| format!("Repair policy {key}: localisation unavailable"))?;
            (key, name)
        }
        Operand::Port(port) => (port.province.to_string(), port_name(port, cat)?),
        Operand::Refusal(why) => return Err(why.clone()),
    };
    Ok(YardChoice { id, name })
}

/// `operand`'s own id, which a reply shows where its name is unavailable.
pub fn raw_id(operand: &Operand) -> String {
    match operand {
        Operand::ShipCache { handle, .. } | Operand::ShipRebuilt { handle, .. } => {
            handle.to_string()
        }
        Operand::Design(raw) => raw.id.to_string(),
        Operand::Class {
            token, spelling, ..
        } => spelling.clone().unwrap_or_else(|_| token.to_string()),
        Operand::Task { handle, .. } => handle.to_string(),
        Operand::Policy { port, value } => format!(
            "NAVY_{}_POLICY_{value}",
            if *port { "DOCKYARD_REPAIR" } else { "REPAIR" }
        ),
        Operand::Port(port) => port.province.to_string(),
        Operand::Refusal(_) => String::new(),
    }
}
/// The notes resolving `operand` raises: a rebuilt ship name's naming groups'.
pub fn notes(operand: &Operand, cat: &Catalogue) -> Vec<String> {
    match operand {
        Operand::ShipRebuilt { naming, group, .. } if ship_names::needs_groups(naming, group) => {
            ship_names::groups(cat)
                .map(|groups| groups.1.clone())
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

