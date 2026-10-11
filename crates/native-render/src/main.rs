use native_render::{render_wav, MAX_FRAMES, SAMPLE_RATE};
use std::fs::OpenOptions;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments
        .next()
        .ok_or("usage: native-render OUTPUT.wav [FRAMES]")?;
    let frames = match arguments.next() {
        Some(value) => value
            .to_str()
            .ok_or("invalid frame count")?
            .parse::<u32>()?,
        None => SAMPLE_RATE,
    };
    if arguments.next().is_some() || frames > MAX_FRAMES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid arguments or frame count exceeds RIFF capacity",
        )
        .into());
    }
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(file);
    render_wav(&mut writer, frames)?;
    writer.flush()?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("native-render: {error}");
            ExitCode::FAILURE
        }
    }
}
