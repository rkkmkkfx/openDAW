use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::path::Path;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Format {
    Clap,
    Vst3,
}
#[derive(Serialize, Deserialize)]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
}
pub struct Parameter {
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub stepped: bool,
    pub read_only: bool,
}
impl Parameter {
    pub fn accepts(&self, value: f64) -> bool {
        value.is_finite()
            && !self.read_only
            && value >= self.min
            && value <= self.max
            && (!self.stepped || value.fract() == 0.0)
    }
}
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Parameter {
        sample: u32,
        id: u32,
        value: f64,
    },
    Midi {
        sample: u32,
        #[serde(default)]
        port: u16,
        data: [u8; 3],
    },
}
impl Event {
    pub fn sample(&self) -> u32 {
        match self {
            Self::Parameter { sample, .. } | Self::Midi { sample, .. } => *sample,
        }
    }
}
pub fn list(format: Format, path: &Path) -> Result<Vec<Descriptor>> {
    Ok(match format {
        Format::Clap => native_clap::list_plugins(path)?
            .into_iter()
            .map(|descriptor| Descriptor {
                id: descriptor.id,
                name: descriptor.name,
                vendor: descriptor.vendor,
                version: descriptor.version,
            })
            .collect(),
        Format::Vst3 => native_vst3::list_plugins(path)?
            .into_iter()
            .map(|descriptor| Descriptor {
                id: descriptor.id,
                name: descriptor.name,
                vendor: descriptor.vendor,
                version: descriptor.version,
            })
            .collect(),
    })
}
pub enum Instance {
    Clap(native_clap::Instance),
    Vst3(native_vst3::Instance),
}
impl Instance {
    pub fn load(format: Format, path: &Path, id: &str, sample_rate: f64) -> Result<Self> {
        Ok(match format {
            Format::Clap => Self::Clap(native_clap::load(path, id, sample_rate, 128)?),
            Format::Vst3 => Self::Vst3(native_vst3::load(path, id, sample_rate, 128)?),
        })
    }
    pub fn parameters(&self) -> Vec<Parameter> {
        match self {
            Self::Clap(instance) => instance
                .parameters()
                .iter()
                .map(|parameter| Parameter {
                    id: parameter.id,
                    name: parameter.name.clone(),
                    min: parameter.min,
                    max: parameter.max,
                    default: parameter.default,
                    stepped: parameter.flags & clap_sys::ext::params::CLAP_PARAM_IS_STEPPED != 0,
                    read_only: parameter.flags & clap_sys::ext::params::CLAP_PARAM_IS_READONLY != 0,
                })
                .collect(),
            Self::Vst3(instance) => instance
                .parameters()
                .iter()
                .map(|parameter| Parameter {
                    id: parameter.id,
                    name: parameter.name.clone(),
                    min: 0.0,
                    max: 1.0,
                    default: parameter.default,
                    stepped: false,
                    read_only: parameter.is_read_only,
                })
                .collect(),
        }
    }
    pub fn process(
        &mut self,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
        events: &[Event],
        start: u32,
    ) -> Result<()> {
        match self {
            Self::Clap(instance) => {
                let events: Vec<_> = events
                    .iter()
                    .map(|event| match *event {
                        Event::Parameter { sample, id, value } => native_clap::Event::Parameter {
                            offset: sample - start,
                            id,
                            value,
                        },
                        Event::Midi { sample, port, data } => native_clap::Event::Midi {
                            offset: sample - start,
                            port,
                            data,
                        },
                    })
                    .collect();
                instance.process(input, output, &events)?;
            }
            Self::Vst3(instance) => {
                let events: Vec<_> = events
                    .iter()
                    .map(|event| match *event {
                        Event::Parameter { sample, id, value } => native_vst3::Event::Parameter {
                            offset: sample - start,
                            id,
                            value,
                        },
                        Event::Midi { sample, port, data } => native_vst3::Event::Midi {
                            offset: sample - start,
                            port,
                            data,
                        },
                    })
                    .collect();
                instance.process(input, output, &events)?;
            }
        }
        Ok(())
    }
    pub fn save_state(&mut self) -> Result<Vec<u8>> {
        Ok(match self {
            Self::Clap(instance) => instance.save_state()?,
            Self::Vst3(instance) => instance.save_state()?,
        })
    }
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<()> {
        match self {
            Self::Clap(instance) => instance.load_state(bytes)?,
            Self::Vst3(instance) => instance.load_state(bytes)?,
        };
        Ok(())
    }
}
