mod identity;

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::io;
use std::ptr;
use std::sync::{Arc, Mutex};
use windows::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY, DestroyPhysicalMonitor, DisplayConfigGetDeviceInfo,
    GetCapabilitiesStringLength, GetDisplayConfigBufferSizes,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
    GetVCPFeatureAndVCPFeatureReply, MC_VCP_CODE_TYPE, PHYSICAL_MONITOR, QDC_ONLY_ACTIVE_PATHS,
    QueryDisplayConfig, SetVCPFeature,
};
use windows::Win32::Foundation::{ERROR_NOT_FOUND, ERROR_SUCCESS, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HMONITOR, MONITORENUMPROC, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY, RegGetValueW};
use windows::core::{BOOL, PCWSTR};

use dusk_app::{BackendError, MonitorBackend};
use dusk_domain::{ControlCapability, ControlKey, Monitor, MonitorId};
use dusk_mccs::{Capabilities, parse_capabilities};

use identity::{Candidate, assign_ids, device_instance, pair_outputs, parse_edid};

/// A physical-monitor handle, released when dropped.
struct OwnedPhysical(PHYSICAL_MONITOR);

// SAFETY: a Dxva2 physical-monitor handle is not tied to the thread that
// opened it; `TimedBackend` sends the calls for one monitor one at a time.
unsafe impl Send for OwnedPhysical {}
unsafe impl Sync for OwnedPhysical {}

impl Drop for OwnedPhysical {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyPhysicalMonitor(self.0.hPhysicalMonitor);
        }
    }
}

struct PhysicalMonitor {
    id: MonitorId,
    name: String,
    unstable_id: bool,
    /// The ID used before IDs came from the EDID: description and position.
    legacy_id: MonitorId,
    /// The monitor's device path (one per connected monitor), or its ID.
    cache_key: String,
    handle: OwnedPhysical,
}

/// One active output of a Windows display (GDI device), from the display
/// configuration.
#[derive(Clone)]
struct DisplayOutput {
    display: String,
    technology: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY,
    device_path: Option<String>,
}

/// The Windows displays (handle and GDI name) a discovery was made for.
type DisplaySignature = Vec<(isize, Option<String>)>;

#[derive(Default)]
pub struct WindowsDdcBackend {
    /// Keyed by device path, so a monitor plugged into another monitor's
    /// place never gets that monitor's capabilities.
    capability_cache: Mutex<HashMap<String, Capabilities>>,
    /// The last discovery, reused while Windows reports the same displays.
    /// Opening monitors, reading the display configuration and the EDIDs
    /// then happens only when displays change or a monitor call fails.
    discovered: Mutex<Option<(DisplaySignature, Arc<[PhysicalMonitor]>)>>,
}

impl WindowsDdcBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// `(old, new)` for each connected monitor whose ID changed when IDs
    /// started to come from the EDID (SPEC-MON-1), so saved presets and
    /// hotkeys can follow it.
    pub fn legacy_id_renames(&self) -> Result<Vec<(MonitorId, MonitorId)>, BackendError> {
        Ok(self
            .discover()?
            .iter()
            .filter(|monitor| monitor.legacy_id != monitor.id)
            .map(|monitor| (monitor.legacy_id.clone(), monitor.id.clone()))
            .collect())
    }

    /// The connected monitors, from the cache while the displays are the same.
    fn discover(&self) -> Result<Arc<[PhysicalMonitor]>, BackendError> {
        let mut display_handles = Vec::new();
        let callback: MONITORENUMPROC = Some(collect_display_handle);
        let callback_data = LPARAM(&mut display_handles as *mut Vec<HMONITOR> as isize);
        if !unsafe { EnumDisplayMonitors(None, None, callback, callback_data).as_bool() } {
            return Err(last_error("EnumDisplayMonitors"));
        }
        let signature: DisplaySignature = display_handles
            .iter()
            .map(|handle: &HMONITOR| (handle.0 as isize, gdi_device_name(*handle)))
            .collect();
        let mut discovered = self
            .discovered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((known, monitors)) = discovered.as_ref()
            && *known == signature
        {
            return Ok(monitors.clone());
        }
        let monitors: Arc<[PhysicalMonitor]> = self.open_monitors(display_handles)?.into();
        *discovered = Some((signature, monitors.clone()));
        Ok(monitors)
    }

    /// Drops the cached discovery, e.g. after a monitor call failed because
    /// the monitor was swapped or its handle went stale.
    fn forget_monitors(&self) {
        *self
            .discovered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    fn open_monitors(
        &self,
        display_handles: Vec<HMONITOR>,
    ) -> Result<Vec<PhysicalMonitor>, BackendError> {
        // If Windows cannot report its display paths, nothing is excluded
        // (listing the panel twice beats hiding a monitor) and IDs fall back
        // to positions.
        let outputs = active_outputs().unwrap_or_else(|error| {
            log::warn!("cannot read the display configuration: {error}");
            Vec::new()
        });
        // The built-in display belongs to the panel backend (SPEC-PNL-1), even
        // when Windows lets it be opened for DDC/CI.
        let built_in = displays_shown_only_on_built_in(
            &outputs
                .iter()
                .map(|output| (output.display.clone(), output.technology))
                .collect::<Vec<_>>(),
        );
        let mut found: Vec<(OwnedPhysical, Option<DisplayOutput>)> = Vec::new();
        for display_handle in display_handles {
            let display = gdi_device_name(display_handle);
            if display.as_ref().is_some_and(|name| built_in.contains(name)) {
                continue;
            }
            let mut physical_count = 0;
            if let Err(error) = unsafe {
                GetNumberOfPhysicalMonitorsFromHMONITOR(display_handle, &mut physical_count)
            } {
                log::warn!("cannot enumerate physical monitors: {error}");
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
                    log::warn!("skipping a display: GetPhysicalMonitorsFromHMONITOR: {error}");
                }
                continue;
            }

            let own_outputs: Vec<DisplayOutput> = outputs
                .iter()
                .filter(|output| display.as_ref() == Some(&output.display))
                .cloned()
                .collect();
            let paired = pair_outputs(physical.len(), &own_outputs, |output| {
                !is_built_in_output(output.technology)
            });
            found.extend(physical.into_iter().map(OwnedPhysical).zip(paired));
        }

        let mut descriptions = Vec::new();
        let mut candidates = Vec::new();
        for (handle, output) in &found {
            let description = description(&handle.0);
            let edid = output
                .as_ref()
                .and_then(|output| output.device_path.as_deref())
                .and_then(device_instance)
                .and_then(|instance| read_edid(&instance))
                .and_then(|bytes| parse_edid(&bytes));
            candidates.push(Candidate::new(edid.as_ref(), &description));
            descriptions.push(description);
        }
        let ids = assign_ids(&candidates);
        let monitor_id = |text: String| {
            MonitorId::new(text).map_err(|error| BackendError::Failed(error.to_string()))
        };
        let mut monitors = Vec::new();
        for (index, ((handle, output), (id, unstable_id))) in found.into_iter().zip(ids).enumerate()
        {
            let cache_key = output
                .and_then(|output| output.device_path)
                .unwrap_or_else(|| id.clone());
            monitors.push(PhysicalMonitor {
                id: monitor_id(id)?,
                name: candidates[index].name.clone(),
                unstable_id,
                legacy_id: monitor_id(format!("{}#{index}", descriptions[index]))?,
                cache_key,
                handle,
            });
        }
        Ok(monitors)
    }

    fn capabilities_for(&self, monitor: &PhysicalMonitor) -> Result<Capabilities, BackendError> {
        if let Some(capabilities) = self
            .capability_cache
            .lock()
            .map_err(|_| BackendError::Failed("capability cache lock is poisoned".into()))?
            .get(&monitor.cache_key)
            .cloned()
        {
            return Ok(capabilities);
        }

        let capabilities = read_capabilities(&monitor.handle.0)?;
        self.capability_cache
            .lock()
            .map_err(|_| BackendError::Failed("capability cache lock is poisoned".into()))?
            .insert(monitor.cache_key.clone(), capabilities.clone());
        Ok(capabilities)
    }
}

impl MonitorBackend for WindowsDdcBackend {
    fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
        self.discover().map(|items| {
            items
                .iter()
                .map(|item| Monitor {
                    id: item.id.clone(),
                    name: item.name.clone(),
                    unstable_id: item.unstable_id,
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
        let result = self.read_physical(physical, control);
        if result.is_err() {
            self.forget_monitors();
        }
        result
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
        let result = self.write_physical(physical, control, native_value);
        if result.is_err() {
            self.forget_monitors();
        }
        result
    }
}

impl WindowsDdcBackend {
    fn read_physical(
        &self,
        physical: &PhysicalMonitor,
        control: ControlKey,
    ) -> Result<Option<(ControlCapability, u32)>, BackendError> {
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

    fn write_physical(
        &self,
        physical: &PhysicalMonitor,
        control: ControlKey,
        native_value: u32,
    ) -> Result<(), BackendError> {
        let code = control.vcp_code();
        if !self.capabilities_for(physical)?.vcp.contains_key(&code) {
            return Err(BackendError::Failed(format!(
                "monitor does not report support for {control}"
            )));
        }
        let result =
            unsafe { SetVCPFeature(physical.handle.0.hPhysicalMonitor, code, native_value) };
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

fn active_outputs() -> Result<Vec<DisplayOutput>, String> {
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
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        target.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        target.header.adapterId = path.targetInfo.adapterId;
        target.header.id = path.targetInfo.id;
        // Without the device path the monitor is still listed, by position.
        let device_path = (unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } == 0)
            .then(|| wide_to_string(&target.monitorDevicePath))
            .filter(|path| !path.is_empty());
        outputs.push(DisplayOutput {
            display: wide_to_string(&source.viewGdiDeviceName),
            technology: path.targetInfo.outputTechnology,
            device_path,
        });
    }
    Ok(outputs)
}

/// The monitor's EDID, which Windows keeps in its device registry key
/// (readable without administrator rights).
fn read_edid(instance: &str) -> Option<Vec<u8>> {
    let key: Vec<u16> = format!(r"SYSTEM\CurrentControlSet\Enum\{instance}\Device Parameters")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let value: Vec<u16> = "EDID".encode_utf16().chain(Some(0)).collect();
    let mut size = 0u32;
    let query = |data: Option<*mut c_void>, size: &mut u32| unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_BINARY,
            None,
            data,
            Some(size),
        )
    };
    if query(None, &mut size) != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut bytes = vec![0u8; size as usize];
    if query(Some(bytes.as_mut_ptr().cast()), &mut size) != ERROR_SUCCESS {
        return None;
    }
    bytes.truncate(size as usize);
    Some(bytes)
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
                monitor.handle.0.hPhysicalMonitor,
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
