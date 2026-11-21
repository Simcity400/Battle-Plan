mod executable;
mod installation;

pub use executable::{Executable, Part, enter, tables};

pub const DEFAULT_EXECUTABLE: &[u8] = b"synthetic executable";
pub use installation::{Built, Dlc, Installation, Mod};
