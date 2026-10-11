use std::{path::PathBuf, process::Command, sync::OnceLock};
fn fixture() -> &'static PathBuf {
    static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/gain/Cargo.toml");
        let target =
            std::env::temp_dir().join(format!("opendaw-vst3-fixture-{}", std::process::id()));
        assert!(Command::new(env!("CARGO"))
            .arg("build")
            .arg("--manifest-path")
            .arg(manifest)
            .arg("--target-dir")
            .arg(&target)
            .status()
            .unwrap()
            .success());
        target.join("debug/libopendaw_vst3_fixture.so")
    })
}
#[test]
fn discovers_real_factory_audio_classes() {
    let descriptors = native_vst3::list_plugins(fixture()).unwrap();
    assert_eq!(descriptors.len(), 1);
    assert!(descriptors[0].name.contains("Gain"));
    assert_eq!(descriptors[0].id.len(), 32);
}

fn instance() -> native_vst3::Instance {
    let id = native_vst3::list_plugins(fixture()).unwrap().remove(0).id;
    native_vst3::load(fixture(), &id, 48_000.0, 128).unwrap()
}
#[test]
fn sample_accurate_stereo_automation_and_state_round_trip() {
    let mut instance = instance();
    assert_eq!(instance.parameters()[0].name, "Gain");
    let input = [1.0f32; 128];
    let right = [-0.5f32; 128];
    let mut left_out = [0.0f32; 128];
    let mut right_out = [0.0f32; 128];
    instance
        .process(
            [&input, &right],
            [&mut left_out, &mut right_out],
            &[
                native_vst3::Event::Parameter {
                    offset: 64,
                    id: 0,
                    value: 0.25,
                },
                native_vst3::Event::Parameter {
                    offset: 127,
                    id: 0,
                    value: 0.5,
                },
            ],
        )
        .unwrap();
    assert_eq!(&left_out[..64], &[1.0; 64]);
    assert_eq!(&left_out[64..127], &[0.25; 63]);
    assert_eq!(left_out[127], 0.5);
    assert_eq!(right_out[64], -0.125);
    assert_eq!(instance.parameters()[0].value, 0.5);
    let state = instance.save_state().unwrap();
    assert!(!state.is_empty());
    instance
        .process(
            [&input, &right],
            [&mut left_out, &mut right_out],
            &[native_vst3::Event::Parameter {
                offset: 0,
                id: 0,
                value: 0.0,
            }],
        )
        .unwrap();
    assert_eq!(left_out, [0.0; 128]);
    instance.load_state(&state).unwrap();
    instance
        .process([&input, &right], [&mut left_out, &mut right_out], &[])
        .unwrap();
    assert_eq!(left_out, [0.5; 128]);
    assert_eq!(right_out, [-0.25; 128]);
}
#[test]
fn rejects_bad_events_without_polluting_next_block() {
    let mut instance = instance();
    let input = [1.0f32; 128];
    let mut output_l = [0.0f32; 128];
    let mut output_r = [0.0f32; 128];
    let bad_events = [
        native_vst3::Event::Parameter {
            offset: 0,
            id: 0,
            value: 0.25,
        },
        native_vst3::Event::Parameter {
            offset: 128,
            id: 0,
            value: 0.5,
        },
    ];
    assert!(instance
        .process(
            [&input, &input],
            [&mut output_l, &mut output_r],
            &bad_events
        )
        .is_err());
    instance
        .process([&input, &input], [&mut output_l, &mut output_r], &[])
        .unwrap();
    assert_eq!(output_l, [1.0; 128]);
    for event in [
        native_vst3::Event::Parameter {
            offset: 0,
            id: 999,
            value: 0.5,
        },
        native_vst3::Event::Parameter {
            offset: 0,
            id: 0,
            value: f64::NAN,
        },
        native_vst3::Event::Midi {
            offset: 0,
            port: 2,
            data: [0x90, 60, 100],
        },
    ] {
        assert!(instance
            .process([&input, &input], [&mut output_l, &mut output_r], &[event])
            .is_err());
    }
    assert!(instance
        .process([&input[..64], &input], [&mut output_l, &mut output_r], &[])
        .is_err());
    assert!(instance.load_state(b"bad").is_err());
}
#[test]
fn independent_instances_survive_drop_and_reload() {
    for _ in 0..3 {
        let mut first = instance();
        let mut second = instance();
        let input = [1.0f32; 32];
        let mut left = [0.0f32; 32];
        let mut right = [0.0f32; 32];
        first
            .process(
                [&input, &input],
                [&mut left, &mut right],
                &[native_vst3::Event::Parameter {
                    offset: 0,
                    id: 0,
                    value: 0.125,
                }],
            )
            .unwrap();
        assert_eq!(left, [0.125; 32]);
        drop(first);
        second
            .process([&input, &input], [&mut left, &mut right], &[])
            .unwrap();
        assert_eq!(left, [1.0; 32]);
    }
}
#[test]
fn rejects_missing_id_and_invalid_configuration() {
    assert!(native_vst3::load(fixture(), "missing", 48_000.0, 128).is_err());
    let id = native_vst3::list_plugins(fixture()).unwrap().remove(0).id;
    assert!(native_vst3::load(fixture(), &id, f64::NAN, 128).is_err());
    assert!(native_vst3::load(fixture(), &id, 48_000.0, 0).is_err());
    assert!(native_vst3::list_plugins("/nonexistent/opendaw.vst3").is_err());
}

#[test]
fn midi_notes_reach_plugin_at_sample_offsets() {
    let mut instance = instance();
    let input = [1.0f32; 128];
    let mut left = [0.0f32; 128];
    let mut right = [0.0f32; 128];
    instance
        .process(
            [&input, &input],
            [&mut left, &mut right],
            &[
                native_vst3::Event::Midi {
                    offset: 32,
                    port: 0,
                    data: [0x90, 60, 127],
                },
                native_vst3::Event::Midi {
                    offset: 96,
                    port: 0,
                    data: [0x80, 60, 0],
                },
            ],
        )
        .unwrap();
    assert_eq!(&left[..96], &[1.0; 96]);
    assert_eq!(&left[96..], &[0.0; 32]);
    assert!(instance
        .process(
            [&input, &input],
            [&mut left, &mut right],
            &[native_vst3::Event::Midi {
                offset: 0,
                port: 0,
                data: [0xb0, 1, 64]
            },]
        )
        .is_err());
}

#[test]
fn adjacent_blocks_preserve_voice_state_without_restarting_processing() {
    let mut instance = instance();
    let input = [1.0f32; 128];
    let mut left = [0.0f32; 128];
    let mut right = [0.0f32; 128];
    instance
        .process(
            [&input, &input],
            [&mut left, &mut right],
            &[native_vst3::Event::Midi {
                offset: 0,
                port: 0,
                data: [0x90, 60, 63],
            }],
        )
        .unwrap();
    let sustained = left[127];
    assert!((sustained - 63.0 / 127.0).abs() < 1e-6);
    for _ in 0..3 {
        instance
            .process([&input, &input], [&mut left, &mut right], &[])
            .unwrap();
        assert_eq!(left, [sustained; 128]);
    }
}

#[test]
fn linux_bundle_path_selects_x86_64_binary() {
    let bundle =
        std::env::temp_dir().join(format!("opendaw-vst3-bundle-{}.vst3", std::process::id()));
    let binaries = bundle.join("Contents/x86_64-linux");
    std::fs::create_dir_all(&binaries).unwrap();
    std::fs::copy(fixture(), binaries.join("fixture.so")).unwrap();
    let id = native_vst3::list_plugins(&bundle).unwrap().remove(0).id;
    drop(native_vst3::load(&bundle, &id, 48_000.0, 128).unwrap());
    std::fs::remove_dir_all(bundle).unwrap();
}

/// Opt-in coverage for plugins whose child interfaces have independent reference counts.
/// Keep third-party binaries outside the repository; subprocess capture observes native stderr.
#[test]
fn third_party_plugin_teardown_when_configured() {
    let Some(path) = std::env::var_os("OPENDAW_VST3_TEST_PLUGIN") else {
        eprintln!(
            "Third-party teardown smoke skipped; set OPENDAW_VST3_TEST_PLUGIN to a VST3 path"
        );
        return;
    };
    if std::env::var_os("OPENDAW_VST3_TEARDOWN_CHILD").is_some() {
        let path = PathBuf::from(path);
        let id = native_vst3::list_plugins(&path).unwrap().remove(0).id;
        let input = [0.0f32; 128];
        let mut left = [0.0f32; 128];
        let mut right = [0.0f32; 128];
        for _ in 0..3 {
            let mut plugin = native_vst3::load(&path, &id, 48_000.0, 128).unwrap();
            for _ in 0..3 {
                plugin
                    .process([&input, &input], [&mut left, &mut right], &[])
                    .unwrap();
            }
            drop(plugin);
        }
        return;
    }
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "third_party_plugin_teardown_when_configured",
            "--nocapture",
        ])
        .env("OPENDAW_VST3_TEARDOWN_CHILD", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        result.status.success(),
        "plugin lifecycle subprocess failed: {stderr}"
    );
    assert!(
        !stderr.contains("asked to delete"),
        "child interface release-order warning: {stderr}"
    );
}
