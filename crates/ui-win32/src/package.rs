//! Whether Dusk runs from its MSIX package (the Microsoft Store version) and
//! how that package was launched. The zip version is not packaged.

use windows::ApplicationModel::Activation::ActivationKind;
use windows::ApplicationModel::AppInstance;
use windows::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;

/// True when running from the MSIX package (package identity).
pub fn is_packaged() -> bool {
    let mut length = 0u32;
    // With no buffer this only reports whether there is a package name.
    unsafe { GetCurrentPackageFullName(&mut length, None) != APPMODEL_ERROR_NO_PACKAGE }
}

/// True when Windows started the packaged Dusk at sign-in through its
/// startup task. That launch has no command line, so `--background` cannot
/// be passed; the daemon starts in the tray when this is true.
pub fn started_at_sign_in() -> bool {
    is_packaged()
        && AppInstance::GetActivatedEventArgs()
            .and_then(|arguments| arguments.Kind())
            .is_ok_and(|kind| kind == ActivationKind::StartupTask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_run_unpackaged() {
        // The test executables have no package identity; the packaged paths
        // are checked with a registered package (RELEASING.md).
        assert!(!is_packaged());
        assert!(!started_at_sign_in());
    }
}
