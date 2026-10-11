use clap::{Parser, Subcommand};
use std::{
    fs::File,
    io::{self, BufWriter},
    path::PathBuf,
};
mod backend;
mod scanner;
use backend::{Event, Format, Instance};

#[derive(Parser)]
#[command(about = "openDAW native offline plugin host")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Discover plugins in child processes with bounded output and timeouts.
    Scan {
        roots: Vec<PathBuf>,
        #[arg(long, value_enum, default_value = "clap")]
        format: Format,
    },
    #[command(hide = true)]
    Probe {
        plugin: PathBuf,
        #[arg(long, value_enum)]
        format: Format,
    },
    /// Enumerate plugin IDs from a CLAP library.
    List {
        plugin: PathBuf,
        #[arg(long, value_enum, default_value = "clap")]
        format: Format,
    },
    /// Print parameter IDs and plain value ranges.
    Parameters {
        plugin: PathBuf,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, value_enum, default_value = "clap")]
        format: Format,
    },
    /// Process an existing mono/stereo WAV through a CLAP plugin.
    Render {
        plugin: PathBuf,
        input: PathBuf,
        output: PathBuf,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, value_enum, default_value = "clap")]
        format: Format,
        #[arg(long)]
        state_in: Option<PathBuf>,
        #[arg(long)]
        state_out: Option<PathBuf>,
        #[arg(long)]
        events: Option<PathBuf>,
        #[arg(long = "param", value_parser = parse_parameter)]
        parameters: Vec<(u32, f64)>,
    },
    /// Render a supported openDAW binary snapshot or .od bundle using its native graph.
    RenderProject {
        project: PathBuf,
        output: PathBuf,
        #[arg(long, default_value_t = 48000)]
        frames: u32,
        #[arg(long, default_value_t = 48000)]
        sample_rate: u32,
        /// Optional plugin processing on the native graph's master output.
        #[arg(long)]
        plugin: Option<PathBuf>,
        #[arg(long, requires = "plugin")]
        id: Option<String>,
        #[arg(long, value_enum, default_value = "clap")]
        format: Format,
        #[arg(long = "param", requires = "plugin", value_parser = parse_parameter)]
        parameters: Vec<(u32, f64)>,
    },
}
fn parse_parameter(text: &str) -> Result<(u32, f64), String> {
    let (id, value) = text.split_once('=').ok_or("expected ID=VALUE")?;
    let id = id.parse().map_err(|_| "invalid parameter ID")?;
    let value: f64 = value.parse().map_err(|_| "invalid parameter value")?;
    if !value.is_finite() {
        return Err("parameter value must be finite".into());
    }
    Ok((id, value))
}
fn read_bounded(path: &std::path::Path, limit: u64) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let file = File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    Ok(bytes)
}
fn select_id(
    format: Format,
    path: &std::path::Path,
    id: Option<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(id) = id {
        return Ok(id);
    }
    let descriptors = backend::list(format, path)?;
    if descriptors.len() != 1 {
        return Err("library contains multiple plugins; supply --id".into());
    }
    Ok(descriptors[0].id.clone())
}
struct OutputFile {
    path: PathBuf,
    writer: BufWriter<File>,
    finished: bool,
}
impl OutputFile {
    fn create(path: &std::path::Path) -> io::Result<Self> {
        let file = File::options().write(true).create_new(true).open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: BufWriter::new(file),
            finished: false,
        })
    }
    fn finish(mut self) -> io::Result<()> {
        std::io::Write::flush(&mut self.writer)?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for OutputFile {
    fn drop(&mut self) {
        if !self.finished {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Arguments::parse().command {
        Command::Probe { plugin, format } => {
            println!(
                "\nOPENDAW_SCAN {}",
                serde_json::to_string(&backend::list(format, &plugin)?)?
            );
        }
        Command::Scan { mut roots, format } => {
            if roots.is_empty() {
                let extension = match format {
                    Format::Clap => "clap",
                    Format::Vst3 => "vst3",
                };
                if let Some(home) = std::env::var_os("HOME") {
                    roots.push(PathBuf::from(home).join(format!(".{extension}")));
                }
                roots.extend([
                    PathBuf::from(format!("/usr/lib/{extension}")),
                    PathBuf::from(format!("/usr/local/lib/{extension}")),
                ]);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&scanner::scan(format, &roots))?
            );
        }
        Command::List { plugin, format } => {
            for descriptor in backend::list(format, &plugin)? {
                println!(
                    "{}\t{}\t{}\t{}",
                    descriptor.id, descriptor.name, descriptor.vendor, descriptor.version
                );
            }
        }
        Command::Parameters { plugin, id, format } => {
            let id = select_id(format, &plugin, id)?;
            let instance = Instance::load(format, &plugin, &id, 48000.0)?;
            for parameter in instance.parameters() {
                println!(
                    "{}\t{}\t{}..{}\tdefault {}",
                    parameter.id, parameter.name, parameter.min, parameter.max, parameter.default
                );
            }
        }
        Command::Render {
            plugin,
            input,
            output,
            id,
            format,
            state_in,
            state_out,
            events,
            parameters,
        } => {
            let mut reader = hound::WavReader::open(input)?;
            let spec = reader.spec();
            if !matches!(spec.channels, 1 | 2) || spec.sample_rate == 0 {
                return Err("input must be mono or stereo WAV with a positive sample rate".into());
            }
            let frames = reader.duration();
            if frames > native_render::MAX_FRAMES {
                return Err("input exceeds WAV output capacity".into());
            }
            let id = select_id(format, &plugin, id)?;
            let mut instance = Instance::load(format, &plugin, &id, spec.sample_rate as f64)?;
            if let Some(path) = state_in {
                instance.load_state(&read_bounded(&path, 16 * 1024 * 1024)?)?;
            }
            let metadata = instance.parameters();
            for (id, value) in &parameters {
                let parameter = metadata
                    .iter()
                    .find(|parameter| parameter.id == *id)
                    .ok_or("unknown parameter ID")?;
                if !parameter.accepts(*value) {
                    return Err("invalid parameter value or read-only parameter".into());
                }
            }
            if parameters.len() > 256 {
                return Err("too many initial parameters".into());
            }
            let mut timeline: Vec<Event> = parameters
                .into_iter()
                .map(|(id, value)| Event::Parameter {
                    sample: 0,
                    id,
                    value,
                })
                .collect();
            if let Some(path) = events {
                timeline.extend(serde_json::from_slice::<Vec<Event>>(&read_bounded(
                    &path,
                    16 * 1024 * 1024,
                )?)?);
            }
            timeline.sort_by_key(Event::sample);
            let mut block_counts = std::collections::BTreeMap::new();
            for event in &timeline {
                if event.sample() >= frames {
                    return Err("event outside render duration".into());
                }
                let count = block_counts.entry(event.sample() / 128).or_insert(0);
                *count += 1;
                if *count > 256 {
                    return Err("more than 256 events in one block".into());
                }
                if let Event::Parameter { id, value, .. } = event {
                    let parameter = metadata
                        .iter()
                        .find(|parameter| parameter.id == *id)
                        .ok_or("unknown automation parameter ID")?;
                    if !parameter.accepts(*value) {
                        return Err("invalid automation value or read-only parameter".into());
                    }
                }
            }
            let mut start = 0;
            let mut event_index = 0;
            let mut input_left = [0.0; 128];
            let mut input_right = [0.0; 128];
            let mut output_left = [0.0; 128];
            let mut output_right = [0.0; 128];
            let mut output_file = OutputFile::create(&output)?;
            native_render::write_wav(
                &mut output_file.writer,
                spec.sample_rate,
                frames,
                |buffer| {
                    input_left.fill(0.0);
                    input_right.fill(0.0);
                    for index in 0..128 {
                        let mut sample = || -> io::Result<Option<f32>> {
                            match spec.sample_format {
                                hound::SampleFormat::Float => reader
                                    .samples::<f32>()
                                    .next()
                                    .transpose()
                                    .map_err(io::Error::other),
                                hound::SampleFormat::Int => reader
                                    .samples::<i32>()
                                    .next()
                                    .transpose()
                                    .map(|sample| {
                                        sample.map(|sample| {
                                            sample as f32
                                                / (2.0_f32).powi(spec.bits_per_sample as i32 - 1)
                                        })
                                    })
                                    .map_err(io::Error::other),
                            }
                        };
                        input_left[index] = sample()?.unwrap_or(0.0);
                        input_right[index] = if spec.channels == 1 {
                            input_left[index]
                        } else {
                            sample()?.unwrap_or(0.0)
                        };
                    }
                    let event_end = timeline.partition_point(|event| event.sample() < start + 128);
                    instance
                        .process(
                            [&input_left, &input_right],
                            [&mut output_left, &mut output_right],
                            &timeline[event_index..event_end],
                            start,
                        )
                        .map_err(|failure| io::Error::other(failure.to_string()))?;
                    event_index = event_end;
                    start += 128;
                    buffer.left.copy_from_slice(&output_left);
                    buffer.right.copy_from_slice(&output_right);
                    Ok(())
                },
            )?;
            output_file.finish()?;
            if let Some(path) = state_out {
                let bytes = instance.save_state()?;
                let mut output_file = OutputFile::create(&path)?;
                std::io::Write::write_all(&mut output_file.writer, &bytes)?;
                output_file.finish()?;
            }
            println!(
                "Rendered {frames} stereo frames at {} Hz to {}",
                spec.sample_rate,
                output.display()
            );
        }
        Command::RenderProject {
            project,
            output,
            frames,
            sample_rate,
            plugin,
            id,
            format,
            parameters,
        } => {
            if frames > native_render::MAX_FRAMES {
                return Err("frame count exceeds WAV capacity".into());
            }
            let mut file = File::open(project)?;
            let bytes = {
                use std::io::{Read, Seek};
                let mut magic = [0; 4];
                file.read_exact(&mut magic)?;
                file.rewind()?;
                let mut bytes = Vec::new();
                if magic == *b"PK\x03\x04" {
                    let mut archive = zip::ZipArchive::new(file)?;
                    let mut version = String::new();
                    archive
                        .by_name("version")?
                        .take(2)
                        .read_to_string(&mut version)?;
                    if version != "1" {
                        return Err("unsupported openDAW bundle version".into());
                    }
                    archive
                        .by_name("project.od")?
                        .take(64 * 1024 * 1024 + 1)
                        .read_to_end(&mut bytes)?;
                } else {
                    file.take(64 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                }
                if bytes.len() > 64 * 1024 * 1024 {
                    return Err("project snapshot exceeds 64 MiB".into());
                }
                bytes
            };
            let mut engine = engine::native::NativeEngine::new(sample_rate as f32)?;
            if bytes.starts_with(b"OPEN") {
                engine.load_project(&bytes)?;
            } else {
                engine.load_snapshot(&bytes)?;
            }
            engine.bind()?;
            engine.stop();
            engine.play();
            let mut plugin = plugin
                .map(|path| -> backend::Result<Instance> {
                    let id = select_id(format, &path, id)?;
                    Instance::load(format, &path, &id, sample_rate as f64)
                })
                .transpose()?;
            let events: Vec<_> = parameters
                .into_iter()
                .map(|(id, value)| Event::Parameter {
                    sample: 0,
                    id,
                    value,
                })
                .collect();
            if events.len() > 256 {
                return Err("too many initial parameters".into());
            }
            if let Some(instance) = &plugin {
                let metadata = instance.parameters();
                for event in &events {
                    if let Event::Parameter { id, value, .. } = event {
                        let parameter = metadata
                            .iter()
                            .find(|parameter| parameter.id == *id)
                            .ok_or("unknown parameter ID")?;
                        if !parameter.accepts(*value) {
                            return Err("invalid parameter value or read-only parameter".into());
                        }
                    }
                }
            }
            let mut first = true;
            let mut output_file = OutputFile::create(&output)?;
            native_render::write_wav(&mut output_file.writer, sample_rate, frames, |buffer| {
                let audio = engine.render().map_err(io::Error::other)?;
                if let Some(instance) = &mut plugin {
                    instance
                        .process(
                            [&audio[..128], &audio[128..]],
                            [&mut buffer.left, &mut buffer.right],
                            if first { &events } else { &[] },
                            0,
                        )
                        .map_err(|failure| io::Error::other(failure.to_string()))?;
                } else {
                    buffer.left.copy_from_slice(&audio[..128]);
                    buffer.right.copy_from_slice(&audio[128..]);
                }
                first = false;
                Ok(())
            })?;
            output_file.finish()?;
            println!(
                "Rendered {frames} native graph frames to {}",
                output.display()
            );
        }
    }
    Ok(())
}
fn main() {
    if let Err(failure) = run() {
        eprintln!("native-host: {failure}");
        std::process::exit(1);
    }
}
