//! Quarantine of unusable configuration files, and what loading reports.
//!
//! A configuration that cannot be interpreted safely is never deleted or
//! overwritten. It is moved aside to `runtime.json.corrupt-<unix time>` so the
//! user keeps it, and only then is the default (deny-everything)
//! configuration written in its place.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::schema::RuntimeConfig;

/// How many names to try before giving up on finding an unused one.
const MAX_QUARANTINE_NAME_ATTEMPTS: u32 = 100;

/// A configuration file that was set aside because it could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRecovery {
    /// Why the file could not be used.
    pub reason: String,
    /// Where the original file now is.
    pub quarantined_path: PathBuf,
}

/// Things worth knowing about after loading the configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigDiagnostics {
    /// Non-fatal observations, such as unrecognised entries that were kept.
    pub warnings: Vec<String>,
    /// Set when the stored configuration was unusable and was replaced by the
    /// default configuration.
    pub recovery: Option<ConfigRecovery>,
}

/// The loaded configuration together with its diagnostics.
#[derive(Debug, Clone)]
pub struct LoadReport {
    pub config: RuntimeConfig,
    pub diagnostics: ConfigDiagnostics,
}

/// Moves `path` aside and returns the new location.
pub(crate) fn quarantine(path: &Path) -> io::Result<PathBuf> {
    quarantine_with(path, unix_seconds(), |from, to| fs::rename(from, to))
}

/// `quarantine` with the time and the move supplied by the caller, so both can
/// be tested.
fn quarantine_with(
    path: &Path,
    timestamp: u64,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let target = unused_quarantine_path(path, timestamp)?;

    rename(path, &target).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not move {} to {}: {error}",
                path.display(),
                target.display()
            ),
        )
    })?;

    Ok(target)
}

/// `<file name>.corrupt-<timestamp>`, with `-<n>` added if that is taken. The
/// result never exists yet, so moving a file there cannot replace anything
/// (renaming over an existing file succeeds silently on Windows).
fn unused_quarantine_path(path: &Path, timestamp: u64) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_string());

    for attempt in 0..MAX_QUARANTINE_NAME_ATTEMPTS {
        let candidate = if attempt == 0 {
            format!("{name}.corrupt-{timestamp}")
        } else {
            format!("{name}.corrupt-{timestamp}-{attempt}")
        };

        let candidate = path.with_file_name(candidate);

        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "too many quarantined copies of {} already exist",
            path.display()
        ),
    ))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_file(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runtime.json");
        fs::write(&path, contents).unwrap();

        (directory, path)
    }

    fn entries(path: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();

        names.sort();
        names
    }

    #[test]
    fn moves_the_file_aside_and_keeps_its_contents() {
        let (_directory, path) = config_file("{ corrupt");

        let target = quarantine_with(&path, 1700000000, |from, to| fs::rename(from, to)).unwrap();

        assert_eq!(
            target,
            path.with_file_name("runtime.json.corrupt-1700000000")
        );
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "{ corrupt");
        assert_eq!(entries(&path), vec!["runtime.json.corrupt-1700000000"]);
    }

    #[test]
    fn quarantined_name_is_not_a_json_configuration() {
        let (_directory, path) = config_file("x");

        let target = quarantine_with(&path, 5, |from, to| fs::rename(from, to)).unwrap();

        assert_ne!(target.extension().and_then(|e| e.to_str()), Some("json"));
        assert_ne!(target, path);
    }

    #[test]
    fn never_overwrites_an_earlier_quarantined_file() {
        let (_directory, path) = config_file("second corrupt file");
        let first = path.with_file_name("runtime.json.corrupt-42");
        fs::write(&first, "first corrupt file").unwrap();

        let target = quarantine_with(&path, 42, |from, to| fs::rename(from, to)).unwrap();

        assert_eq!(target, path.with_file_name("runtime.json.corrupt-42-1"));
        assert_eq!(fs::read_to_string(&first).unwrap(), "first corrupt file");
        assert_eq!(fs::read_to_string(&target).unwrap(), "second corrupt file");
    }

    #[test]
    fn keeps_numbering_when_several_names_are_taken() {
        let (_directory, path) = config_file("latest");
        for name in [
            "runtime.json.corrupt-9",
            "runtime.json.corrupt-9-1",
            "runtime.json.corrupt-9-2",
        ] {
            fs::write(path.with_file_name(name), name).unwrap();
        }

        let target = quarantine_with(&path, 9, |from, to| fs::rename(from, to)).unwrap();

        assert_eq!(target, path.with_file_name("runtime.json.corrupt-9-3"));
        assert_eq!(
            fs::read_to_string(path.with_file_name("runtime.json.corrupt-9")).unwrap(),
            "runtime.json.corrupt-9"
        );
    }

    #[test]
    fn gives_up_when_every_name_is_taken() {
        let (_directory, path) = config_file("latest");
        fs::write(path.with_file_name("runtime.json.corrupt-1"), "a").unwrap();
        for attempt in 1..MAX_QUARANTINE_NAME_ATTEMPTS {
            fs::write(
                path.with_file_name(format!("runtime.json.corrupt-1-{attempt}")),
                "a",
            )
            .unwrap();
        }

        let error = quarantine_with(&path, 1, |from, to| fs::rename(from, to)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("too many quarantined copies"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "latest");
    }

    #[test]
    fn move_failure_is_reported_and_leaves_the_original_in_place() {
        let (_directory, path) = config_file("{ corrupt");

        let error = quarantine_with(&path, 7, |_from, _to| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file is locked",
            ))
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("could not move"));
        assert!(error.to_string().contains("runtime.json"));
        assert!(error.to_string().contains("corrupt-7"));
        assert!(error.to_string().contains("file is locked"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ corrupt");
        assert_eq!(entries(&path), vec!["runtime.json"]);
    }

    #[test]
    fn quarantining_a_missing_file_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runtime.json");

        let error = quarantine(&path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("could not move"));
    }

    #[test]
    fn real_quarantine_uses_the_current_time() {
        let (_directory, path) = config_file("{ corrupt");

        let target = quarantine(&path).unwrap();
        let name = target.file_name().unwrap().to_string_lossy().into_owned();
        let seconds: u64 = name
            .strip_prefix("runtime.json.corrupt-")
            .unwrap()
            .parse()
            .unwrap();

        assert!(seconds > 1_700_000_000);
        assert!(target.exists());
        assert!(!path.exists());
    }

    #[test]
    fn diagnostics_are_empty_by_default() {
        let diagnostics = ConfigDiagnostics::default();

        assert!(diagnostics.warnings.is_empty());
        assert!(diagnostics.recovery.is_none());
    }
}
