# 外观、引导与截图资源回归

以下为待执行的交互验收，不代表已通过。构建或启动成功不等于这些用例通过。

1. 通用 → 外观与语言：保存浅色、深色、跟随系统；在跟随系统时切换系统配色。
   重启应用后设置保持一致；导航、输入框、剪贴板右键菜单和导出对话框文字可读。
   贴图透明区域及截图像素颜色不能变化。
2. 切换 English / 简体中文：主导航、截图设置、快捷键、通用设置主体和四步引导同步更新。
   当前其他功能页、托盘和后端错误保留原文，不能按完整英文版验收。
3. 模拟配置目录不可写：保存显示失败，已生效的主题和语言保持原值；
   修复目录后可重试。外观草稿不能因无关状态刷新而丢失。
4. 帮助页和引导第四步展示当前平台的权限说明；不显示未经查询的“已授权”状态。
   在线翻译、剪贴板留存和日志分享说明可见。
5. 连续十次截图：分别绘制文字/马赛克、撤销重做、复制、保存、取消、转贴图。
   保存对话框取消或导出失败后，图像及撤销栈保留；成功或取消后新截图不带旧标注。
6. 使用 Release 程序分别记录冷启动、首次截图、连续十次截图、关闭后空闲 1 / 5 分钟；
   测量主进程与所有 WebView2 子进程的 Working Set / Private Bytes 总和。
   截图测试包含 1080p 和 4K，同屏内容下比较耗时和峰值；调试构建数据不用于性能结论。

自动验证入口：

```powershell
npx vitest run tests/appearance.test.ts tests/settings.test.ts tests/annotation-editor.test.ts tests/capture-api.test.ts
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib appearance_legacy_defaults_roundtrip_and_rejects_unknown_values
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib preview_encoding_preserves_rgba_pixels
npm run desktop:build:fast
```
