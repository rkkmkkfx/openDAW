use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Output(std::path::PathBuf);

impl Output {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "opendaw-native-{}-{}.wav",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn cli_writes_default_and_partial_frame_wavs() {
    for (frames, expected) in [(None, 48_000), (Some("129"), 129)] {
        let output = Output::new();
        let mut command = Command::new(env!("CARGO_BIN_EXE_native-render"));
        command.arg(&output.0);
        if let Some(frames) = frames {
            command.arg(frames);
        }
        assert!(command.output().unwrap().status.success());
        let bytes = fs::read(&output.0).unwrap();
        let mut reference = Vec::new();
        native_render::render_wav(&mut reference, expected).unwrap();
        assert_eq!(bytes, reference);
    }
}

#[test]
fn cli_preserves_existing_files() {
    let output = Output::new();
    fs::write(&output.0, b"keep me").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_native-render"))
        .arg(&output.0)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(&output.0).unwrap(), b"keep me");
}

#[test]
fn cli_rejects_invalid_arguments_before_creating_files() {
    let result = Command::new(env!("CARGO_BIN_EXE_native-render"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    for arguments in [
        vec!["bad"],
        vec!["-1"],
        vec!["4294967295"],
        vec!["1", "extra"],
    ] {
        let output = Output::new();
        let result = Command::new(env!("CARGO_BIN_EXE_native-render"))
            .arg(&output.0)
            .args(arguments)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!output.0.exists());
    }
}
