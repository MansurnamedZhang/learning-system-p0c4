//! Synchronous fixed Capture entry and its permanently rooted original task.
use crate::BackupError;
use learning_db::snapshot_broker::B1InstalledSourceProcess;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::{future::Future, task::Poll};
use tokio::{
    sync::{Mutex, oneshot},
    task::JoinHandle,
};

static ORIGINAL: OnceLock<OriginalCaptureBook> = OnceLock::new();

#[derive(Clone, Copy)]
struct TaskExit {
    operation_ok: bool,
    db_settled: bool,
}

struct TaskCell {
    handle: Option<JoinHandle<TaskExit>>,
    joined: Option<TaskExit>,
}

impl TaskCell {
    async fn join(&mut self) -> TaskExit {
        if let Some(exit) = self.joined {
            return exit;
        }
        let exit = match self.handle.as_mut() {
            Some(handle) => match handle.await {
                Ok(exit) => exit,
                Err(_) => TaskExit {
                    operation_ok: false,
                    db_settled: false,
                },
            },
            None => TaskExit {
                operation_ok: false,
                db_settled: false,
            },
        };
        // A cancelled borrowed await leaves handle in this same cell. Only a
        // real returned JoinHandle result permits its removal here.
        self.handle = None;
        self.joined = Some(exit);
        exit
    }
}

struct Interest {
    negative: AtomicBool,
}
impl Interest {
    fn new() -> Self {
        Self {
            negative: AtomicBool::new(false),
        }
    }
    fn fail(&self) {
        self.negative.store(true, Ordering::Release);
    }
    fn check(&self) -> Result<(), BackupError> {
        if self.negative.load(Ordering::Acquire) {
            Err(BackupError::Invalid(
                "B1 Capture result permanently abandoned",
            ))
        } else {
            Ok(())
        }
    }
}

struct BorrowedResult<'a> {
    interest: &'a Interest,
    complete: bool,
}
impl Drop for BorrowedResult<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.interest.fail();
        }
    }
}

/// This container is in the original static Book. In particular it is not a
/// local in a spawned future. Clearing requires the actual entry's completed
/// natural cleanup path; losing a work future cannot clear it.
pub(super) struct RetainedOriginal<T> {
    value: Option<T>,
}
impl<T> RetainedOriginal<T> {
    fn new(value: T) -> Self {
        Self { value: Some(value) }
    }
    pub(super) fn borrow_mut(&mut self) -> Result<&mut T, BackupError> {
        self.value.as_mut().ok_or_else(invalid)
    }
    fn clear_after_natural_settlement(&mut self) {
        self.value = None;
    }
}

async fn observe_borrowed<F: Future>(future: F) -> Result<F::Output, ()> {
    // Inline pinning keeps this complete borrowed-future storage in the
    // original task allocation prepaid before spawn, without a later Box.
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
            Err(_) => Poll::Ready(Err(())),
        }
    })
    .await
}

struct OriginalCaptureBook {
    peer: Arc<B1InstalledSourceProcess>,
    interest: Interest,
    cell: Mutex<TaskCell>,
    resources: Mutex<RetainedOriginal<super::capture_job::OriginalCaptureOwners>>,
    started: AtomicBool,
}

impl OriginalCaptureBook {
    fn start(&'static self) -> Result<(), BackupError> {
        if self.started.swap(true, Ordering::AcqRel) {
            self.interest.fail();
            return Err(invalid());
        }
        let mut cell = self.cell.try_lock().map_err(|_| invalid())?;
        let (start, barrier) = oneshot::channel();
        let peer = Arc::clone(&self.peer);
        let driver = async move {
            // No runtime work/options/helper can run before its actual handle
            // was put in the original cell and registration released the gate.
            let operation_ok = if barrier.await.is_ok() {
                match observe_borrowed(super::capture_job::run_original_capture_job(Arc::clone(
                    &peer,
                )))
                .await
                {
                    Ok(Ok(())) => true,
                    _ => {
                        self.interest.fail();
                        false
                    }
                }
            } else {
                false
            };
            let db_settled = self.cleanup_original().await;
            TaskExit {
                operation_ok: operation_ok && self.interest.check().is_ok(),
                db_settled,
            }
        };
        self.peer
            .scan_accounting_meter()
            .charge((std::mem::size_of_val(&driver) + 8192) as u64, 2)?;
        cell.handle = Some(tokio::spawn(driver));
        // These are the original handle and original barrier; there is no
        // callback, detached future or result published before registration.
        if start.send(()).is_err() {
            self.interest.fail();
            return Err(invalid());
        }
        Ok(())
    }
    async fn wait(&self) -> TaskExit {
        let mut waiting = BorrowedResult {
            interest: &self.interest,
            complete: false,
        };
        let exit = self.cell.lock().await.join().await;
        if exit.operation_ok && exit.db_settled {
            waiting.complete = true;
        }
        exit
    }
    async fn cleanup_original(&self) -> bool {
        // These futures borrow the same retained resources. Catching an unwind
        // here is safe because no complete original object is in either future.
        let helpers = observe_borrowed(async {
            self.resources
                .lock()
                .await
                .borrow_mut()?
                .settle_helpers_negative()
                .await
        })
        .await;
        let db = observe_borrowed(self.peer.close_original()).await;
        matches!(helpers, Ok(Ok(()))) && matches!(db, Ok(Ok(())))
    }
}

fn invalid() -> BackupError {
    BackupError::Invalid("B1 original Capture owner rejected")
}

pub(super) fn require_original_interest(
    peer: &Arc<B1InstalledSourceProcess>,
) -> Result<(), BackupError> {
    let original = ORIGINAL.get().ok_or_else(invalid)?;
    if !Arc::ptr_eq(peer, &original.peer) {
        original.interest.fail();
        return Err(invalid());
    }
    original.interest.check()?;
    peer.revalidate().map_err(|_| invalid())
}

pub(super) fn original_phase_owners(
    peer: &Arc<B1InstalledSourceProcess>,
) -> Result<&'static Mutex<RetainedOriginal<super::capture_job::OriginalCaptureOwners>>, BackupError>
{
    let original = ORIGINAL.get().ok_or_else(invalid)?;
    if !Arc::ptr_eq(peer, &original.peer) {
        original.interest.fail();
        return Err(invalid());
    }
    original.interest.check()?;
    Ok(&original.resources)
}

pub(super) fn run_fixed_capture_entry() -> Result<(), BackupError> {
    // The factory itself burns once and completes the synchronous CLOEXEC,
    // actual ELF/native/environment/Root gates. Nothing fallible runs first.
    let admitted = B1InstalledSourceProcess::admit_installed().map_err(|_| invalid())?;
    admitted.scan_accounting_meter().charge(
        (std::mem::size_of::<OriginalCaptureBook>() + 4096) as u64,
        4,
    )?;
    let original = OriginalCaptureBook {
        peer: Arc::new(admitted),
        interest: Interest::new(),
        started: AtomicBool::new(false),
        cell: Mutex::new(TaskCell {
            handle: None,
            joined: None,
        }),
        resources: Mutex::new(RetainedOriginal::new(
            super::capture_job::OriginalCaptureOwners::empty(),
        )),
    };
    if ORIGINAL.set(original).is_err() {
        return Err(invalid());
    }
    let book = ORIGINAL.get().ok_or_else(invalid)?;
    // Runtime creation and all SQLx options happen strictly after qualification.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let started = book.start();
        // Registration can fail after spawn only while releasing its barrier.
        // Always visit the original cell, including that negative path.
        let exit = book.wait().await;
        // Covers startup failure and task unwind too. Qualification poison is
        // independent of whether the original resources actually joined.
        let db_settled = if exit.db_settled {
            true
        } else {
            book.cleanup_original().await
        };
        if !db_settled {
            book.interest.fail();
            // Keep this exact runtime, original peer/book and all DB owners.
            // Unknown is not relabelled as natural settlement by a timeout.
            std::future::pending::<()>().await;
        }
        // Actual original helper EOF/wait and DB resource joins have all
        // completed. This is the sole clearing point for every phase owner.
        book.resources.lock().await.clear_after_natural_settlement();
        if started.is_ok() && exit.operation_ok && book.interest.check().is_ok() {
            Ok(())
        } else {
            Err(invalid())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_result_borrow_keeps_same_task_until_natural_join() {
        let interest = Interest::new();
        let (start, barrier) = oneshot::channel();
        let (release, release_wait) = oneshot::channel();
        let actual_returned = Arc::new(AtomicBool::new(false));
        let returned = Arc::clone(&actual_returned);
        let task = tokio::spawn(async move {
            let _ = start.send(());
            let _ = release_wait.await;
            returned.store(true, Ordering::Release);
            TaskExit {
                operation_ok: true,
                db_settled: true,
            }
        });
        let mut cell = TaskCell {
            handle: Some(task),
            joined: None,
        };
        {
            let _borrow = BorrowedResult {
                interest: &interest,
                complete: false,
            };
            let joining = cell.join();
            tokio::pin!(joining);
            tokio::select! {
                _ = &mut joining => panic!("original returned before release"),
                result = barrier => assert!(result.is_ok()),
            }
        }
        assert!(cell.handle.is_some());
        assert!(!actual_returned.load(Ordering::Acquire));
        assert!(interest.check().is_err());
        assert!(release.send(()).is_ok());
        let settled = cell.join().await;
        assert!(settled.db_settled && actual_returned.load(Ordering::Acquire));
        assert!(cell.handle.is_none());
        assert!(
            interest.check().is_err(),
            "real join cannot undo abandoned result"
        );
        assert!(cell.join().await.db_settled);
    }
    #[tokio::test]
    async fn actual_start_barrier_prevents_work_before_handle_registration() {
        let (start, barrier) = oneshot::channel();
        let ran = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&ran);
        let task = tokio::spawn(async move {
            assert!(barrier.await.is_ok());
            observed.store(true, Ordering::Release);
            TaskExit {
                operation_ok: true,
                db_settled: true,
            }
        });
        let mut cell = TaskCell {
            handle: Some(task),
            joined: None,
        };
        tokio::task::yield_now().await;
        assert!(!ran.load(Ordering::Acquire));
        assert!(cell.handle.is_some());
        assert!(start.send(()).is_ok());
        assert!(cell.join().await.db_settled);
        assert!(ran.load(Ordering::Acquire));
    }
    #[tokio::test]
    async fn task_unwind_is_unknown_and_cannot_be_cached_as_settled() {
        let task = tokio::spawn(async {
            panic!("ordinary original owner unwind");
            #[allow(unreachable_code)]
            TaskExit {
                operation_ok: true,
                db_settled: true,
            }
        });
        let mut cell = TaskCell {
            handle: Some(task),
            joined: None,
        };
        let exit = cell.join().await;
        assert!(!exit.operation_ok && !exit.db_settled);
        assert!(!cell.join().await.db_settled);
    }
    #[test]
    fn cancellation_negative_is_sticky_even_when_later_borrow_completes() {
        let interest = Interest::new();
        drop(BorrowedResult {
            interest: &interest,
            complete: false,
        });
        drop(BorrowedResult {
            interest: &interest,
            complete: true,
        });
        assert!(interest.check().is_err());
    }
}

#[cfg(test)]
mod retained_owner_tests {
    use super::*;
    struct OriginalSentinel(Arc<AtomicBool>);
    impl Drop for OriginalSentinel {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn borrowed_work_unwind_keeps_original_owner_until_natural_cleanup() {
        let dropped = Arc::new(AtomicBool::new(false));
        let original = Arc::new(Mutex::new(RetainedOriginal::new(OriginalSentinel(
            Arc::clone(&dropped),
        ))));
        let (unwound, observed) = oneshot::channel();
        let (release, cleanup_gate) = oneshot::channel();
        let retained = Arc::clone(&original);
        let actual = tokio::spawn(async move {
            let result = observe_borrowed(async {
                let mut book = retained.lock().await;
                let _actual_owner = book.borrow_mut().unwrap();
                panic!("ordinary unwind while borrowing an original phase owner");
            })
            .await;
            assert!(result.is_err());
            let _ = unwound.send(());
            assert!(cleanup_gate.await.is_ok());
            retained.lock().await.clear_after_natural_settlement();
        });
        assert!(observed.await.is_ok());
        assert!(
            !dropped.load(Ordering::Acquire),
            "work unwind cannot destroy Book custody"
        );
        assert!(original.lock().await.value.is_some());
        assert!(release.send(()).is_ok());
        assert!(actual.await.is_ok());
        assert!(dropped.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn fallible_finish_validation_keeps_the_same_original_in_its_slot() {
        let dropped = Arc::new(AtomicBool::new(false));
        let mut book = RetainedOriginal::new(OriginalSentinel(Arc::clone(&dropped)));
        let address = book.borrow_mut().unwrap() as *mut OriginalSentinel as usize;
        let result: Result<(), BackupError> = observe_borrowed(async {
            let original = book.borrow_mut()?;
            assert_eq!(original as *mut OriginalSentinel as usize, address);
            // This models a real finish read/recheck returning Err while its
            // original operand stays borrowed in the production container.
            Err(BackupError::Invalid("ordinary fallible finish validation"))
        })
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(!dropped.load(Ordering::Acquire));
        assert_eq!(
            book.borrow_mut().unwrap() as *mut OriginalSentinel as usize,
            address
        );
        book.clear_after_natural_settlement();
        assert!(dropped.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn unknown_cleanup_retains_original_until_the_same_task_really_returns() {
        let dropped = Arc::new(AtomicBool::new(false));
        let original = Arc::new(Mutex::new(RetainedOriginal::new(OriginalSentinel(
            Arc::clone(&dropped),
        ))));
        let (started, started_rx) = oneshot::channel();
        let (release, actual_cleanup) = oneshot::channel();
        let retained = Arc::clone(&original);
        let actual = tokio::spawn(async move {
            let _ = started.send(());
            assert!(actual_cleanup.await.is_ok());
            retained.lock().await.clear_after_natural_settlement();
        });
        assert!(started_rx.await.is_ok());
        let unknown = observe_borrowed(async {
            let mut retained = original.lock().await;
            let _same_original = retained.borrow_mut()?;
            Err::<(), BackupError>(BackupError::Invalid(
                "ordinary original cleanup still unknown",
            ))
        })
        .await
        .unwrap();
        assert!(unknown.is_err());
        assert!(!dropped.load(Ordering::Acquire));
        assert!(!actual.is_finished());
        assert!(original.lock().await.value.is_some());
        assert!(release.send(()).is_ok());
        assert!(actual.await.is_ok());
        assert!(dropped.load(Ordering::Acquire));
    }
}
