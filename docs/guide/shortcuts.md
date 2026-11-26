---
title: 快捷键速查
permalink: /guide/shortcuts/
description: Snap Desk 全部快捷键：全局快捷键，以及截图、标注、贴图、剪贴板历史、录屏和翻译窗口内的按键。
lead: 全局快捷键可以在主界面“快捷键”页修改；窗口内的按键固定不变。macOS 上，表中的 Ctrl 对应 ⌘ Command。
---

## 全局快捷键 {#global}

| 功能 | Windows / Linux | macOS | 再按一次 |
| --- | --- | --- | --- |
| 截图 | <kbd>F1</kbd> | <kbd>F1</kbd> | 取消截图 |
| 贴图 | <kbd>F2</kbd> | <kbd>F2</kbd> | 截图中：贴当前选区；否则贴剪贴板内容 |
| 录屏 | <kbd>F3</kbd> | <kbd>F3</kbd> | 录制中：结束并保存 |
| 剪贴板历史 | <kbd>Alt</kbd>+<kbd>V</kbd> | <kbd>⌃</kbd>+<kbd>⌥</kbd>+<kbd>V</kbd> | |
| 翻译选中文字 | <kbd>Alt</kbd>+<kbd>T</kbd> | <kbd>⌃</kbd>+<kbd>⌥</kbd>+<kbd>T</kbd> | |
| 顶部暂存架 | <kbd>Alt</kbd>+<kbd>Z</kbd> | <kbd>⌃</kbd>+<kbd>⌥</kbd>+<kbd>Z</kbd> | 收起 |
| 截图识字 | <kbd>Alt</kbd>+<kbd>C</kbd> | <kbd>⌃</kbd>+<kbd>⌥</kbd>+<kbd>C</kbd> | 取消 |

快捷键没反应时，先在“快捷键”页看是否被别的软件占用，再用“开始检测”排查。也可以把系统快捷键绑定到命令行动作，见[常见问题](../faq/#hotkey)。

## 截图 {#capture}

| 按键 | 作用 |
| --- | --- |
| <kbd>Enter</kbd> / <kbd>Ctrl</kbd>+<kbd>C</kbd> | 复制图片并结束（录屏选区时 <kbd>Enter</kbd> 为开始录制） |
| 在选区内双击 | 复制图片并结束 |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> | 保存图片 |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> / 按住 <kbd>Shift</kbd> 点保存按钮 | 快速保存到截图保存目录，不弹对话框 |
| <kbd>F2</kbd> | 把选区贴到桌面 |
| 滚轮 / <kbd>Tab</kbd> / <kbd>Shift</kbd>+<kbd>Tab</kbd> | 未框选时切换吸附层级：控件 → 面板 → 整个窗口 → 整个屏幕 |
| 按住 <kbd>Alt</kbd> | 未框选时直接吸附整个窗口 |
| <kbd>Ctrl</kbd>+<kbd>A</kbd> | 选中整个屏幕 |
| <kbd>R</kbd> | 恢复上一次的选区 |
| <kbd>,</kbd> / <kbd>.</kbd> | 截图历史：上一张 / 下一张（最近 20 次） |
| <kbd>F5</kbd> | 刷新截图：重新拍下屏幕，保留选区和标注 |
| 方向键 | 移动选区 1 像素；加 <kbd>Shift</kbd> 为 10 像素 |
| <kbd>Ctrl</kbd>+方向键 | 把那一侧的边向外扩 1 像素；加 <kbd>Shift</kbd> 为向内收；再加 <kbd>Alt</kbd> 时对边同时变化 |
| 拖动时按住 <kbd>Shift</kbd> | 拖出正方形；拖动边角时保持宽高比 |
| 单击尺寸标签 | 输入位置和尺寸、锁定比例、套用尺寸预设（<kbd>Enter</kbd> 应用，<kbd>Tab</kbd> 切换，<kbd>Esc</kbd> 放弃） |
| <kbd>Esc</kbd> / 右键 | 逐级退出：取消正在画的图形 → 退出标注 → 清除选区 → 结束截图 |
| 取色时 <kbd>C</kbd> | 复制当前颜色值 |
| 取色时 <kbd>Shift</kbd> | 切换 HEX / RGB 格式 |
| 取色时 <kbd>P</kbd> | 切换全局坐标 / 相对选区坐标 |

## 标注（截图与贴图通用） {#annotate}

| 按键 | 作用 |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>Z</kbd> | 撤销 |
| <kbd>Ctrl</kbd>+<kbd>Y</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> | 重做 |
| <kbd>Delete</kbd> / <kbd>Backspace</kbd> | 删除选中的图形 |
| 方向键 | 移动选中的图形 1 像素；加 <kbd>Shift</kbd> 为 10 像素 |
| 双击文字 | 重新编辑这段文字 |
| 输入文字时 <kbd>Ctrl</kbd>+<kbd>Enter</kbd> | 完成输入；<kbd>Esc</kbd> 放弃 |
| 折线 <kbd>Enter</kbd> | 结束当前折线 |

## 贴图 {#pin}

| 按键 | 作用 |
| --- | --- |
| <kbd>Space</kbd> | 显示 / 隐藏工具栏 |
| 滚轮、<kbd>+</kbd> / <kbd>-</kbd> | 放大 / 缩小 |
| <kbd>Ctrl</kbd>+<kbd>C</kbd> | 复制图片 |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> | 保存图片 |
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>P</kbd> | 开关鼠标穿透 |
| <kbd>T</kbd> | 选择文字：在图片上直接选中、复制文字 |
| 选择文字时 <kbd>Ctrl</kbd>+<kbd>A</kbd> / <kbd>Ctrl</kbd>+<kbd>C</kbd> | 全选 / 复制选中的文字 |
| 选择文字时双击 / 三击 | 选中一个词 / 一整行 |
| 选择文字时 <kbd>H</kbd> | 把选中的文字转成荧光笔标注 |
| 选择文字时 <kbd>Esc</kbd> | 退出选择文字 |
| <kbd>Esc</kbd> / <kbd>Ctrl</kbd>+<kbd>W</kbd> | 关闭贴图 |
| 右键 | 更多操作菜单 |

## 剪贴板历史 {#clipboard}

| 按键 | 作用 |
| --- | --- |
| <kbd>↑</kbd> / <kbd>↓</kbd> | 移动焦点 |
| <kbd>Enter</kbd> / 双击 | 粘贴到原窗口（多选时整组一起粘贴） |
| <kbd>Ctrl</kbd>+<kbd>1</kbd>～<kbd>9</kbd> | 粘贴第 1～9 条 |
| <kbd>Ctrl</kbd>+<kbd>C</kbd> | 只复制，不粘贴 |
| <kbd>Ctrl</kbd>+单击 | 多选：加入或移出选择 |
| <kbd>Shift</kbd>+单击 | 连选：从上次点的条目选到这一条 |
| <kbd>Space</kbd> | 把焦点条目加入或移出选择 |
| <kbd>Delete</kbd> | 删除焦点条目（多选时确认后删除整组） |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | 搜索 |
| <kbd>Alt</kbd>+<kbd>←</kbd> / <kbd>→</kbd> | 把当前分类往左 / 右挪 |
| <kbd>Esc</kbd> | 逐级退出：关菜单 → 取消选择 → 清空搜索 → 收起窗口 |

## 滚动截图与录屏 {#scroll-recording}

| 场景 | 按键 | 作用 |
| --- | --- | --- |
| 滚动截图 | <kbd>Ctrl</kbd>+<kbd>C</kbd> / 再按 <kbd>F1</kbd> | 完成拼接 |
| 滚动截图 | <kbd>Esc</kbd> | 取消，不保存 |
| 滚动截图 | 按住 <kbd>Shift</kbd> 滚动 | 横向滚动 |
| 滚动截图结果 | <kbd>Enter</kbd> / <kbd>Ctrl</kbd>+<kbd>C</kbd> | 复制裁剪后的长图并关闭 |
| 滚动截图结果 | <kbd>Ctrl</kbd>+<kbd>S</kbd> / <kbd>Ctrl</kbd>+<kbd>P</kbd> | 保存 / 贴到桌面 |
| 滚动截图结果 | <kbd>Esc</kbd> | 关闭结果窗口（长图已复制并保存） |
| 录屏 | 再按 <kbd>F3</kbd> | 结束并保存 |
| 录屏 | <kbd>Esc</kbd> | 录制中：结束并保存；倒计时中：取消 |

## 翻译窗口 {#translate}

| 按键 | 作用 |
| --- | --- |
| <kbd>Enter</kbd> | 翻译 |
| <kbd>Shift</kbd>+<kbd>Enter</kbd> | 换行 |
| <kbd>Esc</kbd> | 关闭窗口 |
