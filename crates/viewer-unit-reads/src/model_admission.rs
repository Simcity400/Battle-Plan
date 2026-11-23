use content_reader_defines::defines::Defines;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gates {
    pub units: f32,
    pub ships: f32,
    pub arrows: f32,
    pub transport_scale: f32,
    pub scale_up: f32,
    pub scale_down: f32,
    pub ship_scale_down: f32,
    pub naval_movement_speed: f32,
    pub land_movement_speed: f32,
}

impl Gates {
    pub const OFF: Self = Self {
        units: 0.0,
        ships: 0.0,
        arrows: 0.0,
        transport_scale: 1.0,
        scale_up: 0.0,
        scale_down: 0.0,
        ship_scale_down: 0.0,
        naval_movement_speed: 0.0,
        land_movement_speed: 0.0,
    };

    pub fn read(defines: &Defines) -> io::Result<Self> {
        let number = |name| defines.number("NGraphics", name).map(|value| value as f32);
        Ok(Self {
            units: number("UNITS_DISTANCE_CUTOFF")?,
            ships: number("SHIPS_DISTANCE_CUTOFF")?,
            arrows: number("UNIT_ARROW_DISTANCE_CUTOFF")?,
            transport_scale: number("SHIP_IN_PORT_SCALE")?,
            scale_up: number("MESH_POPUP_SCALE_UP_SPEED")?,
            scale_down: number("MESH_POPUP_SCALE_DOWN_SPEED")?,
            ship_scale_down: number("SHIP_POPUP_SCALE_DOWN_SPEED")?,
            naval_movement_speed: number("NAVAL_UNIT_MOVEMENT_SPEED")?,
            land_movement_speed: number("LAND_UNIT_MOVEMENT_SPEED")?,
        })
    }

    pub fn units(&self, height: f32) -> bool {
        height < self.units
    }

    pub fn ships(&self, height: f32) -> bool {
        height < self.ships
    }

    pub fn for_mode(self, visibility: &viewer_foundation::mode_visibility::Visibility) -> Self {
        Self {
            units: if visibility.land_models {
                self.units
            } else {
                0.0
            },
            arrows: if visibility.unit_arrows {
                self.arrows
            } else {
                0.0
            },
            ..self
        }
    }
}
