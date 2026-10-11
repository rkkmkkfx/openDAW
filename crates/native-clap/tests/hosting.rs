use std::{path::PathBuf, process::Command, sync::OnceLock};
fn fixture() -> PathBuf {
    static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-plugin/Cargo.toml");
            let target = std::env::temp_dir().join("opendaw-native-clap-fixture");
            assert!(Command::new(env!("CARGO"))
                .args(["build", "--offline", "--manifest-path"])
                .arg(manifest)
                .arg("--target-dir")
                .arg(&target)
                .status()
                .unwrap()
                .success());
            target.join(format!(
                "debug/{}native_clap_fixture{}",
                std::env::consts::DLL_PREFIX,
                std::env::consts::DLL_SUFFIX
            ))
        })
        .clone()
}
#[test]
fn discovers_real_dynamic_plugin_descriptor() {
    let plugins = native_clap::list_plugins(fixture()).unwrap();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "org.opendaw.fixture.gain");
    assert_eq!(plugins[0].name, "Fixture gain");
    assert_eq!(plugins[0].vendor, "");
}
#[test]
fn gain_automation_is_sample_accurate_and_state_restores() {
    use native_clap::Event;
    let mut plugin =
        native_clap::load(fixture(), "org.opendaw.fixture.gain", 48000.0, 128).unwrap();
    assert_eq!(plugin.parameters()[0].id, 7);
    let input = [1.0; 8];
    let mut left = [0.0; 8];
    let mut right = [0.0; 8];
    plugin
        .process(
            [&input, &input],
            [&mut left, &mut right],
            &[Event::Parameter {
                offset: 4,
                id: 7,
                value: 0.25,
            }],
        )
        .unwrap();
    assert_eq!(left, [1.0, 1.0, 1.0, 1.0, 0.25, 0.25, 0.25, 0.25]);
    assert_eq!(right, left);
    let state = plugin.save_state().unwrap();
    plugin
        .process(
            [&input, &input],
            [&mut left, &mut right],
            &[Event::Parameter {
                offset: 0,
                id: 7,
                value: 0.75,
            }],
        )
        .unwrap();
    assert_eq!(left, [0.75; 8]);
    plugin.load_state(&state).unwrap();
    plugin
        .process([&input, &input], [&mut left, &mut right], &[])
        .unwrap();
    assert_eq!(left, [0.25; 8]);
    drop(plugin);
    let mut again = native_clap::load(fixture(), "org.opendaw.fixture.gain", 44100.0, 8).unwrap();
    again
        .process([&input, &input], [&mut left, &mut right], &[])
        .unwrap();
    assert_eq!(left, [1.0; 8]);
}
#[test]
fn rejects_invalid_configuration_and_bounded_events() {
    use native_clap::Event;
    assert!(native_clap::load(fixture(), "missing", 48000.0, 128).is_err());
    assert!(native_clap::load(fixture(), "org.opendaw.fixture.gain", f64::NAN, 128).is_err());
    let mut plugin = native_clap::load(fixture(), "org.opendaw.fixture.gain", 48000.0, 8).unwrap();
    let input = [1.0; 8];
    let mut left = [0.0; 8];
    let mut right = [0.0; 8];
    for event in [
        Event::Parameter {
            offset: 8,
            id: 7,
            value: 1.0,
        },
        Event::Parameter {
            offset: 0,
            id: 123,
            value: 1.0,
        },
        Event::Parameter {
            offset: 0,
            id: 7,
            value: 9.0,
        },
    ] {
        assert!(plugin
            .process([&input, &input], [&mut left, &mut right], &[event])
            .is_err());
    }
    let events = vec![
        Event::Parameter {
            offset: 0,
            id: 7,
            value: 1.0
        };
        257
    ];
    assert!(plugin
        .process([&input, &input], [&mut left, &mut right], &events)
        .is_err());
    assert!(plugin
        .process([&input, &input], [&mut left[..4], &mut right], &[])
        .is_err());
    assert!(plugin.load_state(&[0]).is_err());
}
#[test]
fn midi_events_keep_offsets_and_collect_plugin_output() {
    use native_clap::Event;
    let mut plugin = native_clap::load(fixture(), "org.opendaw.fixture.gain", 48000.0, 8).unwrap();
    let input = [1.0; 8];
    let mut left = [0.0; 8];
    let mut right = [0.0; 8];
    plugin
        .process(
            [&input, &input],
            [&mut left, &mut right],
            &[Event::Midi {
                offset: 3,
                port: 0,
                data: [0xb0, 7, 0],
            }],
        )
        .unwrap();
    assert_eq!(left, [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert!(matches!(
        plugin.output_events(),
        [Event::Midi {
            offset: 3,
            port: 0,
            data: [0xb0, 7, 0]
        }]
    ));
}
#[test]
fn processing_lifecycle_stays_started_between_adjacent_blocks() {
    let mut plugin = native_clap::load(fixture(), "org.opendaw.fixture.gain", 48000.0, 8).unwrap();
    let input = [1.0; 8];
    let mut left = [0.0; 8];
    let mut right = [0.0; 8];
    for _ in 0..2 {
        plugin
            .process([&input, &input], [&mut left, &mut right], &[])
            .unwrap();
    }
    assert_eq!(plugin.parameter_value(8).unwrap(), 1.0);
    assert_eq!(plugin.parameter_value(7).unwrap(), 1.0);
}
#[test]
fn callback_and_restart_requests_run_between_audio_blocks() {
    use native_clap::Event;
    let mut plugin = native_clap::load(fixture(), "org.opendaw.fixture.gain", 48000.0, 8).unwrap();
    let input = [1.0; 8];
    let mut left = [0.0; 8];
    let mut right = [0.0; 8];
    plugin
        .process(
            [&input, &input],
            [&mut left, &mut right],
            &[Event::Midi {
                offset: 0,
                port: 0,
                data: [0xb0, 119, 0],
            }],
        )
        .unwrap();
    assert_eq!(left, [1.0; 8]);
    plugin
        .process([&input, &input], [&mut left, &mut right], &[])
        .unwrap();
    assert_eq!(left, [0.5; 8]);
    plugin
        .process(
            [&input, &input],
            [&mut left, &mut right],
            &[Event::Midi {
                offset: 0,
                port: 0,
                data: [0xb0, 118, 0],
            }],
        )
        .unwrap();
    plugin
        .process([&input, &input], [&mut left, &mut right], &[])
        .unwrap();
    assert_eq!(plugin.parameter_value(9).unwrap(), 2.0);
}
#[test]
fn rejects_out_of_block_and_per_note_plugin_output() {
    use native_clap::Event;
    let mut plugin = native_clap::load(fixture(), "org.opendaw.fixture.gain", 48000.0, 8).unwrap();
    let input = [1.0; 8];
    let mut left = [0.0; 8];
    let mut right = [0.0; 8];
    let events = [
        Event::Midi {
            offset: 0,
            port: 0,
            data: [0xb0, 117, 0],
        },
        Event::Midi {
            offset: 0,
            port: 0,
            data: [0xb0, 116, 0],
        },
    ];
    plugin
        .process([&input, &input], [&mut left, &mut right], &events)
        .unwrap();
    assert_eq!(plugin.rejected_output_events(), 2);
    assert_eq!(plugin.output_events().len(), 2);
    assert!(plugin
        .output_events()
        .iter()
        .all(|event| matches!(event, Event::Midi { offset: 0, .. })));
}
#[cfg(unix)]
#[test]
fn preserves_non_utf8_plugin_path_for_entry_initialization() {
    use std::os::unix::ffi::OsStringExt;
    let directory = std::env::temp_dir().join(std::ffi::OsString::from_vec(
        format!("opendaw-clap-path-{}-", std::process::id())
            .into_bytes()
            .into_iter()
            .chain([0xff])
            .collect(),
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("fixture.clap");
    std::fs::copy(fixture(), &path).unwrap();
    let result = native_clap::list_plugins(&path);
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(result.unwrap()[0].id, "org.opendaw.fixture.gain");
}
#[test]
fn rejects_fractional_plain_values_for_stepped_parameters() {
    use native_clap::Event;
    let mut instance =
        native_clap::load(fixture(), "org.opendaw.fixture.gain", 48000.0, 128).unwrap();
    let input = [0.0; 128];
    let mut left = [0.0; 128];
    let mut right = [0.0; 128];
    assert!(instance
        .process(
            [&input, &input],
            [&mut left, &mut right],
            &[Event::Parameter {
                offset: 0,
                id: 10,
                value: 0.5
            }]
        )
        .is_err());
    assert!(instance
        .process(
            [&input, &input],
            [&mut left, &mut right],
            &[Event::Parameter {
                offset: 0,
                id: 10,
                value: 1.0
            }]
        )
        .is_ok());
}
