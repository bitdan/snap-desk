---
title: 快速上手
permalink: /guide/
description: Snap Desk 快速上手：安装、第一次截图、默认快捷键和托盘菜单，以及各功能使用说明的入口。
lead: Snap Desk 常驻在托盘，用快捷键唤出各项功能。这一页带你完成第一次截图，并认识几个最常用的入口。
---

## 安装 {#install}

在[官网下载区](/snap-desk/#download)复制终端安装命令，或前往 [GitHub Releases](https://github.com/bitdan/snap-desk/releases) 下载对应安装包。Scoop、Homebrew 及其他渠道的状态见[包管理器](/snap-desk/#install-commands)。

| 平台 | 文件 | 说明 |
| --- | --- | --- |
| Windows x64 | `-setup.exe` 安装包，或 `-standalone.exe` 独立 EXE | 独立 EXE 无需安装，双击即用 |
| macOS Apple 芯片 | `-macos-arm64.dmg` | 需 macOS 13.3 及以上 |
| Linux x64 | `-linux-x64.AppImage` 或 `-linux-x64.deb` | 预览版，需 glibc 2.35 及以上 |

> macOS 首次打开需要在“系统设置 → 隐私与安全性”中放行，并允许 Snap Desk 使用“录屏与系统录音”（macOS 14 及更早叫“屏幕录制”）。遇到截图黑屏，见[常见问题](faq/#macos-permission)。
{: .note}

启动后，Snap Desk 会在托盘（macOS 为菜单栏）放一个图标：**左键单击直接截图，右键打开菜单**。菜单里有截图、贴图、录屏、剪贴板、翻译，以及打开主界面、启用 / 停用全局快捷键、重启和退出。

## 第一次截图 {#first-capture}

1. 按 <kbd>F1</kbd>，屏幕进入截图状态。
2. 把鼠标移到某个窗口或按钮上，它会高亮，**单击**就选中它；也可以**按住左键拖出**一个区域。
3. 选区下方出现工具栏。挑一个标注工具画几笔，或者直接：
   - 按 <kbd>Enter</kbd> 复制图片，然后到聊天窗口里粘贴；
   - 按 <kbd>Ctrl</kbd>+<kbd>S</kbd> 保存为文件；
   - 按 <kbd>F2</kbd> 把它贴在桌面上，方便对照。
4. 不想要了就按 <kbd>Esc</kbd>，或者单击右键。

只用截图不需要任何配置。更多技巧见[截图与标注](capture/)。

## 默认快捷键 {#hotkeys}

这些是**全局快捷键**，在任何应用里都能用，可以在主界面的“快捷键”页修改。

| 功能 | Windows / Linux | macOS |
| --- | --- | --- |
| 截图 | <kbd>F1</kbd> | <kbd>F1</kbd> |
| 贴图 | <kbd>F2</kbd> | <kbd>F2</kbd> |
| 录屏 | <kbd>F3</kbd> | <kbd>F3</kbd> |
| 剪贴板历史 | <kbd>Alt</kbd>+<kbd>V</kbd> | <kbd>⌃ Control</kbd>+<kbd>⌥ Option</kbd>+<kbd>V</kbd> |
| 翻译选中文字 | <kbd>Alt</kbd>+<kbd>T</kbd> | <kbd>⌃ Control</kbd>+<kbd>⌥ Option</kbd>+<kbd>T</kbd> |
| 顶部暂存架 | <kbd>Alt</kbd>+<kbd>Z</kbd> | <kbd>⌃ Control</kbd>+<kbd>⌥ Option</kbd>+<kbd>Z</kbd> |
| 截图识字 | <kbd>Alt</kbd>+<kbd>C</kbd> | <kbd>⌃ Control</kbd>+<kbd>⌥ Option</kbd>+<kbd>C</kbd> |

窗口内的按键（复制、保存、多选等）见[快捷键速查](shortcuts/)。

## 接下来看什么 {#next}

<ul class="cards">
  <li><a href="clipboard/">剪贴板历史</a><span>翻出之前复制过的内容；多选后一次复制、一次粘贴。</span></li>
  <li><a href="pin/">桌面贴图</a><span>把截图、图片或文字钉在桌面上，置顶、缩放、继续标注。</span></li>
  <li><a href="scroll/">滚动截图</a><span>截下整段聊天记录或长网页，自动拼成一张长图。</span></li>
  <li><a href="recording/">录屏</a><span>录成 MP4 或 GIF，可录声音、显示按键，边录边标注。</span></li>
  <li><a href="ocr/">文字识别</a><span>框选一块区域，直接得到可以复制的文字。</span></li>
  <li><a href="shelf/">顶部暂存架</a><span>把常用的文字、图片、链接放在屏幕顶部，随取随用。</span></li>
</ul>
