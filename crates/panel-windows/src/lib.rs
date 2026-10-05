//! The laptop's built-in display as a `MonitorBackend` (SPEC-PNL), through
//! WMI in the `root\WMI` namespace: `WmiMonitorBrightness` for reads and
//! `WmiMonitorBrightnessMethods.WmiSetBrightness` for writes. Built-in
//! displays have no DDC/CI; Windows exposes brightness only, in percent.

use std::cell::RefCell;
use std::mem::ManuallyDrop;

use dispcontrol_app::{BackendError, MonitorBackend};
use dispcontrol_domain::{ControlCapability, ControlKey, Monitor, MonitorId};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoSetProxyBlanket, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::{
    VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BSTR, VT_UI1,
};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE, WbemLocator,
};
use windows::core::{BSTR, HRESULT, PCWSTR, w};

const NAME: &str = "Built-in display";
const ID: &str = "Built-in-display";
const QUERY: &str =
    "SELECT InstanceName, CurrentBrightness FROM WmiMonitorBrightness WHERE Active = TRUE";
/// WMI errors meaning "this PC has no WMI-controllable panel".
const NO_PANEL: [HRESULT; 3] = [
    HRESULT(0x8004_1002_u32 as i32), // WBEM_E_NOT_FOUND
    HRESULT(0x8004_100C_u32 as i32), // WBEM_E_NOT_SUPPORTED
    HRESULT(0x8004_1010_u32 as i32), // WBEM_E_INVALID_CLASS
];

/// `MonitorBackend` for built-in displays. Stateless; WMI connections are
/// cached per thread, since COM objects are created on the calling thread.
#[derive(Default)]
pub struct PanelBackend;

impl PanelBackend {
    pub fn new() -> Self {
        Self
    }
}

struct Panel {
    id: MonitorId,
    instance: String,
    brightness: u32,
}

thread_local! {
    static SERVICES: RefCell<Option<IWbemServices>> = const { RefCell::new(None) };
}

fn failed(what: &'static str) -> impl Fn(windows::core::Error) -> BackendError {
    move |error| BackendError::Failed(format!("{what}: {error}"))
}

/// The `root\WMI` connection for this thread, created on first use.
fn services() -> Result<IWbemServices, BackendError> {
    SERVICES.with_borrow_mut(|cached| {
        if let Some(services) = cached {
            return Ok(services.clone());
        }
        // S_FALSE (already initialized) and RPC_E_CHANGED_MODE (an STA
        // thread) both leave COM usable on this thread.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let locator: IWbemLocator =
            unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }
                .map_err(failed("cannot start WMI"))?;
        let services = unsafe {
            locator.ConnectServer(
                &BSTR::from("ROOT\\WMI"),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                0,
                &BSTR::new(),
                None,
            )
        }
        .map_err(failed("cannot connect to WMI"))?;
        unsafe {
            CoSetProxyBlanket(
                &services,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )
        }
        .map_err(failed("cannot secure the WMI connection"))?;
        *cached = Some(services.clone());
        Ok(services)
    })
}

fn property(object: &IWbemClassObject, name: PCWSTR) -> Result<VARIANT, BackendError> {
    let mut value = VARIANT::default();
    unsafe { object.Get(name, 0, &mut value, None, None) }
        .map_err(failed("cannot read a WMI property"))?;
    Ok(value)
}

/// Active built-in displays, in WMI order.
fn panels() -> Result<Vec<Panel>, BackendError> {
    let services = services()?;
    let flags = WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0);
    let rows =
        match unsafe { services.ExecQuery(&BSTR::from("WQL"), &BSTR::from(QUERY), flags, None) } {
            Ok(rows) => rows,
            Err(error) if NO_PANEL.contains(&error.code()) => return Ok(Vec::new()),
            Err(error) => return Err(failed("cannot query display brightness")(error)),
        };
    let mut panels = Vec::new();
    loop {
        let mut row = [None];
        let mut returned = 0;
        let result = unsafe { rows.Next(WBEM_INFINITE, &mut row, &mut returned) };
        if NO_PANEL.contains(&result) {
            break;
        }
        result
            .ok()
            .map_err(failed("cannot read display brightness"))?;
        let Some(object) = row[0].take().filter(|_| returned > 0) else {
            break;
        };
        let instance = variant_string(&property(&object, w!("InstanceName"))?)
            .ok_or_else(|| BackendError::Failed("WMI returned no display name".into()))?;
        let brightness = u32::try_from(&property(&object, w!("CurrentBrightness"))?)
            .map_err(failed("cannot read the display brightness"))?;
        let id = match panels.len() {
            0 => ID.to_owned(),
            index => format!("{ID}#{}", index + 1),
        };
        panels.push(Panel {
            id: MonitorId::new(id).map_err(|error| BackendError::Failed(error.to_string()))?,
            instance,
            brightness: brightness.min(100),
        });
    }
    Ok(panels)
}

fn find(monitor: &MonitorId) -> Result<Panel, BackendError> {
    panels()?
        .into_iter()
        .find(|panel| panel.id == *monitor)
        .ok_or_else(|| BackendError::NotFound(format!("monitor not found: {monitor}")))
}

fn variant_string(value: &VARIANT) -> Option<String> {
    (value.vt() == VT_BSTR)
        .then(|| unsafe { value.Anonymous.Anonymous.Anonymous.bstrVal.to_string() })
}

fn variant_u8(value: u8) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_UI1,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { bVal: value },
            }),
        },
    }
}

fn set_brightness(panel: &Panel, percent: u8) -> Result<(), BackendError> {
    let services = services()?;
    let no_flags = WBEM_GENERIC_FLAG_TYPE(0);
    let mut class = None;
    unsafe {
        services.GetObject(
            &BSTR::from("WmiMonitorBrightnessMethods"),
            no_flags,
            None,
            Some(&mut class),
            None,
        )
    }
    .map_err(failed("cannot find the brightness method"))?;
    let class: IWbemClassObject =
        class.ok_or_else(|| BackendError::Failed("WMI returned no brightness class".into()))?;
    let (mut input, mut output) = (None, None);
    unsafe { class.GetMethod(w!("WmiSetBrightness"), 0, &mut input, &mut output) }
        .map_err(failed("cannot find WmiSetBrightness"))?;
    let input: IWbemClassObject =
        input.ok_or_else(|| BackendError::Failed("WmiSetBrightness has no parameters".into()))?;
    let parameters = unsafe { input.SpawnInstance(0) }
        .map_err(failed("cannot prepare the brightness change"))?;
    let fill = || -> windows::core::Result<()> {
        unsafe {
            // Timeout 0: the new level stays until something else changes it.
            parameters.Put(w!("Timeout"), 0, &VARIANT::from(0i32), 0)?;
            parameters.Put(w!("Brightness"), 0, &variant_u8(percent), 0)
        }
    };
    fill().map_err(failed("cannot prepare the brightness change"))?;
    let path = format!(
        "WmiMonitorBrightnessMethods.InstanceName=\"{}\"",
        panel.instance.replace('\\', "\\\\")
    );
    unsafe {
        services.ExecMethod(
            &BSTR::from(path),
            &BSTR::from("WmiSetBrightness"),
            no_flags,
            None,
            &parameters,
            None,
            None,
        )
    }
    .map_err(failed("cannot set the display brightness"))
}

fn brightness_capability() -> ControlCapability {
    ControlCapability {
        key: ControlKey::Brightness,
        native_min: 0,
        native_max: 100,
        enum_values: Vec::new(),
        // Not stored in monitor memory (SPEC-PNL-4).
        rate_limited: false,
    }
}

impl MonitorBackend for PanelBackend {
    fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
        Ok(panels()?
            .into_iter()
            .map(|panel| Monitor {
                id: panel.id,
                name: NAME.to_owned(),
                unstable_id: false,
            })
            .collect())
    }

    fn read_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<Option<(ControlCapability, u32)>, BackendError> {
        let panel = find(monitor)?;
        Ok(
            (control == ControlKey::Brightness)
                .then(|| (brightness_capability(), panel.brightness)),
        )
    }

    fn write_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        native_value: u32,
    ) -> Result<(), BackendError> {
        let panel = find(monitor)?;
        if control != ControlKey::Brightness {
            return Err(BackendError::Failed(format!(
                "the built-in display does not support {control}"
            )));
        }
        set_brightness(&panel, native_value.min(100) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hardware check: reads the built-in display and writes only to an
    /// unknown monitor or an unsupported control.
    #[test]
    #[ignore = "reads the built-in display through WMI; run with --ignored"]
    fn panel_backend_meets_the_backend_contract() {
        dispcontrol_ddc_fake::contract::check_backend_contract(&PanelBackend::new());
    }

    /// Hardware check: changes the built-in display's brightness by one step
    /// for a moment, reads it back, and restores it.
    #[test]
    #[ignore = "briefly changes the built-in display's brightness; run with --ignored"]
    fn panel_brightness_round_trips_and_is_restored() {
        let backend = PanelBackend::new();
        let Some(panel) = backend.list_monitors().unwrap().into_iter().next() else {
            panic!("no active built-in display to test");
        };
        let (_, original) = backend
            .read_control(&panel.id, ControlKey::Brightness)
            .unwrap()
            .expect("the built-in display reports brightness");
        let probe = if original >= 50 {
            original - 1
        } else {
            original + 1
        };
        backend
            .write_control(&panel.id, ControlKey::Brightness, probe)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let (_, changed) = backend
            .read_control(&panel.id, ControlKey::Brightness)
            .unwrap()
            .unwrap();
        backend
            .write_control(&panel.id, ControlKey::Brightness, original)
            .unwrap();
        assert_eq!(changed, probe, "the new level stays (Timeout 0)");
    }
}
