# Windows 上的 WebView2 启动故障

## 现象

Snap Desk 在 Tauri 创建主窗口时退出，退出码为 `101`。一次本机排查中，默认 WebView2 数据目录报 `0x800700AA`（资源正在使用），换用全新的数据目录后报 `0x8000FFFF`。禁用 GPU 后仍能复现。

## 排查步骤

1. **先用同一份 EXE 做目录对照。** 复制失败的 EXE 到工作区外的普通目录，用 `Get-FileHash` 确认两份文件的 SHA-256 一致，再启动副本。也可以把已知可用的旧版复制到工作区的干净子目录做反向对照。测试前从托盘退出已有实例。
2. **如果结果随目录变化，比较目录权限。** 用 `icacls` 检查 EXE 所在目录、工作区根目录和可正常启动的目录。`(I)` 表示继承项，沿上级目录找到未标 `(I)` 的对应条目。注意显式拒绝权限和可继承的完整性标签。
3. **恢复目录设置后，从原路径复测。** 清除当前 PowerShell 会话中的 `WEBVIEW2_USER_DATA_FOLDER` 覆盖，使用默认配置启动，以实际出现主窗口为准。

```powershell
icacls .
icacls .\src-tauri\target\release
Remove-Item Env:WEBVIEW2_USER_DATA_FOLDER -ErrorAction SilentlyContinue
```

## 已验证的案例

旧发布版 `v0.2.1-beta.1` 和新版打包产物在工作区内都以 `101` 退出，复制到工作区外后均能打开。工作区根目录存在 `Everyone:(CI)(DENY)(S,DC)` 和可继承的 `Low Mandatory Level`，构建输出目录继承了两项。

移除该拒绝项并恢复中等完整性后，在工作区原路径观察到 Debug 版主窗口正常打开。这次同时调整了两项，具体由哪一项触发故障尚未逐项验证。该机器此前的 Windows 重启早于本次启动报错。
