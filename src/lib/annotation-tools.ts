import { MousePointer2, Square, Circle, Minus, ArrowUpRight, Spline, Pencil, Highlighter, Type, Grid2X2, Droplets, Eraser, ListOrdered, Focus, ZoomIn } from 'lucide-vue-next'
import type { AnnotationTool } from './use-annotations'

// 常用工具直接展示，不再通过下拉菜单切换；名称同时用于悬停提示和读屏。
export const annotationTools = [
  { tool: 'edit', name: '选择 / 编辑', icon: MousePointer2 },
  { tool: 'rectangle', name: '矩形', icon: Square },
  { tool: 'ellipse', name: '椭圆', icon: Circle },
  { tool: 'line', name: '直线', icon: Minus },
  { tool: 'arrow', name: '箭头', icon: ArrowUpRight },
  { tool: 'polyline', name: '折线', icon: Spline },
  { tool: 'pen', name: '画笔', icon: Pencil },
  { tool: 'highlighter', name: '荧光笔', icon: Highlighter },
  { tool: 'text', name: '文字', icon: Type },
  { tool: 'number', name: '序号', icon: ListOrdered },
  { tool: 'spotlight', name: '聚光灯', icon: Focus },
  { tool: 'magnify', name: '放大镜标注', icon: ZoomIn },
  { tool: 'mosaic', name: '马赛克', icon: Grid2X2 },
  { tool: 'blur', name: '模糊', icon: Droplets },
  { tool: 'erase', name: '局部像素橡皮擦', icon: Eraser },
  { tool: 'eraser', name: '删除整项标注', icon: Eraser },
] satisfies { tool: AnnotationTool; name: string; icon: typeof Pencil }[]
