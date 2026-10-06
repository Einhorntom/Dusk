//! SPEC-MON-4: a monitor that does not answer times out and is reported as
//! not responding, without blocking other monitors.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use dusk_app::{BackendError, MonitorBackend, TimedBackend};
use dusk_ddc_fake::{FakeBackend, FakeMonitor};
use dusk_domain::{ControlKey, MonitorId};

const TIMEOUT: Duration = Duration::from_millis(40);

fn setup() -> (Arc<FakeBackend>, TimedBackend, MonitorId, MonitorId) {
    let fake = Arc::new(FakeBackend::new([
        FakeMonitor::reference("hung"),
        FakeMonitor::reference("fine"),
    ]));
    let timed = TimedBackend::new(fake.clone(), TIMEOUT, 3);
    let hung = MonitorId::new("hung").unwrap();
    let fine = MonitorId::new("fine").unwrap();
    (fake, timed, hung, fine)
}

#[test]
fn a_hung_monitor_is_reported_as_not_responding_after_the_full_wait() {
    let (fake, timed, hung, _) = setup();
    fake.hang(&hung);
    let started = Instant::now();
    let result = timed.read_control(&hung, ControlKey::Brightness);
    let elapsed = started.elapsed();
    assert!(
        matches!(result, Err(BackendError::NotResponding(_))),
        "{result:?}"
    );
    assert!(elapsed >= TIMEOUT * 3, "gave up after {elapsed:?}");
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
    fake.release(&hung);
}

#[test]
fn other_monitors_answer_while_one_is_hung() {
    let (fake, timed, hung, fine) = setup();
    let timed = Arc::new(timed);
    fake.hang(&hung);
    let blocked = {
        let timed = timed.clone();
        thread::spawn(move || timed.write_control(&hung, ControlKey::Brightness, 10))
    };
    thread::sleep(TIMEOUT / 4);
    let started = Instant::now();
    let reading = timed.read_control(&fine, ControlKey::Brightness).unwrap();
    assert!(reading.is_some());
    // Well before the hung monitor gives up (3 x TIMEOUT).
    assert!(
        started.elapsed() < TIMEOUT * 2,
        "took {:?}",
        started.elapsed()
    );
    assert!(matches!(
        blocked.join().unwrap(),
        Err(BackendError::NotResponding(_))
    ));
    fake.release(&MonitorId::new("hung").unwrap());
}

/// Retries `call` until it succeeds, for up to a second.
fn eventually<T>(mut call: impl FnMut() -> Result<T, BackendError>) -> T {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match call() {
            Ok(value) => return value,
            Err(error) if Instant::now() < deadline => {
                assert!(matches!(error, BackendError::NotResponding(_)), "{error:?}");
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("still failing: {error:?}"),
        }
    }
}

#[test]
fn a_monitor_answers_again_once_it_recovers() {
    let (fake, timed, hung, _) = setup();
    fake.hang(&hung);
    assert!(timed.read_control(&hung, ControlKey::Brightness).is_err());
    fake.release(&hung);
    eventually(|| timed.write_control(&hung, ControlKey::Brightness, 70));
    assert_eq!(fake.value(&hung, ControlKey::Brightness), Some(70));
    assert_eq!(timed.list_monitors().unwrap().len(), 2);
}

#[test]
fn a_timed_out_write_is_sent_once() {
    // Regression: a timed-out call used to be re-sent on a fresh worker, so
    // the write reached the monitor up to three times once it recovered.
    let (fake, timed, hung, _) = setup();
    fake.hang(&hung);
    assert!(
        timed
            .write_control(&hung, ControlKey::Brightness, 10)
            .is_err()
    );
    fake.release(&hung);
    eventually(|| timed.read_control(&hung, ControlKey::Brightness));
    assert_eq!(fake.written_values(), [10]);
}

#[test]
fn a_busy_monitor_fails_fast_until_its_stuck_call_returns() {
    let (fake, timed, hung, _) = setup();
    fake.hang(&hung);
    assert!(timed.read_control(&hung, ControlKey::Brightness).is_err());

    // The stuck call has not returned: no second call reaches the monitor.
    let started = Instant::now();
    let again = timed.write_control(&hung, ControlKey::Brightness, 30);
    assert!(
        matches!(again, Err(BackendError::NotResponding(_))),
        "{again:?}"
    );
    assert!(started.elapsed() < TIMEOUT, "took {:?}", started.elapsed());

    fake.release(&hung);
    eventually(|| timed.read_control(&hung, ControlKey::Brightness));
    assert!(fake.writes().is_empty(), "{:?}", fake.writes());
}

#[test]
fn a_call_queued_behind_a_stuck_one_is_dropped_when_its_caller_gives_up() {
    let (fake, timed, hung, _) = setup();
    let timed = Arc::new(timed);
    fake.hang(&hung);
    let first = {
        let timed = timed.clone();
        let hung = hung.clone();
        thread::spawn(move || timed.write_control(&hung, ControlKey::Brightness, 10))
    };
    thread::sleep(TIMEOUT / 4);
    // Queued behind the first call; its caller gives up before it starts.
    assert!(
        timed
            .write_control(&hung, ControlKey::Brightness, 20)
            .is_err()
    );
    assert!(first.join().unwrap().is_err());

    fake.release(&hung);
    eventually(|| timed.read_control(&hung, ControlKey::Brightness));
    assert_eq!(fake.written_values(), [10], "the stale write must not run");
}

#[test]
fn backend_errors_pass_through_without_waiting_or_retrying() {
    let (fake, timed, hung, _) = setup();
    fake.fail_writes(
        &hung,
        ControlKey::Contrast,
        BackendError::Failed("DDC/CI error".into()),
    );
    let started = Instant::now();
    assert_eq!(
        timed.write_control(&hung, ControlKey::Contrast, 1),
        Err(BackendError::Failed("DDC/CI error".into()))
    );
    assert!(started.elapsed() < TIMEOUT);
    assert!(fake.writes().is_empty());
}
