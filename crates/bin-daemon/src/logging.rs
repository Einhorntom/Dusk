//! The daemon's `log` sink (SPEC-NFR-3). Warnings and errors always go to
//! stderr (seen only when it is redirected: `duskd` has no console). The log file
//! is off by default; when on, it records info and above in
//! `%LOCALAPPDATA%\Dusk\logs`, rotating 5 files of 1 MB. The user's profile
//! path is replaced by `%USERPROFILE%` so the log holds no user name.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// The current file plus four rotated ones.
const KEPT_FILES: usize = 5;
const FILE_NAME: &str = "duskd";

pub struct DaemonLogger {
    file_enabled: AtomicBool,
    file: Mutex<RotatingFile>,
    profile: Option<String>,
}

impl DaemonLogger {
    pub fn new(directory: PathBuf, file_enabled: bool, profile: Option<String>) -> Self {
        Self {
            file_enabled: AtomicBool::new(file_enabled),
            file: Mutex::new(RotatingFile::new(directory)),
            profile: profile.filter(|path| !path.is_empty()),
        }
    }

    /// Installs `logger` as the process-wide sink and logs panics through it.
    pub fn install(logger: Self) -> &'static Self {
        let logger: &'static Self = Box::leak(Box::new(logger));
        if log::set_logger(logger).is_ok() {
            log::set_max_level(LevelFilter::Info);
        }
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!("panic: {info}");
            previous(info);
        }));
        logger
    }

    pub fn set_file_enabled(&self, enabled: bool) {
        self.file_enabled.store(enabled, Ordering::Relaxed);
    }

    fn redact(&self, text: String) -> String {
        match &self.profile {
            Some(profile) => replace_ignoring_ascii_case(&text, profile, "%USERPROFILE%"),
            None => text,
        }
    }
}

impl Log for DaemonLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = self.redact(record.args().to_string());
        let to_stderr = match record.level() {
            Level::Error => Some("error"),
            Level::Warn => Some("warning"),
            _ => None,
        };
        if let Some(prefix) = to_stderr {
            // A windowless process has no stderr; the write is then ignored.
            let _ = writeln!(io::stderr(), "{prefix}: {message}");
        }
        if self.file_enabled.load(Ordering::Relaxed) {
            let line = format!(
                "{} {:<5} {}: {message}\n",
                utc_timestamp(SystemTime::now()),
                record.level(),
                record.target()
            );
            let mut file = self
                .file
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Err(error) = file.append(line.as_bytes()) {
                let _ = writeln!(io::stderr(), "warning: cannot write the log file: {error}");
            }
        }
    }

    fn flush(&self) {}
}

/// `duskd.log` plus `duskd.1.log` (newest) to `duskd.4.log` (oldest).
struct RotatingFile {
    directory: PathBuf,
    open: Option<(File, u64)>,
}

impl RotatingFile {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            open: None,
        }
    }

    fn path(&self, index: usize) -> PathBuf {
        rotated_path(&self.directory, index)
    }

    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        if self.open.is_none() {
            fs::create_dir_all(&self.directory)?;
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.path(0))?;
            let size = file.metadata()?.len();
            self.open = Some((file, size));
        }
        if let Some((_, size)) = &self.open
            && *size > 0
            && size + line.len() as u64 > MAX_FILE_BYTES
        {
            self.open = None;
            self.rotate()?;
            return self.append(line);
        }
        let (file, size) = self.open.as_mut().expect("opened above");
        file.write_all(line)?;
        *size += line.len() as u64;
        Ok(())
    }

    fn rotate(&self) -> io::Result<()> {
        let oldest = self.path(KEPT_FILES - 1);
        if oldest.exists() {
            fs::remove_file(oldest)?;
        }
        for index in (0..KEPT_FILES - 1).rev() {
            let from = self.path(index);
            if from.exists() {
                fs::rename(from, self.path(index + 1))?;
            }
        }
        Ok(())
    }
}

fn rotated_path(directory: &Path, index: usize) -> PathBuf {
    if index == 0 {
        directory.join(format!("{FILE_NAME}.log"))
    } else {
        directory.join(format!("{FILE_NAME}.{index}.log"))
    }
}

/// Windows paths are case-insensitive; ASCII lowercasing keeps byte offsets.
fn replace_ignoring_ascii_case(text: &str, pattern: &str, replacement: &str) -> String {
    let haystack = text.to_ascii_lowercase();
    let needle = pattern.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut rest = 0;
    while let Some(found) = haystack[rest..].find(&needle) {
        let start = rest + found;
        result.push_str(&text[rest..start]);
        result.push_str(replacement);
        rest = start + needle.len();
    }
    result.push_str(&text[rest..]);
    result
}

/// `2026-10-06T14:03:22Z` (UTC, so no time-zone lookup is needed).
fn utc_timestamp(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temporary_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "dusk-log-test-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn timestamps_are_utc_calendar_dates() {
        assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        // 2024-02-29 (leap day) 13:45:07 UTC.
        let leap_day = UNIX_EPOCH + Duration::from_secs(1_709_214_307);
        assert_eq!(utc_timestamp(leap_day), "2024-02-29T13:45:07Z");
        let end_of_year = UNIX_EPOCH + Duration::from_secs(1_798_761_599);
        assert_eq!(utc_timestamp(end_of_year), "2026-12-31T23:59:59Z");
    }

    #[test]
    fn the_profile_path_is_redacted_in_any_letter_case() {
        let logger = DaemonLogger::new(PathBuf::new(), false, Some(r"C:\Users\Alex".into()));
        assert_eq!(
            logger.redact(r"read C:\Users\Alex\AppData and c:\users\alex\x".into()),
            r"read %USERPROFILE%\AppData and %USERPROFILE%\x"
        );
        let no_profile = DaemonLogger::new(PathBuf::new(), false, Some(String::new()));
        assert_eq!(no_profile.redact("unchanged".into()), "unchanged");
    }

    #[test]
    fn the_log_rotates_and_keeps_five_files_of_at_most_one_megabyte() {
        let directory = temporary_directory("rotate");
        let mut file = RotatingFile::new(directory.clone());
        let line = vec![b'x'; 300 * 1024];
        // 3 lines fit in 1 MB, so 20 lines fill 7 files; the oldest 2 are dropped.
        for _ in 0..20 {
            file.append(&line).unwrap();
        }
        let mut names: Vec<String> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "duskd.1.log",
                "duskd.2.log",
                "duskd.3.log",
                "duskd.4.log",
                "duskd.log"
            ]
        );
        for name in &names {
            let size = fs::metadata(directory.join(name)).unwrap().len();
            assert!(size <= MAX_FILE_BYTES, "{name} is {size} bytes");
        }
        // 20 lines = 6 full files + 2 lines in the current one.
        assert_eq!(
            fs::metadata(rotated_path(&directory, 0)).unwrap().len(),
            2 * 300 * 1024
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_file_is_written_only_while_enabled() {
        let directory = temporary_directory("toggle");
        let logger = DaemonLogger::new(directory.clone(), false, None);
        let record = |message: &str| {
            logger.log(
                &Record::builder()
                    .level(Level::Info)
                    .target("dusk_test")
                    .args(format_args!("{message}"))
                    .build(),
            );
        };
        record("before");
        assert!(!rotated_path(&directory, 0).exists());
        logger.set_file_enabled(true);
        record("while on");
        logger.set_file_enabled(false);
        record("after");
        let text = fs::read_to_string(rotated_path(&directory, 0)).unwrap();
        assert!(text.contains("INFO  dusk_test: while on"), "{text}");
        assert!(!text.contains("before") && !text.contains("after"));
        fs::remove_dir_all(directory).unwrap();
    }
}
