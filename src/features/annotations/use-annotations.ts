import { reactive } from 'vue'
import { commit, emptyHistory, hitTest, moveShape, resizeShape, shapeBounds, undo, redo, nextNumber, type Annotation, type History } from '@/features/annotations/annotations'
import { prepareRaster } from '@/features/annotations/annotation-raster'
import { clamp, type Point, type Rect, type Handle } from '@/shared/geometry/selection'

export type AnnotationTool = 'edit' | 'eraser' | 'arrow-text' | Annotation['kind']
/** 截图/贴图共用编辑状态。一次拖动只提交一次历史，保存失败不改历史也不丢失原图形。 */
export function useAnnotations(bounds: () => Rect, imageUrl: () => string, disabled: () => boolean, persist?: (shapes: Annotation[]) => Promise<void>, rasterizer: typeof prepareRaster = prepareRaster) {
  const state = reactive({ history: emptyHistory(), tool: 'edit' as AnnotationTool, selected: -1,
    // 工具栏读取能力快照，不依赖撤销栈结构；录屏可直接映射画布回传状态。
    get shapes(): Annotation[] { return this.history.present },
    get canUndo(): boolean { return this.history.past.length > 0 },
    get canRedo(): boolean { return this.history.future.length > 0 },
    draft: null as Annotation | null, arrowDraft: null as Annotation | null, pending: false, interacting: false, textEditing: false, textFrame: null as Rect | null, textAutoHeight: true, textMeasuredWidth: 180, textMeasuredHeight: 36, error: '',
    color: '#ef4444', width: 3, opacity: 1, magnification: 2, text: '', fontSize: 24, fontFamily: 'sans' as NonNullable<Annotation['fontFamily']> })
  let gesture: { start: Point; shape: Annotation; mode: 'draw' | 'move' | Handle | 'start' | 'end'; threshold?: number } | null = null
  let generation = 0
  let textSaving = false
  // 输入框和最终字形使用同一原图坐标区域，缩放/DPI 仅由 SVG 负责换算。
  function textBox() {
    if (state.textFrame) return state.textFrame
    const b = bounds(), p = state.draft?.start ?? { x: b.x, y: b.y }
    return { x: p.x, y: p.y, width: Math.min(2048, state.textMeasuredWidth, b.x + b.width - p.x), height: b.y + b.height - p.y }
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
  function abort() { gesture = null; state.draft = null; state.arrowDraft = null; state.interacting = false; state.textEditing = false; state.textFrame = null; state.textAutoHeight = true; state.textMeasuredWidth = 180; state.textMeasuredHeight = 36 }
  /** Esc 放弃本次输入，保留已提交图层；失败提示也随编辑一起关闭。 */
  function cancelText() {
    if (!state.textEditing || textSaving) return
    generation++; abort(); state.pending = false; state.error = ''
  }
  async function commitText(next: History) {
    textSaving = true
    try { return await apply(next) } finally { textSaving = false }
  }
  /** 克隆已有文字作为草稿，取消或保存失败均不会修改原图层。 */
  function editText(index: number) {
    const shape = state.history.present[index]
    if (locked() || state.interacting || shape?.kind !== 'text') return
    select(index)
    gesture = null
    state.draft = { ...shape, start: { ...shape.start }, end: { ...shape.end }, raster: undefined }
    const box = shapeBounds(shape)
    state.textFrame = null; state.textMeasuredWidth = Math.max(120, box.width)
    // 重新编辑仍使用选区剩余高度，可见尺寸由实际文字决定，不锁在旧 PNG 的边界内。
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
  function begin(p: Point, tolerance: number, handle?: Handle | 'start' | 'end', forceDraw = false) {
    if (locked()) return
    p = point(p)
    if (state.tool === 'polyline' && state.draft) {
      if ((state.draft.points?.length ?? 0) < 2048) state.draft = { ...state.draft, end: p, points: [...state.draft.points!, p] }
      return
    }
    if (state.interacting) return
    // 绘制几何图形时也优先命中已有对象；笔刷、折线和连续序号保留连续绘制语义。
    const directSelection = !forceDraw && !['pen', 'highlighter', 'erase', 'polyline', 'number', 'spotlight'].includes(state.tool)
    const index = handle ? state.selected : hitTest(state.history.present, p, tolerance, state.tool !== 'eraser', state.tool !== 'eraser')
    if (handle || state.tool === 'eraser' || state.tool === 'edit' || (directSelection && index >= 0)) {
      select(index)
      if (state.selected < 0) return
      if (state.tool === 'eraser') { void remove(); return }
      const shape = state.history.present[state.selected]
      gesture = { start: p, shape, mode: handle ?? 'move', threshold: tolerance / 2 }; state.draft = shape; state.interacting = true; return
    }
    // 箭头文字复用两个已有图层，预留容量以保证原生保存时不超过标注上限。
    if (state.history.present.length + (state.tool === 'arrow-text' ? 2 : 1) > 100) { state.error = '最多支持 100 个标注。'; return }
    state.selected = -1
    const shape: Annotation = { kind: state.tool === 'arrow-text' ? 'arrow' : state.tool, start: p, end: p, color: state.color, width: state.width, opacity: state.opacity }
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
    if (gesture.mode === 'move' && Math.hypot(p.x - gesture.start.x, p.y - gesture.start.y) < (gesture.threshold ?? 0)) return
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
      abort(); return
    }
    const dx = Math.abs(shape.end.x - shape.start.x), dy = Math.abs(shape.end.y - shape.start.y)
    if (editing && ['rectangle', 'ellipse', 'text', 'number', 'mosaic', 'blur', 'spotlight', 'magnify'].includes(shape.kind) && (dx < 1 || dy < 1)) { abort(); return }
    if (!editing && shape.kind !== 'text' && shape.kind !== 'erase' && (shape.points ? shape.points.length < 2 : ['line', 'arrow'].includes(shape.kind) ? Math.hypot(dx, dy) < 1 : dx < 1 || dy < 1)) { abort(); return }
    if (!editing && state.tool === 'arrow-text' && shape.kind === 'arrow') {
      // 松开箭头后转为文字输入；两层仍是草稿，输入完成才统一保存和记录撤销。
      const b = bounds(), width = Math.min(180, b.width), lineHeight = Math.ceil(state.fontSize * 1.3 + 4)
      const x = clamp((shape.start.x + shape.end.x) / 2 - width / 2, b.x, b.x + b.width - width)
      const y = clamp((shape.start.y + shape.end.y) / 2 - lineHeight / 2, b.y, b.y + Math.max(0, b.height - lineHeight))
      state.arrowDraft = shape
      state.draft = { kind: 'text', start: { x, y }, end: { x, y }, color: shape.color, width: shape.width, opacity: shape.opacity, text: '', fontSize: state.fontSize, fontFamily: state.fontFamily }
      state.textFrame = null; state.textMeasuredWidth = width
      state.textEditing = true; gesture = null
      return
    }
    const version = generation
    state.pending = true
    try {
      if (shape.kind === 'mosaic' || shape.kind === 'blur' || shape.kind === 'magnify' || ((shape.kind === 'text' || shape.kind === 'number') && !editing)) shape = await rasterizer(shape, imageUrl(), bounds())
    } catch (e) { state.error = String(e); abort(); state.pending = false; return }
    state.pending = false
    if (version !== generation) return
    const next = editing ? state.history.present.map((s, i) => i === state.selected ? shape : s) : [...state.history.present, shape]
    await apply(commit(state.history, next)); abort()
  }
  /** 输入完成才栅格化并提交一次历史；失败保留输入框，允许修改后重试。 */
  async function finishText() {
    if (!state.textEditing || !state.draft || locked()) return
    if (!state.draft.text?.trim()) {
      // 不输入文字仍可完成普通箭头；保存失败保留草稿，允许重试。
      if (!state.arrowDraft || await commitText(commit(state.history, [...state.history.present, state.arrowDraft]))) abort()
      return
    }
    if ([...state.draft.text].length > 500) { state.error = '文字最多 500 个字符。'; return }
    const version = generation
    state.pending = true; state.error = ''
    let shape: Annotation
    try { shape = await rasterizer(state.draft, imageUrl(), textBox()) }
    catch (e) { if (version === generation) state.error = String(e); return }
    finally { if (version === generation) state.pending = false }
    if (version !== generation) return
    if (state.arrowDraft) {
      // 字形宽度由实际输入决定，保存时仍以箭头中点居中，并限制在原图选区内。
      const box = shapeBounds(shape), b = bounds(), arrow = state.arrowDraft
      const x = clamp((arrow.start.x + arrow.end.x) / 2 - box.width / 2, b.x, b.x + b.width - box.width)
      shape = { ...shape, start: { x, y: box.y }, end: { x: x + box.width, y: box.y + box.height } }
    }
    const next = state.selected >= 0 ? state.history.present.map((s, i) => i === state.selected ? shape : s) : [...state.history.present, ...(state.arrowDraft ? [state.arrowDraft] : []), shape]
    if (await commitText(commit(state.history, next))) abort()
  }
  async function style(patch: Partial<Pick<Annotation, 'color' | 'width' | 'opacity' | 'text' | 'fontSize' | 'fontFamily' | 'magnification'>>) {
    if (locked()) return
    if (state.textEditing && state.draft && !gesture) {
      Object.assign(state, patch); Object.assign(state.draft, patch); state.error = ''; return
    }
    if (state.interacting) return
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
      if (shape.kind === 'text' || (shape.kind === 'number' && 'color' in patch) || shape.kind === 'mosaic' || shape.kind === 'blur' || shape.kind === 'magnify') shape = await rasterizer(shape, imageUrl(), bounds(), !('text' in patch || 'fontSize' in patch || 'fontFamily' in patch))
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
  return Object.assign(state, { textBox, editText, cancelText, beginTextResize, begin, drag, finish, finishText, abort, reset, setTool, select, remove, clear, style, travel, key })
}
export type AnnotationEditor = ReturnType<typeof useAnnotations>

/** 工具栏只依赖编辑状态和编辑动作；跨窗口宿主可以转发到同一个画布编辑器。 */
export type AnnotationToolbarEditor = Pick<AnnotationEditor,
  'shapes' | 'canUndo' | 'canRedo' | 'tool' | 'selected' | 'pending' | 'interacting' | 'error' | 'color' | 'width' | 'opacity' |
  'magnification' | 'text' | 'fontSize' | 'fontFamily' | 'style' | 'travel' | 'remove' | 'clear'> & { textEditing?: boolean }
