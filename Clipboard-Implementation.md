# 剪贴板历史接入记录

实施日期：2026-10-15。代码已接入；**未执行 Rust 编译、Rust 单测、桌面启动、打包或 Windows 实机验收**，不能据此宣称已经通过桌面验收。

## Phase A：本地存储

- `src-tauri/src/clipboard/model.rs`、`repository.rs`、`migrations/001.sql`：Rust 独占 SQLite；事务迁移使用唯一的 `PRAGMA user_version=1`，不并行维护第二套版本表。
- `blob_store.rs`：内容寻址 PNG/WebP、同磁盘临时文件、提交后回收、启动检查丢失引用/孤儿文件、路径与链接校验。
- `service.rs`：重复内容刷新时间和计数，收藏保护，条数/空间/保留期清理。配额不足拒绝新素材，不删除收藏。SQLite 锁不跨图片编解码、对话框或窗口创建。
- `settings.rs`、`src/lib/settings.ts`：旧配置自动补 `clipboard` 和 `history` 字段，默认不记录历史；原有快捷键值保留。旧动作已占用 Ctrl+Shift+V 时仅将新增 history 映射置空。历史配置独立合并，旧截图/快捷键草稿不能覆盖暂停开关。
- `Cargo.toml` / `Cargo.lock`：锁定 rusqlite 0.37.0（bundled）、BLAKE3 1.8.2。图片仍在文件系统，数据库只存文本及相对引用。

新增 Rust 定向测试覆盖迁移重开、精确文本/去重、相同时间游标分页、收藏配额回滚、保留期、孤儿文件和丢失引用。按项目规则未运行这些需编译的测试。

## Phase B：监听与管理页

- `clipboard/platform/windows.rs`：独立 Win32 消息窗口，AddClipboardFormatListener / WM_CLIPBOARDUPDATE，有界非阻塞通知，暂停与退出注销并 join。
- `reader.rs`：一次打开期间拷贝有上限的原始字节，关闭后解码 PNG/DIB 或 UTF-16 文本。读取前限制字节和图片头尺寸，解码时再限制内存；图片优先；有限重试；跳过常见保密标记，不承诺识别全部密码内容。
- `clipboard/mod.rs`：所有历史命令限制为 main 窗口；恢复、收藏、删除、清空、配置、按 ID 贴图；恢复使用序号和内容摘要避免再次入库。
- `lib.rs`：初始化容错、命令注册、历史托盘入口及 Ctrl+Shift+V；统一使用既有快捷键注册/回滚流程。F1/F2/F3 入口保持不变，F2 不依赖历史数据库。录屏原退出流程完成后停止历史服务。
- `src/components/clipboard/ClipboardHistory.vue`、`ClipboardItem.vue`、`src/lib/clipboard.ts`、`src/lib/api.ts`、`src/App.vue`：搜索、类型/收藏筛选、分页、按需缩略图、文本预览、复制、收藏、多选、删除确认、清空确认、立即暂停与配额设置。
- `storage.rs` / `GeneralSettings.vue`：只通过预设种类打开历史目录。

平台限制：自动监听只支持 Windows。macOS/Linux 不启动监听器，已有手动截图/剪贴板贴图仍使用原实现。极快连续复制允许合并通知，不保证每个中间状态都入库。收藏保护只用于自动清理；用户明确选中删除或勾选“同时删除收藏”时允许删除。

## Phase C：组合导出

- `clipboard/export.rs`：按选择/拖动顺序导出多页 PDF、纵向 PNG、分别 PNG/TXT；PDF 页面/边距、长图间距/背景配置；复用既有保存目录记忆。
- 历史导出使用原素材快照，与当前贴图标注导出分开；快照后允许删除历史，不会破坏本次导出。
- 单文件使用目标目录内临时文件和原子替换；分别导出在所选目录创建独立批次子目录，全部成功才保留。取消返回 cancelled；失败保留选择和已存在目标文件。
- PDF 锁定 printpdf 0.7.0，内置 Noto Sans SC 静态中文字体及 OFL 许可证。文字为实际文本，按字宽换行并分页；遇到缺失字形（含部分 emoji）明确失败，建议 TXT，避免静默丢字。
- 长图限制 3200 万像素；PDF 图片总量限制 6400 万像素、文本至多 1000 页；所有导出快照合计至多 256 MiB；同一时刻只允许一个组合导出任务。

当前导出预览为素材顺序预览，并非实际 PDF 页面渲染。分别导出固定 PNG/TXT；既有单张截图/贴图的 JPEG/WebP 保存仍可使用。P2 的 PDF 文件合并、HTML/RTF、OCR、跨平台自动监听未接入。

## 已执行验证

- `npm run check`：通过。
- `npx vitest run tests/clipboard.test.ts tests/settings.test.ts tests/api-errors.test.ts tests/recording-api.test.ts`：4 个文件、24 个测试通过。
- 对修改的 Rust 文件执行 rustfmt：解析/格式检查通过，不代表类型检查或编译通过。
- `cargo metadata --locked --no-deps --format-version 1`：锁文件与依赖清单读取通过，未触发编译。
- Python SQLite 冒烟检查实际 `001.sql`：约束、唯一性、保留收藏的清空和数据库重开通过。这不替代 Rust Repository 测试。
- 字体元数据检查：静态 TrueType、中英文字形覆盖及缺失字符检测通过。这不替代 PDF 搜索/复制、字形和分页的实机验收。

## 待用户安排的验证

项目 `AGENTS.md` 要求未经明确指示不执行 Rust 编译，因此以下尚未执行：

1. Rust 定向测试：`cargo test --manifest-path src-tauri/Cargo.toml --locked clipboard`，以及 `settings` 快捷键迁移/回滚测试。
2. Windows 实机：开启前不采集、纯文本/中文/emoji、PNG/DIB/透明图、系统剪贴板占用、快速复制、暂停/恢复、托盘隐藏记录、退出释放。
3. 历史重启、收藏配额、只读/磁盘满、手动删除素材、数据库损坏时 F2 可用。
4. PDF 中文显示/搜索/复制/换页、长图比例/透明背景、多选顺序、取消/覆盖/输出目录不可写，以及原 F1/F2/F3 工作流回归。
