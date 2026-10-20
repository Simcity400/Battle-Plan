//! A content stack's game-window resources: the GUI templates read in one interface viewport,
//! the native GUI resources and every texture page they loaded, for one font language and
//! texture decode. A source owns its sessions ([`Sessions`], in its own state): a session holds
//! its stack, so the stack must not hold it, or neither would ever be dropped. Sessions of
//! another stack are dropped when one of a new stack opens. Pages are numbered in load order;
//! artwork requests name them with the session's id, so a page of another session is never
//! confused with one of this.

use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
};

use installed_content_core::{
    ResolvedLayers,
    gui::native::{self, resources::Registry},
};
use installed_content_image::{dds, native::Decode};
use viewer_foundation::map::{
    native_gui::Assets, native_resources::GuiResources, native_shader::draw::gui_viewport,
};

use crate::invalid;

pub struct Session {
    pub id: String,
    pub viewport: [i32; 2],
    pub guis: native::Guis,
    pub gui: GuiResources,
    pub images: Vec<dds::Texture>,
    /// Encoded pages, by id, kept within [`ENCODED_BYTES`]: eviction changes reuse only.
    encoded: std::collections::BTreeMap<usize, Arc<str>>,
    encoded_bytes: usize,
}

/// The encoded pages one session keeps before it encodes again.
const ENCODED_BYTES: usize = 64 * 1024 * 1024;

impl Session {
    fn open(
        layers: &ResolvedLayers,
        language: &str,
        decode: Decode,
        viewport: [i32; 2],
    ) -> io::Result<Self> {
        let guis = native::read_layers_at(layers, viewport)?;
        let registry = Registry::load(layers)?;
        let map_size = shader_pipeline::artwork::map_size(layers).map_err(invalid)?;
        // Font families are selected by the complete `l_` language key.
        let gui = GuiResources::new(
            registry,
            format!("l_{language}").as_bytes(),
            map_size,
            decode,
        );
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            viewport,
            guis,
            gui,
            images: Vec::new(),
            encoded: Default::default(),
            encoded_bytes: 0,
        })
    }

    /// Loads every sprite, font and texture `assets` names that is not loaded yet.
    pub fn load(&mut self, layers: &ResolvedLayers, assets: &Assets) -> io::Result<()> {
        self.gui.load(layers, assets, &mut self.images)
    }

    /// A page already encoded as a PNG data URL.
    pub fn encoded(&self, id: usize) -> Option<Arc<str>> {
        self.encoded.get(&id).cloned()
    }

    /// Keeps an encoded page, dropping every kept page first when the budget is spent.
    pub fn keep_encoded(&mut self, id: usize, image: Arc<str>) {
        if self.encoded_bytes + image.len() > ENCODED_BYTES {
            self.encoded.clear();
            self.encoded_bytes = 0;
        }
        self.encoded_bytes += image.len();
        if let Some(old) = self.encoded.insert(id, image) {
            self.encoded_bytes -= old.len();
        }
    }

    /// A loaded texture page's size, by its id.
    pub fn page_size(&self, id: usize) -> Option<[u32; 2]> {
        self.images.get(id).map(|image| [image.width, image.height])
    }
}

/// A kept session and what it was opened for, readable without waiting for the session.
struct Kept {
    id: String,
    /// The content stack's identity.
    stack: String,
    viewport: [i32; 2],
    language: String,
    decode: Decode,
    session: Arc<Mutex<Session>>,
}

/// A source's sessions: those of its current stack, and which stack that is.
#[derive(Default)]
pub struct Sessions {
    kept: Vec<Kept>,
    /// The stack the latest request read: the source's content as it is now. A read-ahead
    /// reads the content as it was when the source opened, which a request may since have
    /// replaced.
    current: Option<String>,
}

/// Who opens a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opener {
    /// A window's request, which reads the content as it is now and makes its stack current.
    Request,
    /// The read-ahead, which opens nothing for a stack a request has replaced.
    ReadAhead,
}

/// Whether `stack` is the source's content as far as its requests know: no request has read
/// another since.
pub fn is_current(sessions: &Mutex<Sessions>, stack: &str) -> bool {
    lock(sessions)
        .current
        .as_deref()
        .is_none_or(|current| current == stack)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// The session of `stack` (whose layers are `layers`) for `language`, `decode` and `viewport`
/// among `sessions`, opened on first use; opening one drops those of any other stack. A request
/// makes `stack` current; the read-ahead gets nothing, and changes nothing, once a request has
/// made another stack current, so it never replaces the session a newer request opened. The
/// check, the opening and the publication happen under one lock.
pub fn session(
    sessions: &Mutex<Sessions>,
    stack: &str,
    layers: &ResolvedLayers,
    language: &str,
    decode: Decode,
    viewport: [i32; 2],
    opener: Opener,
) -> io::Result<Option<Arc<Mutex<Session>>>> {
    let mut kept = lock(sessions);
    match opener {
        Opener::Request => kept.current = Some(stack.to_owned()),
        Opener::ReadAhead => {
            if kept
                .current
                .as_deref()
                .is_some_and(|current| current != stack)
            {
                return Ok(None);
            }
        }
    }
    if let Some(kept) = kept.kept.iter().find(|kept| {
        kept.stack == stack
            && kept.viewport == viewport
            && kept.language == language
            && kept.decode == decode
    }) {
        return Ok(Some(Arc::clone(&kept.session)));
    }
    let session = Session::open(layers, language, decode, viewport)?;
    let id = session.id.clone();
    let session = Arc::new(Mutex::new(session));
    kept.kept.retain(|kept| kept.stack == stack);
    kept.kept.push(Kept {
        id,
        stack: stack.to_owned(),
        viewport,
        language: language.to_owned(),
        decode,
        session: Arc::clone(&session),
    });
    Ok(Some(session))
}

/// The session among `sessions` whose id is `id`.
pub fn current(sessions: &Mutex<Sessions>, id: &str) -> Option<Arc<Mutex<Session>>> {
    lock(sessions)
        .kept
        .iter()
        .find(|kept| kept.id == id)
        .map(|kept| Arc::clone(&kept.session))
}

/// The interface viewport the game lays its windows out in on this machine: the window size
/// its display settings give over its interface scale ([`installed_content_settings::settings::Display`]),
/// as the map viewer computes it.
pub fn viewport(user_dir: &Path) -> Result<[i32; 2], String> {
    let display = installed_content_settings::settings::read_display(user_dir)
        .map_err(|why| format!("the game's display settings are unavailable ({why})"))?;
    let monitors = monitors().map_err(|why| format!("the monitors are unavailable ({why})"))?;
    let first = *monitors
        .first()
        .ok_or_else(|| "no monitor is attached".to_owned())?;
    let frame = display
        .frame(&monitors)
        .ok_or_else(|| "the game's display settings give no window size".to_owned())?;
    let scale = display.interface_to_frame(frame, display.effective_gui_scale(first));
    Ok(gui_viewport(frame, scale).map(|extent| extent as i32))
}

/// The monitors, read per-monitor DPI aware as the game is: the desktop host's process is, but a
/// service thread of another host need not be, so the read runs in that awareness and restores
/// the thread's own.
#[cfg(windows)]
pub(crate) fn monitors() -> io::Result<Vec<installed_content_settings::settings::Desktop>> {
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
    };
    // SAFETY: the call takes and returns plain awareness handles; the previous one is put back
    // on this same thread before returning.
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let read = installed_content_settings::settings::desktops();
    if !previous.is_null() {
        // SAFETY: as above.
        unsafe { SetThreadDpiAwarenessContext(previous) };
    }
    read
}

#[cfg(not(windows))]
pub(crate) fn monitors() -> io::Result<Vec<installed_content_settings::settings::Desktop>> {
    installed_content_settings::settings::desktops()
}
