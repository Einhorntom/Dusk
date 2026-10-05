use std::collections::HashMap;
use std::io;
use std::ptr;
use std::sync::Mutex;
use windows::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DestroyPhysicalMonitor, GetCapabilitiesStringLength,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
    GetVCPFeatureAndVCPFeatureReply, MC_VCP_CODE_TYPE, PHYSICAL_MONITOR, SetVCPFeature,
};
use windows::Win32::Foundation::{ERROR_NOT_FOUND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HMONITOR, MONITORENUMPROC};
use windows::core::BOOL;

use dispcontrol_app::{BackendError, MonitorBackend};
use dispcontrol_domain::{ControlCapability, ControlKey, Monitor, MonitorId};
use dispcontrol_mccs::{Capabilities, parse_capabilities};

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

        let mut monitors = Vec::new();
        for display_handle in display_handles {
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
        dispcontrol_ddc_fake::contract::check_backend_contract(&WindowsDdcBackend::new());
    }

    #[test]
    fn vcp_code_mapping_stays_in_domain() {
        assert_eq!(ControlKey::Brightness.vcp_code(), 0x10);
        assert_eq!(ControlKey::Input.vcp_code(), 0x60);
    }
}
