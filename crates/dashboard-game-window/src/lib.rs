//! The game's own windows, drawn read-only from a recording's content stack: the native GUI
//! templates, sprites, fonts and localisation the recording's content stack resolves, laid out
//! by the same native GUI code the map viewer draws with ([`viewer_foundation::map::native_gui`]),
//! and bound to the recorded state of one entity at one hour.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the dashboard
//! re-exports its products at `products::game_window` and `products::game_window_artwork`
//! and registers them.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod compose;
pub mod designer;
pub mod mio;
pub mod resources;
pub mod scene;
pub mod session;

pub mod products {
    pub mod game_window;
    pub mod game_window_artwork;

    dashboard_base::reply_warnings!(
        game_window::GameWindowReply,
        game_window_artwork::GameWindowArtworkReply,
    );
}

#[cfg(test)]
mod tests;

pub(crate) fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}
