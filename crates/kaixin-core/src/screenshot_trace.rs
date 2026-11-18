use crate::runtime_log;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static NEXT_CAPTURE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_STAGE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug)]
pub struct ScreenshotTrace {
    id: u64,
    started: Instant,
}

impl Default for ScreenshotTrace {
    fn default() -> Self {
        Self {
            id: NEXT_CAPTURE_ID.fetch_add(1, Ordering::Relaxed),
            started: Instant::now(),
        }
    }
}

impl ScreenshotTrace {
    pub fn id(self) -> u64 {
        self.id
    }

    pub fn stage(self, name: &'static str) -> ScreenshotStage {
        let stage = ScreenshotStage {
            capture: self,
            id: NEXT_STAGE_ID.fetch_add(1, Ordering::Relaxed),
            name,
            started: Instant::now(),
            status: "aborted",
        };
        stage.log("start", 0);
        stage
    }

    pub fn measure<T, E>(
        self,
        name: &'static str,
        work: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        let stage = self.stage(name);
        let result = work();
        stage.finish(if result.is_ok() { "ok" } else { "failed" });
        result
    }
}

pub struct ScreenshotStage {
    capture: ScreenshotTrace,
    id: u64,
    name: &'static str,
    started: Instant,
    status: &'static str,
}

impl ScreenshotStage {
    pub fn finish(mut self, status: &'static str) {
        self.status = status;
    }

    fn log(&self, status: &str, elapsed_ms: u128) {
        runtime_log::log_screenshot_health(
            "screenshot_stage",
            format!(
                "capture_id={} stage_id={} stage={} status={status} elapsed_ms={elapsed_ms} since_start_ms={}",
                self.capture.id,
                self.id,
                self.name,
                self.capture.started.elapsed().as_millis(),
            ),
        );
    }
}

impl Drop for ScreenshotStage {
    fn drop(&mut self) {
        self.log(self.status, self.started.elapsed().as_millis());
    }
}
