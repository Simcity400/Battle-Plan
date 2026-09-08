//! Current desktop dimensions for the a729 graphics-settings finalizer.
//! Native 142107fb0/1421076f0 register monitors in primary then secondary
//! order. 142107bb0 reads the current mode; 142108200 overrides its dimensions
//! with device caps when CreateDCW succeeds. No game process is accessed.

use game_ui::DisplayZeroDesktopMode;
use std::{mem, ptr};
use winapi::shared::minwindef::{BOOL, LPARAM, TRUE};
use winapi::shared::windef::{HDC, HMONITOR, LPRECT};
use winapi::um::wingdi::{CreateDCW, DeleteDC, GetDeviceCaps, DEVMODEW, HORZRES, VERTRES};
use winapi::um::winuser::{
    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, ENUM_CURRENT_SETTINGS,
    MONITORINFOEXW, MONITORINFOF_PRIMARY,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RegisteredDesktop {
    device: [u16; 32],
    dimensions: [u32; 2],
}

/// A pass visits only successfully read/registered desktop modes. Failed
/// monitor-info or current-mode calls are skipped by the native callback.
/// Err means the adapter could not complete acquisition, not a native miss.
trait MonitorRegistrationApi {
    fn enumerate_pass(
        &mut self,
        primary: bool,
        visit: &mut dyn FnMut(RegisteredDesktop),
    ) -> Result<(), ()>;
}

fn observe_with(api: &mut impl MonitorRegistrationApi) -> DisplayZeroDesktopMode {
    let mut first: Option<RegisteredDesktop> = None;
    for primary in [true, false] {
        if api
            .enumerate_pass(primary, &mut |registration| {
                // 1421076f0 updates an existing device-name entry in place.
                // Later distinct registrations cannot change display zero.
                if first.is_none_or(|old| same_device(&old.device, &registration.device)) {
                    first = Some(registration);
                }
            })
            .is_err()
        {
            return DisplayZeroDesktopMode::Unavailable;
        }
    }
    match first {
        None => DisplayZeroDesktopMode::QueryFailed,
        Some(first)
            if first
                .dimensions
                .iter()
                .all(|value| *value > 0 && *value <= i32::MAX as u32) =>
        {
            DisplayZeroDesktopMode::Mode(first.dimensions)
        }
        // Preserve invalid first-registration failure; never select another
        // monitor, substitute a viewport, or turn it into native query failure.
        Some(_) => DisplayZeroDesktopMode::Unavailable,
    }
}

fn same_device(left: &[u16; 32], right: &[u16; 32]) -> bool {
    // 1420af800 compares UTF-16 code units through the first NUL.
    left.iter()
        .take_while(|unit| **unit != 0)
        .eq(right.iter().take_while(|unit| **unit != 0))
}

fn desktop_dimensions(current_mode: [u32; 2], device_caps: Option<[i32; 2]>) -> [u32; 2] {
    device_caps.map_or(current_mode, |caps| caps.map(|value| value as u32))
}

struct Win32MonitorRegistration;

struct EnumerationContext<'a> {
    primary: bool,
    visit: &'a mut dyn FnMut(RegisteredDesktop),
}

unsafe extern "system" fn enumerate_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: LPRECT,
    parameter: LPARAM,
) -> BOOL {
    let context = &mut *(parameter as *mut EnumerationContext<'_>);
    let mut info: MONITORINFOEXW = mem::zeroed();
    info.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, &mut info as *mut _ as _) == 0
        || (info.dwFlags & MONITORINFOF_PRIMARY != 0) != context.primary
    {
        return TRUE;
    }
    let mut mode: DEVMODEW = mem::zeroed();
    mode.dmSize = mem::size_of::<DEVMODEW>() as u16;
    if EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) == 0 {
        return TRUE;
    }
    // Native passes szDevice as CreateDCW's first argument, not the second.
    let dc = CreateDCW(
        info.szDevice.as_ptr(),
        ptr::null(),
        ptr::null(),
        ptr::null(),
    );
    let caps = if dc.is_null() {
        None
    } else {
        let result = [GetDeviceCaps(dc, HORZRES), GetDeviceCaps(dc, VERTRES)];
        DeleteDC(dc);
        Some(result)
    };
    (context.visit)(RegisteredDesktop {
        device: info.szDevice,
        dimensions: desktop_dimensions([mode.dmPelsWidth, mode.dmPelsHeight], caps),
    });
    TRUE
}

impl MonitorRegistrationApi for Win32MonitorRegistration {
    fn enumerate_pass(
        &mut self,
        primary: bool,
        visit: &mut dyn FnMut(RegisteredDesktop),
    ) -> Result<(), ()> {
        let mut context = EnumerationContext { primary, visit };
        unsafe {
            // 142107fb0 ignores EnumDisplayMonitors' return code. Registrations
            // delivered before failure remain, and the second pass still runs.
            EnumDisplayMonitors(
                ptr::null_mut(),
                ptr::null(),
                Some(enumerate_monitor),
                &mut context as *mut _ as LPARAM,
            );
        }
        Ok(())
    }
}

/// Acquire once for the modeled settings-read stage on this viewer's host.
/// The caller retains the result with its admitted graphics generation.
pub(crate) fn native_ui_desktop_mode() -> DisplayZeroDesktopMode {
    observe_with(&mut Win32MonitorRegistration)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct InjectedRegistrationApi {
        passes: [Result<Vec<RegisteredDesktop>, ()>; 2],
        calls: Vec<bool>,
    }

    impl MonitorRegistrationApi for InjectedRegistrationApi {
        fn enumerate_pass(
            &mut self,
            primary: bool,
            visit: &mut dyn FnMut(RegisteredDesktop),
        ) -> Result<(), ()> {
            self.calls.push(primary);
            for registration in self.passes[usize::from(!primary)]
                .as_ref()
                .map_err(|_| ())?
            {
                visit(*registration);
            }
            Ok(())
        }
    }

    fn registration(device: u16, dimensions: [u32; 2]) -> RegisteredDesktop {
        let mut name = [0; 32];
        name[0] = device;
        RegisteredDesktop {
            device: name,
            dimensions,
        }
    }

    #[test]
    fn primary_registration_precedes_secondary_and_later_primary() {
        let mut api = InjectedRegistrationApi {
            passes: [
                Ok(vec![
                    registration(1, [1920, 1080]),
                    registration(2, [1280, 720]),
                ]),
                Ok(vec![registration(3, [2560, 1440])]),
            ],
            calls: Vec::new(),
        };
        assert_eq!(
            observe_with(&mut api),
            DisplayZeroDesktopMode::Mode([1920, 1080])
        );
        assert_eq!(api.calls, [true, false]);
    }

    #[test]
    fn failed_primary_registrations_allow_first_successful_secondary() {
        let mut api = InjectedRegistrationApi {
            passes: [Ok(vec![]), Ok(vec![registration(2, [2560, 1440])])],
            calls: Vec::new(),
        };
        assert_eq!(
            observe_with(&mut api),
            DisplayZeroDesktopMode::Mode([2560, 1440])
        );
        assert_eq!(api.calls, [true, false]);
    }

    #[test]
    fn no_registered_displays_is_known_query_failure_but_adapter_error_is_unavailable() {
        for (passes, expected) in [
            (
                [Ok(vec![]), Ok(vec![])],
                DisplayZeroDesktopMode::QueryFailed,
            ),
            ([Err(()), Ok(vec![])], DisplayZeroDesktopMode::Unavailable),
            (
                [Ok(vec![registration(1, [1920, 1080])]), Err(())],
                DisplayZeroDesktopMode::Unavailable,
            ),
        ] {
            assert_eq!(
                observe_with(&mut InjectedRegistrationApi {
                    passes,
                    calls: Vec::new()
                }),
                expected
            );
        }
    }

    #[test]
    fn device_caps_override_both_dimensions_and_dc_failure_retains_current_mode() {
        assert_eq!(
            desktop_dimensions([1920, 1080], Some([1536, 864])),
            [1536, 864]
        );
        assert_eq!(desktop_dimensions([1920, 1080], None), [1920, 1080]);
    }

    #[test]
    fn invalid_first_dimensions_never_default_or_select_another_monitor() {
        for invalid in [[0, 1080], [1920, 0], [u32::MAX, 1080]] {
            let mut api = InjectedRegistrationApi {
                passes: [
                    Ok(vec![registration(1, invalid)]),
                    Ok(vec![registration(2, [1920, 1080])]),
                ],
                calls: Vec::new(),
            };
            assert_eq!(observe_with(&mut api), DisplayZeroDesktopMode::Unavailable);
        }
        assert_eq!(desktop_dimensions([1920, 1080], Some([0, 0])), [0, 0]);
    }

    #[test]
    fn repeated_device_registration_updates_existing_display_zero() {
        let mut api = InjectedRegistrationApi {
            passes: [
                Ok(vec![registration(1, [1920, 1080])]),
                Ok(vec![registration(1, [2560, 1440])]),
            ],
            calls: Vec::new(),
        };
        assert_eq!(
            observe_with(&mut api),
            DisplayZeroDesktopMode::Mode([2560, 1440])
        );
        let mut terminated = registration(1, [2560, 1440]).device;
        terminated[2] = 9;
        assert!(same_device(
            &registration(1, [1920, 1080]).device,
            &terminated
        ));
        assert!(!same_device(
            &registration(1, [1920, 1080]).device,
            &registration(2, [1920, 1080]).device
        ));
    }
}
