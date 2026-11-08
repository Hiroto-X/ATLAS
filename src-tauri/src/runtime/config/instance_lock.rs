//! Keeps two ATLAS runtimes from using the same configuration at once.
//!
//! A mutex only protects against threads inside one process. Two ATLAS
//! processes would each load `runtime.json`, then overwrite each other's
//! permission decisions. This takes an exclusive operating system lock on a
//! file next to the configuration, and holds it for as long as the runtime
//! lives. The system releases it when the holder exits, even after a crash, so
//! there is no stale lock to clean up.

use std::{
    fmt,
    fs::{File, OpenOptions, TryLockError},
    io,
    path::Path,
};

/// File name of the lock, next to `runtime.json`. It is never deleted: the
/// lock is on the open file, not on its existence.
pub const LOCK_FILE_NAME: &str = "runtime.lock";

#[derive(Debug)]
pub enum InstanceLockError {
    /// Another runtime, in this process or another, holds the lock.
    AlreadyRunning,
    /// The lock file could not be opened or locked.
    Io(io::Error),
}

impl fmt::Display for InstanceLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstanceLockError::AlreadyRunning => {
                write!(
                    f,
                    "another ATLAS runtime is already using this configuration"
                )
            }
            InstanceLockError::Io(error) => write!(f, "could not lock the configuration: {error}"),
        }
    }
}

impl std::error::Error for InstanceLockError {}

/// An exclusive lock, held until it is dropped.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    /// Takes the lock without waiting. The directory of `path` must exist.
    pub fn acquire(path: &Path) -> Result<Self, InstanceLockError> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(InstanceLockError::Io)?;

        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(InstanceLockError::AlreadyRunning),
            Err(TryLockError::Error(error)) => Err(InstanceLockError::Io(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn lock_path() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOCK_FILE_NAME);

        (directory, path)
    }

    #[test]
    fn first_lock_is_acquired() {
        let (_directory, path) = lock_path();

        assert!(InstanceLock::acquire(&path).is_ok());
        assert!(path.exists());
    }

    #[test]
    fn second_lock_on_the_same_file_is_refused() {
        let (_directory, path) = lock_path();
        let _first = InstanceLock::acquire(&path).unwrap();

        let second = InstanceLock::acquire(&path);

        assert!(matches!(second, Err(InstanceLockError::AlreadyRunning)));
    }

    #[test]
    fn refused_lock_does_not_disturb_the_holder() {
        let (_directory, path) = lock_path();
        let _first = InstanceLock::acquire(&path).unwrap();

        for _ in 0..5 {
            assert!(matches!(
                InstanceLock::acquire(&path),
                Err(InstanceLockError::AlreadyRunning)
            ));
        }
    }

    #[test]
    fn lock_is_released_when_dropped() {
        let (_directory, path) = lock_path();

        let first = InstanceLock::acquire(&path).unwrap();
        drop(first);

        assert!(InstanceLock::acquire(&path).is_ok());
    }

    #[test]
    fn lock_file_is_kept_after_release() {
        let (_directory, path) = lock_path();

        drop(InstanceLock::acquire(&path).unwrap());

        assert!(path.exists());
    }

    #[test]
    fn different_files_do_not_conflict() {
        let directory = tempfile::tempdir().unwrap();
        let _first = InstanceLock::acquire(&directory.path().join("a.lock")).unwrap();

        assert!(InstanceLock::acquire(&directory.path().join("b.lock")).is_ok());
    }

    #[test]
    fn lock_does_not_change_existing_contents() {
        let (_directory, path) = lock_path();
        fs::write(&path, "left alone").unwrap();

        let lock = InstanceLock::acquire(&path).unwrap();
        // On Windows the lock is mandatory and blocks reads through other
        // handles, so release it before reading the file back.
        drop(lock);

        assert_eq!(fs::read_to_string(&path).unwrap(), "left alone");
    }

    #[test]
    fn missing_directory_is_an_io_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing").join(LOCK_FILE_NAME);

        match InstanceLock::acquire(&path) {
            Err(InstanceLockError::Io(error)) => {
                assert_eq!(error.kind(), io::ErrorKind::NotFound)
            }
            other => panic!("expected an io error, got {other:?}"),
        }
    }

    #[test]
    fn errors_are_readable() {
        assert!(InstanceLockError::AlreadyRunning
            .to_string()
            .contains("already using"));
        assert!(InstanceLockError::Io(io::Error::other("boom"))
            .to_string()
            .contains("boom"));
    }

    #[test]
    fn many_threads_competing_yield_exactly_one_holder() {
        let (_directory, path) = lock_path();
        let path = std::sync::Arc::new(path);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (path, barrier) = (path.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    let lock = InstanceLock::acquire(&path);
                    // Hold any acquired lock until every thread has tried.
                    barrier.wait();
                    lock.is_ok()
                })
            })
            .collect();

        let winners = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|won| *won)
            .count();

        assert_eq!(winners, 1);
    }
}
