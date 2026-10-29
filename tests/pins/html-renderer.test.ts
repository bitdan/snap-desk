import { beforeEach, expect, it, vi } from 'vitest'

const mock = vi.hoisted(() => ({ subscribe: vi.fn(), render: vi.fn(), finish: vi.fn() }))
vi.mock('../../src/features/pins/events', () => ({ onHtmlPinRender: mock.subscribe }))
vi.mock('../../src/features/pins/api', () => ({ pinsApi: { finishHtmlRender: mock.finish } }))
vi.mock('../../src/features/pins/html-pin', () => ({ renderHtmlPin: mock.render }))
import { installHtmlPinRenderer } from '../../src/features/pins/html-renderer'

beforeEach(() => { vi.resetAllMocks(); mock.finish.mockResolvedValue(undefined) })

it('原生请求由排版器生成图片，回调只携带请求号和 PNG', async () => {
  mock.render.mockResolvedValue('data:image/png;base64,rendered')
  const off = vi.fn()
  mock.subscribe.mockResolvedValue(off)
  expect(await installHtmlPinRenderer()).toBe(off)
  mock.subscribe.mock.calls[0][0]({ id: 9, html: '<b>private</b>' })
  await vi.waitFor(() => expect(mock.finish).toHaveBeenCalledExactlyOnceWith(9, 'data:image/png;base64,rendered'))
  expect(mock.render).toHaveBeenCalledExactlyOnceWith('<b>private</b>')
})

it('排版失败明确结束同一请求，不把异常正文或原始 HTML 作为结果提交', async () => {
  mock.render.mockRejectedValue(new Error('private content'))
  await installHtmlPinRenderer()
  mock.subscribe.mock.calls[0][0]({ id: 10, html: '<script>private</script>' })
  await vi.waitFor(() => expect(mock.finish).toHaveBeenCalledExactlyOnceWith(10, null))
})
