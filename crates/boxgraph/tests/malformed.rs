use boxgraph::{boxes::Registry, bytes::ByteReader, graph::BoxGraph, updates};

#[test]
fn negative_or_impossible_counts_fail_before_allocating() {
    for count in [-1i32, i32::MAX, 1] {
        let bytes = count.to_be_bytes();
        assert!(BoxGraph::from_bytes(&bytes, &Registry::new()).is_err());
        assert!(updates::decode(&mut ByteReader::new(&bytes)).is_err());
        assert!(updates::decode_forward(&mut ByteReader::new(&bytes)).is_err());
        assert!(ByteReader::new(&bytes).read_utf16().is_err());
    }
}

#[test]
fn enormous_raw_length_cannot_overflow_the_reader_cursor() {
    let bytes = [0u8, 0];
    let mut reader = ByteReader::new(&bytes);
    reader.read_byte().unwrap();
    assert!(reader.read_raw(usize::MAX).is_err());
    assert_eq!(reader.remaining(), 1);
}

#[test]
fn negative_address_path_count_is_rejected() {
    let mut bytes = vec![0u8; 16];
    bytes.push(255);
    assert!(boxgraph::address::Address::read(&mut ByteReader::new(&bytes)).is_err());
}
