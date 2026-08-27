
use std::ffi::c_void;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::ptr;

use crate::install_paths::{ProductPaths, ResolveOptions};

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;

const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
const OFN_EXPLORER: u32 = 0x0008_0000;
const OFN_NOCHANGEDIR: u32 = 0x0000_0008;
const OFN_DONTADDTORECENT: u32 = 0x0200_0000;
const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
const STD_ERROR_HANDLE: u32 = (-12i32) as u32;
const FILE_TYPE_UNKNOWN: u32 = 0;
const COINIT_APARTMENTTHREADED: u32 = 0x2;
const BIF_RETURNONLYFSDIRS: u32 = 0x0000_0001;
const BIF_NEWDIALOGSTYLE: u32 = 0x0000_0040;
const BIF_NONEWFOLDERBUTTON: u32 = 0x0000_0200;

#[repr(C)]
struct OpenFileNameW {
    struct_size: u32,
    owner: Hwnd,
    instance: Hinstance,
    filter: *const u16,
    custom_filter: *mut u16,
    max_custom_filter: u32,
    filter_index: u32,
    file: *mut u16,
    max_file: u32,
    file_title: *mut u16,
    max_file_title: u32,
    initial_dir: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    file_extension: u16,
    default_extension: *const u16,
    custom_data: isize,
    hook: *mut c_void,
    template_name: *const u16,
    reserved: *mut c_void,
    reserved_u32: u32,
    flags_ex: u32,
}

#[repr(C)]
struct BrowseInfoW {
    owner: Hwnd,
    root: *mut c_void,
    display_name: *mut u16,
    title: *const u16,
    flags: u32,
    callback: *mut c_void,
    param: isize,
    image: i32,
}

#[link(name = "comdlg32")]
extern "system" {
    fn GetOpenFileNameW(options: *mut OpenFileNameW) -> i32;
    fn CommDlgExtendedError() -> u32;
}

#[link(name = "shell32")]
extern "system" {
    fn SHBrowseForFolderW(info: *mut BrowseInfoW) -> *mut c_void;
    fn SHGetPathFromIDListEx(
        item: *mut c_void,
        path: *mut u16,
        path_length: u32,
        flags: u32,
    ) -> i32;
}

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, concurrency: u32) -> i32;
    fn CoUninitialize();
    fn CoTaskMemFree(memory: *mut c_void);
}

#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(hwnd: Hwnd, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn AttachConsole(process_id: u32) -> i32;
    fn GetConsoleWindow() -> Hwnd;
    fn GetStdHandle(standard_handle: u32) -> *mut c_void;
    fn GetFileType(file: *mut c_void) -> u32;
    fn GetLastError() -> u32;
}

pub enum ProductLaunch {
    Cancelled,
    Run {
        args: Vec<String>,
        visible_errors: bool,
    },
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn positional_replay(path: &str) -> Result<Vec<String>, String> {
    let candidate = Path::new(path);
    let extension = candidate.extension().and_then(|value| value.to_str());
    if !extension.is_some_and(|value| value.eq_ignore_ascii_case("hoi4replay")) {
        return Err(format!(
            "The selected file is not a .battleplan recording:\n{}",
            candidate.display()
        ));
    }
    Ok(vec![
        "--recording".to_string(),
        path.to_string(),
        "--non-interactive".to_string(),
    ])
}

fn existing_picker_directory(candidate: Option<PathBuf>) -> Option<PathBuf> {
    candidate.filter(|path| path.is_dir())
}

fn choose_replay() -> Result<Option<String>, String> {
    let mut file = vec![0u16; 32_768];
    let mut filter: Vec<u16> =
        "Battle Plan recordings (*.battleplan)\0*.battleplan\0All files (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
    if !filter.ends_with(&[0, 0]) {
        filter.extend_from_slice(&[0, 0]);
    }
    let title = wide("Open a Battle Plan");
    let default_extension = wide("hoi4replay");
    let initial_dir =
        existing_picker_directory(crate::install_paths::recorder_recordings_root().ok()).map(
            |path| {
                path.as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect::<Vec<_>>()
            },
        );
    let mut options: OpenFileNameW = unsafe { mem::zeroed() };
    options.struct_size = mem::size_of::<OpenFileNameW>() as u32;
    options.filter = filter.as_ptr();
    options.filter_index = 1;
    options.file = file.as_mut_ptr();
    options.max_file = file.len() as u32;
    options.initial_dir = initial_dir
        .as_ref()
        .map_or(ptr::null(), |path| path.as_ptr());
    options.title = title.as_ptr();
    options.flags = OFN_PATHMUSTEXIST
        | OFN_FILEMUSTEXIST
        | OFN_EXPLORER
        | OFN_NOCHANGEDIR
        | OFN_DONTADDTORECENT;
    options.default_extension = default_extension.as_ptr();
    if unsafe { GetOpenFileNameW(&mut options) } == 0 {
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 {
            Ok(None)
        } else {
            Err(format!(
                "Windows replay file picker failed with error 0x{error:08x}"
            ))
        };
    }
    let end = file
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(file.len());
    String::from_utf16(&file[..end])
        .map(Some)
        .map_err(|_| "Windows replay file picker returned invalid UTF-16".to_string())
}

fn choose_folder(title: &str) -> Result<Option<PathBuf>, String> {
    let initialized = unsafe { CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED) };
    if initialized < 0 {
        return Err(format!(
            "initialize Windows folder picker failed: HRESULT 0x{:08x}",
            initialized as u32
        ));
    }
    let title = wide(title);
    let mut display_name = vec![0u16; 260];
    let mut info = BrowseInfoW {
        owner: ptr::null_mut(),
        root: ptr::null_mut(),
        display_name: display_name.as_mut_ptr(),
        title: title.as_ptr(),
        flags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE | BIF_NONEWFOLDERBUTTON,
        callback: ptr::null_mut(),
        param: 0,
        image: 0,
    };
    let item = unsafe { SHBrowseForFolderW(&mut info) };
    if item.is_null() {
        unsafe { CoUninitialize() };
        return Ok(None);
    }
    let mut path = vec![0u16; 32_768];
    let converted = unsafe { SHGetPathFromIDListEx(item, path.as_mut_ptr(), path.len() as u32, 0) };
    unsafe {
        CoTaskMemFree(item);
        CoUninitialize();
    }
    if converted == 0 {
        return Err("Windows folder picker could not resolve the selected filesystem path".into());
    }
    let end = path
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(path.len());
    let selected = String::from_utf16(&path[..end])
        .map_err(|_| "Windows folder picker returned invalid UTF-16".to_string())?;
    if selected.is_empty() {
        return Err("Windows folder picker returned an empty path".into());
    }
    Ok(Some(PathBuf::from(selected)))
}

fn choose_validated_folder(
    title: &str,
    label: &str,
    validate: impl Fn(&Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    loop {
        let Some(path) = choose_folder(title)? else {
            return Err(format!(
                "first-launch setup was cancelled before selecting {label}"
            ));
        };
        match validate(&path) {
            Ok(()) => return Ok(path),
            Err(error) => show_error(&format!(
                "That folder is not a valid {label}:\n\n{}\n\n{error}\n\nChoose the correct folder or cancel setup.",
                path.display()
            )),
        }
    }
}

fn strict_saved_profile_options() -> ResolveOptions {
    ResolveOptions {
        game_override: None,
        user_data_override: None,
        force_reselect: false,
        non_interactive: true,
    }
}

fn strict_replacement_options(paths: ProductPaths) -> ResolveOptions {
    ResolveOptions {
        game_override: Some(paths.game_install),
        user_data_override: Some(paths.user_data),
        force_reselect: true,
        non_interactive: true,
    }
}

pub fn ensure_product_profile() -> Result<(), String> {
    if crate::install_paths::resolve_process_paths(None, strict_saved_profile_options()).is_ok() {
        return Ok(());
    }
    loop {
        let game_install = choose_validated_folder(
            "Select the game installation folder (the folder containing hoi4.exe)",
            "the game installation folder",
            crate::install_paths::validate_game_install,
        )?;
        let user_data = choose_validated_folder(
            "Select the game user-data folder (the folder containing dlc_load.json)",
            "the game user-data folder",
            crate::install_paths::validate_user_data,
        )?;
        let paths = ProductPaths {
            game_install,
            user_data,
        };
        if let Err(error) = crate::install_paths::validate_product_profile(&paths) {
            show_error(&format!(
                "Those folders do not form a complete playback profile:\n\n{error}\n\nChoose the folders again or cancel setup."
            ));
            continue;
        }
        return crate::install_paths::resolve_process_paths(
            None,
            strict_replacement_options(paths),
        )
        .map(|_| ())
        .map_err(|error| format!("save validated playback profile failed: {error}"));
    }
}

pub fn normalize(args: Vec<String>) -> Result<ProductLaunch, String> {
    if args.is_empty() {
        return match choose_replay()? {
            Some(path) => Ok(ProductLaunch::Run {
                args: positional_replay(&path)?,
                visible_errors: true,
            }),
            None => Ok(ProductLaunch::Cancelled),
        };
    }
    if args.len() == 1 && !args[0].starts_with('-') {
        return Ok(ProductLaunch::Run {
            args: positional_replay(&args[0])?,
            visible_errors: true,
        });
    }
    Ok(ProductLaunch::Run {
        args,
        visible_errors: false,
    })
}

pub fn show_error(message: &str) {
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(message).as_ptr(),
            wide("Battle Plan").as_ptr(),
            0x0000_0010,
        );
    }
}

/// A non-fatal statement the viewer must see. Playback continues after it.
pub fn show_notice(message: &str) {
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(message).as_ptr(),
            wide("Battle Plan").as_ptr(),
            0x0000_0040,
        );
    }
}

static VISIBLE_DIAGNOSTICS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Set once from the entry point: this process has no console the viewer reads,
/// so anything they must see goes through a dialog.
pub fn set_visible_diagnostics(visible: bool) {
    VISIBLE_DIAGNOSTICS.store(visible, std::sync::atomic::Ordering::Relaxed);
}

pub fn visible_diagnostics() -> bool {
    VISIBLE_DIAGNOSTICS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn attach_parent_console() -> bool {
    unsafe {
        !GetConsoleWindow().is_null()
            || (AttachConsole(ATTACH_PARENT_PROCESS) != 0 && !GetConsoleWindow().is_null())
    }
}

pub fn inherited_stderr_available() -> bool {
    let handle = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    if handle.is_null() || handle as isize == -1 {
        return false;
    }
    let kind = unsafe { GetFileType(handle) };
    kind != FILE_TYPE_UNKNOWN || unsafe { GetLastError() } == 0
}

pub fn visible_errors_for_route(product_route: bool, diagnostics_available: bool) -> bool {
    product_route || !diagnostics_available
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positional_replay_becomes_non_interactive_product_open() {
        let ProductLaunch::Run {
            args,
            visible_errors,
        } = normalize(vec!["Example.battleplan".to_string()]).unwrap()
        else {
            panic!("expected product run");
        };
        assert!(visible_errors);
        assert_eq!(
            args,
            vec!["--recording", "Example.battleplan", "--non-interactive"]
        );
    }

    #[test]
    fn unrelated_positional_file_fails_closed() {
        let error = normalize(vec!["not-a-replay.zip".to_string()])
            .err()
            .expect("wrong extension must fail");
        assert!(error.contains("not a .battleplan"));
    }

    #[test]
    fn explicit_cli_is_preserved() {
        let ProductLaunch::Run {
            args,
            visible_errors,
        } = normalize(vec!["--list-blockers".to_string()]).unwrap()
        else {
            panic!("expected CLI run");
        };
        assert!(!visible_errors);
        assert_eq!(args, vec!["--list-blockers"]);
    }

    #[test]
    fn consoleless_cli_routes_fall_back_to_visible_errors() {
        assert!(visible_errors_for_route(false, false));
        assert!(!visible_errors_for_route(false, true));
        assert!(visible_errors_for_route(true, true));
    }

    #[test]
    fn replay_picker_uses_only_an_existing_recorder_root() {
        let root = std::env::temp_dir().join(format!(
            "replay-app-picker-root-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        assert!(existing_picker_directory(Some(root.clone())).is_none());
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(existing_picker_directory(Some(root.clone())), Some(root.clone()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn product_profile_resolution_and_replacement_never_read_stdin() {
        let saved = strict_saved_profile_options();
        assert!(saved.non_interactive);
        assert!(!saved.force_reselect);
        assert!(saved.game_override.is_none());
        assert!(saved.user_data_override.is_none());

        let replacement = strict_replacement_options(ProductPaths {
            game_install: PathBuf::from("game"),
            user_data: PathBuf::from("user"),
        });
        assert!(replacement.non_interactive);
        assert!(replacement.force_reselect);
        assert_eq!(replacement.game_override, Some(PathBuf::from("game")));
        assert_eq!(replacement.user_data_override, Some(PathBuf::from("user")));
    }
}
