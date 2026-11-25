#![cfg(windows)]

use pinyin_ime::{clipboard_store, ipc_service::EnginePipeClient, user_dict};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{SystemTime, UNIX_EPOCH};

const TEXT: &str = "开心剪贴板日志验收";
const PHRASE: &str = "开心词典日志验收";

struct Fixture {
    root: PathBuf,
    pipe: String,
    mutex: String,
    engine: Option<Child>,
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = format!("{}.{}", std::process::id(), nonce);
        let root = std::env::temp_dir().join(format!("kaixin-storage-test-{id}"));
        fs::create_dir_all(root.join("kaixin")).unwrap();
        Self {
            root,
            pipe: format!(r"\\.\pipe\KaixinStorageTest.{id}"),
            mutex: format!(r"Local\KaixinStorageTest.{id}"),
            engine: None,
        }
    }

    fn isolate(&self, command: &mut Command) {
        command
            .env("LOCALAPPDATA", &self.root)
            .env("APPDATA", &self.root)
            .env("SRF_USER_DICT_PATH", self.root.join("kaixin/user_dict.kxj"))
            .env("SRF_ENGINE_PIPE_NAME", &self.pipe)
            .env("SRF_ENGINE_MUTEX_NAME", &self.mutex)
            .env("SRF_IME_LOG_LEVEL", "error");
    }

    fn start(&mut self) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_srf_ime_engine"));
        self.isolate(&mut command);
        self.engine = Some(command.arg("--no-warmup").spawn().unwrap());
    }

    fn client(&self, operation: &str) {
        let mut command = Command::new(std::env::current_exe().unwrap());
        self.isolate(&mut command);
        let output = command
            .args(["--ignored", "--exact", "storage_client"])
            .env("KX_STORAGE_TEST_OPERATION", operation)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn stop(&mut self) {
        self.client("shutdown");
        assert!(self.engine.take().unwrap().wait().unwrap().success());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.engine.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn single_engine_owns_both_journals_across_two_clients_and_restart() {
    let mut fixture = Fixture::new();
    let dir = fixture.root.join("kaixin");
    // Old on-disk storage must never be loaded or modified by this release.
    for name in ["user_dict.sqlite", "clipboard_store.sqlite"] {
        fs::write(dir.join(name), b"old data").unwrap();
    }
    fixture.start();
    fixture.client("add");
    fixture.client("read");
    fixture.stop();

    assert_header(&dir.join("user_dict.kxj"), b"KXUDJNL1");
    assert_header(&dir.join("clipboard_store.kxj"), b"KXCBJNL1");
    for name in ["user_dict.sqlite", "clipboard_store.sqlite"] {
        assert_eq!(fs::read(dir.join(name)).unwrap(), b"old data");
    }

    fixture.start();
    fixture.client("read");
    fixture.client("clear");
    fixture.stop();
    fixture.start();
    fixture.client("empty");
    fixture.stop();
}

fn assert_header(path: &Path, header: &[u8]) {
    assert!(fs::read(path).unwrap().starts_with(header));
}

#[test]
#[ignore = "child process invoked by the isolated integration test"]
fn storage_client() {
    match std::env::var("KX_STORAGE_TEST_OPERATION").unwrap().as_str() {
        "add" => {
            user_dict::add_user_phrase("kaixinyanshou", PHRASE).unwrap();
            assert!(clipboard_store::record_text(TEXT).unwrap());
        }
        "read" => {
            assert!(user_dict::list_user_phrases()
                .unwrap()
                .iter()
                .any(|entry| entry.phrase == PHRASE));
            let snapshot = clipboard_store::snapshot().unwrap();
            assert!(snapshot.history.iter().any(|entry| entry.text == TEXT));
        }
        "clear" => {
            user_dict::clear_user_dict().unwrap();
            clipboard_store::clear_all().unwrap();
        }
        "empty" => {
            assert!(user_dict::list_user_phrases().unwrap().is_empty());
            assert!(clipboard_store::snapshot().unwrap().history.is_empty());
        }
        "shutdown" => {
            EnginePipeClient::connect().unwrap().shutdown().unwrap();
        }
        operation => panic!("unknown storage fixture operation: {operation}"),
    }
}
