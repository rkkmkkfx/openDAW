use std::{path::PathBuf, process::Command};
fn fixture() -> PathBuf {
    let manifest =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../native-clap/test-plugin/Cargo.toml");
    let target = std::env::temp_dir().join("opendaw-native-host-clap-fixture");
    assert!(Command::new(env!("CARGO"))
        .args(["build", "--offline", "--manifest-path"])
        .arg(manifest)
        .arg("--target-dir")
        .arg(&target)
        .status()
        .unwrap()
        .success());
    target.join("debug/libnative_clap_fixture.so")
}
#[test]
fn real_plugin_cli_lists_and_renders_stereo_with_state() {
    let fixture = fixture();
    let directory =
        std::env::temp_dir().join(format!("opendaw-native-host-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("input.wav");
    let output = directory.join("output.wav");
    let state = directory.join("state.bin");
    let mut writer = hound::WavWriter::create(
        &input,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..129 {
        writer.write_sample(8192_i16).unwrap();
        writer.write_sample(-8192_i16).unwrap();
    }
    writer.finalize().unwrap();
    let listed = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("list")
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(listed.status.success());
    assert!(String::from_utf8(listed.stdout)
        .unwrap()
        .contains("org.opendaw.fixture.gain"));
    let rendered = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("render")
        .arg(&fixture)
        .arg(&input)
        .arg(&output)
        .args(["--param", "7=0.5", "--state-out"])
        .arg(&state)
        .output()
        .unwrap();
    let installed = directory.join("fixture.clap");
    std::fs::copy(&fixture, &installed).unwrap();
    let scanned = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("scan")
        .arg(&directory)
        .output()
        .unwrap();
    assert!(
        scanned.status.success(),
        "{}",
        String::from_utf8_lossy(&scanned.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&scanned.stdout).unwrap();
    assert_eq!(
        report["plugins"][0]["descriptors"][0]["id"],
        "org.opendaw.fixture.gain"
    );
    assert_eq!(report["issues"].as_array().unwrap().len(), 0);
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let mut reader = hound::WavReader::open(&output).unwrap();
    assert_eq!(reader.duration(), 129);
    let samples: Vec<_> = reader.samples::<i16>().map(Result::unwrap).collect();
    assert!(samples
        .chunks_exact(2)
        .all(|samples| (samples[0] - 4096).abs() <= 1 && (samples[1] + 4096).abs() <= 1));
    assert!(!std::fs::read(state).unwrap().is_empty());
    let restored = directory.join("restored.wav");
    let rendered = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("render")
        .arg(&fixture)
        .arg(&input)
        .arg(&restored)
        .arg("--state-in")
        .arg(directory.join("state.bin"))
        .output()
        .unwrap();
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    assert_eq!(
        std::fs::read(&output).unwrap(),
        std::fs::read(restored).unwrap()
    );
    let invalid_output = directory.join("invalid.wav");
    let invalid = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("render")
        .arg(&fixture)
        .arg(&input)
        .arg(&invalid_output)
        .args(["--param", "10=0.5"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(!invalid_output.exists());
    let bad_events = directory.join("bad-events.json");
    std::fs::write(
        &bad_events,
        r#"[{"type":"midi","sample":0,"data":[0,0,0]}]"#,
    )
    .unwrap();
    let invalid = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("render")
        .arg(&fixture)
        .arg(&input)
        .arg(&invalid_output)
        .arg("--events")
        .arg(&bad_events)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(
        !invalid_output.exists(),
        "failed rendering must remove its incomplete output"
    );
    let overwrite = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("render")
        .arg(&fixture)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(!overwrite.status.success());
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn production_project_cli_renders_matching_wasm_fixture() {
    let directory =
        std::env::temp_dir().join(format!("opendaw-native-project-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("project.wav");
    let fixture_directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../engine/tests/fixtures");
    let rendered = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("render-project")
        .arg(fixture_directory.join("vaporisateur.od"))
        .arg(&output)
        .args(["--frames", "2560"])
        .output()
        .unwrap();
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let mut reader = hound::WavReader::open(output).unwrap();
    assert_eq!(reader.duration(), 2560);
    let samples: Vec<_> = reader.samples::<i16>().map(Result::unwrap).collect();
    let reference = std::fs::read(fixture_directory.join("vaporisateur.wasm.f32")).unwrap();
    for (quantum, pcm) in samples.chunks_exact(256).enumerate() {
        for index in 0..128 {
            for channel in 0..2 {
                let position = (quantum * 256 + channel * 128 + index) * 4;
                let value =
                    f32::from_le_bytes(reference[position..position + 4].try_into().unwrap());
                let expected = (value.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
                assert!((pcm[index * 2 + channel] as i32 - expected as i32).abs() <= 2);
            }
        }
    }
    let bundle = directory.join("bundle.od");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&bundle).unwrap());
    use std::io::Write;
    archive
        .start_file("version", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"1").unwrap();
    archive
        .start_file("project.od", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(&std::fs::read(fixture_directory.join("vaporisateur.od")).unwrap())
        .unwrap();
    archive.finish().unwrap();
    let post_output = directory.join("post.wav");
    let rendered = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("render-project")
        .arg(bundle)
        .arg(&post_output)
        .args(["--frames", "2560", "--plugin"])
        .arg(fixture())
        .args(["--param", "7=0.5"])
        .output()
        .unwrap();
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let post_samples: Vec<_> = hound::WavReader::open(post_output)
        .unwrap()
        .samples::<i16>()
        .map(Result::unwrap)
        .collect();
    assert!(samples
        .iter()
        .zip(post_samples)
        .all(|(source, processed)| (*source as i32 - processed as i32 * 2).abs() <= 2));
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn oversized_bundle_version_is_rejected_without_output() {
    use std::io::Write;
    let directory =
        std::env::temp_dir().join(format!("opendaw-native-bad-bundle-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let bundle = directory.join("bundle.od");
    let output = directory.join("output.wav");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&bundle).unwrap());
    archive
        .start_file("version", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(&vec![b'1'; 1024 * 1024]).unwrap();
    archive.finish().unwrap();
    let rendered = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("render-project")
        .arg(bundle)
        .arg(&output)
        .output()
        .unwrap();
    assert!(!rendered.status.success());
    assert!(!output.exists());
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn vst3_cli_processes_real_plugin_with_sample_offset_automation() {
    let manifest =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../native-vst3/fixtures/gain/Cargo.toml");
    let directory =
        std::env::temp_dir().join(format!("opendaw-native-vst-cli-{}", std::process::id()));
    assert!(Command::new(env!("CARGO"))
        .args(["build", "--offline", "--manifest-path"])
        .arg(manifest)
        .arg("--target-dir")
        .arg(directory.join("target"))
        .status()
        .unwrap()
        .success());
    let fixture = directory.join("target/debug/libopendaw_vst3_fixture.so");
    let input = directory.join("input.wav");
    let output = directory.join("output.wav");
    let events = directory.join("events.json");
    let mut writer = hound::WavWriter::create(
        &input,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..129 {
        writer.write_sample(8192_i16).unwrap();
    }
    writer.finalize().unwrap();
    std::fs::write(
        &events,
        r#"[{"type":"parameter","sample":64,"id":0,"value":0.5}]"#,
    )
    .unwrap();
    let rendered = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .arg("render")
        .arg(fixture)
        .arg(input)
        .arg(&output)
        .args(["--format", "vst3", "--events"])
        .arg(events)
        .output()
        .unwrap();
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let mut reader = hound::WavReader::open(output).unwrap();
    assert_eq!(reader.duration(), 129);
    let samples: Vec<_> = reader.samples::<i16>().map(Result::unwrap).collect();
    for (index, pair) in samples.chunks_exact(2).enumerate() {
        let expected = if index < 64 { 8192 } else { 4096 };
        assert!(pair
            .iter()
            .all(|sample| (*sample as i32 - expected).abs() <= 1));
    }
    std::fs::remove_dir_all(directory).unwrap();
}
