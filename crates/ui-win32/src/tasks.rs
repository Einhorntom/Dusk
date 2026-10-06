//! Monitor I/O for the Settings window, off the UI thread (SPEC-MON-4).
//! `run` sends work to one worker thread (so results arrive in request
//! order) and hands each result back to the window as `TASK_DONE`, where
//! `finish` installs it. A slow or hung monitor then never freezes the
//! window, the tray or the hotkeys.
//!
//! Buffered writes are committed by the application's committer thread
//! (`MonitorService::start_committer`); `CommitNotifier` reports them here
//! as `COMMITTED`.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{self, Sender};

use dusk_app::{Api, CommitObserver, UseCaseError};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::WindowContext;

/// A finished task, carried in `LPARAM` as a leaked `Box<Done>`.
pub(crate) const TASK_DONE: u32 = WM_APP + 4;
/// A commit result, carried in `LPARAM` as a leaked `Box<Result<usize, String>>`.
pub(crate) const COMMITTED: u32 = WM_APP + 5;

type Done = Box<dyn FnOnce(&mut WindowContext) + Send>;
type Work = Box<dyn FnOnce(&dyn Api) -> Done + Send>;

#[derive(Default)]
pub(crate) struct Tasks {
    work: Option<Sender<Work>>,
}

/// Runs `work` on the task thread, then `finish` on the UI thread.
pub(crate) fn run<T: Send + 'static>(
    context: &mut WindowContext,
    work: impl FnOnce(&dyn Api) -> T + Send + 'static,
    finish: impl FnOnce(&mut WindowContext, T) + Send + 'static,
) {
    let job: Work = Box::new(move |api| {
        let result = work(api);
        Box::new(move |context: &mut WindowContext| finish(context, result))
    });
    let window = context.window;
    let api = context.model.api();
    let tasks = &mut context.tasks;
    let sender = tasks
        .work
        .get_or_insert_with(|| start_worker(window, api.clone()));
    if let Err(mpsc::SendError(job)) = sender.send(job) {
        // The worker stopped (a task panicked); start a new one.
        let sender = tasks.work.insert(start_worker(window, api));
        let _ = sender.send(job);
    }
}

fn start_worker(window: HWND, api: Arc<dyn Api>) -> Sender<Work> {
    let (sender, jobs) = mpsc::channel::<Work>();
    // HWND is not Send; the worker only posts messages to it.
    let window = window.0 as isize;
    let spawned = std::thread::Builder::new()
        .name("dusk-ui-tasks".into())
        .spawn(move || {
            while let Ok(job) = jobs.recv() {
                let done: Box<Done> = Box::new(job(api.as_ref()));
                post(window, TASK_DONE, done);
            }
        });
    if let Err(error) = spawned {
        log::error!("could not start the Settings task worker: {error}");
    }
    sender
}

/// Posts `payload` to `window`; drops it if the window is gone.
fn post<T>(window: isize, message: u32, payload: Box<T>) {
    if window == 0 {
        return;
    }
    let raw = Box::into_raw(payload);
    let posted = unsafe {
        PostMessageW(
            Some(HWND(window as *mut c_void)),
            message,
            WPARAM(0),
            LPARAM(raw as isize),
        )
    };
    if posted.is_err() {
        // SAFETY: not posted, so this is still the only owner.
        drop(unsafe { Box::from_raw(raw) });
    }
}

/// Handles `TASK_DONE`.
pub(crate) fn on_task_done(context: &mut WindowContext, lparam: LPARAM) {
    // SAFETY: only `start_worker` posts this message, with a leaked Box<Done>.
    let done = unsafe { Box::from_raw(lparam.0 as *mut Done) };
    (*done)(context);
}

/// Takes the result carried by `COMMITTED`.
pub(crate) fn take_commit(lparam: LPARAM) -> Result<usize, String> {
    // SAFETY: only `CommitNotifier` posts this message, with a leaked Box.
    *unsafe { Box::from_raw(lparam.0 as *mut Result<usize, String>) }
}

/// Forwards commit results to the Settings window once it exists.
#[derive(Default)]
pub struct CommitNotifier {
    window: AtomicIsize,
}

impl CommitNotifier {
    pub(crate) fn attach(&self, window: HWND) {
        self.window.store(window.0 as isize, Ordering::SeqCst);
    }
}

impl CommitObserver for CommitNotifier {
    fn committed(&self, result: Result<usize, UseCaseError>) {
        let result = result.map_err(|error| error.to_string());
        post(
            self.window.load(Ordering::SeqCst),
            COMMITTED,
            Box::new(result),
        );
    }
}
