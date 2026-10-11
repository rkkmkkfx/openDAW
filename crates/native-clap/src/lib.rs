use clap_sys::{
    audio_buffer::*,
    entry::*,
    events::*,
    ext::{audio_ports::*, note_ports::*, params::*, state::*, thread_check::*},
    factory::plugin_factory::*,
    host::*,
    plugin::*,
    process::*,
    stream::*,
    version::*,
};
use libloading::Library;
use std::{
    ffi::{CStr, CString},
    path::Path,
};
#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
fn error(message: impl Into<String>) -> Error {
    Error(message.into())
}
#[derive(Debug, Clone)]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
}
struct LoadedLibrary {
    _library: Library,
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
}
impl LoadedLibrary {
    fn open(path: &Path) -> Result<Self, Error> {
        let path = std::fs::canonicalize(path).map_err(|failure| error(failure.to_string()))?;
        #[cfg(unix)]
        let encoded = {
            use std::os::unix::ffi::OsStrExt;
            CString::new(path.as_os_str().as_bytes()).map_err(|_| error("NUL in plugin path"))?
        };
        #[cfg(not(unix))]
        let encoded = CString::new(
            path.to_str()
                .ok_or_else(|| error("plugin path is not UTF-8"))?,
        )
        .map_err(|_| error("NUL in plugin path"))?;
        unsafe {
            let library = Library::new(&path).map_err(|failure| error(failure.to_string()))?;
            let entry = *library
                .get::<*const clap_plugin_entry>(b"clap_entry\0")
                .map_err(|failure| error(failure.to_string()))?;
            if entry.is_null() || !clap_version_is_compatible((*entry).clap_version) {
                return Err(error("incompatible CLAP entry"));
            }
            if !(*entry).init.ok_or_else(|| error("missing entry init"))?(encoded.as_ptr()) {
                return Err(error("entry initialization failed"));
            }
            let mut loaded = Self {
                _library: library,
                entry,
                factory: std::ptr::null(),
            };
            loaded.factory = (*entry)
                .get_factory
                .ok_or_else(|| error("missing factory callback"))?(
                CLAP_PLUGIN_FACTORY_ID.as_ptr()
            )
            .cast();
            if loaded.factory.is_null() {
                return Err(error("missing CLAP plugin factory"));
            }
            Ok(loaded)
        }
    }
    fn descriptors(&self) -> Result<Vec<Descriptor>, Error> {
        unsafe {
            let factory = &*self.factory;
            let count = factory
                .get_plugin_count
                .ok_or_else(|| error("missing plugin count"))?(self.factory);
            if count > 4096 {
                return Err(error("plugin descriptor capacity exceeded"));
            }
            (0..count)
                .map(|index| {
                    let descriptor = factory
                        .get_plugin_descriptor
                        .ok_or_else(|| error("missing descriptor callback"))?(
                        self.factory, index
                    );
                    if descriptor.is_null()
                        || !clap_version_is_compatible((*descriptor).clap_version)
                    {
                        return Err(error("invalid plugin descriptor"));
                    }
                    let descriptor = &*descriptor;
                    Ok(Descriptor {
                        id: string(descriptor.id)?,
                        name: string(descriptor.name)?,
                        vendor: optional_string(descriptor.vendor),
                        version: optional_string(descriptor.version),
                    })
                })
                .collect()
        }
    }
}
impl Drop for LoadedLibrary {
    fn drop(&mut self) {
        unsafe {
            if let Some(deinit) = (*self.entry).deinit {
                deinit();
            }
        }
    }
}
unsafe fn string(pointer: *const std::ffi::c_char) -> Result<String, Error> {
    if pointer.is_null() {
        return Err(error("null plugin string"));
    }
    Ok(CStr::from_ptr(pointer).to_string_lossy().into_owned())
}
unsafe fn optional_string(pointer: *const std::ffi::c_char) -> String {
    if pointer.is_null() {
        String::new()
    } else {
        CStr::from_ptr(pointer).to_string_lossy().into_owned()
    }
}
pub fn list_plugins(path: impl AsRef<Path>) -> Result<Vec<Descriptor>, Error> {
    LoadedLibrary::open(path.as_ref())?.descriptors()
}
#[derive(Clone, Copy, Debug)]
pub enum Event {
    Parameter {
        offset: u32,
        id: u32,
        value: f64,
    },
    Midi {
        offset: u32,
        port: u16,
        data: [u8; 3],
    },
}
#[derive(Debug, Clone)]
pub struct Parameter {
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub flags: u32,
}

use std::{
    ffi::{c_char, c_void},
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    thread::{self, ThreadId},
};
pub const EVENT_CAPACITY: usize = 256;
pub const STATE_CAPACITY: usize = 16 * 1024 * 1024;
struct HostData {
    owner: ThreadId,
    audio: AtomicBool,
    restart: AtomicBool,
    callback: AtomicBool,
    process: AtomicBool,
}
unsafe fn host_data<'a>(host: *const clap_host) -> &'a HostData {
    &*((*host).host_data as *const HostData)
}
unsafe extern "C" fn is_main(host: *const clap_host) -> bool {
    host_data(host).owner == thread::current().id()
        && !host_data(host).audio.load(Ordering::Relaxed)
}
unsafe extern "C" fn is_audio(host: *const clap_host) -> bool {
    host_data(host).owner == thread::current().id() && host_data(host).audio.load(Ordering::Relaxed)
}
static THREAD_CHECK: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(is_main),
    is_audio_thread: Some(is_audio),
};
unsafe extern "C" fn host_extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    if !id.is_null() && CStr::from_ptr(id) == CLAP_EXT_THREAD_CHECK {
        (&THREAD_CHECK as *const clap_host_thread_check).cast()
    } else {
        std::ptr::null()
    }
}
unsafe extern "C" fn restart(host: *const clap_host) {
    host_data(host).restart.store(true, Ordering::Release);
}
unsafe extern "C" fn callback(host: *const clap_host) {
    host_data(host).callback.store(true, Ordering::Release);
}
unsafe extern "C" fn request_process(host: *const clap_host) {
    host_data(host).process.store(true, Ordering::Release);
}
enum RawEvent {
    Parameter(clap_event_param_value),
    Midi(clap_event_midi),
}
impl RawEvent {
    fn header(&self) -> &clap_event_header {
        match self {
            Self::Parameter(event) => &event.header,
            Self::Midi(event) => &event.header,
        }
    }
}
unsafe extern "C" fn event_count(list: *const clap_input_events) -> u32 {
    (&*((*list).ctx as *const Vec<RawEvent>)).len() as u32
}
unsafe extern "C" fn event_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    (&*((*list).ctx as *const Vec<RawEvent>))
        .get(index as usize)
        .map_or(std::ptr::null(), |event| event.header())
}
struct OutputEvents {
    frames: u32,
    events: Vec<Event>,
    rejected: u64,
}
unsafe extern "C" fn push_event(
    list: *const clap_output_events,
    header: *const clap_event_header,
) -> bool {
    let output = &mut *((*list).ctx as *mut OutputEvents);
    if header.is_null() || output.events.len() == EVENT_CAPACITY || (*header).time >= output.frames
    {
        output.rejected += 1;
        return false;
    }
    let event = match ((*header).space_id, (*header).type_) {
        (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE)
            if (*header).size as usize >= std::mem::size_of::<clap_event_param_value>() =>
        {
            let event = &*(header as *const clap_event_param_value);
            if event.note_id != -1
                || event.port_index != -1
                || event.channel != -1
                || event.key != -1
            {
                output.rejected += 1;
                return false;
            }
            Event::Parameter {
                offset: event.header.time,
                id: event.param_id,
                value: event.value,
            }
        }
        (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI)
            if (*header).size as usize >= std::mem::size_of::<clap_event_midi>() =>
        {
            let event = &*(header as *const clap_event_midi);
            Event::Midi {
                offset: event.header.time,
                port: event.port_index,
                data: event.data,
            }
        }
        _ => {
            output.rejected += 1;
            return false;
        }
    };
    output.events.push(event);
    true
}
pub struct Instance {
    library: LoadedLibrary,
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    data: Box<HostData>,
    descriptor: Descriptor,
    parameters: Vec<Parameter>,
    input_ports: u32,
    midi_ports: Vec<u32>,
    sample_rate: f64,
    max_frames: u32,
    active: bool,
    processing: bool,
    steady_time: i64,
    events: Vec<RawEvent>,
    output: OutputEvents,
    input: [Vec<f32>; 2],
    _not_send: PhantomData<Rc<()>>,
}
pub fn load(
    path: impl AsRef<Path>,
    id: &str,
    sample_rate: f64,
    max_frames: u32,
) -> Result<Instance, Error> {
    if !sample_rate.is_finite() || sample_rate <= 0.0 || max_frames == 0 || max_frames > 1_048_576 {
        return Err(error("invalid processing configuration"));
    }
    let library = LoadedLibrary::open(path.as_ref())?;
    let descriptor = library
        .descriptors()?
        .into_iter()
        .find(|descriptor| descriptor.id == id)
        .ok_or_else(|| error("plugin ID not found"))?;
    let id = CString::new(id).map_err(|_| error("invalid plugin ID"))?;
    let mut data = Box::new(HostData {
        owner: thread::current().id(),
        audio: AtomicBool::new(false),
        restart: AtomicBool::new(false),
        callback: AtomicBool::new(false),
        process: AtomicBool::new(false),
    });
    let host = Box::new(clap_host {
        clap_version: CLAP_VERSION,
        host_data: (&mut *data as *mut HostData).cast(),
        name: c"openDAW offline".as_ptr(),
        vendor: c"openDAW".as_ptr(),
        url: c"https://opendaw.org".as_ptr(),
        version: c"0.1.0".as_ptr(),
        get_extension: Some(host_extension),
        request_restart: Some(restart),
        request_process: Some(request_process),
        request_callback: Some(callback),
    });
    unsafe {
        let plugin = (*library.factory)
            .create_plugin
            .ok_or_else(|| error("missing create callback"))?(
            library.factory, &*host, id.as_ptr()
        );
        if plugin.is_null() {
            return Err(error("plugin construction failed"));
        }
        let mut instance = Instance {
            library,
            plugin,
            _host: host,
            data,
            descriptor,
            parameters: Vec::new(),
            input_ports: 0,
            midi_ports: Vec::new(),
            sample_rate,
            max_frames,
            active: false,
            processing: false,
            steady_time: 0,
            events: Vec::with_capacity(EVENT_CAPACITY),
            output: OutputEvents {
                frames: 0,
                events: Vec::with_capacity(EVENT_CAPACITY),
                rejected: 0,
            },
            input: [
                vec![0.0; max_frames as usize],
                vec![0.0; max_frames as usize],
            ],
            _not_send: PhantomData,
        };
        let plugin = &*instance.plugin;
        if plugin.destroy.is_none()
            || plugin.init.is_none()
            || plugin.activate.is_none()
            || plugin.deactivate.is_none()
            || plugin.start_processing.is_none()
            || plugin.stop_processing.is_none()
            || plugin.process.is_none()
        {
            return Err(error("missing required plugin lifecycle callback"));
        }
        if !(plugin.init.unwrap())(instance.plugin) {
            return Err(error("plugin initialization failed"));
        }
        instance.read_layout()?;
        instance.activate()?;
        Ok(instance)
    }
}
impl Instance {
    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }
    pub fn parameter_value(&self, id: u32) -> Result<f64, Error> {
        unsafe {
            let get = self
                .extension::<clap_plugin_params>(CLAP_EXT_PARAMS)
                .and_then(|params| params.get_value)
                .ok_or_else(|| error("parameter read unsupported"))?;
            let mut value = 0.0;
            if !get(self.plugin, id, &mut value) {
                return Err(error("parameter read failed"));
            }
            Ok(value)
        }
    }
    pub fn output_events(&self) -> &[Event] {
        &self.output.events
    }
    pub fn rejected_output_events(&self) -> u64 {
        self.output.rejected
    }
    pub fn process_requested(&self) -> bool {
        self.data.process.swap(false, Ordering::AcqRel)
    }
    unsafe fn extension<T>(&self, id: &CStr) -> Option<&T> {
        let pointer = (*self.plugin).get_extension?(self.plugin, id.as_ptr());
        (pointer as *const T).as_ref()
    }
    unsafe fn read_layout(&mut self) -> Result<(), Error> {
        let ports = self
            .extension::<clap_plugin_audio_ports>(CLAP_EXT_AUDIO_PORTS)
            .ok_or_else(|| error("audio-port extension required"))?;
        let count = ports
            .count
            .ok_or_else(|| error("missing audio-port count"))?;
        let get = ports.get.ok_or_else(|| error("missing audio-port info"))?;
        let input_count = count(self.plugin, true);
        if input_count > 1 || count(self.plugin, false) != 1 {
            return Err(error(
                "only zero/one stereo input and one stereo output supported",
            ));
        }
        for is_input in [true, false] {
            if is_input && input_count == 0 {
                continue;
            }
            let mut info: clap_audio_port_info = std::mem::zeroed();
            if !get(self.plugin, 0, is_input, &mut info) || info.channel_count != 2 {
                return Err(error("only stereo f32 audio ports supported"));
            }
        }
        self.input_ports = input_count;
        let mut parameters = Vec::new();
        if let Some(params) = self.extension::<clap_plugin_params>(CLAP_EXT_PARAMS) {
            let count =
                params
                    .count
                    .ok_or_else(|| error("missing parameter count"))?(self.plugin);
            if count > 65536 {
                return Err(error("parameter capacity exceeded"));
            }
            for index in 0..count {
                let mut info: clap_param_info = std::mem::zeroed();
                if !params
                    .get_info
                    .ok_or_else(|| error("missing parameter info"))?(
                    self.plugin, index, &mut info
                ) {
                    return Err(error("parameter info failed"));
                }
                let name: Vec<u8> = info
                    .name
                    .iter()
                    .take_while(|character| **character != 0)
                    .map(|character| *character as u8)
                    .collect();
                parameters.push(Parameter {
                    id: info.id,
                    name: String::from_utf8_lossy(&name).into_owned(),
                    min: info.min_value,
                    max: info.max_value,
                    default: info.default_value,
                    flags: info.flags,
                });
            }
        }
        self.parameters = parameters;
        let mut midi_ports = Vec::new();
        if let Some(ports) = self.extension::<clap_plugin_note_ports>(CLAP_EXT_NOTE_PORTS) {
            let count = ports
                .count
                .ok_or_else(|| error("missing note-port count"))?(
                self.plugin, true
            );
            if count > 256 {
                return Err(error("note-port capacity exceeded"));
            }
            for index in 0..count {
                let mut info: clap_note_port_info = std::mem::zeroed();
                if !ports.get.ok_or_else(|| error("missing note-port info"))?(
                    self.plugin,
                    index,
                    true,
                    &mut info,
                ) {
                    return Err(error("note-port info failed"));
                }
                midi_ports.push(info.supported_dialects);
            }
        }
        self.midi_ports = midi_ports;
        Ok(())
    }
    unsafe fn activate(&mut self) -> Result<(), Error> {
        if !(*self.plugin).activate.unwrap()(self.plugin, self.sample_rate, 1, self.max_frames) {
            return Err(error("plugin activation failed"));
        }
        self.active = true;
        Ok(())
    }
    unsafe fn stop(&mut self) {
        if self.processing {
            self.data.audio.store(true, Ordering::Relaxed);
            (*self.plugin).stop_processing.unwrap()(self.plugin);
            self.data.audio.store(false, Ordering::Relaxed);
            self.processing = false;
        }
    }
    unsafe fn deactivate(&mut self) {
        self.stop();
        if self.active {
            (*self.plugin).deactivate.unwrap()(self.plugin);
            self.active = false;
        }
    }
    pub fn service_main_thread(&mut self) -> Result<(), Error> {
        unsafe {
            if self.data.callback.swap(false, Ordering::AcqRel) {
                self.stop();
                if let Some(callback) = (*self.plugin).on_main_thread {
                    callback(self.plugin);
                }
            }
            if self.data.restart.swap(false, Ordering::AcqRel) {
                self.deactivate();
                self.read_layout()?;
                self.activate()?;
            }
        }
        Ok(())
    }
    pub fn process(
        &mut self,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
        events: &[Event],
    ) -> Result<i32, Error> {
        self.service_main_thread()?;
        let frames = output[0].len();
        if frames == 0
            || frames > self.max_frames as usize
            || output[1].len() != frames
            || (self.input_ports != 0 && input.iter().any(|channel| channel.len() != frames))
        {
            return Err(error("invalid audio buffer lengths"));
        }
        if events.len() > EVENT_CAPACITY {
            return Err(error("input event capacity exceeded"));
        }
        self.events.clear();
        for event in events {
            let (offset, kind, size) = match event {
                Event::Parameter { offset, .. } => (
                    *offset,
                    CLAP_EVENT_PARAM_VALUE,
                    std::mem::size_of::<clap_event_param_value>(),
                ),
                Event::Midi { offset, .. } => (
                    *offset,
                    CLAP_EVENT_MIDI,
                    std::mem::size_of::<clap_event_midi>(),
                ),
            };
            if offset as usize >= frames {
                return Err(error("event outside block"));
            }
            let header = clap_event_header {
                size: size as u32,
                time: offset,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: kind,
                flags: 0,
            };
            let raw = match *event {
                Event::Parameter { id, value, .. } => {
                    let parameter = self
                        .parameters
                        .iter()
                        .find(|parameter| parameter.id == id)
                        .ok_or_else(|| error("unknown parameter ID"))?;
                    if !value.is_finite()
                        || value < parameter.min
                        || value > parameter.max
                        || parameter.flags & CLAP_PARAM_IS_READONLY != 0
                        || (parameter.flags & CLAP_PARAM_IS_STEPPED != 0 && value.fract() != 0.0)
                    {
                        return Err(error("invalid parameter value"));
                    }
                    RawEvent::Parameter(clap_event_param_value {
                        header,
                        param_id: id,
                        cookie: std::ptr::null_mut(),
                        note_id: -1,
                        port_index: -1,
                        channel: -1,
                        key: -1,
                        value,
                    })
                }
                Event::Midi { port, data, .. } => {
                    if data[0] < 0x80 || data[1] > 127 || data[2] > 127 {
                        return Err(error("invalid MIDI message bytes"));
                    }
                    if self
                        .midi_ports
                        .get(port as usize)
                        .is_none_or(|dialects| dialects & CLAP_NOTE_DIALECT_MIDI == 0)
                    {
                        return Err(error("unsupported MIDI input port"));
                    }
                    RawEvent::Midi(clap_event_midi {
                        header,
                        port_index: port,
                        data,
                    })
                }
            };
            let insertion = self
                .events
                .partition_point(|existing| existing.header().time <= offset);
            self.events.insert(insertion, raw);
        }
        if !self.active {
            return Err(error("plugin is inactive"));
        }
        for channel in 0..2 {
            if self.input_ports != 0 {
                self.input[channel][..frames].copy_from_slice(input[channel]);
            }
            output[channel].fill(0.0);
        }
        let mut input_pointers = [self.input[0].as_mut_ptr(), self.input[1].as_mut_ptr()];
        let mut output_pointers = [output[0].as_mut_ptr(), output[1].as_mut_ptr()];
        let input_buffer = clap_audio_buffer {
            data32: input_pointers.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let mut output_buffer = clap_audio_buffer {
            data32: output_pointers.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let input_events = clap_input_events {
            ctx: (&mut self.events as *mut Vec<RawEvent>).cast(),
            size: Some(event_count),
            get: Some(event_get),
        };
        self.output.frames = frames as u32;
        self.output.events.clear();
        let output_events = clap_output_events {
            ctx: (&mut self.output as *mut OutputEvents).cast(),
            try_push: Some(push_event),
        };
        let process = clap_process {
            steady_time: self.steady_time,
            frames_count: frames as u32,
            transport: std::ptr::null(),
            audio_inputs: if self.input_ports == 0 {
                std::ptr::null()
            } else {
                &input_buffer
            },
            audio_outputs: &mut output_buffer,
            audio_inputs_count: self.input_ports,
            audio_outputs_count: 1,
            in_events: &input_events,
            out_events: &output_events,
        };
        self.data.audio.store(true, Ordering::Relaxed);
        let status = unsafe {
            if !self.processing {
                if !(*self.plugin).start_processing.unwrap()(self.plugin) {
                    self.data.audio.store(false, Ordering::Relaxed);
                    return Err(error("start processing failed"));
                }
                self.processing = true;
            }
            (*self.plugin).process.unwrap()(self.plugin, &process)
        };
        self.data.audio.store(false, Ordering::Relaxed);
        self.steady_time = self.steady_time.saturating_add(frames as i64);
        if !(CLAP_PROCESS_CONTINUE..=CLAP_PROCESS_SLEEP).contains(&status) {
            return Err(error("plugin process failed or returned invalid status"));
        }
        Ok(status)
    }
    pub fn save_state(&mut self) -> Result<Vec<u8>, Error> {
        self.service_main_thread()?;
        unsafe {
            let save = self
                .extension::<clap_plugin_state>(CLAP_EXT_STATE)
                .and_then(|state| state.save)
                .ok_or_else(|| error("state save unsupported"))?;
            self.deactivate();
            let mut bytes = Vec::new();
            let stream = clap_ostream {
                ctx: (&mut bytes as *mut Vec<u8>).cast(),
                write: Some(write_state),
            };
            let success = save(self.plugin, &stream);
            self.activate()?;
            if !success {
                return Err(error("state save failed"));
            }
            Ok(bytes)
        }
    }
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > STATE_CAPACITY {
            return Err(error("state capacity exceeded"));
        }
        self.service_main_thread()?;
        unsafe {
            let load = self
                .extension::<clap_plugin_state>(CLAP_EXT_STATE)
                .and_then(|state| state.load)
                .ok_or_else(|| error("state load unsupported"))?;
            self.deactivate();
            let mut reader = StateReader { bytes, position: 0 };
            let stream = clap_istream {
                ctx: (&mut reader as *mut StateReader<'_>).cast(),
                read: Some(read_state),
            };
            let success = load(self.plugin, &stream);
            self.read_layout()?;
            self.activate()?;
            if !success {
                return Err(error("state load failed"));
            }
            Ok(())
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            self.deactivate();
            if let Some(destroy) = (*self.plugin).destroy {
                destroy(self.plugin);
            }
        }
        let _ = &self.library;
    }
}
unsafe extern "C" fn write_state(
    stream: *const clap_ostream,
    buffer: *const c_void,
    size: u64,
) -> i64 {
    let bytes = &mut *((*stream).ctx as *mut Vec<u8>);
    if size > STATE_CAPACITY.saturating_sub(bytes.len()) as u64 || (size != 0 && buffer.is_null()) {
        return -1;
    }
    if size != 0 {
        bytes.extend_from_slice(std::slice::from_raw_parts(buffer.cast(), size as usize));
    }
    size as i64
}
struct StateReader<'a> {
    bytes: &'a [u8],
    position: usize,
}
unsafe extern "C" fn read_state(
    stream: *const clap_istream,
    buffer: *mut c_void,
    size: u64,
) -> i64 {
    let reader = &mut *((*stream).ctx as *mut StateReader<'_>);
    let count = size.min((reader.bytes.len() - reader.position) as u64) as usize;
    if count != 0 && buffer.is_null() {
        return -1;
    }
    if count != 0 {
        std::ptr::copy_nonoverlapping(
            reader.bytes.as_ptr().add(reader.position),
            buffer.cast(),
            count,
        );
    }
    reader.position += count;
    count as i64
}
