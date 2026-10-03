#![deny(unsafe_code)]

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use dispcontrol_app::{BackendError, SettingsRepository};
use dispcontrol_domain::AppSettings;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct FileSettingsRepository {
    path: PathBuf,
}

impl FileSettingsRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn default_path() -> Result<PathBuf, BackendError> {
        let root = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| BackendError::Failed("APPDATA is not set".into()))?;
        Ok(root.join("dispcontrol").join("config.toml"))
    }
}

#[derive(Serialize, Deserialize)]
struct SettingsFile {
    version: u32,
    debounce_ms: u32,
    confirm_input_change: bool,
    input_revert_seconds: u32,
    live_preview: bool,
}

impl From<AppSettings> for SettingsFile {
    fn from(value: AppSettings) -> Self {
        Self {
            version: 1,
            debounce_ms: value.debounce_ms,
            confirm_input_change: value.confirm_input_change,
            input_revert_seconds: value.input_revert_seconds,
            live_preview: value.live_preview,
        }
    }
}

impl TryFrom<SettingsFile> for AppSettings {
    type Error = BackendError;

    fn try_from(value: SettingsFile) -> Result<Self, Self::Error> {
        if value.version != 1 {
            return Err(BackendError::Failed(format!(
                "unsupported settings file version {}",
                value.version
            )));
        }
        let settings = Self {
            debounce_ms: value.debounce_ms,
            confirm_input_change: value.confirm_input_change,
            input_revert_seconds: value.input_revert_seconds,
            live_preview: value.live_preview,
        };
        validate(&settings)?;
        Ok(settings)
    }
}

impl SettingsRepository for FileSettingsRepository {
    fn load(&self) -> Result<AppSettings, BackendError> {
        if !self.path.exists() {
            return Ok(AppSettings::default());
        }
        let content = fs::read_to_string(&self.path).map_err(io_error)?;
        let file: SettingsFile = toml::from_str(&content).map_err(|error| {
            BackendError::Failed(format!("cannot parse {}: {error}", self.path.display()))
        })?;
        AppSettings::try_from(file)
    }

    fn save(&self, settings: &AppSettings) -> Result<(), BackendError> {
        validate(settings)?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| BackendError::Failed("settings path has no parent".into()))?;
        fs::create_dir_all(parent).map_err(io_error)?;
        let content = toml::to_string_pretty(&SettingsFile::from(settings.clone()))
            .map_err(|error| BackendError::Failed(format!("cannot encode settings: {error}")))?;
        atomic_write(&self.path, content.as_bytes()).map_err(io_error)
    }
}

fn validate(settings: &AppSettings) -> Result<(), BackendError> {
    if !(150..=2000).contains(&settings.debounce_ms) {
        return Err(BackendError::Failed(
            "debounce must be between 150 and 2000 milliseconds".into(),
        ));
    }
    if settings.input_revert_seconds != 0 && !(5..=60).contains(&settings.input_revert_seconds) {
        return Err(BackendError::Failed(
            "input revert must be 0 or between 5 and 60 seconds".into(),
        ));
    }
    Ok(())
}

fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("toml.tmp");
    {
        let mut file = File::create(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
    }
    if path.exists() {
        let backup = path.with_extension("toml.bak");
        fs::copy(path, backup)?;
    }
    replace_file(&temporary, path)
}

#[allow(unsafe_code)]
fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        use windows::core::PCWSTR;

        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|error| io::Error::other(error.to_string()))
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to)
    }
}

fn io_error(error: io::Error) -> BackendError {
    BackendError::Failed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_file_round_trips_defaults() {
        let original = AppSettings::default();
        let text = toml::to_string(&SettingsFile::from(original.clone())).unwrap();
        let decoded: SettingsFile = toml::from_str(&text).unwrap();
        assert_eq!(AppSettings::try_from(decoded).unwrap(), original);
    }

    #[test]
    fn saving_replaces_existing_settings_and_keeps_backup() {
        let unique = format!(
            "dispcontrol-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        let path = directory.join("config.toml");
        let repository = FileSettingsRepository::new(&path);
        let original = AppSettings::default();
        repository.save(&original).unwrap();

        let mut changed = original.clone();
        changed.debounce_ms = 750;
        repository.save(&changed).unwrap();

        assert_eq!(repository.load().unwrap(), changed);
        let backup = path.with_extension("toml.bak");
        let backup_content = fs::read_to_string(backup).unwrap();
        let backup_file: SettingsFile = toml::from_str(&backup_content).unwrap();
        assert_eq!(AppSettings::try_from(backup_file).unwrap(), original);
        fs::remove_dir_all(directory).unwrap();
    }
}
