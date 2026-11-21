mod executable;
mod installation;

pub use executable::{Executable, Part, enter, enter_fixed_modifiers, tables};

pub const DEFAULT_EXECUTABLE: &[u8] = b"synthetic executable";
pub use installation::{Built, Dlc, Installation, Mod};
