//! The built duskd.exe carries its icons (duskd.rc): the app icon Explorer
//! shows and the two tray glyphs.

use windows::Win32::Foundation::FreeLibrary;
use windows::Win32::System::LibraryLoader::{
    FindResourceW, LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE, LoadLibraryExW,
};
use windows::Win32::UI::WindowsAndMessaging::RT_GROUP_ICON;
use windows::core::{HSTRING, PCWSTR};

#[test]
fn duskd_embeds_the_app_and_tray_icons() {
    let exe = HSTRING::from(env!("CARGO_BIN_EXE_duskd"));
    let module = unsafe {
        LoadLibraryExW(
            &exe,
            None,
            LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
        )
    }
    .expect("duskd.exe loads as a resource file");
    for id in [1u16, 101, 102] {
        let found = unsafe {
            FindResourceW(
                Some(module),
                PCWSTR(id as usize as *const u16),
                RT_GROUP_ICON,
            )
        };
        assert!(!found.is_invalid(), "icon resource {id} is missing");
    }
    unsafe {
        let _ = FreeLibrary(module);
    }
}
