use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, TryLockError};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const COLD_TIMEOUT: Duration = Duration::from_secs(90);
pub(super) const CANCELLED: &str = "OCR 任务已取消。";
pub(super) const TIMED_OUT: &str = "OCR 处理超时，请重试。";

pub(super) struct WorkerPool(Mutex<Option<Worker>>);

pub(super) static OCR_WORKER: WorkerPool = WorkerPool(Mutex::new(None));

impl WorkerPool {
    pub(super) fn request<T: DeserializeOwned>(
        &self,
        payload: &Value,
        cancel: &AtomicBool,
        timeout: Duration,
        keep_alive: bool,
    ) -> Result<T, String> {
        let executable = std::env::current_exe().map_err(|err| err.to_string())?;
        let mut command = Command::new(executable);
        command.args([
            "--tool=ocr",
            "--ocr-worker",
            "--stdio",
            "--validate-installed",
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        self.request_with(payload, cancel, timeout, keep_alive, &mut command)
    }

    fn request_with<T: DeserializeOwned>(
        &self,
        payload: &Value,
        cancel: &AtomicBool,
        timeout: Duration,
        keep_alive: bool,
        command: &mut Command,
    ) -> Result<T, String> {
        let start = Instant::now();
        let cold_deadline = start + timeout.max(COLD_TIMEOUT);
        let mut slot = loop {
            remaining(cancel, cold_deadline)?;
            match self.0.try_lock() {
                Ok(slot) => break slot,
                Err(TryLockError::WouldBlock) => std::thread::sleep(POLL_INTERVAL),
                Err(TryLockError::Poisoned(_)) => return Err("OCR 工作进程锁定失败。".into()),
            }
        };
        remaining(cancel, cold_deadline)?;
        if !keep_alive
            || slot
                .as_mut()
                .is_some_and(|worker| !matches!(worker.child.try_wait(), Ok(None)))
        {
            *slot = None;
        }
        let deadline = if slot.is_none() {
            *slot = Some(
                Worker::spawn(command).map_err(|err| format!("启动 OCR 工作进程失败：{err}"))?,
            );
            cold_deadline
        } else {
            (Instant::now() + timeout).min(cold_deadline)
        };
        let response = slot
            .as_mut()
            .expect("OCR worker initialized")
            .exchange(payload, cancel, deadline);
        if response.is_err() || !keep_alive {
            *slot = None;
        }
        let response = response?;
        if response.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("OCR 工作进程返回无效状态。")
                .to_string());
        }
        serde_json::from_value(response).map_err(|err| format!("解析 OCR 响应失败：{err}"))
    }
}

fn remaining(cancel: &AtomicBool, deadline: Instant) -> Result<Duration, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or_else(|| TIMED_OUT.into())
}

struct Worker {
    child: Child,
    requests: mpsc::Sender<String>,
    responses: mpsc::Receiver<io::Result<String>>,
    #[cfg(windows)]
    _job: Option<std::os::windows::io::OwnedHandle>,
}

impl Worker {
    fn spawn(command: &mut Command) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        #[cfg(windows)]
        let job = attach_kill_on_close_job(&child).ok();
        let mut input = child.stdin.take().expect("piped OCR stdin");
        let mut output = BufReader::new(child.stdout.take().expect("piped OCR stdout"));
        let (requests, rx) = mpsc::channel::<String>();
        let (tx, responses) = mpsc::channel();
        let worker = Self {
            child,
            requests,
            responses,
            #[cfg(windows)]
            _job: job,
        };
        // Blocking pipe I/O stays off the thread enforcing cancellation and deadlines.
        std::thread::Builder::new()
            .name("ocr-worker-io".into())
            .spawn(move || {
                for request in rx {
                    let response = (|| {
                        let sent = writeln!(input, "{request}").and_then(|_| input.flush());
                        let mut line = String::new();
                        if output.read_line(&mut line)? == 0 {
                            sent?;
                            return Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "OCR worker closed stdout",
                            ));
                        }
                        Ok(line)
                    })();
                    let failed = response.is_err();
                    if tx.send(response).is_err() || failed {
                        break;
                    }
                }
            })?;
        Ok(worker)
    }

    fn exchange(
        &mut self,
        payload: &Value,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Value, String> {
        remaining(cancel, deadline)?;
        self.requests
            .send(payload.to_string())
            .map_err(|_| "OCR 工作进程已退出。".to_string())?;
        loop {
            let wait = remaining(cancel, deadline)?.min(POLL_INTERVAL);
            match self.responses.recv_timeout(wait) {
                Ok(response) => {
                    remaining(cancel, deadline)?;
                    let line = response.map_err(|err| format!("OCR 工作进程通信失败：{err}"))?;
                    return serde_json::from_str(&line)
                        .map_err(|err| format!("OCR 工作进程返回无效 JSON：{err}"));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("OCR 工作进程已退出。".into())
                }
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        #[cfg(not(windows))]
        let _ = self.child.wait();
    }
}

#[cfg(windows)]
fn attach_kill_on_close_job(child: &Child) -> io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle == 0 {
        return Err(io::Error::last_os_error());
    }
    let job = unsafe { OwnedHandle::from_raw_handle(handle as _) };
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    };
    if configured == 0
        || unsafe { AssignProcessToJobObject(handle, child.as_raw_handle() as _) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    #[test]
    fn reuses_worker_and_restarts_after_timeout_or_cancellation() {
        let marker =
            std::env::temp_dir().join(format!("kaixin-ocr-test-{}.ready", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let fixture = || {
            let mut command = Command::new("powershell.exe");
            command.creation_flags(CREATE_NO_WINDOW);
            command.env("KAIXIN_TEST_OCR_MARKER", &marker);
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                r#"
                while ($line = [Console]::ReadLine()) {
                    if (($line | ConvertFrom-Json).hang) {
                        [IO.File]::WriteAllText($env:KAIXIN_TEST_OCR_MARKER, '')
                        [Threading.Thread]::Sleep(60000)
                    }
                    [Console]::WriteLine(('{"ok":true,"pid":' + $PID + '}'))
                }
            "#,
            ]);
            command
        };
        let pool = WorkerPool(Mutex::new(None));
        let live = AtomicBool::new(false);
        let hang = json!({"hang": true});
        let ready = || {
            pool.request_with::<Value>(
                &json!({}),
                &live,
                Duration::from_secs(5),
                true,
                &mut fixture(),
            )
            .unwrap()["pid"]
                .as_u64()
                .unwrap()
        };
        let first = ready();
        assert_eq!(ready(), first);
        let timeout = pool.request_with::<Value>(
            &hang,
            &live,
            Duration::from_millis(500),
            true,
            &mut fixture(),
        );
        assert_eq!(timeout.unwrap_err(), TIMED_OUT);
        assert!(pool.0.lock().unwrap().is_none());
        let second = ready();
        assert_ne!(second, first);
        let _ = std::fs::remove_file(&marker);

        let cancel = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let pending = scope.spawn(|| {
                pool.request_with::<Value>(
                    &hang,
                    &cancel,
                    Duration::from_secs(5),
                    true,
                    &mut fixture(),
                )
            });
            let deadline = Instant::now() + Duration::from_secs(3);
            while !marker.is_file() && Instant::now() < deadline {
                std::thread::sleep(POLL_INTERVAL);
            }
            let was_in_flight = marker.is_file();
            cancel.store(true, Ordering::Relaxed);
            let third = ready();
            assert_eq!(pending.join().unwrap().unwrap_err(), CANCELLED);
            assert!(was_in_flight);
            assert_ne!(third, second);
        });
        let _ = std::fs::remove_file(&marker);
    }
}
