use dsp::osc::{BandLimitedOscillator, ClassicWaveform};
use engine_env::audio_buffer::shared_audio_buffer;
use engine_env::audio_bus_processor::AudioBusProcessor;
use engine_env::process_info::ProcessInfo;
use engine_env::processor::Processor;
use engine_env::RENDER_QUANTUM;
use std::io::{self, Write};

pub const SAMPLE_RATE: u32 = 48_000;
pub const MAX_FRAMES: u32 = (u32::MAX - 36) / 4;

pub fn render_wav(mut writer: impl Write, frames: u32) -> io::Result<()> {
    if frames > MAX_FRAMES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame count exceeds RIFF capacity",
        ));
    }
    let data_size = frames * 4;
    writer.write_all(b"RIFF")?;
    writer.write_all(&(36 + data_size).to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16_u32.to_le_bytes())?;
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&2_u16.to_le_bytes())?;
    writer.write_all(&SAMPLE_RATE.to_le_bytes())?;
    writer.write_all(&(SAMPLE_RATE * 4).to_le_bytes())?;
    writer.write_all(&4_u16.to_le_bytes())?;
    writer.write_all(&16_u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_size.to_le_bytes())?;
    let source = shared_audio_buffer();
    let output = shared_audio_buffer();
    let mut bus = AudioBusProcessor::new(output.clone());
    bus.add_audio_source(source.clone());
    let mut oscillator = BandLimitedOscillator::new(SAMPLE_RATE as f32);
    let mut pcm = [0_u8; RENDER_QUANTUM * 4];
    let mut remaining = frames;
    while remaining > 0 {
        let count = remaining.min(RENDER_QUANTUM as u32) as usize;
        {
            let mut buffer = source.borrow_mut();
            buffer.clear();
            oscillator.generate(&mut buffer.left, 1_000.0, ClassicWaveform::Sine, 0, count);
            for index in 0..count {
                buffer.left[index] *= 0.2;
                buffer.right[index] = buffer.left[index];
            }
        }
        bus.process(&ProcessInfo { blocks: &[] });
        let buffer = output.borrow();
        for index in 0..count {
            let offset = index * 4;
            let left = (buffer.left[index].clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            let right = (buffer.right[index].clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            pcm[offset..offset + 2].copy_from_slice(&left.to_le_bytes());
            pcm[offset + 2..offset + 4].copy_from_slice(&right.to_le_bytes());
        }
        writer.write_all(&pcm[..count * 4])?;
        remaining -= count as u32;
    }
    Ok(())
}
