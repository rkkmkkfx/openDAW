import {writeFileSync, readFileSync, mkdirSync} from 'node:fs'
import {ProjectSkeleton} from '../../../../packages/studio/adapters/src/project/ProjectSkeleton'
import {AudioUnitBox, VaporisateurDeviceBox, TrackBox, NoteEventCollectionBox, NoteEventBox, NoteRegionBox} from '@opendaw/studio-boxes'
import {UUID} from '@opendaw/lib-std'
import {TrackType} from '../../../../packages/studio/adapters/src/timeline/TrackType'
import {linkDevice} from '../../../../packages/studio/core-wasm/src/device-linker'
import {connectSyncToEngine} from '../../../../packages/studio/core-wasm/test/helpers/connect-sync'
import type {EngineExports} from '../../../../packages/studio/core-wasm/src/engine-exports'
let uuidCounter = 0
Object.defineProperty(UUID, 'generate', {value: () => UUID.parse(
    `00000000-0000-4000-8000-${(++uuidCounter).toString(16).padStart(12, '0')}`)})
const {boxGraph: graph, mandatoryBoxes:{rootBox,primaryAudioBusBox}} = ProjectSkeleton.empty({createOutputMaximizer:false,createDefaultUser:false})
graph.beginTransaction()
rootBox.created.setValue('2026-10-11T00:00:00.000Z')
const unit=AudioUnitBox.create(graph,UUID.generate(),box=>{box.collection.refer(rootBox.audioUnits);box.output.refer(primaryAudioBusBox.input);box.index.setValue(1)})
VaporisateurDeviceBox.create(graph,UUID.generate(),box=>{
 box.host.refer(unit.input);box.oscillators.fields()[0].waveform.setValue(0);box.oscillators.fields()[0].volume.setValue(-6.0206);
 box.attack.setValue(.001);box.decay.setValue(.001);box.sustain.setValue(1);box.release.setValue(.05);box.cutoff.setValue(20000);box.resonance.setValue(.7)
})
const track=TrackBox.create(graph,UUID.generate(),box=>{box.type.setValue(TrackType.Notes);box.enabled.setValue(true);box.target.refer(unit);box.tracks.refer(unit.tracks)})
const events=NoteEventCollectionBox.create(graph,UUID.generate())
NoteEventBox.create(graph,UUID.generate(),box=>{box.events.refer(events.events);box.position.setValue(0);box.duration.setValue(960);box.pitch.setValue(69);box.velocity.setValue(.8);box.cent.setValue(0)})
NoteRegionBox.create(graph,UUID.generate(),box=>{box.regions.refer(track.regions);box.events.refer(events.owners);box.position.setValue(0);box.duration.setValue(3840);box.loopDuration.setValue(3840)})
graph.endTransaction()
const out='crates/engine/tests/fixtures';mkdirSync(out,{recursive:true})
writeFileSync(out+'/vaporisateur.snapshot',Buffer.from(graph.toArrayBuffer() as ArrayBuffer))
writeFileSync(out+'/vaporisateur.od',Buffer.from(ProjectSkeleton.encode(graph) as ArrayBuffer))
writeFileSync(out+'/vaporisateur.checksum',Buffer.from(graph.checksum()))
console.log('BOXES',graph.boxes().map(box=>box.name))
const memory=new WebAssembly.Memory({initial:256}),table=new WebAssembly.Table({initial:512,element:'anyfunc'})
const wasm='packages/studio/core-wasm/dist/wasm/'
const module=await WebAssembly.compile(readFileSync(process.env.NATIVE_ENGINE_WASM ?? wasm+'engine.wasm'))
const engine=new WebAssembly.Instance(module,{env:{memory,__indirect_function_table:table,host_perf_now:()=>performance.now()*1000}}).exports as unknown as EngineExports
engine.init(48000)
linkDevice(engine,memory,table,await WebAssembly.compile(readFileSync(process.env.NATIVE_VAP_WASM ?? wasm+'plugins/device_vaporisateur.wasm')),'VaporisateurDeviceBox',48000,{})
let inputPointer = 0
const transactions: Array<Uint8Array> = []
const capture: Pick<EngineExports, 'input_reserve' | 'checksum_ptr' | 'apply_updates'> = {
    input_reserve: length => inputPointer = engine.input_reserve(length),
    checksum_ptr: engine.checksum_ptr,
    apply_updates: length => {
        transactions.push(new Uint8Array(memory.buffer, inputPointer, length).slice())
        return engine.apply_updates(length)
    }
}
const sync=connectSyncToEngine(capture,memory,graph);await sync.settle();
writeFileSync(out+'/vaporisateur.initial-update',transactions[0])
engine.bind();await sync.settle();engine.set_metronome_enabled(0);engine.stop();engine.play()
const frames=new Float32Array(20*256)
for(let q=0;q<20;q++){engine.render();frames.set(new Float32Array(memory.buffer,engine.output_ptr(),256),q*256)}
writeFileSync(out+'/vaporisateur.wasm.f32',Buffer.from(frames.buffer))
graph.beginTransaction();unit.volume.setValue(-6.0206);graph.endTransaction();await sync.settle()
writeFileSync(out+'/vaporisateur.gain-update',transactions[transactions.length-1])
writeFileSync(out+'/vaporisateur.gain-checksum',Buffer.from(graph.checksum()))
engine.stop();engine.play()
for(let quantum=0;quantum<20;quantum++){engine.render();frames.set(new Float32Array(memory.buffer,engine.output_ptr(),256),quantum*256)}
writeFileSync(out+'/vaporisateur.gain.wasm.f32',Buffer.from(frames.buffer))
sync.close();console.log('ENERGY',frames.reduce((sum,value)=>sum+value*value,0))
