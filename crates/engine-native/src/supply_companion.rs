
use engine_sites::Rva;

pub const STRIDE: usize = 800;
pub const NODES: usize = 0xb8;
pub const NODE_CAPACITY: usize = 0xc0;
pub const RELATED_CONTROLLERS: usize = 0xd0;
pub const REQUIRED_TRUCKS_RAW: usize = 0x130;
pub const TRUCK_FULFILLMENT_RATIO: usize = 0x138;
pub const REQUIRED_TRAINS: usize = 0x140;
pub const ROUTES_SUBTYPE_ZERO: usize = 0x2c0;
pub const SUPPLY_HUB_FULL_MOTORIZATION_TRUCK_COST_RVA: Rva = Rva(0x033e_2cd8);
