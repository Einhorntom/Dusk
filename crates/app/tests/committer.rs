//! SPEC-WR-2: buffered values are written by the application after the
//! quiet period, without any UI driving it.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dusk_app::ControlApi;
use dusk_app::{BackendError, CommitObserver, UseCaseError};
use dusk_ddc_fake::{FakeBackend, FakeMonitor, Harness};
use dusk_domain::{ControlKey, ControlValue, MonitorId};

#[derive(Default)]
struct Recorder(Mutex<Vec<Result<usize, String>>>);

impl CommitObserver for Recorder {
    fn committed(&self, result: Result<usize, UseCaseError>) {
        self.0
            .lock()
            .unwrap()
            .push(result.map_err(|error| error.to_string()));
    }
}

impl Recorder {
    fn wait_for_one(&self) -> Result<usize, String> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(result) = self.0.lock().unwrap().first() {
                return result.clone();
            }
            assert!(Instant::now() < deadline, "the committer did not report");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn started() -> (Harness, Arc<Recorder>, MonitorId) {
    let h = Harness::new(FakeBackend::single(FakeMonitor::reference("m")));
    let recorder = Arc::new(Recorder::default());
    h.service.start_committer(recorder.clone()).unwrap();
    (h, recorder, MonitorId::new("m").unwrap())
}

#[test]
fn a_buffered_value_is_written_once_its_quiet_period_ends() {
    let (h, recorder, id) = started();
    for value in [60, 65, 70] {
        h.service
            .adjust(&id, ControlKey::Brightness, ControlValue::Normalized(value))
            .unwrap();
    }
    h.clock.advance(Duration::from_millis(400));
    assert_eq!(recorder.wait_for_one(), Ok(1));
    assert_eq!(h.backend.written_values(), [70]);
}

#[test]
fn a_failed_write_is_reported() {
    let (h, recorder, id) = started();
    h.backend.fail_writes(
        &id,
        ControlKey::Brightness,
        BackendError::Failed("DDC/CI error".into()),
    );
    h.service
        .adjust(&id, ControlKey::Brightness, ControlValue::Normalized(30))
        .unwrap();
    h.clock.advance(Duration::from_millis(400));
    let reported = recorder.wait_for_one().unwrap_err();
    assert!(reported.contains("DDC/CI error"), "{reported}");
}
