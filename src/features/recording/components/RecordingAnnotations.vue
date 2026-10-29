<script setup lang="ts">
import { recordingPollInterval } from '../polling'
import { useRecordingPolling } from '@/features/recording/use-recording-polling'
import { handleEscape } from '@/shared/ui/escape'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { api } from '@/services/api'
import { t, te } from '@/shared/i18n/i18n'
import type { RecordingView } from '@/features/recording/recording'
import { prepareRecordingRaster, annotationAlpha, type RecordingAnnotationView, type RecordingAnnotationEditorRequest, type RecordingAnnotationTools, type RecordingAnnotationEditorStatus } from '@/features/recording/recording-annotations'
import { useAnnotations, type AnnotationEditor } from '@/features/annotations/use-annotations'
import AnnotationWorkspace from '@/features/annotations/components/AnnotationWorkspace.vue'
import { useRecordingAnnotationLayout } from '../use-recording-annotation-layout'
import type { AnnotationTool } from '@/features/annotations/use-annotations'

const props = defineProps<{ sessionId?: number; initialEditor?: AnnotationEditor }>()
const emit = defineEmits<{ closed: [] }>()
const view = ref<RecordingAnnotationView | null>(null)
const recording = ref<RecordingView | null>(null)
const busy = ref(false), error = ref('')
const toolbar = ref<HTMLElement | null>(null)
const layout = useRecordingAnnotationLayout(view, toolbar, async failure => {
  // 裁剪失败时退回桌面模式，不能留下覆盖桌面的不可操作透明窗口。
  await deactivate(); error.value = String(failure); editor.error = String(failure)
})
const bounds = computed(() => ({ x: 0, y: 0, width: view.value?.width ?? 1, height: view.value?.height ?? 1 }))
const editor = useAnnotations(() => bounds.value, () => '', () => busy.value || !view.value, async shapes => {
  const id = view.value?.id
  if (id === undefined) throw new Error('录制已结束。')
  const next = await api.updateRecordingAnnotations(id, shapes, view.value?.revision)
  if (!disposed && view.value?.id === id) view.value = { ...view.value, ...next }
}, prepareRecordingRaster)
const stage = computed(() => {
  const v = view.value
  return v ? { left: `${100 * v.x / v.viewportWidth}%`, top: `${100 * v.y / v.viewportHeight}%`,
    width: `${100 * v.width / v.viewportWidth}%`, height: `${100 * v.height / v.viewportHeight}%` } : {}
})
let disposed = false, closedThrough = -1
let operation = 0
let off: (() => void) | undefined
let offEditorAction: (() => void) | undefined
const editorConnected=ref(false)
let executingAction=false
let publishing:Promise<void>|null=null
let queuedEditor:ReturnType<typeof editorSnapshot>|null=null
let publishedEditor=''
/** 只在首次加载/外部版本变化时重置历史，轮询不能打断正在画的一笔或撤销栈。 */
async function poll() {
  const known = view.value
  const epoch = operation
  try {
    const [next, status] = await Promise.all([api.recordingAnnotations(known?.revision), api.currentRecording()])
    if (disposed || epoch !== operation) return
    recording.value = status
    let inheritedTools=false
    if (!next || next.id <= closedThrough || (props.sessionId !== undefined && next.id !== props.sessionId)) { view.value = null; editor.reset() }
    else if (!view.value || next.id !== view.value.id) {
      view.value = next; editor.reset(next.shapes ?? []); editor.setTool(props.initialEditor?.tool ?? 'edit'); error.value = ''
      inheritedTools=!!props.initialEditor
      if (props.initialEditor) for (const key of ['color', 'width', 'opacity', 'magnification', 'fontSize', 'fontFamily'] as const) {
        Object.assign(editor, { [key]: props.initialEditor[key] })
      }
    } else {
      // 提交期间的查询可能返回旧版本，不可覆盖刚保存的版本或模式。
      if (next.revision >= view.value.revision && !busy.value && !editor.pending && !executingAction) {
        const changed=next.revision !== view.value.revision
        if (changed) editor.reset(next.shapes ?? [])
        view.value = { ...next, tracks: next.tracks ?? (changed ? [] : view.value.tracks) }
        if (!next.editing) { editor.abort(); editor.selected = -1 }
      }
    }
    const tools = view.value?.tools
    if (tools && !inheritedTools && !editor.interacting && !editor.pending && !publishing && !executingAction) {
      if (editor.tool !== tools.tool) editor.setTool(tools.tool)
      for (const key of ['color', 'width', 'opacity', 'magnification', 'fontSize', 'fontFamily'] as const) Object.assign(editor, { [key]: tools[key] })
    }
  } catch (e) { if (!disposed) error.value = String(e) }
}
/** 截图工具栏读取这个编辑器的状态，工具条窗口不创建第二份撤销栈。 */
function editorSnapshot() {
  if (!view.value || !editorConnected.value) return null
  const tools:RecordingAnnotationTools={...view.value.tools,tool:editor.tool,color:editor.color,width:editor.width,
    opacity:editor.opacity,fontSize:editor.fontSize,fontFamily:editor.fontFamily,magnification:editor.magnification}
  const status:RecordingAnnotationEditorStatus={ready:true,selected:editor.selected,pending:editor.pending,
    interacting:editor.interacting,canUndo:!!editor.history.past.length,canRedo:!!editor.history.future.length,
    text:editor.text,error:editor.error}
  return{id:view.value.id,status,tools}
}
function publishEditor():Promise<void> {
  queuedEditor=editorSnapshot()
  if(publishing) return publishing
  publishing=(async()=>{
    while(queuedEditor && !disposed) {
      const next=queuedEditor; queuedEditor=null
      const signature=JSON.stringify(next)
      if(signature===publishedEditor) continue
      await api.publishRecordingAnnotationEditor(next.id,next.status,next.tools)
      publishedEditor=signature
    }
  })().finally(()=>{publishing=null})
  return publishing
}
watch(()=>JSON.stringify(editorSnapshot()),()=>{
  operation++
  void publishEditor().catch(e=>{if(!disposed && view.value) error.value=String(e)})
},{flush:'post'})
/** 所有编辑命令都在画布的原始 useAnnotations 实例执行，沿用截图工具的历史和样式逻辑。 */
async function editorAction(request:RecordingAnnotationEditorRequest) {
  let failure:string|null=null
  let started=false
  try {
    if(disposed || view.value?.id!==request.id) throw new Error('录屏标注会话已过期。')
    if(executingAction || editor.pending || editor.interacting) throw new Error('请先完成当前标注。')
    executingAction=true; started=true; operation++; editor.error=''
    const action=request.action
    if(action.kind==='tool') editor.setTool(action.tool)
    else if(action.kind==='style') await editor.style(action.style)
    else if(action.kind==='undo' || action.kind==='redo') await editor.travel(action.kind)
    else if(action.kind==='remove') await editor.remove()
    else if(action.kind==='clear') await editor.clear()
    if(editor.error) throw new Error(editor.error)
    await publishEditor()
  } catch(e) { failure=String(e) }
  finally {
    if(started) {executingAction=false; operation++}
    await api.finishRecordingAnnotationEditorAction(request.id,request.requestId,failure).catch(()=>{})
  }
}
useRecordingPolling(poll, () => recording.value?.phase, e => { if (!disposed) error.value = String(e) }, { initialSnapshot: true, intervalMs: () => view.value?.tracks?.some(track => track.endMs !== null && track.endMs > view.value!.mediaTimeMs) ? 33 : recordingPollInterval(recording.value?.phase) })
// 完成标注后整窗穿透，独立录制控制条仍可重新进入绘制。
async function desktop() {
  if (!view.value) return
  const id=view.value.id
  await api.setRecordingAnnotationMode(id, false)
  operation++
  if (view.value?.id===id) { view.value.editing=false; editor.abort(); editor.selected=-1 }
}
async function prepareDesktop() {
  await poll(); await nextTick()
  if (!view.value) throw new Error(error.value || '录屏工具栏尚未就绪。')
  await desktop()
}
defineExpose({ prepareDesktop })
async function deactivate() {
  if (busy.value || editor.pending) return
  editor.abort(); busy.value=true; error.value=''; operation++
  try { await desktop() } catch(e) { error.value=String(e) } finally { busy.value=false }
}
function chooseTool(tool: AnnotationTool) { if (!busy.value) editor.setTool(tool) }
async function setTiming(event: Event, field: 'lifetimeMs' | 'fadeOutMs') {
  if (!view.value || busy.value || editor.pending || editor.interacting) return
  const id = view.value.id
  const input = event.target as HTMLInputElement
  const tools = { ...(editorSnapshot()?.tools ?? view.value.tools), [field]: field === 'lifetimeMs' ? Number(input.value) : input.checked ? 500 : 0 }
  busy.value = true; error.value = ''; operation++
  try {
    await api.setRecordingAnnotationTools(id, tools)
    if (view.value?.id === id) view.value.tools = tools
  } catch (e) { error.value = String(e) }
  finally { busy.value = false; operation++ }
}
/** 透明度只来自原生媒体时钟，绝不修改历史中的原始颜色或透明度。 */
const presentation = computed(() => editor.history.present.map((shape, index) => {
  const track=view.value?.tracks?.find(track => track.order === index)
  return track ? { ...shape, opacity: (shape.opacity ?? 1) * annotationAlpha(track, view.value?.mediaTimeMs ?? 0) } : shape
}))
function key(event: KeyboardEvent) {
  if (!view.value?.editing || event.defaultPrevented || event.isComposing || event.keyCode === 229) return
  if (handleEscape(event, () => {
    if (busy.value || editor.pending) return
    editor.abort(); void deactivate()
  })) return
  // 标注键盘只编辑标注；空格和 Esc 不再隐式控制录屏生命周期。
  editor.key(event)
}
onMounted(async () => {
  window.addEventListener('keydown', key)
  try {
    const release = await listen<number>('recording-annotations-closed', event => {
      closedThrough = Math.max(closedThrough, event.payload)
      if (view.value && view.value.id <= closedThrough) { view.value = null; editor.reset() }
      if (props.sessionId === event.payload) emit('closed')
    })
    if (disposed) { release(); return }
    off = release
    const releaseEditor=await listen<RecordingAnnotationEditorRequest>('recording-annotation-editor-action',event=>{void editorAction(event.payload)})
    if(disposed) {releaseEditor();return}
    offEditorAction=releaseEditor; editorConnected.value=true
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; off?.(); offEditorAction?.(); window.removeEventListener('keydown', key) })
</script>

<template>
  <main v-if="view" class="live-annotations" :aria-label="t('录制中标注')">
    <AnnotationWorkspace :key="view.id" :editor="editor" :presentation="presentation" :width="view.width" :height="view.height" :bounds="bounds" :surface-visible="view.editing || presentation.some(shape => (shape.opacity ?? 1) > 0)" :toolbar-visible="view.editing" :enabled="view.editing && !busy" :active="view.editing" :disabled="busy" :surface-style="stage" :toolbar-style="layout.style.value" :outlined="false"
      @toolbar-element="toolbar = $event" @tool="chooseTool" @float="layout.start" @reset-position="layout.reset">
      <template #actions>
        <label>{{ t('保留时间') }} <select :value="view.tools.lifetimeMs" :disabled="busy || editor.pending || editor.interacting" :aria-label="t('标注保留时间')" @change="setTiming($event, 'lifetimeMs')"><option :value="0">{{ t('手动清除') }}</option><option :value="3000">{{ t('3 秒') }}</option><option :value="5000">{{ t('5 秒') }}</option></select></label>
        <label><input type="checkbox" :checked="view.tools.fadeOutMs > 0" :disabled="busy || editor.pending || editor.interacting || !view.tools.lifetimeMs" @change="setTiming($event, 'fadeOutMs')" />{{ t('自动淡出') }}</label>
        <button :disabled="busy || editor.pending" :aria-label="t('完成标注')" @click="deactivate">{{ t('完成标注') }}</button>
      </template>
      <template #footer><p v-if="error || editor.error" class="annotation-error" role="alert">{{ te(error || editor.error) }}</p></template>
    </AnnotationWorkspace>
  </main>
</template>

<style scoped>
.live-annotations { position: fixed; inset: 0; background: transparent; font-size: 13px; }
p { margin: 4px; font-size: 12px; }
[role=alert] { color: #b91c1c; }
.annotation-error { max-width:100%; margin:0; overflow-wrap:anywhere; pointer-events:none; }
</style>
