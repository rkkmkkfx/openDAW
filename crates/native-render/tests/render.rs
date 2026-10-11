use native_render::render_wav;
use std::io::{self, Write};

fn render(frames: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    render_wav(&mut bytes, frames).unwrap();
    bytes
}

#[test]
fn wav_has_exact_stereo_frame_count_including_partial_blocks() {
    for frames in [0, 1, 127, 128, 129, 48_000] {
        let bytes = render(frames);
        assert_eq!(bytes.len(), 44 + frames as usize * 4);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            36 + frames * 4
        );
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 16);
        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            48_000
        );
        assert_eq!(
            u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
            192_000
        );
        assert_eq!(u16::from_le_bytes(bytes[32..34].try_into().unwrap()), 4);
        assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 16);
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
            frames * 4
        );
    }
}

#[test]
fn audio_matches_a_one_kilohertz_sine_across_quantum_boundaries() {
    let bytes = render(48_000);
    let mut energy = 0.0;
    for index in 0..48_000 {
        let frame = &bytes[44 + index * 4..48 + index * 4];
        let left = i16::from_le_bytes(frame[..2].try_into().unwrap());
        let right = i16::from_le_bytes(frame[2..].try_into().unwrap());
        assert_eq!(left, right);
        let sample = f64::from(left) / 32767.0;
        let expected = 0.2 * (std::f64::consts::TAU * index as f64 / 48.0).sin();
        assert!(
            (sample - expected).abs() < 0.001,
            "sample {index}: {sample} != {expected}"
        );
        energy += sample * sample;
    }
    assert!((energy / 48_000.0 - 0.02).abs() < 0.0001);
}

#[test]
fn output_is_repeatable_and_short_renders_are_prefixes() {
    let full = render(48_000);
    assert_eq!(full, render(48_000));
    assert_eq!(&render(129)[44..], &full[44..44 + 129 * 4]);
}

#[test]
fn oversized_riff_is_rejected_before_writing() {
    let mut bytes = Vec::new();
    assert_eq!(
        render_wav(&mut bytes, u32::MAX).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(bytes.is_empty());
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "test writer",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn write_errors_are_propagated() {
    assert_eq!(
        render_wav(FailingWriter, 128).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}
