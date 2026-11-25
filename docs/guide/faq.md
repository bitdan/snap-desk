---
title: 常见问题
permalink: /guide/faq/
description: Snap Desk 常见问题：快捷键无效、macOS 截图黑屏与权限、离线识别翻译、费用、Linux Wayland 限制，以及如何反馈问题。
lead: 遇到问题先看看这里。没有找到答案，欢迎到 GitHub 提交 Issue。
---

## 快捷键按了没反应？ {#hotkey}

1. 打开主界面的“快捷键”页，看看是否提示冲突，是否被其他软件占用（截图软件、输入法、显卡软件经常会占用 <kbd>F1</kbd>～<kbd>F3</kbd> 或 <kbd>Alt</kbd> 组合键）。可以改成别的组合。
2. 用页面上的“开始检测”确认按键能不能被 Snap Desk 收到。
3. 检查托盘菜单里的“启用全局快捷键”是否勾选。
4. 临时可以用托盘菜单操作。

无法注册全局快捷键时（例如 Linux Wayland），可以在系统设置里把快捷键绑定到命令行动作：

```bash
snap-desk --action capture
```

可用的动作：`capture`（截图）、`pin`（贴图）、`record`（录屏）、`history`（剪贴板历史）、`translate`（翻译）、`shelf`（暂存架）、`ocr`（截图识字）。

## macOS 截图黑屏，或录屏提示没有权限？ {#macos-permission}

macOS 按应用签名记录“录屏与系统录音”授权。换了签名的新版本会让旧授权失效，即使开关仍然显示开启。在终端执行：

```bash
tccutil reset All io.snapdesk.app
```

然后退出并重新打开 Snap Desk，截图时按系统提示重新授权，再重启一次应用。

也可以在“系统设置 → 隐私与安全性 → 录屏与系统录音”里选中 Snap Desk，点“−”移除后重新添加。上面的命令会同时清除辅助功能、输入监控等授权，用到时按提示重新允许即可。

## 剪贴板历史按回车没有粘贴进去？ {#paste}

见[剪贴板历史的常见问题](../clipboard/#faq)。

## 识别和翻译可以离线用吗？ {#offline}

可以。文字识别有 PaddleOCR、Windows OCR、Apple Vision 三种离线引擎，翻译有 CTranslate2 离线中英互译。PaddleOCR 和离线翻译目前仅 Windows 可用，macOS 和 Linux 可以改用在线服务。详见[文字识别](../ocr/#engines)和[翻译](../translate/#services)。

## 收费吗？ {#price}

Snap Desk 免费开源（GPL-3.0-only）。你自己配置的第三方在线服务（如 DeepL、OpenAI）可能按其规则收费。

## 我的截图和剪贴板会被上传吗？ {#privacy}

不会。截图、录屏、贴图和剪贴板历史都只在本机处理和保存。只有你主动选择在线识别或在线翻译服务时，才会把对应的图片或文字发给该服务商。

## Linux 上有哪些限制？ {#linux}

Linux 版目前是预览版。Wayland 会话下：

- 只支持单显示器截图；
- 录屏经过系统的屏幕共享授权，暂停 / 继续、键鼠特效和录制中标注仅 X11 可用；
- 剪贴板历史和划词翻译通过 Xwayland 工作，直接粘贴会退化为只复制。

## 怎么反馈问题？ {#feedback}

到 [GitHub Issue](https://github.com/bitdan/snap-desk/issues/new/choose) 提交，中英文都可以。请附上版本号、系统和复现步骤。主界面的“日志”页可以导出诊断包；分享截图、日志或诊断包之前，请先移除密钥和私人数据。
