//! CMap dimensions and content rasters.
//! Weather display dimensions divide each signed full dimension by four,
//! rounding toward zero.

use engine_sites::Rva;
/// Map-dimensions owner singleton pointer.
pub const SINGLETON_RVA: Rva = Rva(0x033e_7700);
/// Owner's map width, native i32.
pub const WIDTH: usize = 0x40;
/// Owner's map height, native i32.
pub const HEIGHT: usize = 0x44;
