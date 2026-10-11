use native_render::write_wav;
use std::io;

#[test]
fn arbitrary_source_writes_exact_frames_without_padding() {
    let mut bytes = Vec::new();
    let mut calls = 0;
    write_wav(&mut bytes, 44_100, 129, |buffer| {
        calls += 1;
        buffer.left.fill(0.25);
        buffer.right.fill(-0.25);
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(bytes.len(), 44 + 129 * 4);
    assert_eq!(
        u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
        44_100
    );
    assert_eq!(i16::from_le_bytes(bytes[44..46].try_into().unwrap()), 8192);
    assert_eq!(i16::from_le_bytes(bytes[46..48].try_into().unwrap()), -8192);
}

#[test]
fn source_errors_and_nonfinite_audio_are_not_hidden() {
    let error = write_wav(Vec::new(), 48_000, 1, |_| {
        Err(io::Error::new(io::ErrorKind::InvalidData, "source failed"))
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let error = write_wav(Vec::new(), 48_000, 1, |buffer| {
        buffer.left[0] = f32::NAN;
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn invalid_metadata_is_rejected_before_writes_or_source_calls() {
    let mut bytes = Vec::new();
    let error = write_wav(&mut bytes, 0, 1, |_| panic!("source must not run")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(bytes.is_empty());
}
