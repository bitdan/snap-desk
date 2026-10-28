import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Vue from 'vue'
import { clientComponent, MemoryNode as Node, memoryRenderer as renderer } from '../helpers/component-host'
const api = vi.hoisted(() => ({ prepareRecordingEditor: vi.fn(), closeRecordingEditor: vi.fn(), trimRecording: vi.fn(), cancelRecordingTrim: vi.fn(), saveRecordingProject: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (token: string) => `recording://${token}` }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ startDragging: vi.fn() }) }))
vi.mock('../../src/features/recording/components/RecordingProjectTracks.vue',async()=>{
  const {defineComponent,h}=await import('vue')
  return{default:defineComponent({props:['project','currentMs','disabled'],emits:['update','seek'],setup:(props,{emit})=>()=>h('project-tracks',{},[h('button',{'aria-label':'测试删除标注',onClick:()=>emit('update',[])})])})}
})
vi.mock('../../src/features/recording/components/RecordingProjectPreview.vue',async()=>{
  const {defineComponent,h}=await import('vue')
  return{default:defineComponent({props:['project','currentMs'],setup:props=>()=>h('project-preview',{'data-count':props.project.annotations.length})})}
})
import Editor from '../../src/features/recording/components/RecordingEditor.vue'
const RecordingEditor = clientComponent(Editor, new URL('../../src/features/recording/components/RecordingEditor.vue', import.meta.url))

let root: Node
let close: ReturnType<typeof vi.fn>
function find(predicate: (node: Node) => boolean, parent = root): Node | undefined {
  if (predicate(parent)) return parent
  for (const child of parent.children) { const found = find(predicate, child); if (found) return found }
}
function control(label: string) { return find(node => node.props['aria-label'] === label)! }
const flush = async () => { for (let i = 0; i < 5; i++) { await Promise.resolve(); await Vue.nextTick() } }
async function mount(format = 'mp4', project: unknown = null) {
  api.prepareRecordingEditor.mockResolvedValue({ token: 9, format, durationMs: 3000, previewAvailable: true, project })
  renderer.render(Vue.h(RecordingEditor, { resultId: 42, disabled: false, floating: true, frameRate: 30, onClose: close,
    layout: { x: -1000, y: 100, width: 800, height: 600, video: { left: 5, top: 0, width: 95, height: 75 }, toolbar: { left: 0, top: 80, width: 100, height: 20 } },
  }), root)
  await flush()
  const media = find(node => node.type === 'video')
  media?.props.onLoadedmetadata(); await flush()
  return media
}
beforeEach(() => {
  vi.resetAllMocks(); root = new Node('root'); close = vi.fn()
  api.closeRecordingEditor.mockResolvedValue(undefined); api.trimRecording.mockResolvedValue(null)
})
afterEach(() => renderer.render(null, root))

describe('原选区内录屏编辑', () => {
  it('uses native placement, commits handle drags once, supports undo and exports the selected interval', async () => {
    await mount()
    expect(control('录屏画面，空格播放，方向键逐帧定位').props.style).toMatchObject({ left: '5%', height: '75%' })
    const handle = control('裁剪开始端点')
    handle.props.onInput({ target: { value: '1000' } }); handle.props.onInput({ target: { value: '1500' } }); handle.props.onChange()
    await flush()
    control('另存所选片段').props.onClick(); await flush()
    expect(api.trimRecording).toHaveBeenCalledWith(9, { startMs: 1500, endMs: 3000 })
    control('撤销裁剪').props.onClick(); await flush()
    expect(control('裁剪开始端点').props.value).toBe(0)
    control('重做裁剪').props.onClick(); await flush()
    expect(control('裁剪开始端点').props.value).toBe(1500)
  })
  it('loops the selected clip and releases media and native token when closed', async () => {
    const media = (await mount())!
    control('循环所选片段').props.onClick()
    await control('播放所选片段').props.onClick(); await flush()
    expect(media.play).toHaveBeenCalledOnce()
    media.currentTime = 3; media.ended = true; media.props.onTimeupdate()
    await media.props.onEnded(); await flush()
    expect(media.currentTime).toBe(0)
    expect(media.play).toHaveBeenCalledTimes(2)
    control('静音预览（不改变导出声音）').props.onClick(); expect(media.muted).toBe(true)
    control('关闭预览，保留已保存录屏').props.onClick(); expect(close).toHaveBeenCalledOnce()
    renderer.render(null, root); await flush()
    expect(media.pause).toHaveBeenCalled()
    expect(media.removeAttribute).toHaveBeenCalledWith('src')
    expect(media.load).toHaveBeenCalledOnce()
    expect(api.closeRecordingEditor).toHaveBeenCalledWith(9)
  })
  it('编辑轨道使用干净原片，保存新轨道后才重新合成导出',async()=>{
    const project={version:1,width:640,height:480,fps:30,durationMs:3000,format:'mp4',annotations:[{id:'a-1',shape:{kind:'arrow',start:{x:0,y:0},end:{x:30,y:30},color:'#ff0000',width:2},startMs:0,endMs:3000,fadeOutMs:0,order:0}]}
    api.saveRecordingProject.mockResolvedValue({...project,annotations:[]})
    const media=(await mount('mp4',project))!
    const toggle=find(n=>n.type==='button'&&n.text==='标注轨道')!
    toggle.props.onClick();await flush()
    expect(media.props.src).toContain('/clean?')
    control('测试删除标注').props.onClick();await flush()
    expect(find(n=>n.type==='project-preview')?.props['data-count']).toBe(0)
    control('另存所选片段').props.onClick();await flush()
    expect(api.saveRecordingProject).toHaveBeenCalledWith(9,[])
    expect(api.trimRecording).toHaveBeenCalledWith(9,{startMs:0,endMs:3000})
    expect(api.saveRecordingProject.mock.invocationCallOrder[0]).toBeLessThan(api.trimRecording.mock.invocationCallOrder[0])
  })
  it('项目保存失败时保留修改，不从旧轨道错误导出',async()=>{
    const project={version:1,width:640,height:480,fps:30,durationMs:3000,format:'mp4',annotations:[]}
    await mount('mp4',project)
    find(n=>n.type==='button'&&n.text==='标注轨道')!.props.onClick();await flush()
    control('测试删除标注').props.onClick();await flush()
    api.saveRecordingProject.mockRejectedValue(new Error('磁盘空间不足'))
    control('另存所选片段').props.onClick();await flush()
    expect(api.trimRecording).not.toHaveBeenCalled()
    const content=(node?:Node):string=>node?node.text+node.children.map(content).join(''):''
    expect(content(find(n=>n.props.role==='alert'))).toContain('磁盘空间不足')
  })
  it('keeps GIF trim handles usable without pretending to provide frame-controlled GIF playback', async () => {
    expect(await mount('gif')).toBeUndefined()
    expect(control('播放所选片段').props.disabled).toBe(true)
    const handle = control('裁剪结束端点')
    expect(handle.props.disabled).toBe(false)
    handle.props.onInput({ target: { value: '1234' } }); handle.props.onChange(); await flush()
    control('另存所选片段').props.onClick(); await flush()
    expect(api.trimRecording).toHaveBeenCalledWith(9, { startMs: 0, endMs: 1230 })
  })
})
