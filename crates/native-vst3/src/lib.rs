//! Same-thread offline VST3 hosting. Not suitable for a realtime audio callback:
//! the maintained hosting dependency takes locks and may allocate during processing.
use std::{marker::PhantomData, path::Path, rc::Rc};
pub use vst3_host::parameters::Parameter;
use vst3_host::{
    midi::{MidiChannel, MidiEvent},
    AudioBuffers, Plugin, Vst3Host,
};

#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<vst3_host::Error> for Error {
    fn from(e: vst3_host::Error) -> Self {
        Self(e.to_string())
    }
}
fn error(message: &str) -> Error {
    Error(message.into())
}
pub const EVENT_CAPACITY: usize = 256;
pub const STATE_CAPACITY: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone)]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
}
/// Executes plugin code in this process; callers needing crash isolation must use a scanner helper.
pub fn list_plugins(path: impl AsRef<Path>) -> Result<Vec<Descriptor>, Error> {
    let info = vst3_host::discovery::get_detailed_plugin_info(path.as_ref())?;
    Ok(info
        .classes
        .into_iter()
        .filter(|class| class.category == "Audio Module Class")
        .map(|class| Descriptor {
            id: class.class_id,
            name: class.name,
            vendor: info.info.vendor.clone(),
            version: class.version,
        })
        .collect())
}
#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// VST3 values are normalized, including stepped parameters.
    Parameter { offset: u32, id: u32, value: f64 },
    /// MIDI 1 channel messages; this stereo adapter supports event input bus zero only.
    Midi {
        offset: u32,
        port: u16,
        data: [u8; 3],
    },
}
pub struct Instance {
    plugin: Plugin,
    descriptor: Descriptor,
    parameters: Vec<Parameter>,
    buffers: AudioBuffers,
    input_channels: usize,
    midi_input: bool,
    max_frames: usize,
    requires_reload: bool,
    _same_thread: PhantomData<Rc<()>>,
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
    let mut host = Vst3Host::builder()
        .sample_rate(sample_rate)
        .block_size(max_frames as usize)
        .input_channels(2)
        .output_channels(2)
        .build()?;
    let plugin = host.load_plugin_class(path, id)?;
    let layout = plugin.audio_bus_layout()?;
    if layout.inputs.len() > 1
        || layout.outputs.len() != 1
        || layout
            .inputs
            .iter()
            .any(|bus| !bus.active || bus.channel_count != 2)
        || !layout.outputs[0].active
        || layout.outputs[0].channel_count != 2
    {
        return Err(error(
            "only zero/one active stereo input and one active stereo output supported",
        ));
    }
    let info = plugin.info();
    let descriptor = Descriptor {
        id: info.uid.clone(),
        name: info.name.clone(),
        vendor: info.vendor.clone(),
        version: info.version.clone(),
    };
    let midi_input = info.has_midi_input;
    let parameters = plugin.get_parameters()?;
    let input_channels = if layout.inputs.is_empty() { 0 } else { 2 };
    Ok(Instance {
        plugin,
        descriptor,
        parameters,
        buffers: AudioBuffers::new(input_channels, 2, max_frames as usize, sample_rate),
        input_channels,
        midi_input,
        max_frames: max_frames as usize,
        requires_reload: false,
        _same_thread: PhantomData,
    })
}
impl Instance {
    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }
    pub fn latency_samples(&self) -> u32 {
        self.plugin.latency_samples()
    }
    pub fn tail_samples(&self) -> u32 {
        self.plugin.tail_samples()
    }
    /// Validate the complete batch before queuing anything. Preserve input order at equal offsets.
    pub fn process(
        &mut self,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
        events: &[Event],
    ) -> Result<(), Error> {
        let frames = output[0].len();
        if frames == 0
            || frames > self.max_frames
            || output[1].len() != frames
            || (self.input_channels != 0 && input.iter().any(|channel| channel.len() != frames))
        {
            return Err(error("invalid audio buffer lengths"));
        }
        if events.len() > EVENT_CAPACITY {
            return Err(error("input event capacity exceeded"));
        }
        self.service_control()?;
        let mut ordered = events.to_vec();
        for event in &ordered {
            let offset = match *event {
                Event::Parameter { offset, .. } | Event::Midi { offset, .. } => offset,
            };
            if offset as usize >= frames {
                return Err(error("event outside block"));
            }
            match *event {
                Event::Parameter { id, value, .. } => {
                    let parameter = self
                        .parameters
                        .iter()
                        .find(|p| p.id == id)
                        .ok_or_else(|| error("unknown parameter ID"))?;
                    if parameter.is_read_only || !value.is_finite() || !(0.0..=1.0).contains(&value)
                    {
                        return Err(error("invalid normalized parameter value"));
                    }
                }
                Event::Midi { port, data, .. } => {
                    self.validate_midi(port, data)?;
                }
            }
        }
        ordered.sort_by_key(|event| match event {
            Event::Parameter { offset, .. } | Event::Midi { offset, .. } => *offset,
        });
        for event in ordered {
            match event {
                Event::Parameter { offset, id, value } => {
                    self.plugin.set_parameter_at(id, value, offset as i32)?
                }
                Event::Midi { offset, data, .. } => {
                    self.plugin.send_midi_event_at(midi(data)?, offset as i32)?
                }
            }
        }
        self.buffers.block_size = frames;
        for (buffer, channel) in self.buffers.inputs.iter_mut().zip(input) {
            buffer.resize(frames, 0.0);
            buffer.copy_from_slice(channel);
        }
        for channel in &mut self.buffers.outputs {
            channel.resize(frames, 0.0);
            channel.fill(0.0);
        }
        if !self.plugin.is_processing() {
            self.plugin.start_processing()?;
        }
        if let Err(failure) = self.plugin.process_audio(&mut self.buffers) {
            let _ = self.plugin.stop_processing();
            return Err(failure.into());
        }
        self.parameters = self.plugin.get_parameters()?;
        output[0].copy_from_slice(&self.buffers.outputs[0]);
        output[1].copy_from_slice(&self.buffers.outputs[1]);
        Ok(())
    }
    fn service_control(&mut self) -> Result<(), Error> {
        if self.requires_reload {
            return Err(error("plugin requested replacement; reload the instance"));
        }
        let flags = self.plugin.service_host_requests()?;
        if flags.reload_component() {
            self.requires_reload = true;
            return Err(error("plugin requested replacement; reload the instance"));
        }
        let layout = self.plugin.audio_bus_layout()?;
        if layout.inputs.len() * 2 != self.input_channels
            || layout.outputs.len() != 1
            || layout
                .inputs
                .iter()
                .chain(layout.outputs.iter())
                .any(|bus| !bus.active || bus.channel_count != 2)
        {
            return Err(error(
                "plugin bus layout changed; reload a supported stereo instance",
            ));
        }
        if !flags.is_empty() {
            self.parameters = self.plugin.get_parameters()?;
        }
        Ok(())
    }
    fn validate_midi(&self, port: u16, data: [u8; 3]) -> Result<(), Error> {
        if !self.midi_input || port != 0 {
            return Err(error("unsupported MIDI input bus"));
        }
        midi(data)?;
        // VST3 controller messages require a declared parameter mapping. Reject absent mappings
        // rather than the upstream host's silent successful drop.
        let controller = match data[0] & 0xf0 {
            0xb0 => Some(data[1] as u16),
            0xd0 => Some(128),
            0xe0 => Some(129),
            _ => None,
        };
        if let Some(controller) = controller {
            if self
                .plugin
                .midi_cc_to_parameter(0, (data[0] & 0xf) as i16, controller)
                .is_none()
            {
                return Err(error("MIDI controller is not mapped by plugin"));
            }
        }
        Ok(())
    }
    pub fn save_state(&mut self) -> Result<Vec<u8>, Error> {
        self.plugin.stop_processing()?;
        self.service_control()?;
        let bytes = self.plugin.save_state()?;
        if bytes.len() > STATE_CAPACITY {
            return Err(error("state capacity exceeded"));
        }
        Ok(bytes)
    }
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > STATE_CAPACITY {
            return Err(error("state capacity exceeded"));
        }
        self.plugin.stop_processing()?;
        self.plugin.load_state(bytes)?;
        self.service_control()?;
        self.parameters = self.plugin.get_parameters()?;
        Ok(())
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        let _ = self.plugin.stop_processing();
    }
}
fn midi(data: [u8; 3]) -> Result<MidiEvent, Error> {
    if data[0] < 0x80 || data[0] >= 0xf0 || data[1] > 127 || data[2] > 127 {
        return Err(error("invalid MIDI channel message"));
    }
    let channel =
        MidiChannel::from_index(data[0] & 0x0f).ok_or_else(|| error("invalid MIDI channel"))?;
    Ok(match data[0] & 0xf0 {
        0x80 => MidiEvent::NoteOff {
            channel,
            note: data[1],
            velocity: data[2],
        },
        0x90 if data[2] == 0 => MidiEvent::NoteOff {
            channel,
            note: data[1],
            velocity: 0,
        },
        0x90 => MidiEvent::NoteOn {
            channel,
            note: data[1],
            velocity: data[2],
        },
        0xa0 => MidiEvent::PolyAftertouch {
            channel,
            note: data[1],
            pressure: data[2],
        },
        0xb0 => MidiEvent::ControlChange {
            channel,
            controller: data[1],
            value: data[2],
        },
        0xd0 => MidiEvent::ChannelAftertouch {
            channel,
            pressure: data[1],
        },
        0xe0 => MidiEvent::PitchBend {
            channel,
            value: data[1] as u16 | ((data[2] as u16) << 7),
        },
        _ => {
            return Err(error(
                "unsupported MIDI message (program changes require a future program-list adapter)",
            ))
        }
    })
}
