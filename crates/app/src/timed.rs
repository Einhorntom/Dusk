//! Bounded monitor I/O (SPEC-MON-4): each monitor gets one worker thread,
//! so DDC/CI calls to one monitor are serialized and a hung monitor never
//! blocks another. A call is sent once and waits up to `timeout` x
//! `attempts` for its answer. A driver call cannot be cancelled, so when
//! one times out the monitor is marked busy: further calls fail at once as
//! "not responding" until the stuck call returns, instead of piling up or
//! reaching the monitor alongside it. Calls still queued when their caller
//! gives up are dropped, so a stale write never runs late.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use dusk_domain::{ControlCapability, ControlKey, Monitor, MonitorId};

use crate::{BackendError, MonitorBackend};

/// SPEC-MON-4 defaults: one call waits up to 3 x 2 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_ATTEMPTS: u32 = 3;

type Call = Box<dyn FnOnce(&dyn MonitorBackend) + Send>;

struct Job {
    call: Call,
    /// Set when the caller gave up; the job is then skipped if not started.
    cancelled: Arc<AtomicBool>,
    /// Set by the worker when the job has finished (or was skipped).
    done: Arc<AtomicBool>,
}

#[derive(Clone)]
struct Worker {
    jobs: Sender<Job>,
    /// A call timed out and has not returned yet.
    busy: Arc<AtomicBool>,
}

/// Which worker runs a call: one per monitor, plus one for discovery.
type WorkerKey = Option<MonitorId>;

pub struct TimedBackend {
    inner: Arc<dyn MonitorBackend>,
    wait: Duration,
    workers: Mutex<HashMap<WorkerKey, Worker>>,
}

impl TimedBackend {
    pub fn new(inner: Arc<dyn MonitorBackend>, timeout: Duration, attempts: u32) -> Self {
        Self {
            inner,
            wait: timeout * attempts.max(1),
            workers: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_defaults(inner: Arc<dyn MonitorBackend>) -> Self {
        Self::new(inner, DEFAULT_TIMEOUT, DEFAULT_ATTEMPTS)
    }

    fn run<T: Send + 'static>(
        &self,
        key: WorkerKey,
        call: impl FnOnce(&dyn MonitorBackend) -> Result<T, BackendError> + Send + 'static,
    ) -> Result<T, BackendError> {
        let worker = self.worker(&key)?;
        if worker.busy.load(Ordering::SeqCst) {
            return Err(not_responding(&key, "is still busy with an earlier call"));
        }
        let (reply, result) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let job = Job {
            call: Box::new(move |backend| {
                let _ = reply.send(call(backend));
            }),
            cancelled: cancelled.clone(),
            done: done.clone(),
        };
        if worker.jobs.send(job).is_err() {
            self.workers().remove(&key);
            return Err(BackendError::Failed("the monitor worker stopped".into()));
        }
        match result.recv_timeout(self.wait) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                cancelled.store(true, Ordering::SeqCst);
                // Busy until the call returns. If it finished meanwhile, the
                // worker may already have cleared the flag before we set it.
                worker.busy.store(true, Ordering::SeqCst);
                if done.load(Ordering::SeqCst) {
                    worker.busy.store(false, Ordering::SeqCst);
                }
                Err(not_responding(
                    &key,
                    &format!("did not answer within {} ms", self.wait.as_millis()),
                ))
            }
            // The call panicked and took the worker down; start a new one next time.
            Err(RecvTimeoutError::Disconnected) => {
                self.workers().remove(&key);
                Err(BackendError::Failed(
                    "the monitor call failed unexpectedly".into(),
                ))
            }
        }
    }

    /// The key's worker, started if needed.
    fn worker(&self, key: &WorkerKey) -> Result<Worker, BackendError> {
        let mut workers = self.workers();
        if let Some(worker) = workers.get(key) {
            return Ok(worker.clone());
        }
        let (jobs, queue) = mpsc::channel::<Job>();
        let busy = Arc::new(AtomicBool::new(false));
        let inner = self.inner.clone();
        let worker_busy = busy.clone();
        let name = match key {
            Some(monitor) => format!("dusk-monitor-{monitor}"),
            None => "dusk-discovery".to_owned(),
        };
        thread::Builder::new()
            .name(name)
            .spawn(move || {
                // Ends when the worker is dropped from the map and the
                // current call has returned.
                while let Ok(job) = queue.recv() {
                    if !job.cancelled.load(Ordering::SeqCst) {
                        (job.call)(inner.as_ref());
                    }
                    job.done.store(true, Ordering::SeqCst);
                    worker_busy.store(false, Ordering::SeqCst);
                }
            })
            .map_err(|error| {
                log::error!("could not start a monitor worker: {error}");
                BackendError::Failed(format!("could not start a monitor worker: {error}"))
            })?;
        let worker = Worker { jobs, busy };
        workers.insert(key.clone(), worker.clone());
        Ok(worker)
    }

    fn workers(&self) -> std::sync::MutexGuard<'_, HashMap<WorkerKey, Worker>> {
        self.workers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn not_responding(key: &WorkerKey, why: &str) -> BackendError {
    let what = match key {
        Some(monitor) => format!("monitor {monitor}"),
        None => "monitor discovery".to_owned(),
    };
    BackendError::NotResponding(format!("{what} is not responding: it {why}"))
}

impl MonitorBackend for TimedBackend {
    fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
        let monitors = self.run(None, |backend| backend.list_monitors())?;
        // Workers of monitors that are gone end once their current call returns.
        let listed: HashSet<&MonitorId> = monitors.iter().map(|monitor| &monitor.id).collect();
        self.workers()
            .retain(|key, _| key.as_ref().is_none_or(|monitor| listed.contains(monitor)));
        Ok(monitors)
    }

    fn read_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<Option<(ControlCapability, u32)>, BackendError> {
        let target = monitor.clone();
        self.run(Some(monitor.clone()), move |backend| {
            backend.read_control(&target, control)
        })
    }

    fn write_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        native_value: u32,
    ) -> Result<(), BackendError> {
        let target = monitor.clone();
        self.run(Some(monitor.clone()), move |backend| {
            backend.write_control(&target, control, native_value)
        })
    }
}
