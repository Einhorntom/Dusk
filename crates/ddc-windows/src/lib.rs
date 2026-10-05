use std::collections::{HashMap, HashSet};
use std::io;
use std::ptr;
use std::sync::Mutex;
use windows::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY,
    DestroyPhysicalMonitor, DisplayConfigGetDeviceInfo, GetCapabilitiesStringLength,
    GetDisplayConfigBufferSizes, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, MC_VCP_CODE_TYPE,
    PHYSICAL_MONITOR, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig, SetVCPFeature,
};
use windows::Win32::Foundation::{ERROR_NOT_FOUND, ERROR_SUCCESS, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HMONITOR, MONITORENUMPROC, MONITORINFO, MONITORINFOEXW,
};
use windows::core::BOOL;

use dusk_app::{BackendError, MonitorBackend};
use dusk_domain::{ControlCapability, ControlKey, Monitor, MonitorId};
use dusk_mccs::{Capabilities, parse_capabilities};

struct PhysicalMonitor {
    id: MonitorId,
    name: String,
    handle: PHYSICAL_MONITOR,
}

impl Drop for PhysicalMonitor {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyPhysicalMonitor(self.handle.hPhysicalMonitor);
        }
    }
}

#[derive(Default)]
pub struct WindowsDdcBackend {
    capability_cache: Mutex<HashMap<MonitorId, Capabilities>>,
}

impl WindowsDdcBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn discover(&self) -> Result<Vec<PhysicalMonitor>, BackendError> {
        let mut display_handles = Vec::new();
        let callback: MONITORENUMPROC = Some(collect_display_handle);
        let callback_data = LPARAM(&mut display_handles as *mut Vec<HMONITOR> as isize);
        if !unsafe { EnumDisplayMonitors(None, None, callback, callback_data).as_bool() } {
            return Err(last_error("EnumDisplayMonitors"));
        }

        // The built-in display belongs to the panel backend (SPEC-PNL-1), even
        // when Windows lets it be opened for DDC/CI.
        let built_in = built_in_only_displays();
        let mut monitors = Vec::new();
        for display_handle in display_handles {
            if gdi_device_name(display_handle).is_some_and(|name| built_in.contains(&name)) {
                continue;
            }
            let mut physical_count = 0;
            if let Err(error) = unsafe {
                GetNumberOfPhysicalMonitorsFromHMONITOR(display_handle, &mut physical_count)
            } {
                eprintln!("warning: cannot enumerate physical monitors: {error}");
                continue;
            }
            if physical_count == 0 {
                continue;
            }

            let mut physical = std::iter::repeat_with(PHYSICAL_MONITOR::default)
                .take(physical_count as usize)
                .collect::<Vec<_>>();
            if let Err(error) =
                unsafe { GetPhysicalMonitorsFromHMONITOR(display_handle, &mut physical) }
            {
                for monitor in physical {
                    unsafe {
                        let _ = DestroyPhysicalMonitor(monitor.hPhysicalMonitor);
                    }
                }
                // One display that cannot be opened (e.g. a laptop panel,
                // which has no DDC/CI) must not hide the others. Windows
                // reports that case as "element not found".
                if error.code() != ERROR_NOT_FOUND.to_hresult() {
                    eprintln!(
                        "warning: skipping a display: GetPhysicalMonitorsFromHMONITOR: {error}"
                    );
                }
                continue;
            }

            for handle in physical {
                let description = description(&handle);
                let index = monitors.len();
                let id = MonitorId::new(format!("{description}#{index}"))
                    .map_err(|error| BackendError::Failed(error.to_string()))?;
                monitors.push(PhysicalMonitor {
                    id,
                    name: description,
                    handle,
                });
            }
        }
        Ok(monitors)
    }

    fn capabilities_for(&self, monitor: &PhysicalMonitor) -> Result<Capabilities, BackendError> {
        if let Some(capabilities) = self
            .capability_cache
            .lock()
            .map_err(|_| BackendError::Failed("capability cache lock is poisoned".into()))?
            .get(&monitor.id)
            .cloned()
        {
            return Ok(capabilities);
        }

        let capabilities = read_capabilities(&monitor.handle)?;
        self.capability_cache
            .lock()
            .map_err(|_| BackendError::Failed("capability cache lock is poisoned".into()))?
            .insert(monitor.id.clone(), capabilities.clone());
        Ok(capabilities)
    }
}

impl MonitorBackend for WindowsDdcBackend {
    fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
        self.discover().map(|items| {
            items
                .into_iter()
                .map(|item| Monitor {
                    id: item.id.clone(),
                    name: item.name.clone(),
                    unstable_id: true,
                })
                .collect()
        })
    }

    fn read_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<Option<(ControlCapability, u32)>, BackendError> {
        let monitors = self.discover()?;
        let Some(physical) = monitors.iter().find(|item| &item.id == monitor) else {
            return Err(BackendError::NotFound(format!(
                "monitor not found: {monitor}"
            )));
        };
        let vcp = control.vcp_code();
        let capabilities = self.capabilities_for(physical)?;
        let Some(enum_values) = capabilities.vcp.get(&vcp) else {
            return Ok(None);
        };
        let (current, maximum) = read_vcp(physical, control)?;
        let capability = if control.is_numeric() {
            ControlCapability {
                key: control,
                native_min: 0,
                native_max: maximum,
                enum_values: vec![],
                rate_limited: true,
            }
        } else {
            ControlCapability {
                key: control,
                native_min: 0,
                native_max: 0,
                enum_values: enum_values.iter().map(|value| u32::from(*value)).collect(),
                rate_limited: true,
            }
        };
        Ok(Some((capability, current)))
    }

    fn write_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        native_value: u32,
    ) -> Result<(), BackendError> {
        let monitors = self.discover()?;
        let Some(physical) = monitors.iter().find(|item| &item.id == monitor) else {
            return Err(BackendError::NotFound(format!(
                "monitor not found: {monitor}"
            )));
        };
        let code = control.vcp_code();
        if !self.capabilities_for(physical)?.vcp.contains_key(&code) {
            return Err(BackendError::Failed(format!(
                "monitor does not report support for {control}"
            )));
        }
        let result = unsafe { SetVCPFeature(physical.handle.hPhysicalMonitor, code, native_value) };
        check_result(result, "SetVCPFeature")
    }
}

unsafe extern "system" fn collect_display_handle(
    monitor: HMONITOR,
    _dc: windows::Win32::Graphics::Gdi::HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    monitors.push(monitor);
    BOOL(1)
}

/// Whether an output is a built-in panel rather than an external connector.
fn is_built_in_output(technology: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY) -> bool {
    [
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS,
    ]
    .contains(&technology)
}

/// The Windows displays (GDI device names) shown only on built-in panels,
/// from (display, output technology) pairs. A display duplicated onto an
/// external monitor is kept, so that monitor stays controllable.
fn displays_shown_only_on_built_in(
    outputs: &[(String, DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY)],
) -> HashSet<String> {
    let mut built_in_only = HashMap::<&str, bool>::new();
    for (display, technology) in outputs {
        *built_in_only.entry(display).or_insert(true) &= is_built_in_output(*technology);
    }
    built_in_only
        .into_iter()
        .filter(|(_, only)| *only)
        .map(|(display, _)| display.to_owned())
        .collect()
}

/// Reads the active display paths. If Windows cannot report them, nothing
/// is excluded: listing the panel twice beats hiding a monitor.
fn built_in_only_displays() -> HashSet<String> {
    match active_outputs() {
        Ok(outputs) => displays_shown_only_on_built_in(&outputs),
        Err(error) => {
            eprintln!("warning: cannot identify the built-in display: {error}");
            HashSet::new()
        }
    }
}

fn active_outputs() -> Result<Vec<(String, DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY)>, String> {
    let (mut path_count, mut mode_count) = (0, 0);
    let result = unsafe {
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
    };
    if result != ERROR_SUCCESS {
        return Err(format!("GetDisplayConfigBufferSizes failed: {}", result.0));
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    let result = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    };
    if result != ERROR_SUCCESS {
        return Err(format!("QueryDisplayConfig failed: {}", result.0));
    }
    paths.truncate(path_count as usize);

    let mut outputs = Vec::new();
    for path in paths {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        source.header.adapterId = path.sourceInfo.adapterId;
        source.header.id = path.sourceInfo.id;
        let result = unsafe { DisplayConfigGetDeviceInfo(&mut source.header) };
        if result != 0 {
            return Err(format!("DisplayConfigGetDeviceInfo failed: {result}"));
        }
        outputs.push((
            wide_to_string(&source.viewGdiDeviceName),
            path.targetInfo.outputTechnology,
        ));
    }
    Ok(outputs)
}

/// The GDI device name of a Windows display, such as `\\.\DISPLAY1`.
fn gdi_device_name(display_handle: HMONITOR) -> Option<String> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let found = unsafe {
        GetMonitorInfoW(
            display_handle,
            (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>(),
        )
    };
    found.as_bool().then(|| wide_to_string(&info.szDevice))
}

fn wide_to_string(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn description(monitor: &PHYSICAL_MONITOR) -> String {
    let units = unsafe { ptr::addr_of!(monitor.szPhysicalMonitorDescription).read_unaligned() };
    String::from_utf16_lossy(&units)
        .trim_end_matches('\0')
        .to_owned()
}

fn read_capabilities(monitor: &PHYSICAL_MONITOR) -> Result<Capabilities, BackendError> {
    let mut length = 0;
    check_result(
        unsafe { GetCapabilitiesStringLength(monitor.hPhysicalMonitor, &mut length) },
        "GetCapabilitiesStringLength",
    )?;
    if length == 0 {
        return Err(BackendError::Failed(
            "monitor returned an empty MCCS capabilities string".into(),
        ));
    }
    if length > 65_536 {
        return Err(BackendError::Failed(
            "monitor returned an unreasonable MCCS capabilities length".into(),
        ));
    }

    let mut buffer = vec![0u8; length as usize];
    check_result(
        unsafe { CapabilitiesRequestAndCapabilitiesReply(monitor.hPhysicalMonitor, &mut buffer) },
        "CapabilitiesRequestAndCapabilitiesReply",
    )?;
    let value = String::from_utf8_lossy(&buffer);
    parse_capabilities(value.trim_matches(char::from(0)))
        .map_err(|error| BackendError::Failed(format!("invalid MCCS capabilities: {error}")))
}

fn read_vcp(monitor: &PhysicalMonitor, control: ControlKey) -> Result<(u32, u32), BackendError> {
    let mut kind = MC_VCP_CODE_TYPE(0);
    let mut current = 0;
    let mut maximum = 0;
    check_result(
        unsafe {
            GetVCPFeatureAndVCPFeatureReply(
                monitor.handle.hPhysicalMonitor,
                control.vcp_code(),
                Some(&mut kind),
                &mut current,
                Some(&mut maximum),
            )
        },
        "GetVCPFeatureAndVCPFeatureReply",
    )?;
    Ok((current, maximum))
}

fn check_result(result: i32, operation: &str) -> Result<(), BackendError> {
    if result == 0 {
        Err(last_error(operation))
    } else {
        Ok(())
    }
}

fn last_error(operation: &str) -> BackendError {
    let error = io::Error::last_os_error();
    BackendError::Failed(format!("{operation} failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hardware check: reads every control of every connected monitor and
    /// writes only to a nonexistent monitor or an unreported control.
    #[test]
    #[ignore = "reads the real monitors over DDC/CI; run with --ignored"]
    fn windows_backend_meets_the_backend_contract() {
        dusk_ddc_fake::contract::check_backend_contract(&WindowsDdcBackend::new());
    }

    const HDMI: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY = DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY(5);
    const DISPLAYPORT: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY =
        DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY(10);

    fn output(
        display: &str,
        technology: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY,
    ) -> (String, DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY) {
        (display.to_owned(), technology)
    }

    #[test]
    fn built_in_panels_are_left_to_the_panel_backend() {
        let excluded = displays_shown_only_on_built_in(&[
            output("DISPLAY1", DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL),
            output("DISPLAY2", DISPLAYPORT),
            output(
                "DISPLAY3",
                DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
            ),
            output("DISPLAY4", HDMI),
        ]);
        assert_eq!(
            excluded,
            HashSet::from(["DISPLAY1".to_owned(), "DISPLAY3".to_owned()])
        );
    }

    #[test]
    fn a_display_duplicated_onto_an_external_monitor_is_kept() {
        let excluded = displays_shown_only_on_built_in(&[
            output("DISPLAY1", DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL),
            output("DISPLAY1", DISPLAYPORT),
        ]);
        assert!(excluded.is_empty());
    }

    #[test]
    fn every_built_in_output_technology_is_recognized() {
        for technology in [
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS,
        ] {
            assert!(is_built_in_output(technology));
        }
        assert!(!is_built_in_output(HDMI));
        assert!(!is_built_in_output(DISPLAYPORT));
    }

    #[test]
    fn vcp_code_mapping_stays_in_domain() {
        assert_eq!(ControlKey::Brightness.vcp_code(), 0x10);
        assert_eq!(ControlKey::Input.vcp_code(), 0x60);
    }
}
