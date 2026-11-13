import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h, ref } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { onboardingSteps, tourLayout } from '../../src/app/onboarding'

vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import OnboardingGuide from '../../src/app/components/OnboardingGuide.vue'
const Component = clientComponent(OnboardingGuide, new URL('../../src/app/components/OnboardingGuide.vue', import.meta.url))

class Target extends EventTarget {
  rect = { left: 20, top: 100, width: 120, height: 36 }
  getBoundingClientRect = () => this.rect
  scrollIntoView = vi.fn()
  focus = vi.fn()
  matches = () => true
  querySelectorAll = () => []
  isConnected = true
}
class Observer {
  static instances: Observer[] = []
  observe = vi.fn(); disconnect = vi.fn()
  constructor(public callback: () => void) { Observer.instances.push(this) }
}
let root: MemoryNode, body: MemoryNode, targets: Map<string, Target>, screen: EventTarget & { innerWidth: number; innerHeight: number }
let navigate: ReturnType<typeof vi.fn>, complete: ReturnType<typeof vi.fn>
const busy = ref(false), error = ref('')
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
const button = (label: string) => findNode(body, node => node.type === 'button' && text(node) === label)!
async function flush() { await flushComponent(); await vi.advanceTimersByTimeAsync(0); await flushComponent() }
async function mount() {
  memoryRenderer.render(h({ setup: () => () => h(Component, { busy: busy.value, error: error.value, onNavigate: navigate, onComplete: complete }) }), root)
  await flush()
}
beforeEach(() => {
  vi.useFakeTimers(); busy.value = false; error.value = ''
  root = new MemoryNode('root'); body = new MemoryNode('body')
  targets = new Map(onboardingSteps.map(step => [step.target, new Target()]))
  navigate = vi.fn(); complete = vi.fn(); Observer.instances = []
  screen = Object.assign(new EventTarget(), { innerWidth: 1100, innerHeight: 680 })
  vi.stubGlobal('window', screen)
  vi.stubGlobal('document', {
    body, activeElement: null,
    querySelector: (selector: string) => selector === '.app-shell' ? root : targets.get(selector.match(/data-onboarding="([^"]+)"/)?.[1] ?? '') ?? null,
  })
  vi.stubGlobal('MutationObserver', Observer); vi.stubGlobal('ResizeObserver', Observer)
  vi.stubGlobal('requestAnimationFrame', (callback: () => void) => setTimeout(callback, 0))
  vi.stubGlobal('cancelAnimationFrame', (id: ReturnType<typeof setTimeout>) => clearTimeout(id))
  Object.defineProperty(MemoryNode.prototype, 'getBoundingClientRect', { configurable: true, value: () => ({ left: 0, top: 0, width: 320, height: 220 }) })
})
afterEach(async () => {
  memoryRenderer.render(null, root); await flushComponent()
  Reflect.deleteProperty(MemoryNode.prototype, 'getBoundingClientRect')
  vi.unstubAllGlobals(); vi.useRealTimers()
})

describe('界面点击引导', () => {
  it('高亮真实控件，点击导航推进，支持返回上一步', async () => {
    await mount()
    expect(navigate).toHaveBeenLastCalledWith('截图')
    expect(text(body)).toContain('1 / 6')
    expect(findNode(body, node => node.props.class === 'tour-focus')).toBeDefined()
    targets.get('capture-controls')!.dispatchEvent(new Event('click')); await flush()
    expect(text(body)).toContain('1 / 6')
    button('下一步').props.onClick(); await flush()
    targets.get('nav-快捷键')!.dispatchEvent(new Event('click')); await flush()
    expect(navigate).toHaveBeenLastCalledWith('快捷键')
    expect(text(body)).toContain('3 / 6')
    button('上一步').props.onClick(); await flush()
    expect(text(body)).toContain('2 / 6')
    expect(navigate).toHaveBeenLastCalledWith('截图')
  })
  it('异步目标尚未出现时可跳过，出现后自动恢复定位', async () => {
    targets.delete('capture-controls')
    await mount()
    expect(button('下一步').props.disabled).toBe(true)
    expect(button('跳过引导').props.disabled).toBe(false)
    expect(text(body)).toContain('正在定位操作区域…')
    targets.set('capture-controls', new Target())
    Observer.instances[0].callback(); await flush()
    expect(button('下一步').props.disabled).toBe(false)
    expect(text(body)).not.toContain('正在定位操作区域…')
  })
  it('缩放和滚动重新定位，卸载后清理观察和点击监听', async () => {
    await mount()
    const target = targets.get('capture-controls')!
    target.rect = { left: 50, top: 150, width: 100, height: 40 }
    screen.dispatchEvent(new Event('scroll')); await flush()
    expect(findNode(body, node => node.props.class === 'tour-focus')?.props.style.left).toBe('46px')
    screen.innerWidth = 280; screen.innerHeight = 420
    screen.dispatchEvent(new Event('resize')); await flush()
    expect(findNode(body, node => node.props.class === 'tour-card')?.props.style.width).toBe('256px')
    const remove = vi.spyOn(target, 'removeEventListener')
    memoryRenderer.render(null, root); await flushComponent()
    expect(remove).toHaveBeenCalledWith('click', expect.any(Function))
    expect(Observer.instances.every(observer => observer.disconnect.mock.calls.length > 0)).toBe(true)
    expect(vi.getTimerCount()).toBe(0)
  })
  it('保存期间禁止重复结束，失败原因留在引导中供重试', async () => {
    await mount(); busy.value = true; await flush()
    button('跳过引导').props.onClick()
    const escape = new Event('keydown', { cancelable: true }); Object.assign(escape, { key: 'Escape' })
    screen.dispatchEvent(escape)
    expect(complete).not.toHaveBeenCalled()
    expect(escape.defaultPrevented).toBe(true)
    busy.value = false; error.value = '无法保存设置'; await flush()
    expect(findNode(body, node => node.props.role === 'alert')?.text).toBe('无法保存设置')
    button('跳过引导').props.onClick()
    expect(complete).toHaveBeenCalledOnce()
  })
  it('点击最后的帮助入口才提交完成，不会提前标记已读', async () => {
    await mount()
    button('下一步').props.onClick(); await flush()
    targets.get('nav-快捷键')!.dispatchEvent(new Event('click')); await flush()
    button('下一步').props.onClick(); await flush()
    targets.get('nav-识别与翻译')!.dispatchEvent(new Event('click')); await flush()
    targets.get('nav-通用')!.dispatchEvent(new Event('click')); await flush()
    expect(complete).not.toHaveBeenCalled()
    expect(text(body)).toContain('6 / 6')
    targets.get('nav-关于与帮助')!.dispatchEvent(new Event('click'))
    expect(complete).toHaveBeenCalledOnce()
  })
})

describe('引导布局边界', () => {
  it('窄窗口、长提示和部分不可见目标保持在视口中', () => {
    const layout = tourLayout({ left: -20, top: -10, width: 180, height: 80 }, { width: 280, height: 300 }, 500)
    expect(layout.focus).toEqual({ left: 0, top: 0, width: 164, height: 74 })
    expect(layout.card.left).toBeGreaterThanOrEqual(12)
    expect(layout.card.left + layout.card.width).toBeLessThanOrEqual(268)
    expect(layout.card.top).toBe(12)
    expect(tourLayout({ left: 900, top: 900, width: 20, height: 20 }, { width: 280, height: 300 }, 200).focus).toBeNull()
  })
})
