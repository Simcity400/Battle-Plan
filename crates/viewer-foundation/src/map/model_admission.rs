//! Content-derived model visibility and popup operands.
use installed_content::defines::Defines;
use std::io;

/// The camera heights, transport scale and popup rates the content's
/// `NGraphics` defines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gates {
    /// `UNITS_DISTANCE_CUTOFF`.
    pub units: f32,
    /// `SHIPS_DISTANCE_CUTOFF`.
    pub ships: f32,
    /// `UNIT_ARROW_DISTANCE_CUTOFF`: its visibility transition also forces cleanup.
    pub arrows: f32,
    /// `SHIP_IN_PORT_SCALE`, which the game applies to the transport
    /// visual states `0x13` and `0x14`.
    pub transport_scale: f32,
    /// `MESH_POPUP_SCALE_UP_SPEED`, scale per second a shown model grows.
    pub scale_up: f32,
    /// `MESH_POPUP_SCALE_DOWN_SPEED`, scale per second a land model shrinks.
    pub scale_down: f32,
    /// `SHIP_POPUP_SCALE_DOWN_SPEED`, scale per second a formation shrinks.
    pub ship_scale_down: f32,
    /// the game's per-frame free arc speeds; viewer-owned presentation.
    pub naval_movement_speed: f32,
    pub land_movement_speed: f32,
}

impl Gates {
    /// No camera height draws a model.
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

    /// Whether this camera draws land unit models; the
    /// cutoff is compared against the raw camera height.
    pub fn units(&self, height: f32) -> bool {
        height < self.units
    }

    pub fn ships(&self, height: f32) -> bool {
        height < self.ships
    }
}
