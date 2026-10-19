//! Retry only the read-only backup stage, before any synthetic copy is sent.

pub(crate) const SNAPSHOT_ATTEMPTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotStage {
    Open,
    SequenceChanged,
    CountFormats,
    EnumerateFormats,
    ReadFormat,
    ReadMetafile,
    LockMemory,
    SizeLimit,
    NoRestorableFormats,
}

impl SnapshotStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::SequenceChanged => "sequence-changed",
            Self::CountFormats => "count-formats",
            Self::EnumerateFormats => "enumerate-formats",
            Self::ReadFormat => "read-format",
            Self::ReadMetafile => "read-metafile",
            Self::LockMemory => "lock-memory",
            Self::SizeLimit => "size-limit",
            Self::NoRestorableFormats => "no-restorable-formats",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotError {
    pub(crate) stage: SnapshotStage,
    pub(crate) format: Option<u32>,
    pub(crate) native_error: Option<u32>,
    pub(crate) sequence: u32,
}

impl SnapshotError {
    pub(crate) fn retryable(self) -> bool {
        !matches!(
            self.stage,
            SnapshotStage::SizeLimit | SnapshotStage::NoRestorableFormats
        )
    }
}

/// `prepare` validates context/deadline before every attempt, including after
/// its retry delay. A failed snapshot must never be used as a partial backup.
pub(crate) fn retry_snapshot<T>(
    mut capture: impl FnMut() -> Result<T, SnapshotError>,
    mut prepare: impl FnMut(usize) -> bool,
    mut report: impl FnMut(usize, SnapshotError),
) -> Option<T> {
    for attempt in 0..SNAPSHOT_ATTEMPTS {
        if !prepare(attempt) {
            return None;
        }
        match capture() {
            Ok(snapshot) => return Some(snapshot),
            Err(error) => {
                report(attempt, error);
                if !error.retryable() {
                    return None;
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn failure(stage: SnapshotStage) -> SnapshotError {
        SnapshotError {
            stage,
            format: Some(13),
            native_error: Some(5),
            sequence: 42,
        }
    }

    #[test]
    fn successful_backup_is_returned_without_retry_or_error_report() {
        let preparations = RefCell::new(Vec::new());
        let snapshot = retry_snapshot(
            || Ok("original clipboard"),
            |attempt| {
                preparations.borrow_mut().push(attempt);
                true
            },
            |_, _| panic!("a successful backup must not report failure"),
        );
        assert_eq!(snapshot, Some("original clipboard"));
        assert_eq!(*preparations.borrow(), vec![0]);
    }

    #[test]
    fn transient_snapshot_failures_recover_before_copy_and_keep_error_details() {
        for stage in [
            SnapshotStage::Open,
            SnapshotStage::SequenceChanged,
            SnapshotStage::CountFormats,
            SnapshotStage::EnumerateFormats,
            SnapshotStage::ReadFormat,
            SnapshotStage::ReadMetafile,
            SnapshotStage::LockMemory,
        ] {
            let attempts = Cell::new(0);
            let preparations = RefCell::new(Vec::new());
            let errors = RefCell::new(Vec::new());
            let snapshot = retry_snapshot(
                || {
                    attempts.set(attempts.get() + 1);
                    if attempts.get() < 3 {
                        Err(failure(stage))
                    } else {
                        Ok("complete backup")
                    }
                },
                |attempt| {
                    preparations.borrow_mut().push(attempt);
                    true
                },
                |attempt, error| errors.borrow_mut().push((attempt, error)),
            );
            // The caller can inject Ctrl+C only after receiving this backup.
            assert_eq!(snapshot, Some("complete backup"));
            assert_eq!(*preparations.borrow(), vec![0, 1, 2]);
            assert_eq!(
                *errors.borrow(),
                vec![(0, failure(stage)), (1, failure(stage))]
            );
            assert!(!stage.as_str().is_empty());
        }
    }

    #[test]
    fn persistent_failure_is_bounded_and_does_not_authorize_copy() {
        let attempts = Cell::new(0);
        let snapshot: Option<()> = retry_snapshot(
            || {
                attempts.set(attempts.get() + 1);
                Err(failure(SnapshotStage::ReadFormat))
            },
            |_| true,
            |_, _| {},
        );
        assert!(snapshot.is_none());
        assert_eq!(attempts.get(), SNAPSHOT_ATTEMPTS);
    }

    #[test]
    fn unsupported_or_oversized_backups_are_terminal() {
        for stage in [SnapshotStage::SizeLimit, SnapshotStage::NoRestorableFormats] {
            let attempts = Cell::new(0);
            let snapshot: Option<()> = retry_snapshot(
                || {
                    attempts.set(attempts.get() + 1);
                    Err(failure(stage))
                },
                |_| true,
                |_, _| {},
            );
            assert!(snapshot.is_none());
            assert_eq!(attempts.get(), 1);
        }
    }

    #[test]
    fn cancellation_context_change_or_deadline_stops_before_the_next_snapshot() {
        for invalid_at in 0..SNAPSHOT_ATTEMPTS {
            let attempts = Cell::new(0);
            let snapshot: Option<()> = retry_snapshot(
                || {
                    attempts.set(attempts.get() + 1);
                    Err(failure(SnapshotStage::Open))
                },
                |attempt| attempt < invalid_at,
                |_, _| {},
            );
            assert!(snapshot.is_none());
            assert_eq!(attempts.get(), invalid_at);
        }
    }
}
