use clap_sys::{entry::*, factory::plugin_factory::*, plugin::*, version::*};
use std::ffi::{c_char, c_void, CStr};
static DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: b"org.opendaw.fixture.gain\0".as_ptr().cast(),
    name: b"Fixture gain\0".as_ptr().cast(),
    vendor: std::ptr::null(),
    url: b"\0".as_ptr().cast(),
    manual_url: b"\0".as_ptr().cast(),
    support_url: b"\0".as_ptr().cast(),
    version: b"1.0\0".as_ptr().cast(),
    description: b"Test fixture\0".as_ptr().cast(),
    features: std::ptr::null(),
};
unsafe extern "C" fn init(path: *const c_char) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::path::Path::new(std::ffi::OsStr::from_bytes(CStr::from_ptr(path).to_bytes())).is_file()
    }
    #[cfg(not(unix))]
    {
        CStr::from_ptr(path)
            .to_str()
            .is_ok_and(|path| std::path::Path::new(path).is_file())
    }
}
unsafe extern "C" fn deinit() {}
unsafe extern "C" fn count(_: *const clap_plugin_factory) -> u32 {
    1
}
unsafe extern "C" fn descriptor(
    _: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    if index == 0 {
        &DESCRIPTOR
    } else {
        std::ptr::null()
    }
}
static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(count),
    get_plugin_descriptor: Some(descriptor),
    create_plugin: Some(create),
};
unsafe extern "C" fn factory(id: *const c_char) -> *const c_void {
    if CStr::from_ptr(id) == CLAP_PLUGIN_FACTORY_ID {
        (&FACTORY as *const clap_plugin_factory).cast()
    } else {
        std::ptr::null()
    }
}
#[no_mangle]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(init),
    deinit: Some(deinit),
    get_factory: Some(factory),
};
use clap_sys::{
    events::*,
    ext::{audio_ports::*, note_ports::*, params::*, state::*, thread_check::*},
    host::*,
    process::*,
    stream::*,
};
struct Gain {
    plugin: clap_plugin,
    host: *const clap_host,
    gain: f64,
    active: bool,
    processing: bool,
    starts: u32,
    activations: u32,
}
unsafe fn gain<'a>(plugin: *const clap_plugin) -> &'a mut Gain {
    &mut *((*plugin).plugin_data as *mut Gain)
}
unsafe fn thread_is(plugin: *const clap_plugin, audio: bool) -> bool {
    let host = gain(plugin).host;
    let extension = (*host).get_extension.unwrap()(host, CLAP_EXT_THREAD_CHECK.as_ptr())
        as *const clap_host_thread_check;
    if extension.is_null() {
        return false;
    }
    if audio {
        (*extension).is_audio_thread.unwrap()(host)
    } else {
        (*extension).is_main_thread.unwrap()(host)
    }
}
unsafe extern "C" fn plugin_init(plugin: *const clap_plugin) -> bool {
    thread_is(plugin, false)
}
unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
    assert!(thread_is(plugin, false));
    assert!(!gain(plugin).active);
    drop(Box::from_raw((*plugin).plugin_data as *mut Gain));
}
unsafe extern "C" fn activate(plugin: *const clap_plugin, _: f64, _: u32, _: u32) -> bool {
    assert!(thread_is(plugin, false));
    assert!(!gain(plugin).active);
    gain(plugin).active = true;
    gain(plugin).activations += 1;
    true
}
unsafe extern "C" fn deactivate(plugin: *const clap_plugin) {
    assert!(thread_is(plugin, false));
    assert!(!gain(plugin).processing);
    gain(plugin).active = false;
}
unsafe extern "C" fn start(plugin: *const clap_plugin) -> bool {
    assert!(thread_is(plugin, true));
    assert!(gain(plugin).active);
    assert!(!gain(plugin).processing);
    gain(plugin).processing = true;
    gain(plugin).starts += 1;
    true
}
unsafe extern "C" fn stop(plugin: *const clap_plugin) {
    assert!(thread_is(plugin, true));
    assert!(gain(plugin).processing);
    gain(plugin).processing = false;
}
unsafe extern "C" fn process(plugin: *const clap_plugin, process: *const clap_process) -> i32 {
    assert!(thread_is(plugin, true));
    assert!(gain(plugin).processing);
    let process = &*process;
    let mut next_event = 0;
    let count = (*process.in_events).size.unwrap()(process.in_events);
    for frame in 0..process.frames_count {
        while next_event < count {
            let header = (*process.in_events).get.unwrap()(process.in_events, next_event);
            if (*header).time != frame {
                break;
            }
            if (*header).type_ == CLAP_EVENT_PARAM_VALUE {
                let event = &*(header as *const clap_event_param_value);
                if event.param_id == 7 {
                    gain(plugin).gain = event.value;
                }
            }
            if (*header).type_ == CLAP_EVENT_MIDI {
                let event = &*(header as *const clap_event_midi);
                if event.data[0] == 0xb0 && event.data[1] == 7 {
                    gain(plugin).gain = event.data[2] as f64 / 127.0;
                }
                if event.data[0] == 0xb0 && event.data[1] == 119 {
                    (*gain(plugin).host).request_callback.unwrap()(gain(plugin).host);
                }
                if event.data[0] == 0xb0 && event.data[1] == 118 {
                    (*gain(plugin).host).request_restart.unwrap()(gain(plugin).host);
                }
                if event.data[0] == 0xb0 && event.data[1] == 117 {
                    let mut invalid = *event;
                    invalid.header.time = process.frames_count;
                    (*process.out_events).try_push.unwrap()(process.out_events, &invalid.header);
                }
                if event.data[0] == 0xb0 && event.data[1] == 116 {
                    let targeted = clap_event_param_value {
                        header: clap_event_header {
                            size: std::mem::size_of::<clap_event_param_value>() as u32,
                            time: frame,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_PARAM_VALUE,
                            flags: 0,
                        },
                        param_id: 7,
                        cookie: std::ptr::null_mut(),
                        note_id: 12,
                        port_index: 0,
                        channel: 0,
                        key: 60,
                        value: 1.0,
                    };
                    (*process.out_events).try_push.unwrap()(process.out_events, &targeted.header);
                }
                assert!((*process.out_events).try_push.unwrap()(
                    process.out_events,
                    header
                ));
            }
            next_event += 1;
        }
        for channel in 0..2 {
            let input = *(*process.audio_inputs).data32.add(channel);
            let output = *(*process.audio_outputs).data32.add(channel);
            *output.add(frame as usize) = *input.add(frame as usize) * gain(plugin).gain as f32;
        }
    }
    CLAP_PROCESS_CONTINUE
}
unsafe extern "C" fn port_count(_: *const clap_plugin, _: bool) -> u32 {
    1
}
unsafe extern "C" fn port_get(
    _: *const clap_plugin,
    index: u32,
    _: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    if index != 0 {
        return false;
    }
    *info = clap_audio_port_info {
        id: 0,
        name: [0; 256],
        flags: CLAP_AUDIO_PORT_IS_MAIN,
        channel_count: 2,
        port_type: CLAP_PORT_STEREO.as_ptr(),
        in_place_pair: u32::MAX,
    };
    true
}
static PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(port_count),
    get: Some(port_get),
};
unsafe extern "C" fn param_count(_: *const clap_plugin) -> u32 {
    4
}
unsafe extern "C" fn param_info(
    _: *const clap_plugin,
    index: u32,
    info: *mut clap_param_info,
) -> bool {
    if index >= 4 {
        return false;
    }
    let mut name = [0; 256];
    for (index, byte) in b"Gain".iter().enumerate() {
        name[index] = *byte as c_char;
    }
    *info = clap_param_info {
        id: 7 + index,
        flags: if index == 0 {
            CLAP_PARAM_IS_AUTOMATABLE
        } else if index == 3 {
            CLAP_PARAM_IS_AUTOMATABLE | CLAP_PARAM_IS_STEPPED
        } else {
            CLAP_PARAM_IS_READONLY | CLAP_PARAM_IS_HIDDEN
        },
        cookie: std::ptr::null_mut(),
        name,
        module: [0; 1024],
        min_value: 0.0,
        max_value: if index == 0 { 2.0 } else { 1_000_000.0 },
        default_value: 1.0,
    };
    true
}
unsafe extern "C" fn param_value(plugin: *const clap_plugin, id: u32, value: *mut f64) -> bool {
    if id == 10 {
        *value = 1.0;
        return true;
    }
    if id == 9 {
        *value = gain(plugin).activations as f64;
        return true;
    }
    if id == 8 {
        *value = gain(plugin).starts as f64;
        return true;
    }
    if id != 7 {
        return false;
    }
    *value = gain(plugin).gain;
    true
}
static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(param_count),
    get_info: Some(param_info),
    get_value: Some(param_value),
    value_to_text: None,
    text_to_value: None,
    flush: None,
};
unsafe extern "C" fn save(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool {
    assert!(thread_is(plugin, false));
    assert!(!gain(plugin).active);
    let bytes = gain(plugin).gain.to_le_bytes();
    (*stream).write.unwrap()(stream, bytes.as_ptr().cast(), 8) == 8
}
unsafe extern "C" fn load(plugin: *const clap_plugin, stream: *const clap_istream) -> bool {
    assert!(thread_is(plugin, false));
    assert!(!gain(plugin).active);
    let mut bytes = [0; 8];
    if (*stream).read.unwrap()(stream, bytes.as_mut_ptr().cast(), 8) != 8 {
        return false;
    }
    let value = f64::from_le_bytes(bytes);
    if !value.is_finite() || !(0.0..=2.0).contains(&value) {
        return false;
    }
    gain(plugin).gain = value;
    true
}
static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(save),
    load: Some(load),
};
unsafe extern "C" fn extension(_: *const clap_plugin, id: *const c_char) -> *const c_void {
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_AUDIO_PORTS {
        (&PORTS as *const clap_plugin_audio_ports).cast()
    } else if id == CLAP_EXT_PARAMS {
        (&PARAMS as *const clap_plugin_params).cast()
    } else if id == CLAP_EXT_NOTE_PORTS {
        (&NOTE_PORTS as *const clap_plugin_note_ports).cast()
    } else if id == CLAP_EXT_STATE {
        (&STATE as *const clap_plugin_state).cast()
    } else {
        std::ptr::null()
    }
}
unsafe extern "C" fn create(
    _: *const clap_plugin_factory,
    host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    if CStr::from_ptr(id) != CStr::from_ptr(DESCRIPTOR.id) {
        return std::ptr::null();
    }
    let mut instance = Box::new(Gain {
        plugin: clap_plugin {
            desc: &DESCRIPTOR,
            plugin_data: std::ptr::null_mut(),
            init: Some(plugin_init),
            destroy: Some(destroy),
            activate: Some(activate),
            deactivate: Some(deactivate),
            start_processing: Some(start),
            stop_processing: Some(stop),
            reset: None,
            process: Some(process),
            get_extension: Some(extension),
            on_main_thread: Some(main_callback),
        },
        host,
        gain: 1.0,
        active: false,
        processing: false,
        starts: 0,
        activations: 0,
    });
    instance.plugin.plugin_data = (&mut *instance as *mut Gain).cast();
    let pointer = &instance.plugin as *const clap_plugin;
    let _ = Box::into_raw(instance);
    pointer
}

unsafe extern "C" fn note_count(_: *const clap_plugin, _: bool) -> u32 {
    1
}
unsafe extern "C" fn note_get(
    _: *const clap_plugin,
    index: u32,
    _: bool,
    info: *mut clap_note_port_info,
) -> bool {
    if index != 0 {
        return false;
    }
    *info = clap_note_port_info {
        id: 0,
        supported_dialects: CLAP_NOTE_DIALECT_MIDI,
        preferred_dialect: CLAP_NOTE_DIALECT_MIDI,
        name: [0; 256],
    };
    true
}
static NOTE_PORTS: clap_plugin_note_ports = clap_plugin_note_ports {
    count: Some(note_count),
    get: Some(note_get),
};

unsafe extern "C" fn main_callback(plugin: *const clap_plugin) {
    assert!(thread_is(plugin, false));
    assert!(!gain(plugin).processing);
    gain(plugin).gain = 0.5;
}
