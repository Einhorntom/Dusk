//! Bounded monitor I/O (SPEC-MON-4): each monitor gets its own worker
//! thread, so DDC/CI calls to one monitor are serialized and a hung monitor
//! never blocks another. A call waits at most `timeout`; after a timeout the
//! stuck worker is abandoned (it exits once the driver call returns) and the
//! call is retried on a fresh worker, up to `attempts` times, before the
//! monitor is reported as not responding.

use std::collections::HashMap;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use dusk_domain::{ControlCapability, ControlKey, Monitor, MonitorId};

use crate::{BackendError, MonitorBackend};

/// SPEC-MON-4 defaults.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_ATTEMPTS: u32 = 3;

type Job = Box<dyn FnOnce(&dyn MonitorBackend) + Send>;

/// Which worker runs a call: one per monitor, plus one for discovery.
type WorkerKey = Option<MonitorId>;

pub struct TimedBackend {
    inner: Arc<dyn MonitorBackend>,
    timeout: Duration,
    attempts: u32,
    workers: Mutex<HashMap<WorkerKey, Sender<Job>>>,
}

impl TimedBackend {
    pub fn new(inner: Arc<dyn MonitorBackend>, timeout: Duration, attempts: u32) -> Self {
        Self {
            inner,
            timeout,
            attempts: attempts.max(1),
            workers: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_defaults(inner: Arc<dyn MonitorBackend>) -> Self {
        Self::new(inner, DEFAULT_TIMEOUT, DEFAULT_ATTEMPTS)
    }

    fn run<T: Send + 'static>(
        &self,
        key: WorkerKey,
        call: impl Fn(&dyn MonitorBackend) -> Result<T, BackendError> + Clone + Send + 'static,
    ) -> Result<T, BackendError> {
        for _ in 0..self.attempts {
            let (reply, result) = mpsc::channel();
            let call = call.clone();
            let job: Job = Box::new(move |backend| {
                let _ = reply.send(call(backend));
            });
            if !self.send(&key, job) {
                continue;
            }
            match result.recv_timeout(self.timeout) {
                Ok(result) => return result,
                // Stuck or crashed: abandon this worker and retry on a new one.
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    self.workers().remove(&key);
                }
            }
        }
        let what = match &key {
            Some(monitor) => format!("monitor {monitor}"),
            None => "monitor discovery".to_owned(),
        };
        Err(BackendError::NotResponding(format!(
            "{what} is not responding (no answer within {} ms, {} attempts)",
            self.timeout.as_millis(),
            self.attempts
        )))
    }

    /// Queues `job` on the key's worker, starting one if needed.
    fn send(&self, key: &WorkerKey, job: Job) -> bool {
        let mut workers = self.workers();
        let sender = workers.entry(key.clone()).or_insert_with(|| {
            let (sender, jobs) = mpsc::channel::<Job>();
            let inner = self.inner.clone();
            let name = match key {
                Some(monitor) => format!("dusk-monitor-{monitor}"),
                None => "dusk-discovery".to_owned(),
            };
            let spawned = thread::Builder::new().name(name).spawn(move || {
                while let Ok(job) = jobs.recv() {
                    job(inner.as_ref());
                }
            });
            if let Err(error) = spawned {
                eprintln!("error: could not start a monitor worker: {error}");
            }
            sender
        });
        if sender.send(job).is_ok() {
            true
        } else {
            workers.remove(key);
            false
        }
    }

    fn workers(&self) -> std::sync::MutexGuard<'_, HashMap<WorkerKey, Sender<Job>>> {
        self.workers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl MonitorBackend for TimedBackend {
    fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
        self.run(None, |backend| backend.list_monitors())
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
