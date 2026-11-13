/** 引导只切换页面和介绍已有控件，不代替用户执行截图、下载或修改设置。 */
export const onboardingSteps = [
  { page: '截图', target: 'capture-controls', title: '截图与标注', description: '在这里设置延时并开始截图，选区后可标注、复制或保存。', click: false },
  { page: '截图', target: 'nav-快捷键', title: '常用快捷键', description: '点击“快捷键”，查看或修改功能按键。', click: true },
  { page: '快捷键', target: 'shortcut-settings', title: '修改快捷键', description: '点击按键框录入快捷键，修改后自动保存；清空可禁用。', click: false },
  { page: '快捷键', target: 'nav-识别与翻译', title: '识别与翻译', description: '点击“识别与翻译”安装本地资源、选择翻译引擎或添加在线服务。', click: true },
  { page: '识别与翻译', target: 'nav-通用', title: '通用设置', description: '点击“通用”，设置外观、保存目录和启动偏好。', click: true },
  { page: '通用', target: 'nav-关于与帮助', title: '关于与帮助', description: '点击“关于与帮助”完成引导，以后也可以在这里重新查看。', click: true },
] as const

export interface TourRect { left: number; top: number; width: number; height: number }

/** 使用视口 CSS 像素；高亮裁切到可见区域，提示优先放在侧边，其次下方或上方。 */
export function tourLayout(target: TourRect | null, viewport: { width: number; height: number }, cardHeight: number) {
  const width = Math.max(1, Math.min(320, viewport.width - 24))
  const height = Math.max(1, Math.min(cardHeight, viewport.height - 24))
  const clamp = (value: number, max: number) => Math.max(12, Math.min(value, Math.max(12, max)))
  if (!target) return { focus: null, card: { left: (viewport.width - width) / 2, top: Math.max(12, (viewport.height - height) / 2), width } }
  const left = Math.max(0, target.left - 4), top = Math.max(0, target.top - 4)
  const right = Math.min(viewport.width, target.left + target.width + 4)
  const bottom = Math.min(viewport.height, target.top + target.height + 4)
  const focus = right > left && bottom > top ? { left, top, width: right - left, height: bottom - top } : null
  if (!focus) return tourLayout(null, viewport, cardHeight)
  const side = right + width + 24 <= viewport.width
  const cardTop = side ? top : bottom + height + 24 <= viewport.height ? bottom + 12 : top - height - 12
  return { focus, card: { left: clamp(side ? right + 12 : left, viewport.width - width - 12), top: clamp(cardTop, viewport.height - height - 12), width } }
}
