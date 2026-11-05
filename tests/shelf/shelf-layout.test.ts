import { expect, it } from 'vitest'
import { shelfLayout, shelfPhotoSize } from '../../src/features/shelf/shelf-layout'

it('横图与竖图的相框贴合图片比例，不保留统一高度的空白', () => {
  expect(shelfPhotoSize(1920, 1080)).toEqual({ width: 144, height: 81 })
  expect(shelfPhotoSize(600, 1200)).toEqual({ width: 54, height: 108 })
  expect(shelfPhotoSize(null, null)).toEqual({ width: 144, height: 96 })
})

it.each([270, 600, 966, 1920])('窗口宽度 %i 时卡片居中、夹子落在绳索上，并限制每组数量', width => {
  const layout = shelfLayout(width, 30)
  expect(layout.capacity).toBeGreaterThanOrEqual(1)
  expect(layout.capacity).toBeLessThanOrEqual(5)
  const positions = layout.positions
  expect(positions.length).toBe(layout.capacity)
  expect(positions[0].left).toBeGreaterThanOrEqual(0)
  expect(positions.at(-1)!.left + 156).toBeLessThanOrEqual(width)
  expect((positions[0].left + positions.at(-1)!.left + 156) / 2).toBeCloseTo(width / 2)
  for (const point of positions) {
    const f = (point.left + 78) / width
    expect(point.top + 9).toBeCloseTo(12 + 4 * Math.min(20, width * .024) * f * (1 - f))
  }
})

it('最后一组不足一排时仍居中；空列表不生成卡片', () => {
  expect(shelfLayout(966, 1).positions[0].left).toBe(966 / 2 - 78)
  expect(shelfLayout(966, 0).positions).toEqual([])
})
