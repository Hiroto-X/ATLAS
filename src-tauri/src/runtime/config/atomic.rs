//! Crash-safe file replacement for `runtime.json`.
//!
//! `atomic_write` writes the complete contents to a uniquely named temporary
//! file next to the destination, flushes it to disk, and only then moves it
//! over the destination. The destination therefore holds either the old
//! contents or the new contents, never a partial write, and a failure at any
//! step leaves the old file untouched.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

/// Suffix of temporary files. It is intentionally not `.json`, so an
/// incomplete temporary file can never be mistaken for a configuration.
const TEMP_SUFFIX: &str = "tmp";

/// How many unique names to try if a temporary file already exists, for
/// example a leftover from an earlier process that had the same process id.
const MAX_TEMP_NAME_ATTEMPTS: usize = 8;

/// Number of attempts to move the temporary file into place. On Windows the
/// move can fail briefly while antivirus, backup or indexing software has the
/// destination open, so it is retried a few times. Elsewhere it is not.
#[cfg(windows)]
const REPLACE_ATTEMPTS: u32 = 5;
#[cfg(not(windows))]
const REPLACE_ATTEMPTS: u32 = 1;

const REPLACE_RETRY_DELAY: Duration = Duration::from_millis(25);

/// Distinguishes temporary files created by the same process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Atomically replaces `path` with `contents`.
///
/// * The temporary file is created with `create_new`, so an existing file is
///   never truncated or reused, and concurrent writers never share one.
/// * The full contents are written and `sync_all` is called before the
///   destination is replaced.
/// * If anything fails, the temporary file is removed and the original
///   `path` is left exactly as it was.
///
/// The directory of `path` must already exist.
pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    atomic_write_with(path, contents, replace_file)
}

/// `atomic_write` with the final replace step supplied by the caller, so that
/// failures of that step can be tested.
fn atomic_write_with(
    path: &Path,
    contents: &[u8],
    replace: impl Fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let (temp_path, file) = create_temp_file(|| unique_temp_path(path))
        .map_err(|error| with_context(error, "create temporary file for", path))?;

    let result = write_and_sync(file, contents)
        .map_err(|error| with_context(error, "write temporary file for", path))
        .and_then(|()| {
            replace(&temp_path, path).map_err(|error| with_context(error, "replace", path))
        });

    if result.is_err() {
        // Never leave a temporary file behind. A failure to remove it is not
        // worth hiding the original error.
        let _ = fs::remove_file(&temp_path);
    }

    result
}

/// Writes everything and flushes it to disk. The file is closed on return,
/// which Windows requires before the file can be renamed.
fn write_and_sync(mut file: File, contents: &[u8]) -> io::Result<()> {
    file.write_all(contents)?;
    file.sync_all()
}

/// Creates a new file using names from `next_path`, never opening a file that
/// already exists.
fn create_temp_file(mut next_path: impl FnMut() -> PathBuf) -> io::Result<(PathBuf, File)> {
    let mut last_error = None;

    for _ in 0..MAX_TEMP_NAME_ATTEMPTS {
        let temp_path = next_path();

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.unwrap_or_else(|| io::Error::from(io::ErrorKind::AlreadyExists)))
}

/// `<file name>.<process id>.<counter>.tmp`, in the same directory as `path`
/// so the final move never crosses a filesystem boundary.
fn unique_temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_string());

    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);

    path.with_file_name(format!(
        "{name}.{}.{counter}.{TEMP_SUFFIX}",
        std::process::id()
    ))
}

/// Moves `temp_path` over `path`.
///
/// `fs::rename` replaces an existing destination on both Unix and Windows
/// (`MOVEFILE_REPLACE_EXISTING`). On Windows it can fail transiently when
/// another process has the destination open, so that case is retried.
fn replace_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    retry_transient(
        REPLACE_ATTEMPTS,
        REPLACE_RETRY_DELAY,
        is_transient_replace_error,
        || fs::rename(temp_path, path),
    )
}

/// Errors worth retrying when replacing a file on Windows: access denied (5),
/// sharing violation (32) and lock violation (33).
fn is_transient_replace_error(error: &io::Error) -> bool {
    cfg!(windows)
        && (error.kind() == io::ErrorKind::PermissionDenied
            || matches!(error.raw_os_error(), Some(5 | 32 | 33)))
}

/// Runs `operation` up to `attempts` times, waiting `delay` between attempts,
/// but only while the failure is transient. The last error is returned.
fn retry_transient<T>(
    attempts: u32,
    delay: Duration,
    is_transient: impl Fn(&io::Error) -> bool,
    mut operation: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut attempt = 1;

    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempt < attempts && is_transient(&error) => {
                attempt += 1;
                thread::sleep(delay);
            }
            Err(error) => return Err(error),
        }
    }
}

/// Keeps the original error kind and adds what was being attempted.
fn with_context(error: io::Error, action: &str, path: &Path) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("failed to {action} {}: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::HashSet, sync::Arc, thread};

    fn target() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runtime.json");

        (directory, path)
    }

    /// Names of all entries in the directory of `path`.
    fn directory_entries(path: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();

        names.sort();
        names
    }

    fn leftover_temp_files(path: &Path) -> Vec<String> {
        directory_entries(path)
            .into_iter()
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn writes_a_new_file() {
        let (_directory, path) = target();

        atomic_write(&path, b"{\"a\":1}").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"{\"a\":1}");
        assert_eq!(directory_entries(&path), vec!["runtime.json"]);
    }

    #[test]
    fn replaces_an_existing_file_completely() {
        let (_directory, path) = target();
        fs::write(
            &path,
            "a much longer original configuration than the new one",
        )
        .unwrap();

        atomic_write(&path, b"short").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"short");
        assert_eq!(directory_entries(&path), vec!["runtime.json"]);
    }

    #[test]
    fn writes_empty_contents() {
        let (_directory, path) = target();
        fs::write(&path, "old").unwrap();

        atomic_write(&path, b"").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"");
    }

    #[test]
    fn repeated_writes_leave_only_the_destination() {
        let (_directory, path) = target();

        for index in 0..20 {
            atomic_write(&path, format!("{{\"n\":{index}}}").as_bytes()).unwrap();
        }

        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"n\":19}");
        assert_eq!(directory_entries(&path), vec!["runtime.json"]);
    }

    #[test]
    fn failure_to_create_the_temporary_file_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing").join("runtime.json");

        let error = atomic_write(&path, b"data").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error
            .to_string()
            .contains("failed to create temporary file for"));
        assert!(error.to_string().contains("runtime.json"));
        assert!(!path.exists());
        assert!(!directory.path().join("missing").exists());
    }

    #[test]
    fn replacement_failure_preserves_the_original_and_removes_the_temporary_file() {
        let (_directory, path) = target();
        fs::write(&path, "original configuration").unwrap();

        let error = atomic_write_with(&path, b"new configuration", |_temp, _path| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "destination is locked",
            ))
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("failed to replace"));
        assert!(error.to_string().contains("destination is locked"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "original configuration");
        assert!(leftover_temp_files(&path).is_empty());
        assert_eq!(directory_entries(&path), vec!["runtime.json"]);
    }

    #[test]
    fn replacement_failure_without_an_original_leaves_nothing_behind() {
        let (_directory, path) = target();

        let result = atomic_write_with(&path, b"data", |_temp, _path| {
            Err(io::Error::other("rename failed"))
        });

        assert!(result.is_err());
        assert!(!path.exists());
        assert!(directory_entries(&path).is_empty());
    }

    #[test]
    fn temporary_file_is_fully_written_and_closed_before_replacement() {
        let (_directory, path) = target();
        let saw_complete_contents = Cell::new(false);

        atomic_write_with(&path, b"complete contents", |temp, destination| {
            // At this point the temporary file must already hold everything,
            // and the destination must still be untouched.
            saw_complete_contents.set(fs::read(temp).unwrap() == b"complete contents");
            assert!(!destination.exists());

            fs::rename(temp, destination)
        })
        .unwrap();

        assert!(saw_complete_contents.get());
        assert_eq!(fs::read(&path).unwrap(), b"complete contents");
    }

    #[test]
    fn destination_is_unchanged_while_the_temporary_file_exists() {
        let (_directory, path) = target();
        fs::write(&path, "old").unwrap();

        atomic_write_with(&path, b"new", |temp, destination| {
            assert_eq!(fs::read_to_string(destination).unwrap(), "old");
            assert_eq!(fs::read_to_string(temp).unwrap(), "new");

            fs::rename(temp, destination)
        })
        .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn temporary_file_names_are_unique() {
        let path = Path::new("dir").join("runtime.json");

        let names: HashSet<PathBuf> = (0..1000).map(|_| unique_temp_path(&path)).collect();

        assert_eq!(names.len(), 1000);
    }

    #[test]
    fn temporary_file_names_are_unique_across_threads() {
        let path = Arc::new(Path::new("dir").join("runtime.json"));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = Arc::clone(&path);
                thread::spawn(move || {
                    (0..200)
                        .map(|_| unique_temp_path(&path))
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        let mut names = HashSet::new();
        let mut total = 0;

        for handle in handles {
            for name in handle.join().unwrap() {
                total += 1;
                names.insert(name);
            }
        }

        assert_eq!(names.len(), total);
    }

    #[test]
    fn temporary_file_is_beside_the_destination_and_is_not_json() {
        let path = Path::new("some").join("dir").join("runtime.json");

        let temp = unique_temp_path(&path);
        let name = temp.file_name().unwrap().to_string_lossy().into_owned();

        assert_eq!(temp.parent(), path.parent());
        assert!(name.starts_with("runtime.json."));
        assert!(name.ends_with(".tmp"));
        assert_ne!(temp.extension().and_then(|e| e.to_str()), Some("json"));
        assert_ne!(temp, path);
    }

    #[test]
    fn existing_file_with_the_chosen_name_is_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let taken = directory.path().join("taken.tmp");
        let free = directory.path().join("free.tmp");
        fs::write(&taken, "someone else's data").unwrap();

        let names = [taken.clone(), free.clone()];
        let next = Cell::new(0);

        let (chosen, _file) = create_temp_file(|| {
            let name = names[next.get()].clone();
            next.set(next.get() + 1);
            name
        })
        .unwrap();

        assert_eq!(chosen, free);
        assert_eq!(fs::read_to_string(&taken).unwrap(), "someone else's data");
    }

    #[test]
    fn gives_up_when_every_temporary_name_is_taken() {
        let directory = tempfile::tempdir().unwrap();
        let taken = directory.path().join("taken.tmp");
        fs::write(&taken, "data").unwrap();

        let error = match create_temp_file(|| taken.clone()) {
            Ok(_) => panic!("must not open an existing file"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&taken).unwrap(), "data");
    }

    #[test]
    fn leftover_temporary_file_from_a_crash_is_not_a_configuration() {
        let (_directory, path) = target();
        fs::write(&path, "valid configuration").unwrap();
        // A crashed writer left a partial temporary file behind.
        fs::write(path.with_file_name("runtime.json.4242.0.tmp"), "{\"half").unwrap();

        atomic_write(&path, b"newer configuration").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "newer configuration");
        assert_eq!(
            fs::read_to_string(path.with_file_name("runtime.json.4242.0.tmp")).unwrap(),
            "{\"half"
        );
    }

    #[test]
    fn concurrent_writers_never_produce_a_partial_file() {
        let (_directory, path) = target();
        let path = Arc::new(path);

        let payloads: Vec<String> = (0..8)
            .map(|writer| {
                format!(
                    "{{\"writer\":{writer},\"padding\":\"{}\"}}",
                    "x".repeat(4096)
                )
            })
            .collect();

        let handles: Vec<_> = payloads
            .iter()
            .cloned()
            .map(|payload| {
                let path = Arc::clone(&path);
                thread::spawn(move || {
                    (0..25)
                        .map(|_| atomic_write(&path, payload.as_bytes()))
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        let mut results = Vec::new();
        for handle in handles {
            results.extend(handle.join().unwrap());
        }

        // The file is always exactly one writer's complete payload.
        let final_contents = fs::read_to_string(&*path).unwrap();
        assert!(payloads.contains(&final_contents));

        // Writers never trip over each other's temporary files.
        assert!(leftover_temp_files(&path).is_empty());
        assert_eq!(directory_entries(&path), vec!["runtime.json"]);

        // On Windows another writer can briefly hold the destination open, in
        // which case a write may report an error; it still must not corrupt
        // anything. Elsewhere every write succeeds.
        #[cfg(not(windows))]
        assert!(results.iter().all(|result| result.is_ok()));
        #[cfg(windows)]
        assert!(results.iter().any(|result| result.is_ok()));
    }

    #[test]
    fn readers_never_see_a_partial_file_during_concurrent_writes() {
        let (_directory, path) = target();
        let first = format!("{{\"v\":1,\"padding\":\"{}\"}}", "a".repeat(8192));
        let second = format!("{{\"v\":2,\"padding\":\"{}\"}}", "b".repeat(8192));
        atomic_write(&path, first.as_bytes()).unwrap();

        let path = Arc::new(path);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let writer = {
            let (path, stop) = (Arc::clone(&path), Arc::clone(&stop));
            let (first, second) = (first.clone(), second.clone());
            thread::spawn(move || {
                for index in 0..100 {
                    let payload = if index % 2 == 0 { &second } else { &first };
                    let _ = atomic_write(&path, payload.as_bytes());
                }
                stop.store(true, Ordering::SeqCst);
            })
        };

        let mut reads = 0;
        while !stop.load(Ordering::SeqCst) {
            // Reads can fail transiently on Windows during a replace; a
            // successful read must always be one complete version.
            if let Ok(contents) = fs::read_to_string(&*path) {
                assert!(contents == first || contents == second, "partial read");
                reads += 1;
            }
        }

        writer.join().unwrap();
        assert!(reads > 0 || cfg!(windows));
    }

    #[test]
    fn retry_succeeds_after_transient_failures() {
        let calls = Cell::new(0);

        let result = retry_transient(
            5,
            Duration::ZERO,
            |_| true,
            || {
                calls.set(calls.get() + 1);

                if calls.get() < 3 {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    Ok("done")
                }
            },
        );

        assert_eq!(result.unwrap(), "done");
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn retry_stops_after_the_attempt_limit_and_returns_the_last_error() {
        let calls = Cell::new(0);

        let error = retry_transient(
            3,
            Duration::ZERO,
            |_| true,
            || -> io::Result<()> {
                calls.set(calls.get() + 1);
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("attempt {}", calls.get()),
                ))
            },
        )
        .unwrap_err();

        assert_eq!(calls.get(), 3);
        assert!(error.to_string().contains("attempt 3"));
    }

    #[test]
    fn retry_does_not_repeat_non_transient_errors() {
        let calls = Cell::new(0);

        let error = retry_transient(
            5,
            Duration::ZERO,
            |_| false,
            || -> io::Result<()> {
                calls.set(calls.get() + 1);
                Err(io::Error::from(io::ErrorKind::NotFound))
            },
        )
        .unwrap_err();

        assert_eq!(calls.get(), 1);
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_single_attempt_never_retries() {
        let calls = Cell::new(0);

        let _ = retry_transient(
            1,
            Duration::ZERO,
            |_| true,
            || -> io::Result<()> {
                calls.set(calls.get() + 1);
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            },
        );

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn transient_error_classification_matches_the_platform() {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        let missing = io::Error::from(io::ErrorKind::NotFound);

        assert_eq!(is_transient_replace_error(&denied), cfg!(windows));
        assert!(!is_transient_replace_error(&missing));
    }

    #[test]
    fn replace_attempts_are_only_used_on_windows() {
        assert_eq!(REPLACE_ATTEMPTS > 1, cfg!(windows));
    }

    #[test]
    fn errors_keep_their_kind_and_name_the_file() {
        let error = with_context(
            io::Error::from(io::ErrorKind::PermissionDenied),
            "replace",
            Path::new("dir/runtime.json"),
        );

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("failed to replace"));
        assert!(error.to_string().contains("runtime.json"));
    }
}
