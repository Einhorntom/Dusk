#![cfg(windows)]

use std::error::Error;
use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Devices::Display::MC_VCP_CODE_TYPE;
use windows::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DestroyPhysicalMonitor, GetCapabilitiesStringLength,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
    GetVCPFeatureAndVCPFeatureReply, PHYSICAL_MONITOR, SetVCPFeature,
};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR, MONITORENUMPROC};
use windows::core::BOOL;

const TEST_RESTORE_DELAY: Duration = Duration::from_secs(5);
const CONTROLS: [(&str, u8); 3] = [("brightness", 0x10), ("input", 0x60), ("volume", 0x62)];

struct Monitor {
    index: usize,
    handle: PHYSICAL_MONITOR,
    description: String,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyPhysicalMonitor(self.handle.hPhysicalMonitor);
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command] if command == "list" => list_monitors(),
        [command, index] if command == "read" => read_monitor(parse_index(index)?),
        [command, index, control, value] if command == "test-write" => {
            test_write(parse_index(index)?, control, parse_value(value)?)
        }
        _ => {
            print_usage();
            Err(invalid_input("invalid command or arguments").into())
        }
    }
}

fn print_usage() {
    eprintln!(
        "Usage:\n  dispcontrol-windows-ddc-spike list\n  \
         dispcontrol-windows-ddc-spike read <monitor-index>\n  \
         dispcontrol-windows-ddc-spike test-write <monitor-index> \
         <brightness|input|volume> <numeric-value>"
    );
}

fn parse_index(value: &str) -> Result<usize, Box<dyn Error>> {
    value
        .parse()
        .map_err(|_| invalid_input("monitor index must be a non-negative integer").into())
}

fn parse_value(value: &str) -> Result<u32, Box<dyn Error>> {
    value
        .parse()
        .map_err(|_| invalid_input("VCP value must be an unsigned integer").into())
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn enumerate_monitors() -> Result<Vec<Monitor>, Box<dyn Error>> {
    let mut display_handles = Vec::new();
    let callback: MONITORENUMPROC = Some(collect_display_handle);
    let callback_data = LPARAM(&mut display_handles as *mut Vec<HMONITOR> as isize);
    let enumerated = unsafe { EnumDisplayMonitors(None, None, callback, callback_data).as_bool() };
    if !enumerated {
        return Err(io::Error::other("EnumDisplayMonitors failed").into());
    }

    let mut monitors = Vec::new();
    for display_handle in display_handles {
        let mut physical_count = 0;
        if let Err(error) =
            unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(display_handle, &mut physical_count) }
        {
            eprintln!("warning: cannot count physical monitors for a display: {error}");
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
            for handle in physical {
                unsafe {
                    let _ = DestroyPhysicalMonitor(handle.hPhysicalMonitor);
                }
            }
            return Err(error.into());
        }

        for handle in physical {
            let description_units =
                unsafe { std::ptr::addr_of!(handle.szPhysicalMonitorDescription).read_unaligned() };
            let description = String::from_utf16_lossy(&description_units)
                .trim_end_matches('\0')
                .to_owned();
            monitors.push(Monitor {
                index: monitors.len(),
                handle,
                description,
            });
        }
    }
    Ok(monitors)
}

unsafe extern "system" fn collect_display_handle(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    monitors.push(monitor);
    BOOL(1)
}

fn list_monitors() -> Result<(), Box<dyn Error>> {
    let monitors = enumerate_monitors()?;
    if monitors.is_empty() {
        println!("No physical monitors were found.");
        return Ok(());
    }

    for monitor in &monitors {
        println!(
            "[{}] {}",
            monitor.index,
            nonempty_or(&monitor.description, "(unnamed physical monitor)")
        );
        let started = Instant::now();
        match capabilities(monitor) {
            Ok(value) => println!(
                "    MCCS capabilities ({} ms): {value}",
                started.elapsed().as_millis()
            ),
            Err(error) => println!("    MCCS capabilities: unavailable ({error})"),
        }
    }
    Ok(())
}

fn read_monitor(index: usize) -> Result<(), Box<dyn Error>> {
    let monitors = enumerate_monitors()?;
    let monitor = find_monitor(&monitors, index)?;
    println!(
        "[{}] {}",
        monitor.index,
        nonempty_or(&monitor.description, "(unnamed physical monitor)")
    );

    let started = Instant::now();
    match capabilities(monitor) {
        Ok(value) => println!(
            "MCCS capabilities ({} ms): {value}",
            started.elapsed().as_millis()
        ),
        Err(error) => println!("MCCS capabilities: unavailable ({error})"),
    }
    for (name, code) in CONTROLS {
        let started = Instant::now();
        match read_vcp(monitor, code) {
            Ok((current, maximum)) => {
                println!(
                    "{name} (VCP 0x{code:02X}): current={current}, maximum={maximum} ({} ms)",
                    started.elapsed().as_millis()
                )
            }
            Err(error) => println!("{name} (VCP 0x{code:02X}): unavailable ({error})"),
        }
    }
    Ok(())
}

fn test_write(index: usize, name: &str, value: u32) -> Result<(), Box<dyn Error>> {
    let Some((_, code)) = CONTROLS.iter().find(|(control, _)| *control == name) else {
        return Err(invalid_input("control must be brightness, input, or volume").into());
    };
    let monitors = enumerate_monitors()?;
    let monitor = find_monitor(&monitors, index)?;
    let (original, maximum) = read_vcp(monitor, *code)?;
    if value > maximum {
        return Err(invalid_input("requested value exceeds the reported maximum").into());
    }
    if value == original {
        println!("{name} is already {value}; no write was sent.");
        return Ok(());
    }

    if name == "input" {
        eprintln!(
            "WARNING: changing input can disconnect the monitor USB hub or cause loss of picture."
        );
    }
    println!(
        "Monitor [{}] {}; {name}: {original} -> {value}; \
         automatic restore in {} seconds.",
        monitor.index,
        nonempty_or(&monitor.description, "(unnamed physical monitor)"),
        TEST_RESTORE_DELAY.as_secs()
    );
    print!("Type APPLY to continue: ");
    io::stdout().flush()?;
    let mut confirmation = String::new();
    io::stdin().read_line(&mut confirmation)?;
    if confirmation.trim() != "APPLY" {
        println!("Cancelled; monitor was not changed.");
        return Ok(());
    }

    let started = Instant::now();
    if let Err(write_error) = write_vcp(monitor, *code, value) {
        let restore_result = write_vcp(monitor, *code, original);
        return match restore_result {
            Ok(()) => Err(io::Error::other(format!(
                "test write failed ({write_error}); restoration was attempted"
            ))
            .into()),
            Err(restore_error) => Err(io::Error::other(format!(
                "test write failed ({write_error}) and restoration also failed ({restore_error})"
            ))
            .into()),
        };
    }
    println!("Write completed in {} ms.", started.elapsed().as_millis());
    thread::sleep(TEST_RESTORE_DELAY);

    let restore_started = Instant::now();
    write_vcp(monitor, *code, original)?;
    println!(
        "Restored {name} to {original} in {} ms.",
        restore_started.elapsed().as_millis()
    );
    Ok(())
}

fn find_monitor(monitors: &[Monitor], index: usize) -> Result<&Monitor, Box<dyn Error>> {
    monitors
        .get(index)
        .ok_or_else(|| invalid_input("monitor index is not present").into())
}

fn read_vcp(monitor: &Monitor, code: u8) -> Result<(u32, u32), Box<dyn Error>> {
    let mut vcp_type = MC_VCP_CODE_TYPE(0);
    let mut current = 0;
    let mut maximum = 0;
    check_win32_result(
        unsafe {
            GetVCPFeatureAndVCPFeatureReply(
                monitor.handle.hPhysicalMonitor,
                code,
                Some(&mut vcp_type),
                &mut current,
                Some(&mut maximum),
            )
        },
        "GetVCPFeatureAndVCPFeatureReply",
    )?;
    Ok((current, maximum))
}

fn write_vcp(monitor: &Monitor, code: u8, value: u32) -> Result<(), Box<dyn Error>> {
    check_win32_result(
        unsafe { SetVCPFeature(monitor.handle.hPhysicalMonitor, code, value) },
        "SetVCPFeature",
    )
}

fn capabilities(monitor: &Monitor) -> Result<String, Box<dyn Error>> {
    let mut length = 0;
    check_win32_result(
        unsafe { GetCapabilitiesStringLength(monitor.handle.hPhysicalMonitor, &mut length) },
        "GetCapabilitiesStringLength",
    )?;
    if length == 0 {
        return Ok(String::new());
    }
    if length > 65_536 {
        return Err(
            invalid_input("monitor reported an unreasonable capabilities-string length").into(),
        );
    }

    let mut buffer = vec![0u8; length as usize];
    check_win32_result(
        unsafe {
            CapabilitiesRequestAndCapabilitiesReply(monitor.handle.hPhysicalMonitor, &mut buffer)
        },
        "CapabilitiesRequestAndCapabilitiesReply",
    )?;
    let value = String::from_utf8_lossy(&buffer);
    Ok(value.trim_matches(char::from(0)).to_owned())
}

fn check_win32_result(result: i32, operation: &str) -> Result<(), Box<dyn Error>> {
    if result == 0 {
        Err(io::Error::other(format!("{operation} failed")).into())
    } else {
        Ok(())
    }
}

fn nonempty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_phase_zero_controls_have_expected_vcp_codes() {
        assert_eq!(
            CONTROLS,
            [("brightness", 0x10), ("input", 0x60), ("volume", 0x62)]
        );
    }

    #[test]
    fn empty_descriptions_have_a_readable_fallback() {
        assert_eq!(nonempty_or("", "unnamed"), "unnamed");
        assert_eq!(nonempty_or("Display", "unnamed"), "Display");
    }
}
