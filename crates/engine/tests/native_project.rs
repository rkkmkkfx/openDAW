#![cfg(feature = "native")]
use engine::native::{NativeEngine, NativeError};

#[test]
fn production_typescript_project_matches_wasm_render_and_checksum() {
    let mut engine = NativeEngine::new(48_000.0).unwrap();
    engine.load_project(include_bytes!("fixtures/vaporisateur.od")).unwrap();
    assert_eq!(&engine.checksum(), include_bytes!("fixtures/vaporisateur.checksum"));
    engine.bind().unwrap();
    engine.stop(); engine.play();
    let golden = include_bytes!("fixtures/vaporisateur.wasm.f32");
    let mut maximum = 0.0f32;
    let mut energy = 0.0f64;
    for quantum in 0..20 {
        let output = engine.render().unwrap();
        for (frame, actual) in output.into_iter().enumerate() {
            let offset = (quantum * 256 + frame) * 4;
            let expected = f32::from_le_bytes(golden[offset..offset+4].try_into().unwrap());
            assert!(actual.is_finite());
            maximum = maximum.max((expected-actual).abs());
            energy += (actual as f64).powi(2);
        }
    }
    assert!(energy > 1.0,"audible project, energy={energy}");
    println!("baseline native/WASM maximum absolute difference: {maximum}, energy: {energy}");
    assert!(maximum < 1.0e-5,"native/WASM sample maximum difference {maximum}");
    assert!((engine.position()-102.4).abs()<1.0e-6);
    engine.stop(); assert!(engine.render().unwrap().iter().all(|&v|v==0.0));
}
#[test]
fn native_instances_on_different_threads_have_independent_host_contexts() {
    let threads: Vec<_> = [44_100.0,48_000.0].into_iter().map(|rate| std::thread::spawn(move || {
        let mut engine = NativeEngine::new(rate).unwrap();
        engine.load_project(include_bytes!("fixtures/vaporisateur.od")).unwrap();
        engine.bind().unwrap(); engine.play();
        let output = engine.render().unwrap();
        assert!(output.iter().any(|&v|v!=0.0));
        engine.position()
    })).collect();
    let positions: Vec<_> = threads.into_iter().map(|thread|thread.join().unwrap()).collect();
    assert!(positions[0]>positions[1]);
}
#[test]
fn invalid_envelopes_do_not_change_loaded_checksum() {
    let mut engine=NativeEngine::new(48_000.0).unwrap();
    engine.load_project(include_bytes!("fixtures/vaporisateur.od")).unwrap();
    let checksum=engine.checksum();
    assert_eq!(engine.load_project(b"OPEN"),Err(NativeError::InvalidSnapshot));
    assert_eq!(engine.checksum(),checksum);
}

#[test]
fn malformed_snapshots_transactions_and_values_leave_loaded_graph_unchanged() {
    use boxgraph::{address::Address, bytes::ByteWriter};
    let mut engine = NativeEngine::new(48_000.0).unwrap();
    engine.load_project(include_bytes!("fixtures/vaporisateur.od")).unwrap();
    let checksum = engine.checksum();
    for count in [-1i32, i32::MAX] {
        assert_eq!(engine.load_snapshot(&count.to_be_bytes()), Err(NativeError::InvalidSnapshot));
        assert_eq!(engine.apply_updates(&count.to_be_bytes()), Err(NativeError::InvalidTransaction));
    }
    for value in [0.0, f32::NAN, f32::INFINITY] {
        let mut writer = ByteWriter::new();
        writer.write_int(1);
        writer.write_string("update-primitive");
        Address::of([0,0,0,0,0,0,0x40,0,0x80,0,0,0,0,0,0,2], vec![5]).write(&mut writer);
        writer.write_string("float32");
        writer.write_float(value);
        assert!(matches!(engine.apply_updates(writer.as_slice()), Err(NativeError::InvalidValue(_))));
        assert_eq!(engine.checksum(), checksum);
    }
    engine.bind().unwrap(); engine.stop(); engine.play();
    assert!(engine.render().unwrap().iter().any(|&value| value != 0.0));
}

#[test]
fn a_note_missing_its_required_pitch_is_rejected_before_binding() {
    use boxgraph::graph::BoxGraph;
    let graph = BoxGraph::from_bytes(include_bytes!("fixtures/vaporisateur.snapshot"), &studio_boxes::registry()).unwrap();
    let mut boxes = Vec::new();
    for name in graph.box_names().into_iter().collect::<std::collections::BTreeSet<_>>() {
        for graph_box in graph.find_all_by_name(name) {
            let mut graph_box = graph_box.clone();
            if name == "NoteEventBox" { graph_box.fields.remove(&20); }
            boxes.push(graph_box);
        }
    }
    let mut engine = NativeEngine::new(48_000.0).unwrap();
    assert!(matches!(engine.load_snapshot(&BoxGraph::from_boxes(boxes).to_bytes()), Err(NativeError::InvalidValue(_))));
}

#[test]
fn invalid_pointer_roles_are_rejected_for_snapshots_and_bound_live_updates() {
    use boxgraph::{address::Address, bytes::ByteWriter, graph::BoxGraph, updates::Update};
    let registry = studio_boxes::registry();
    let bytes = include_bytes!("fixtures/vaporisateur.snapshot");
    let graph = BoxGraph::from_bytes(bytes, &registry).unwrap();
    let unit = graph.find_all_by_name("AudioUnitBox").into_iter()
        .find(|unit| graph.field_value(&Address::of(unit.uuid, vec![1])).and_then(|value| value.as_str()) == Some("instrument")).unwrap().uuid;
    let collection = graph.find_by_name("NoteEventCollectionBox").unwrap().uuid;
    let region = graph.find_by_name("NoteRegionBox").unwrap().uuid;
    for (source, target) in [(Address::of(unit, vec![25]), Address::of(collection, vec![1])),
        (Address::of(region, vec![2]), Address::of(unit, vec![20]))] {
        let mut malformed = BoxGraph::from_bytes(bytes, &registry).unwrap();
        malformed.transaction(&[Update::Pointer {address: source.clone(), old: None, new: Some(target.clone())}], &registry).unwrap();
        let mut engine = NativeEngine::new(48_000.0).unwrap();
        assert!(matches!(engine.load_snapshot(&malformed.to_bytes()), Err(NativeError::InvalidValue(_))));
        engine.load_snapshot(bytes).unwrap(); engine.bind().unwrap();
        let checksum = engine.checksum();
        let mut writer = ByteWriter::new();
        writer.write_int(1); writer.write_string("update-pointer"); source.write(&mut writer);
        writer.write_bool(true); target.write(&mut writer);
        assert!(matches!(engine.apply_updates(writer.as_slice()), Err(NativeError::InvalidValue(_))));
        assert_eq!(engine.checksum(), checksum);
        engine.stop(); engine.play(); assert!(engine.render().unwrap().iter().any(|&value| value != 0.0));
    }
}

#[test]
fn production_forward_dump_and_live_gain_edit_match_wasm_checksums_and_audio() {
    let mut engine = NativeEngine::new(48_000.0).unwrap();
    engine.apply_updates(include_bytes!("fixtures/vaporisateur.initial-update")).unwrap();
    assert_eq!(&engine.checksum(), include_bytes!("fixtures/vaporisateur.checksum"));
    engine.bind().unwrap(); engine.stop(); engine.play();
    for _ in 0..20 { engine.render().unwrap(); }
    engine.apply_updates(include_bytes!("fixtures/vaporisateur.gain-update")).unwrap();
    assert_eq!(&engine.checksum(), include_bytes!("fixtures/vaporisateur.gain-checksum"));
    engine.stop(); engine.play();
    let golden = include_bytes!("fixtures/vaporisateur.gain.wasm.f32");
    let mut maximum = 0.0f32;
    let mut energy = 0.0f64;
    for quantum in 0..20 {
        for (frame, actual) in engine.render().unwrap().into_iter().enumerate() {
            let offset = (quantum * 256 + frame) * 4;
            let expected = f32::from_le_bytes(golden[offset..offset+4].try_into().unwrap());
            maximum = maximum.max((actual-expected).abs());
            energy += (actual as f64).powi(2);
        }
    }
    assert!(maximum < 1.0e-5, "gain edit native/WASM sample difference {maximum}");
    println!("gain edit native/WASM maximum absolute difference: {maximum}, energy: {energy}");
    assert!(energy > 85.0 && energy < 95.0, "gain edit energy {energy}");
}
