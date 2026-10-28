import { afterEach, describe, expect, it, vi } from 'vitest'
import { h, nextTick, ref } from 'vue'
import { clientComponent, MemoryNode as Node, memoryRenderer as renderer } from '../helpers/component-host'
import Tracks from '../../src/features/recording/components/RecordingProjectTracks.vue'
import type { RecordingProject } from '../../src/features/recording/recording-annotations'
const Component=clientComponent(Tracks,new URL('../../src/features/recording/components/RecordingProjectTracks.vue',import.meta.url))
let root=new Node('root')
const seed=():RecordingProject=>({version:1,width:640,height:480,fps:30,durationMs:5000,format:'mp4',annotations:[{id:'a-1',shape:{kind:'arrow',start:{x:10,y:10},end:{x:80,y:50},color:'#ff0000',width:2},startMs:1000,endMs:4000,fadeOutMs:500,order:0}]})
function find(test:(n:Node)=>boolean,current=root):Node|undefined { if(test(current))return current; for(const child of current.children){const found=find(test,child);if(found)return found} }
async function mount(){
  root=new Node('root'); const project=ref(seed()),update=vi.fn(),seek=vi.fn()
  renderer.render(h({setup:()=>()=>h(Component,{project:project.value,currentMs:2000,disabled:false,onUpdate:(tracks:RecordingProject['annotations'])=>{update(tracks);project.value={...project.value,annotations:tracks}},onSeek:seek})}),root)
  await nextTick();return{project,update,seek}
}
afterEach(()=>renderer.render(null,root))
describe('录屏标注轨道编辑',()=>{
  it('拖动两端调整媒体时间，端点不能倒置或超过原片',async()=>{
    const {project,update}=await mount()
    const start=find(n=>n.props['aria-label']==='标注开始端点')!
    ;(start.props.onChange as (e:unknown)=>void)({target:{valueAsNumber:4900}});await nextTick()
    expect(project.value.annotations[0].startMs).toBe(3980)
    const end=find(n=>n.props['aria-label']==='标注结束端点')!
    ;(end.props.onChange as (e:unknown)=>void)({target:{valueAsNumber:100000}});await nextTick()
    expect(project.value.annotations[0].endMs).toBe(5000)
    expect(update).toHaveBeenCalledTimes(2)
  })
  it('选中轨道定位到开始时间，并能重新编辑图形样式',async()=>{
    const {project,seek}=await mount()
    const name=find(n=>n.type==='button'&&n.text.includes('箭头'))!
    ;(name.props.onClick as ()=>void)();await nextTick()
    expect(seek).toHaveBeenCalledWith(1000)
    const color=find(n=>n.type==='input'&&n.props.type==='color')!
    ;(color.props.onChange as (e:unknown)=>void)({target:{value:'#123456'}});await nextTick()
    expect(project.value.annotations[0].shape.color).toBe('#123456')
  })
  it('删除和添加只更新项目轨道，生成唯一标识',async()=>{
    const {project}=await mount()
    const remove=find(n=>n.props['aria-label']==='删除标注')!
    ;(remove.props.onClick as ()=>void)();await nextTick()
    expect(project.value.annotations).toEqual([])
    const add=find(n=>n.type==='button'&&n.text==='添加标注')!
    ;(add.props.onClick as ()=>void)();await nextTick()
    expect(project.value.annotations[0]).toMatchObject({id:'edit-1',startMs:2000,endMs:5000,shape:{kind:'rectangle'}})
    ;(add.props.onClick as ()=>void)();await nextTick()
    expect(new Set(project.value.annotations.map(t=>t.id)).size).toBe(2)
  })
})
