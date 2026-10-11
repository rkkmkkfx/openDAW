use dsp::osc::{BandLimitedOscillator, ClassicWaveform};
use engine_env::audio_buffer::shared_audio_buffer;
use engine_env::audio_bus_processor::AudioBusProcessor;
use engine_env::process_info::ProcessInfo;
use engine_env::processor::Processor;
use engine_env::RENDER_QUANTUM;
use std::io::{self, Write};

pub const SAMPLE_RATE: u32 = 48_000;
pub const MAX_FRAMES: u32 = (u32::MAX - 36) / 4;

pub fn write_wav(
    mut writer: impl Write,
    sample_rate: u32,
    frames: u32,
    mut render: impl FnMut(&mut engine_env::audio_buffer::AudioBuffer) -> io::Result<()>,
) -> io::Result<()> {
    if frames > MAX_FRAMES || sample_rate == 0 || sample_rate > u32::MAX / 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid sample rate or frame count exceeds RIFF capacity",
        ));
    }
    let data_size = frames * 4;
    writer.write_all(b"RIFF")?;
    writer.write_all(&(36 + data_size).to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16_u32.to_le_bytes())?;
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&2_u16.to_le_bytes())?;
    writer.write_all(&sample_rate.to_le_bytes())?;
    writer.write_all(&(sample_rate * 4).to_le_bytes())?;
    writer.write_all(&4_u16.to_le_bytes())?;
    writer.write_all(&16_u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_size.to_le_bytes())?;
    let mut buffer = engine_env::audio_buffer::AudioBuffer::new();
    let mut pcm = [0_u8; RENDER_QUANTUM * 4];
    let mut remaining = frames;
    while remaining > 0 {
        let count = remaining.min(RENDER_QUANTUM as u32) as usize;
        buffer.clear();
        render(&mut buffer)?;
        for index in 0..count {
            if !buffer.left[index].is_finite() || !buffer.right[index].is_finite() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "nonfinite audio from renderer",
                ));
            }
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

pub fn render_wav(writer: impl Write, frames: u32) -> io::Result<()> {
    let source = shared_audio_buffer();
    let output = shared_audio_buffer();
    let mut bus = AudioBusProcessor::new(output.clone());
    bus.add_audio_source(source.clone());
    let mut oscillator = BandLimitedOscillator::new(SAMPLE_RATE as f32);
    write_wav(writer, SAMPLE_RATE, frames, |buffer| {
        {
            let mut source_buffer = source.borrow_mut();
            oscillator.generate(
                &mut source_buffer.left,
                1_000.0,
                ClassicWaveform::Sine,
                0,
                RENDER_QUANTUM,
            );
            for index in 0..RENDER_QUANTUM {
                source_buffer.left[index] *= 0.2;
                source_buffer.right[index] = source_buffer.left[index];
            }
        }
        bus.process(&ProcessInfo { blocks: &[] });
        let output = output.borrow();
        buffer.left.copy_from_slice(&output.left);
        buffer.right.copy_from_slice(&output.right);
        Ok(())
    })
}
