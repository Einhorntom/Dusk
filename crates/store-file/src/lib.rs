#![deny(unsafe_code)]

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use dispcontrol_app::{BackendError, HotkeyRepository, PresetRepository, SettingsRepository};
use dispcontrol_domain::{
    AppSettings, ControlKey, ControlValue, HotkeyBinding, MonitorId, Preset, PresetEntry,
    validate_preset_name,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug)]
pub struct FileSettingsRepository {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl FileSettingsRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Mutex::new(()),
        }
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
    // Added in v1 hotkeys; defaults keep older files loading.
    #[serde(default = "default_step")]
    brightness_step: u32,
    #[serde(default = "default_step")]
    contrast_step: u32,
    #[serde(default = "default_step")]
    volume_step: u32,
    #[serde(default = "default_true")]
    show_osd: bool,
}

fn default_step() -> u32 {
    AppSettings::default().brightness_step
}

fn default_true() -> bool {
    true
}

impl From<AppSettings> for SettingsFile {
    fn from(value: AppSettings) -> Self {
        Self {
            version: 1,
            debounce_ms: value.debounce_ms,
            confirm_input_change: value.confirm_input_change,
            input_revert_seconds: value.input_revert_seconds,
            live_preview: value.live_preview,
            brightness_step: value.brightness_step,
            contrast_step: value.contrast_step,
            volume_step: value.volume_step,
            show_osd: value.show_osd,
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
            brightness_step: value.brightness_step,
            contrast_step: value.contrast_step,
            volume_step: value.volume_step,
            show_osd: value.show_osd,
        };
        validate(&settings)?;
        Ok(settings)
    }
}

#[derive(Serialize, Deserialize)]
struct PresetFile {
    name: String,
    #[serde(default)]
    entries: Vec<PresetEntryFile>,
}

#[derive(Serialize, Deserialize)]
struct PresetEntryFile {
    monitor: String,
    control: String,
    kind: String,
    value: u32,
}

impl From<&Preset> for PresetFile {
    fn from(preset: &Preset) -> Self {
        Self {
            name: preset.name.clone(),
            entries: preset
                .entries
                .iter()
                .map(|entry| {
                    let (kind, value) = match entry.value {
                        ControlValue::Normalized(value) => ("normalized", value),
                        ControlValue::Enum(value) => ("enum", value),
                    };
                    PresetEntryFile {
                        monitor: entry.monitor.as_str().to_owned(),
                        control: entry.control.as_str().to_owned(),
                        kind: kind.to_owned(),
                        value,
                    }
                })
                .collect(),
        }
    }
}

impl TryFrom<PresetFile> for Preset {
    type Error = BackendError;

    fn try_from(file: PresetFile) -> Result<Self, Self::Error> {
        let fail = |message: String| BackendError::Failed(format!("preset: {message}"));
        let name = validate_preset_name(&file.name).map_err(|error| fail(error.to_string()))?;
        let mut entries = Vec::with_capacity(file.entries.len());
        for entry in file.entries {
            let control: ControlKey = entry
                .control
                .parse()
                .map_err(|_| fail(format!("unknown control '{}'", entry.control)))?;
            let value = match (entry.kind.as_str(), control.is_numeric()) {
                ("normalized", true) if entry.value <= 100 => ControlValue::Normalized(entry.value),
                ("enum", false) => ControlValue::Enum(entry.value),
                _ => {
                    return Err(fail(format!(
                        "invalid {} value for '{}' in '{name}'",
                        entry.kind, entry.control
                    )));
                }
            };
            entries.push(PresetEntry {
                monitor: MonitorId::new(entry.monitor).map_err(|error| fail(error.to_string()))?,
                control,
                value,
            });
        }
        Ok(Preset { name, entries })
    }
}

/// One `[[hotkeys]]` entry: `keys = "Ctrl+Alt+Up"`, `action = "brightness+"`,
/// and an optional `monitor`.
#[derive(Serialize, Deserialize)]
struct HotkeyFile {
    keys: String,
    action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    monitor: Option<String>,
}

impl From<&HotkeyBinding> for HotkeyFile {
    fn from(binding: &HotkeyBinding) -> Self {
        Self {
            keys: binding.keys.to_string(),
            action: binding.action.to_string(),
            monitor: binding
                .monitor
                .as_ref()
                .map(|monitor| monitor.as_str().to_owned()),
        }
    }
}

impl TryFrom<HotkeyFile> for HotkeyBinding {
    type Error = BackendError;

    fn try_from(file: HotkeyFile) -> Result<Self, Self::Error> {
        let fail = |error: dispcontrol_domain::DomainError| {
            BackendError::Failed(format!("hotkey '{}': {error}", file.keys))
        };
        Ok(Self {
            keys: file.keys.parse().map_err(fail)?,
            action: file.action.parse().map_err(fail)?,
            monitor: file
                .monitor
                .as_deref()
                .map(MonitorId::new)
                .transpose()
                .map_err(fail)?,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct ConfigFile {
    #[serde(flatten)]
    settings: SettingsFile,
    #[serde(default)]
    presets: Vec<PresetFile>,
    #[serde(default)]
    hotkeys: Vec<HotkeyFile>,
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            settings: SettingsFile::from(AppSettings::default()),
            presets: Vec::new(),
            hotkeys: Vec::new(),
        }
    }
}

impl FileSettingsRepository {
    fn read_file(&self) -> Result<Option<ConfigFile>, BackendError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&self.path).map_err(io_error)?;
        toml::from_str(&content).map(Some).map_err(|error| {
            BackendError::Failed(format!("cannot parse {}: {error}", self.path.display()))
        })
    }

    fn write_file(&self, file: &ConfigFile) -> Result<(), BackendError> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| BackendError::Failed("settings path has no parent".into()))?;
        fs::create_dir_all(parent).map_err(io_error)?;
        let content = toml::to_string_pretty(file)
            .map_err(|error| BackendError::Failed(format!("cannot encode settings: {error}")))?;
        atomic_write(&self.path, content.as_bytes()).map_err(io_error)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Reads the file (or defaults), applies `change` and writes it back, so
    /// saving one section keeps the others. An unreadable file is not
    /// overwritten.
    fn update(&self, change: impl FnOnce(&mut ConfigFile)) -> Result<(), BackendError> {
        let _guard = self.lock();
        let mut file = self.read_file()?.unwrap_or_default();
        change(&mut file);
        self.write_file(&file)
    }
}

impl SettingsRepository for FileSettingsRepository {
    fn load(&self) -> Result<AppSettings, BackendError> {
        let _guard = self.lock();
        match self.read_file()? {
            Some(file) => AppSettings::try_from(file.settings),
            None => Ok(AppSettings::default()),
        }
    }

    fn save(&self, settings: &AppSettings) -> Result<(), BackendError> {
        validate(settings)?;
        self.update(|file| file.settings = SettingsFile::from(settings.clone()))
    }
}

impl PresetRepository for FileSettingsRepository {
    fn load_presets(&self) -> Result<Vec<Preset>, BackendError> {
        let _guard = self.lock();
        match self.read_file()? {
            Some(file) => file.presets.into_iter().map(Preset::try_from).collect(),
            None => Ok(Vec::new()),
        }
    }

    fn save_presets(&self, presets: &[Preset]) -> Result<(), BackendError> {
        self.update(|file| file.presets = presets.iter().map(PresetFile::from).collect())
    }

    fn export_text(&self, presets: &[Preset]) -> Result<String, BackendError> {
        let doc = PresetsDocument {
            version: 1,
            presets: presets.iter().map(PresetFile::from).collect(),
        };
        let body = toml::to_string_pretty(&doc)
            .map_err(|error| BackendError::Failed(format!("cannot encode presets: {error}")))?;
        Ok(format!("{EXPORT_HEADER}{body}"))
    }

    fn parse_text(&self, text: &str) -> Result<Vec<Preset>, BackendError> {
        let doc: PresetsDocument = toml::from_str(text)
            .map_err(|error| BackendError::Failed(format!("cannot parse presets: {error}")))?;
        if doc.version != 1 {
            return Err(BackendError::Failed(format!(
                "unsupported presets file version {}",
                doc.version
            )));
        }
        doc.presets.into_iter().map(Preset::try_from).collect()
    }

    fn location(&self) -> String {
        self.path.display().to_string()
    }
}

impl HotkeyRepository for FileSettingsRepository {
    fn load_hotkeys(&self) -> Result<Vec<HotkeyBinding>, BackendError> {
        let _guard = self.lock();
        match self.read_file()? {
            Some(file) => file
                .hotkeys
                .into_iter()
                .map(HotkeyBinding::try_from)
                .collect(),
            None => Ok(Vec::new()),
        }
    }

    fn save_hotkeys(&self, hotkeys: &[HotkeyBinding]) -> Result<(), BackendError> {
        self.update(|file| file.hotkeys = hotkeys.iter().map(HotkeyFile::from).collect())
    }
}

#[derive(Serialize, Deserialize)]
struct PresetsDocument {
    version: u32,
    #[serde(default)]
    presets: Vec<PresetFile>,
}

const EXPORT_HEADER: &str = "\
# dispcontrol presets - edit freely, then import with `dispcontrol preset import <file>`.
# Each [[presets]] block is one preset; each [[presets.entries]] block is one saved control.
#   monitor: monitor id (see `dispcontrol list`)
#   control: brightness | contrast | volume | gain-red | gain-green | gain-blue |
#            color-preset | input | power
#   kind:    \"normalized\" (percent, 0-100) for brightness, contrast, volume and gains;
#            \"enum\" (the monitor's native value, e.g. input 49 = 0x31 USB-C) for the others
#   color-preset: 1 sRGB, 2 native, 5 6500K, 6 7500K, 8 9300K, 11-13 user profiles, ...
#   gain-* entries only take effect with a user profile (11-13); with any other
#   color-preset they are ignored, because writing a gain selects the user profile.
# Preset names are case-insensitive and 1-40 characters.
";

fn validate(settings: &AppSettings) -> Result<(), BackendError> {
    settings
        .validate()
        .map_err(|message| BackendError::Failed(message.into()))
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

    #[test]
    fn presets_round_trip_and_settings_and_presets_preserve_each_other() {
        let unique = format!(
            "dispcontrol-presets-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        let repository = FileSettingsRepository::new(directory.join("config.toml"));
        assert!(repository.load_presets().unwrap().is_empty());

        let monitor = MonitorId::new("mon").unwrap();
        let presets = vec![Preset {
            name: "Night".into(),
            entries: vec![
                PresetEntry {
                    monitor: monitor.clone(),
                    control: ControlKey::Brightness,
                    value: ControlValue::Normalized(20),
                },
                PresetEntry {
                    monitor,
                    control: ControlKey::ColorPreset,
                    value: ControlValue::Enum(5),
                },
            ],
        }];
        repository.save_presets(&presets).unwrap();
        let settings = AppSettings {
            debounce_ms: 800,
            ..AppSettings::default()
        };
        repository.save(&settings).unwrap();

        assert_eq!(repository.load_presets().unwrap(), presets);
        assert_eq!(repository.load().unwrap(), settings);
        repository.save_presets(&[]).unwrap();
        assert_eq!(repository.load().unwrap(), settings);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn hotkeys_round_trip_and_every_section_survives_the_others_saves() {
        let directory = std::env::temp_dir().join(format!(
            "dispcontrol-hotkeys-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("config.toml");
        let repository = FileSettingsRepository::new(&path);
        let hotkeys = vec![
            HotkeyBinding {
                keys: "Ctrl+Alt+Up".parse().unwrap(),
                action: "brightness+".parse().unwrap(),
                monitor: None,
            },
            HotkeyBinding {
                keys: "F9".parse().unwrap(),
                action: "preset:Night mode".parse().unwrap(),
                monitor: Some(MonitorId::new("L32p-30#0").unwrap()),
            },
        ];
        repository.save_hotkeys(&hotkeys).unwrap();
        let settings = AppSettings {
            volume_step: 2,
            show_osd: false,
            ..AppSettings::default()
        };
        repository.save(&settings).unwrap();
        repository.save_presets(&[]).unwrap();

        assert_eq!(repository.load_hotkeys().unwrap(), hotkeys);
        assert_eq!(repository.load().unwrap(), settings);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("keys = \"Ctrl+Alt+Up\""));
        assert!(text.contains("action = \"preset:Night mode\""));
        repository.save_hotkeys(&[]).unwrap();
        assert_eq!(repository.load().unwrap(), settings);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn older_files_load_with_default_hotkey_settings_and_bad_hotkeys_are_rejected() {
        let old = "version = 1\ndebounce_ms = 600\nconfirm_input_change = true\ninput_revert_seconds = 15\nlive_preview = false\n";
        let file: ConfigFile = toml::from_str(old).unwrap();
        let settings = AppSettings::try_from(file.settings).unwrap();
        assert_eq!(settings.brightness_step, 5);
        assert!(settings.show_osd);
        assert!(file.hotkeys.is_empty());

        for (keys, action) in [("Up", "brightness+"), ("Ctrl+Up", "sharpness+")] {
            let entry = HotkeyFile {
                keys: keys.into(),
                action: action.into(),
                monitor: None,
            };
            assert!(HotkeyBinding::try_from(entry).is_err());
        }
        let steps = format!("{old}volume_step = 40\n");
        let file: ConfigFile = toml::from_str(&steps).unwrap();
        assert!(AppSettings::try_from(file.settings).is_err());
    }

    #[test]
    fn invalid_preset_values_are_rejected_on_load() {
        let text = "version = 1\ndebounce_ms = 600\nconfirm_input_change = true\ninput_revert_seconds = 15\nlive_preview = false\n[[presets]]\nname = \"Bad\"\n[[presets.entries]]\nmonitor = \"m\"\ncontrol = \"brightness\"\nkind = \"normalized\"\nvalue = 500\n";
        let file: ConfigFile = toml::from_str(text).unwrap();
        let result: Result<Vec<_>, _> = file.presets.into_iter().map(Preset::try_from).collect();
        assert!(result.is_err());
    }

    #[test]
    fn exported_presets_round_trip_and_reject_bad_documents() {
        let repository = FileSettingsRepository::new("unused.toml");
        let presets = vec![Preset {
            name: "Day".into(),
            entries: vec![PresetEntry {
                monitor: MonitorId::new("L32p-30#0").unwrap(),
                control: ControlKey::Brightness,
                value: ControlValue::Normalized(70),
            }],
        }];
        let text = repository.export_text(&presets).unwrap();
        assert!(text.starts_with("# dispcontrol presets"));
        assert_eq!(repository.parse_text(&text).unwrap(), presets);
        assert!(repository.parse_text("version = 2").is_err());
        assert!(repository.parse_text("not toml [").is_err());
        let bad = text.replace("value = 70", "value = 700");
        assert!(repository.parse_text(&bad).is_err());
    }
}
