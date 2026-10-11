//! Owned native adapter to the existing graph and transport. Vaporisateur instruments
//! and engine channel strips are supported; all other devices fail validation.
use super::*;
use std::marker::PhantomData;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeError { InvalidSampleRate, InstanceAlreadyActive, InvalidSnapshot,
    InvalidTransaction, UnsupportedBox(String), MissingTimeline, AlreadyBound,
    NotBound, InvalidPosition, InvalidValue(String) }
impl core::fmt::Display for NativeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for NativeError {}
std::thread_local! { static ACTIVE: Cell<bool> = const { Cell::new(false) }; }
/// One owner per thread; this type cannot be sent or shared across threads.
/// The existing host context is thread-local and leased until this owner drops.
pub struct NativeEngine {
    engine: Engine, bound: bool, state: [u8; ENGINE_STATE_LEN],
    _thread: PhantomData<Rc<()>>,
}
impl NativeEngine {
    pub fn new(sample_rate: f32) -> Result<Self, NativeError> {
        if !sample_rate.is_finite() || !(8_000.0..=384_000.0).contains(&sample_rate) {
            return Err(NativeError::InvalidSampleRate);
        }
        if ACTIVE.with(|active| active.replace(true)) { return Err(NativeError::InstanceAlreadyActive); }
        let mut engine = Engine::new(sample_rate);
        let id = engine.device_register(u32::MAX, 0, abi::DEVICE_KIND_INSTRUMENT,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
        engine.set_device_box_type("VaporisateurDeviceBox".into(), id as usize);
        unsafe { PULL.get().scratch.reserve(DEVICE_MAX_EVENTS); }
        Ok(Self { engine, bound: false, state: [0; ENGINE_STATE_LEN], _thread: PhantomData })
    }
    /// Load the existing BoxGraph binary snapshot codec before binding.
    pub fn load_snapshot(&mut self, bytes: &[u8]) -> Result<(), NativeError> {
        if self.bound { return Err(NativeError::AlreadyBound); }
        let graph = BoxGraph::from_bytes(bytes, &self.engine.registry).map_err(|_| NativeError::InvalidSnapshot)?;
        if graph.to_bytes().len() != bytes.len() { return Err(NativeError::InvalidSnapshot); }
        validate(&graph)?;
        self.engine.graph = graph;
        Ok(())
    }
    /// Read the production OPEN/version-2 `.od` envelope, retaining its graph codec.
    pub fn load_project(&mut self, bytes: &[u8]) -> Result<(), NativeError> {
        if bytes.len() < 12 || &bytes[..4] != b"OPEN" || bytes[4..8] != 2i32.to_be_bytes() {
            return Err(NativeError::InvalidSnapshot);
        }
        let length = i32::from_be_bytes(bytes[8..12].try_into().unwrap());
        if length < 0 || length as usize != bytes.len() - 12 { return Err(NativeError::InvalidSnapshot); }
        self.load_snapshot(&bytes[12..])
    }
    /// Validate an isolated candidate before applying the browser's forward codec.
    pub fn apply_updates(&mut self, bytes: &[u8]) -> Result<(), NativeError> {
        let mut reader = ByteReader::new(bytes);
        let updates = decode_forward(&mut reader).map_err(|_| NativeError::InvalidTransaction)?;
        if reader.remaining() != 0 { return Err(NativeError::InvalidTransaction); }
        let mut candidate = BoxGraph::from_bytes(&self.engine.graph.to_bytes(), &self.engine.registry)
            .map_err(|_| NativeError::InvalidSnapshot)?;
        candidate.transaction(&updates, &self.engine.registry).map_err(|_| NativeError::InvalidTransaction)?;
        validate(&candidate)?;
        self.engine.apply_updates(bytes).map_err(|_| NativeError::InvalidTransaction)?;
        if self.bound { self.engine.reconcile_units(); }
        Ok(())
    }
    pub fn bind(&mut self) -> Result<(), NativeError> {
        if self.bound { return Err(NativeError::AlreadyBound); }
        validate(&self.engine.graph)?;
        if self.engine.bind() != 0 { return Err(NativeError::MissingTimeline); }
        self.bound = true;
        Ok(())
    }
    /// Original 128-frame graph quantum, planar L followed by R.
    pub fn render(&mut self) -> Result<[f32; RENDER_QUANTUM * 2], NativeError> {
        if !self.bound { return Err(NativeError::NotBound); }
        let mut output = [0.0; RENDER_QUANTUM * 2];
        self.engine.render(&mut output, &mut self.state);
        Ok(output)
    }
    pub fn checksum(&self) -> [u8; 32] { self.engine.checksum() }
    pub fn state_bytes(&self) -> &[u8] { &self.state }
    pub fn position(&self) -> f64 { self.engine.transport.position() }
    pub fn sample_rate(&self) -> f32 { self.engine.sample_rate }
    pub fn play(&mut self) { self.engine.play(); }
    pub fn stop(&mut self) { self.engine.stop(); }
    pub fn seek(&mut self, position: f64) -> Result<(), NativeError> {
        if !position.is_finite() || position < 0.0 { return Err(NativeError::InvalidPosition); }
        self.engine.transport.seek(position);
        self.engine.paused_locate = !self.engine.transport.is_playing();
        Ok(())
    }
}
impl Drop for NativeEngine {
    fn drop(&mut self) {
        self.engine.stop();
        unsafe {
            *PULL.get() = PullContext::new(); *BASE_FREQUENCY.get() = 440.0;
            INPUTS.get().clear(); IGNORED_REGIONS.get().clear(); SUSPENDED_AUTOMATION.get().clear();
        }
        ACTIVE.with(|active| active.set(false));
    }
}
fn validate(graph: &BoxGraph) -> Result<(), NativeError> {
    let schemas = registry();
    for name in graph.box_names().into_iter().collect::<std::collections::BTreeSet<_>>() {
        match name {
            "RootBox" | "TimelineBox" | "AudioUnitBox" | "AudioBusBox" | "GrooveShuffleBox" | "VaporisateurDeviceBox" |
            "TrackBox" | "NoteRegionBox" | "NoteEventCollectionBox" | "NoteEventBox" |
            "ValueRegionBox" | "ValueEventCollectionBox" | "ValueEventBox" |
            "SignatureEventBox" | "MarkerBox" | "SelectionBox" | "UserInterfaceBox" |
            "ProjectMetaBox" | "MetaDataBox" => {},
            _ => return Err(NativeError::UnsupportedBox(name.into())),
        }
        for graph_box in graph.find_all_by_name(name) {
            let schema = schemas.get(name).ok_or_else(|| NativeError::UnsupportedBox(name.into()))?;
            validate_fields(name, &graph_box.fields, schema, &mut Vec::new())?;
            if name == "TimelineBox" {
                let field = |path| graph.field_value(&Address::of(graph_box.uuid, path));
                let enabled = field(vec![11,1]).and_then(|value| value.as_bool()).unwrap_or(false);
                let from = field(vec![11,2]).and_then(|value| value.as_int32()).unwrap_or(0);
                let to = field(vec![11,3]).and_then(|value| value.as_int32()).unwrap_or(0);
                if enabled && (from < 0 || to <= from) {
                    return Err(NativeError::InvalidValue("TimelineBox.loopArea".into()));
                }
            }
        }
    }
    validate_edges(graph)
}

fn validate_edges(graph: &BoxGraph) -> Result<(), NativeError> {
    for edge in graph.edges() {
        let Some(target) = edge.target else { continue };
        let source_box = graph.find_box(&edge.source.uuid).ok_or(NativeError::InvalidSnapshot)?;
        let target_box = graph.find_box(&target.uuid).ok_or_else(|| NativeError::InvalidValue(format!("dangling pointer {target}")))?;
        if !target.field_keys.is_empty() && graph.field_value(&target).is_none() {
            return Err(NativeError::InvalidValue(format!("missing pointer target {target}")));
        }
        let source = (source_box.name.as_str(), edge.source.field_keys.as_slice());
        let destination = (target_box.name.as_str(), target.field_keys.as_slice());
        let source_valid = match source {
            ("RootBox", [1]) => destination == ("TimelineBox", &[1]),
            ("RootBox", [4]) => destination == ("GrooveShuffleBox", &[]),
            ("RootBox", [100]) => false,
            ("RootBox", [101]) => destination == ("ProjectMetaBox", &[]),
            ("AudioUnitBox", [2]) => destination == ("RootBox", &[20]),
            ("AudioUnitBox", [25]) => destination == ("AudioBusBox", &[3]) || destination == ("RootBox", &[30]),
            ("AudioBusBox", [1]) => destination == ("RootBox", &[21]),
            ("AudioBusBox", [2]) => destination == ("AudioUnitBox", &[22]),
            ("VaporisateurDeviceBox", [1]) => destination == ("AudioUnitBox", &[22]),
            ("TrackBox", [1]) => destination == ("AudioUnitBox", &[20]),
            ("TrackBox", [2]) => {
                let kind = graph.field_value(&Address::of(source_box.uuid, vec![11])).and_then(|value| value.as_int32());
                match kind {
                    Some(1) => destination == ("AudioUnitBox", &[]),
                    Some(3) => matches!(graph.field_value(&target), Some(boxgraph::field::FieldValue::Float32(_) |
                        boxgraph::field::FieldValue::Int32(_) | boxgraph::field::FieldValue::Boolean(_))),
                    _ => false,
                }
            },
            ("NoteRegionBox" | "ValueRegionBox", [1]) => destination == ("TrackBox", &[3]),
            ("NoteRegionBox", [2]) => destination == ("NoteEventCollectionBox", &[2]),
            ("ValueRegionBox", [2]) | ("TimelineBox", [22,1]) => destination == ("ValueEventCollectionBox", &[2]),
            ("NoteEventBox", [1]) => destination == ("NoteEventCollectionBox", &[1]),
            ("ValueEventBox", [1]) => destination == ("ValueEventCollectionBox", &[1]),
            ("MarkerBox", [1]) => destination == ("TimelineBox", &[21,1]),
            ("SignatureEventBox", [1]) => destination == ("TimelineBox", &[23,1]),
            ("UserInterfaceBox", [1]) => destination == ("RootBox", &[2]),
            ("SelectionBox", [1]) => destination == ("UserInterfaceBox", &[10]),
            _ => true,
        };
        if !source_valid { return Err(NativeError::InvalidValue(format!("{} -> {target}: invalid pointer destination", edge.source))); }
        let valid = match (target_box.name.as_str(), target.field_keys.as_slice()) {
            ("TimelineBox", [1]) => source == ("RootBox", &[1]),
            ("TimelineBox", [21,1]) => source == ("MarkerBox", &[1]),
            ("TimelineBox", [23,1]) => source == ("SignatureEventBox", &[1]),
            ("RootBox", [2]) => source == ("UserInterfaceBox", &[1]),
            ("RootBox", [20]) => source == ("AudioUnitBox", &[2]),
            ("RootBox", [21]) => source == ("AudioBusBox", &[1]),
            ("RootBox", [30]) | ("AudioBusBox", [3]) => source == ("AudioUnitBox", &[25]),
            ("RootBox", [10] | [11] | [35]) => false,
            ("AudioUnitBox", [20]) => source == ("TrackBox", &[1]),
            ("AudioUnitBox", [22]) => source == ("VaporisateurDeviceBox", &[1]) || source == ("AudioBusBox", &[2]),
            ("AudioUnitBox", [21] | [23] | [24]) | ("TrackBox", [4]) => false,
            ("TrackBox", [3]) => {
                let kind = graph.field_value(&Address::of(target.uuid, vec![11])).and_then(|value| value.as_int32());
                match kind {
                    Some(1) => source == ("NoteRegionBox", &[1]),
                    Some(3) => source == ("ValueRegionBox", &[1]),
                    _ => false,
                }
            },
            ("NoteEventCollectionBox", [1]) => source == ("NoteEventBox", &[1]),
            ("NoteEventCollectionBox", [2]) => source == ("NoteRegionBox", &[2]),
            ("ValueEventCollectionBox", [1]) => source == ("ValueEventBox", &[1]),
            ("ValueEventCollectionBox", [2]) => source == ("ValueRegionBox", &[2]) || source == ("TimelineBox", &[22,1]),
            _ => true,
        };
        if !valid { return Err(NativeError::InvalidValue(format!("{} -> {target}: invalid pointer role", edge.source))); }
    }
    Ok(())
}

fn validate_fields(name: &str, fields: &boxgraph::field::Fields,
    schema: &boxgraph::field::Schema, path: &mut Vec<u16>) -> Result<(), NativeError> {
    for (key, field_type) in schema {
        path.push(*key);
        let value = fields.get(key).ok_or_else(|| NativeError::InvalidValue(format!("{name}{path:?}: missing field")))?;
        validate_value(name, value, field_type, path)?;
        path.pop();
    }
    Ok(())
}

fn validate_value(name: &str, value: &boxgraph::field::FieldValue,
    field_type: &boxgraph::field::FieldType, path: &mut Vec<u16>) -> Result<(), NativeError> {
    use boxgraph::field::{FieldValue as V, FieldType as T};
    let invalid = || NativeError::InvalidValue(format!("{name}{path:?}"));
    match (value, field_type) {
        (V::Int32(value), T::Int32) => {
            if (name == "TimelineBox" && (path.as_slice() == [10,1] || path.as_slice() == [10,2])) && *value <= 0 {
                return Err(invalid());
            }
            if name == "SignatureEventBox" && (path.as_slice() == [21] || path.as_slice() == [22]) && *value <= 0 {
                return Err(invalid());
            }
            if name == "NoteEventBox" && path.as_slice() == [20] && !(0..=127).contains(value) { return Err(invalid()); }
            if name == "VaporisateurDeviceBox" {
                let id = device_vaporisateur::parameter_paths().iter().find(|(_, keys)| *keys == path.as_slice()).map(|(id,_)| *id);
                let valid = match id {
                    Some(0 | 4 | 22) => (0..=3).contains(value),
                    Some(2 | 6) => (-3..=3).contains(value),
                    Some(15) => (1..=4).contains(value),
                    Some(18) => (0..=1).contains(value),
                    Some(19) => [1,3,5].contains(value),
                    _ => true,
                };
                if !valid { return Err(invalid()); }
            }
        },
        (V::Float32(value), T::Float32) => {
            let gain = (name == "AudioUnitBox" && path.as_slice() == [12]) ||
                (name == "VaporisateurDeviceBox" && path.len() == 3 && path[0] == 40 && path[2] == 2);
            if !value.is_finite() && !(gain && *value == f32::NEG_INFINITY) { return Err(invalid()); }
            if (name == "RootBox" && path.as_slice() == [5] ||
                name == "TimelineBox" && path.as_slice() == [31]) && *value <= 0.0 { return Err(invalid()); }
            if name == "RootBox" && path.as_slice() == [5] && *value > 20_000.0 { return Err(invalid()); }
            if name == "TimelineBox" && path.as_slice() == [31] && *value > 1000.0 { return Err(invalid()); }
            if name == "NoteEventBox" && path.as_slice() == [21] && !(0.0..=1.0).contains(value) { return Err(invalid()); }
            if name == "VaporisateurDeviceBox" {
                let id = device_vaporisateur::parameter_paths().iter().find(|(_, keys)| *keys == path.as_slice()).map(|(id,_)| *id);
                let valid = match id {
                    Some(1 | 5) => *value == f32::NEG_INFINITY || (-72.0..=0.0).contains(value),
                    Some(3 | 7) => (-1200.0..=1200.0).contains(value),
                    Some(8 | 9 | 11) => (0.001..=5.0).contains(value),
                    Some(10 | 17 | 21) => (0.0..=1.0).contains(value),
                    Some(12) => (20.0..=20_000.0).contains(value),
                    Some(13) => (0.01..=10.0).contains(value),
                    Some(14 | 16 | 24 | 25 | 26) => (-1.0..=1.0).contains(value),
                    Some(20) => (1.0..=1200.0).contains(value),
                    Some(23) => (0.0001..=30.0).contains(value),
                    _ => true,
                };
                if !valid { return Err(invalid()); }
            }
        },
        (V::Boolean(_), T::Boolean) | (V::String(_), T::String) |
        (V::Utf16(_), T::String) | (V::Bytes(_), T::Bytes) |
        (V::Pointer(_), T::Pointer) | (V::Hook, T::Hook) => {},
        (V::Object(fields), T::Object(schema)) => validate_fields(name, fields, schema, path)?,
        (V::Array(elements), T::Array {element,length}) if elements.len() == *length => {
            for (index, value) in elements.iter().enumerate() {
                path.push(index as u16);
                validate_value(name, value, element, path)?;
                path.pop();
            }
        },
        _ => return Err(invalid()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use boxgraph::boxes::GraphBox;
    use boxgraph::field::{FieldValue as V, Fields};
    fn b(id: u8, name: &str, fields: &[(u16, V)]) -> GraphBox {
        GraphBox { uuid: [id; 16], creation_index: id as i32, name: name.into(),
            fields: fields.iter().cloned().collect::<Fields>() }
    }
    fn pointer(id: u8, key: u16) -> V { V::Pointer(Some(Address::of([id; 16], vec![key]))) }
    fn fixture() -> BoxGraph {
        BoxGraph::from_boxes(vec![
            b(1, "RootBox", &[(20, V::Hook)]),
            b(2, "TimelineBox", &[(31, V::Float32(120.0))]),
            b(3, "AudioUnitBox", &[(1, V::String("instrument".into())), (2, pointer(1,20)),
                (12, V::Float32(0.0)), (20,V::Hook),(21,V::Hook),(22,V::Hook),(23,V::Hook)]),
            b(4, "VaporisateurDeviceBox", &[(1, pointer(3,22)),(4,V::Boolean(true)),
                (14,V::Float32(20000.0)),(15,V::Float32(0.7)),(16,V::Float32(0.001)),
                (17,V::Float32(0.05)),(18,V::Float32(0.0)),(19,V::Float32(0.001)),
                (20,V::Float32(1.0)),(21,V::Float32(0.0)),(22,V::Int32(1)),(23,V::Int32(1)),
                (24,V::Float32(30.0)),(25,V::Float32(1.0)),(26,V::Int32(1)),(27,V::Float32(0.0)),
                (30,V::Object([(1,V::Int32(0)),(2,V::Float32(0.0001)),(10,V::Float32(0.0)),
                    (11,V::Float32(0.0)),(12,V::Float32(0.0))].into_iter().collect())),
                (40,V::Array(vec![
                    V::Object([(1,V::Int32(0)),(2,V::Float32(-6.0206)),(3,V::Int32(0)),(4,V::Float32(0.0))].into_iter().collect()),
                    V::Object([(1,V::Int32(0)),(2,V::Float32(f32::NEG_INFINITY)),(3,V::Int32(0)),(4,V::Float32(0.0))].into_iter().collect())
                ]))]),
            b(5, "TrackBox", &[(1,pointer(3,20)),(3,V::Hook),(11,V::Int32(1)),(20,V::Boolean(true))]),
            b(6, "NoteRegionBox", &[(1,pointer(5,3)),(2,pointer(7,2)),
                (10,V::Int32(0)),(11,V::Int32(3840)),(12,V::Int32(0)),(13,V::Int32(3840))]),
            b(7, "NoteEventCollectionBox", &[(1,V::Hook),(2,V::Hook)]),
            b(8, "NoteEventBox", &[(1,pointer(7,1)),(10,V::Int32(0)),(11,V::Int32(960)),
                (20,V::Int32(69)),(21,V::Float32(0.8)),(24,V::Float32(0.0))])
        ])
    }
    #[test]
    fn native_snapshot_runs_real_sequencer_stock_vaporisateur_and_master_bus() {
        let graph = BoxGraph::from_bytes(include_bytes!("../tests/fixtures/vaporisateur.snapshot"), &registry()).unwrap();
        let mut engine = NativeEngine::new(48_000.0).unwrap();
        engine.load_snapshot(&graph.to_bytes()).unwrap();
        assert_eq!(engine.checksum(),graph.checksum());
        engine.bind().unwrap(); engine.play();
        let mut energy = 0.0f64;
        for _ in 0..20 {
            let output = engine.render().unwrap();
            assert!(output.iter().all(|value| value.is_finite()));
            assert_eq!(output[..128],output[128..]);
            energy += output.iter().map(|&v| (v as f64).powi(2)).sum::<f64>();
        }
        assert!(energy > 1.0,"real project note must produce audible DSP output, energy {energy}");
        assert!((engine.position() - 102.4).abs() < 1e-6);
        engine.stop();
        assert!(engine.render().unwrap().iter().all(|&v| v == 0.0));
    }
    #[test]
    fn lease_prevents_overlapping_context_and_releases_on_drop() {
        let engine = NativeEngine::new(48_000.0).unwrap();
        assert!(matches!(NativeEngine::new(48_000.0),Err(NativeError::InstanceAlreadyActive)));
        drop(engine); assert!(NativeEngine::new(48_000.0).is_ok());
    }
    #[test]
    fn unsupported_devices_are_rejected_before_binding() {
        let mut engine = NativeEngine::new(48_000.0).unwrap();
        let graph = BoxGraph::from_boxes(vec![b(1,"StereoToolDeviceBox",&[])]);
        assert_eq!(engine.load_snapshot(&graph.to_bytes()),
            Err(NativeError::UnsupportedBox("StereoToolDeviceBox".into())));
    }
    #[test]
    fn incomplete_schema_is_rejected_without_touching_the_live_graph() {
        let mut engine = NativeEngine::new(48_000.0).unwrap();
        engine.load_project(include_bytes!("../tests/fixtures/vaporisateur.od")).unwrap();
        let checksum = engine.checksum();
        assert!(matches!(engine.load_snapshot(&fixture().to_bytes()), Err(NativeError::InvalidValue(_))));
        assert_eq!(engine.checksum(), checksum);
    }
}
