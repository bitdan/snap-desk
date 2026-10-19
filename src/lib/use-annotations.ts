import { reactive } from 'vue'
import { commit, emptyHistory, hitTest, moveShape, resizeShape, shapeBounds, undo, redo, nextNumber, type Annotation, type History } from './annotations'
import { prepareRaster } from './annotation-raster'
import { clamp, type Point, type Rect, type Handle } from './selection'

export type AnnotationTool = 'edit' | 'eraser' | Annotation['kind']
/** 截图/贴图共用编辑状态。一次拖动只提交一次历史，保存失败不改历史也不丢失原图形。 */
export function useAnnotations(bounds: () => Rect, imageUrl: () => string, disabled: () => boolean, persist?: (shapes: Annotation[]) => Promise<void>) {
  const state = reactive({ history: emptyHistory(), tool: 'edit' as AnnotationTool, selected: -1,
    draft: null as Annotation | null, pending: false, interacting: false, textEditing: false, textFrame: null as Rect | null, textAutoHeight: true, error: '',
    color: '#ef4444', width: 3, opacity: 1, magnification: 2, text: '', fontSize: 24, fontFamily: 'sans' as NonNullable<Annotation['fontFamily']> })
  let gesture: { start: Point; shape: Annotation; mode: 'draw' | 'move' | Handle | 'start' | 'end' } | null = null
  let generation = 0
  // 输入框和最终字形使用同一原图坐标区域，缩放/DPI 仅由 SVG 负责换算。
  function textBox() {
    if (state.textFrame) return state.textFrame
    const b = bounds(), p = state.draft?.start ?? { x: b.x, y: b.y }
    return { x: p.x, y: p.y, width: Math.min(180, b.x + b.width - p.x), height: Math.min(180, b.y + b.height - p.y) }
  }
  function locked() { return disabled() || state.pending }
  function point(p: Point): Point { const b = bounds(); return { x: clamp(p.x, b.x, b.x + b.width), y: clamp(p.y, b.y, b.y + b.height) } }
  async function apply(next: History) {
    if (locked()) return false
    if (next.present.reduce((sum, s) => sum + (s.raster?.length ?? 0), 0) > 8_000_000) { state.error = '文字和滤镜数据总量过大，请删除部分标注。'; return false }
    const version = generation
    state.pending = true; state.error = ''
    try { await persist?.(next.present); if (version !== generation) return false; state.history = next; return true }
    catch (e) { state.error = String(e); return false }
    finally { state.pending = false }
  }
  function select(index: number) {
    state.selected = index
    const s = state.history.present[index]
    if (s) { state.color = s.color; state.width = s.width; state.opacity = s.opacity ?? 1; state.magnification = s.magnification ?? 2; state.text = s.text ?? ''; state.fontSize = s.fontSize ?? 24; state.fontFamily = s.fontFamily ?? 'sans' }
  }
  function setTool(tool: AnnotationTool) {
    if (locked() || state.interacting) return
    state.tool = tool; state.selected = -1
    if (tool === 'highlighter') { state.width = 20; state.opacity = 0.35 }
    else if (tool === 'spotlight') {
      const existing = state.history.present.find(s => s.kind === 'spotlight')
      state.color = existing?.color ?? '#000000'; state.opacity = existing?.opacity ?? 0.7; state.width = 3
    }
    else if (tool === 'erase') { state.width = 24; state.opacity = 1 }
    else if (tool === 'mosaic' || tool === 'blur') { state.width = 16; state.opacity = 1 }
    else { state.width = 3; state.opacity = 1 }
  }
  function reset(shapes: Annotation[] = []) { generation++; state.history = emptyHistory(shapes); state.selected = -1; abort(); state.error = '' }
  function abort() { gesture = null; state.draft = null; state.interacting = false; state.textEditing = false; state.textFrame = null; state.textAutoHeight = true }
  /** 克隆已有文字作为草稿，取消或保存失败均不会修改原图层。 */
  function editText(index: number) {
    const shape = state.history.present[index]
    if (locked() || shape?.kind !== 'text') return
    select(index)
    gesture = null
    state.draft = { ...shape, start: { ...shape.start }, end: { ...shape.end }, raster: undefined }
    const box = shapeBounds(shape), b = bounds()
    state.textFrame = { ...box, height: Math.min(Math.max(180, box.height), b.y + b.height - box.y) }
    // frame.height 是排版可用上限；重新编辑时可见高度由实际文字行数决定。
    state.textAutoHeight = true
    state.textEditing = true; state.interacting = true; state.error = ''
  }
  /** 输入期间拖动右边、下边或右下角，宽高独立变化；尺寸使用原图像素。 */
  function beginTextResize(p: Point, handle: Handle, height: number) {
    if (locked() || !state.textEditing || !state.draft) return
    const box = { ...textBox(), height }
    const shape = { ...state.draft, start: { x: box.x, y: box.y }, end: { x: box.x + box.width, y: box.y + box.height } }
    state.textFrame = box
    state.textAutoHeight = false
    gesture = { start: p, shape, mode: handle }
  }
  async function remove() {
    if (locked() || state.interacting || state.selected < 0) return
    const next = state.history.present.filter((_, i) => i !== state.selected)
    await apply(commit(state.history, next)); state.selected = -1
  }
  async function clear() {
    if (locked() || state.interacting || !state.history.present.length) return
    await apply(commit(state.history, [])); state.selected = -1
  }
  function begin(p: Point, tolerance: number, handle?: Handle | 'start' | 'end') {
    if (locked()) return
    p = point(p)
    if (state.tool === 'polyline' && state.draft) {
      if ((state.draft.points?.length ?? 0) < 2048) state.draft = { ...state.draft, end: p, points: [...state.draft.points!, p] }
      return
    }
    if (state.interacting) return
    if (state.tool === 'text') {
      const index = hitTest(state.history.present, p, tolerance)
      if (state.history.present[index]?.kind === 'text') { editText(index); return }
    }
    if (state.tool === 'edit' || state.tool === 'eraser') {
      if (!handle) select(hitTest(state.history.present, p, tolerance))
      if (state.selected < 0) return
      if (state.tool === 'eraser') { void remove(); return }
      const shape = state.history.present[state.selected]
      gesture = { start: p, shape, mode: handle ?? 'move' }; state.draft = shape; state.interacting = true; return
    }
    if (state.history.present.length >= 100) { state.error = '最多支持 100 个标注。'; return }
    state.selected = -1
    const shape: Annotation = { kind: state.tool, start: p, end: p, color: state.color, width: state.width, opacity: state.opacity }
    if (shape.kind === 'magnify') shape.magnification = state.magnification
    if (shape.kind === 'number') {
      const b = bounds(), size = Math.min(36, b.width, b.height), value = nextNumber(state.history.present)
      if (value > 999 || size < 12) { state.error = value > 999 ? '序号最多为 999。' : '选区太小，无法放置序号。'; return }
      const x = clamp(p.x - size / 2, b.x, b.x + b.width - size), y = clamp(p.y - size / 2, b.y, b.y + b.height - size)
      shape.start = { x, y }; shape.end = { x: x + size, y: y + size }; shape.text = String(value)
    }
    if (['pen', 'highlighter', 'polyline', 'erase'].includes(shape.kind)) shape.points = [p]
    if (shape.kind === 'text') {
      // 先放置空输入框，临近右/下边缘时让出可输入的空间，不复用上一次文字。
      const b = bounds()
      const lineHeight = Math.ceil(state.fontSize * 1.3 + 4)
      shape.start = { x: Math.min(p.x, b.x + Math.max(0, b.width - 120)), y: Math.min(p.y, b.y + Math.max(0, b.height - lineHeight)) }
      shape.end = { ...shape.start }; shape.text = ''; shape.fontSize = state.fontSize; shape.fontFamily = state.fontFamily
      state.textEditing = true; state.error = ''
    }
    state.draft = shape; state.interacting = true
    if (shape.kind !== 'polyline' && shape.kind !== 'text') gesture = { start: p, shape, mode: 'draw' }
  }
  function drag(p: Point) {
    if (!gesture || !state.draft || locked()) return
    p = point(p)
    if (state.textEditing) {
      const origin = gesture.shape.start
      p = point({ x: Math.max(origin.x + 24, p.x), y: Math.max(origin.y + Math.ceil((state.draft.fontSize ?? 24) * 1.3 + 4), p.y) })
      const box = shapeBounds(resizeShape(gesture.shape, gesture.mode as Handle, p, bounds()))
      if (box.width >= 24 && box.height >= Math.ceil((state.draft.fontSize ?? 24) * 1.3 + 4)) state.textFrame = box
      return
    }
    if (gesture.mode === 'move') state.draft = moveShape(gesture.shape, { x: p.x - gesture.start.x, y: p.y - gesture.start.y }, bounds())
    else if (gesture.mode !== 'draw') state.draft = resizeShape(gesture.shape, gesture.mode, p, bounds())
    else if (state.draft.points) {
      const last = state.draft.points.at(-1)!
      if (Math.hypot(p.x - last.x, p.y - last.y) >= 1 && state.draft.points.length < 2048) state.draft = { ...state.draft, end: p, points: [...state.draft.points, p] }
    } else if (state.draft.kind !== 'text' && state.draft.kind !== 'number') state.draft = { ...state.draft, end: p }
  }
  async function finish(polyline = false) {
    if (state.textEditing) { gesture = null; return } // 放置或调整输入框时不提交文字。
    if (!state.draft || locked() || (state.tool === 'polyline' && !polyline)) return
    let shape = state.draft
    const editing = state.selected >= 0 && gesture?.mode !== 'draw'
    if (editing && JSON.stringify(shape) === JSON.stringify(state.history.present[state.selected])) {
      if (shape.kind === 'text' && gesture?.mode === 'move') { editText(state.selected); return }
      abort(); return
    }
    const dx = Math.abs(shape.end.x - shape.start.x), dy = Math.abs(shape.end.y - shape.start.y)
    if (editing && ['rectangle', 'ellipse', 'text', 'number', 'mosaic', 'blur', 'spotlight', 'magnify'].includes(shape.kind) && (dx < 1 || dy < 1)) { abort(); return }
    if (!editing && shape.kind !== 'text' && shape.kind !== 'erase' && (shape.points ? shape.points.length < 2 : ['line', 'arrow'].includes(shape.kind) ? Math.hypot(dx, dy) < 1 : dx < 1 || dy < 1)) { abort(); return }
    const version = generation
    state.pending = true
    try {
      if (shape.kind === 'mosaic' || shape.kind === 'blur' || shape.kind === 'magnify' || ((shape.kind === 'text' || shape.kind === 'number') && !editing)) shape = await prepareRaster(shape, imageUrl(), bounds())
    } catch (e) { state.error = String(e); abort(); state.pending = false; return }
    state.pending = false
    if (version !== generation) return
    const next = editing ? state.history.present.map((s, i) => i === state.selected ? shape : s) : [...state.history.present, shape]
    await apply(commit(state.history, next)); abort()
  }
  /** 输入完成才栅格化并提交一次历史；失败保留输入框，允许修改后重试。 */
  async function finishText() {
    if (!state.textEditing || !state.draft || locked()) return
    if (!state.draft.text?.trim()) { abort(); return }
    if ([...state.draft.text].length > 500) { state.error = '文字最多 500 个字符。'; return }
    const version = generation
    state.pending = true; state.error = ''
    let shape: Annotation
    try { shape = await prepareRaster(state.draft, imageUrl(), textBox()) }
    catch (e) { if (version === generation) state.error = String(e); return }
    finally { state.pending = false }
    if (version !== generation) return
    const next = state.selected >= 0 ? state.history.present.map((s, i) => i === state.selected ? shape : s) : [...state.history.present, shape]
    if (await apply(commit(state.history, next))) abort()
  }
  async function style(patch: Partial<Pick<Annotation, 'color' | 'width' | 'opacity' | 'text' | 'fontSize' | 'fontFamily' | 'magnification'>>) {
    if (locked() || state.interacting) return
    if (patch.magnification !== undefined && (!Number.isFinite(patch.magnification) || patch.magnification < 2 || patch.magnification > 4)) { state.error = '放大倍数须为 2～4。'; return }
    Object.assign(state, patch)
    const original = state.history.present[state.selected]
    if (!original) {
      if (state.tool === 'spotlight' && state.history.present.some(s => s.kind === 'spotlight') && ('color' in patch || 'opacity' in patch)) {
        await apply(commit(state.history, state.history.present.map(s => s.kind === 'spotlight' ? { ...s, color: state.color, opacity: state.opacity } : s)))
      }
      return
    }
    const version = generation
    let shape = { ...original, ...patch }
    state.pending = true
    try {
      if (shape.kind === 'text' || (shape.kind === 'number' && 'color' in patch) || shape.kind === 'mosaic' || shape.kind === 'blur' || shape.kind === 'magnify') shape = await prepareRaster(shape, imageUrl(), bounds(), !('text' in patch || 'fontSize' in patch || 'fontFamily' in patch))
    } catch (e) { state.error = String(e); state.pending = false; return }
    state.pending = false
    if (version !== generation) return
    // 多个亮区共用一层遮罩；编辑任意亮区的颜色/透明度同步到整组，并只提交一步历史。
    await apply(commit(state.history, state.history.present.map((s, i) =>
      original.kind === 'spotlight' && s.kind === 'spotlight'
        ? { ...s, color: shape.color, opacity: shape.opacity }
        : i === state.selected ? shape : s)))
    select(state.selected)
  }
  async function travel(direction: 'undo' | 'redo') {
    if (state.interacting || locked()) return
    await apply(direction === 'undo' ? undo(state.history) : redo(state.history)); state.selected = -1
  }
  function key(e: KeyboardEvent): boolean {
    if ((e.target as HTMLElement)?.closest?.('input, textarea, select, [contenteditable=true]')) return false
    const command = e.ctrlKey || e.metaKey, k = e.key.toLowerCase()
    if (command && (k === 'z' || k === 'y')) { e.preventDefault(); if (!e.repeat) void travel(k === 'y' || e.shiftKey ? 'redo' : 'undo'); return true }
    if (k === 'escape' && state.interacting) { e.preventDefault(); if (!locked()) abort(); return true }
    if (k === 'enter' && state.tool === 'polyline' && state.draft) { e.preventDefault(); void finish(true); return true }
    if (k === 'delete' || k === 'backspace') { e.preventDefault(); void remove(); return true }
    const delta = ({ arrowleft: { x: -1, y: 0 }, arrowright: { x: 1, y: 0 }, arrowup: { x: 0, y: -1 }, arrowdown: { x: 0, y: 1 } } as Record<string, Point>)[k]
    if (!command && delta && state.selected >= 0) {
      e.preventDefault()
      if (!locked() && !state.interacting) {
        const shape = state.history.present[state.selected], step = e.shiftKey ? 10 : 1
        gesture = { start: shape.start, shape, mode: 'move' }; state.interacting = true
        state.draft = moveShape(shape, { x: delta.x * step, y: delta.y * step }, bounds())
        void finish()
      }
      return true
    }
    return false
  }
  return Object.assign(state, { textBox, beginTextResize, begin, drag, finish, finishText, abort, reset, setTool, select, remove, clear, style, travel, key })
}
export type AnnotationEditor = ReturnType<typeof useAnnotations>
