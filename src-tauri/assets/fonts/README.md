# 内置中文字体

`NotoSansSC.ttf` 来自 Noto CJK 官方仓库的简体中文 TrueType 字体：
https://github.com/notofonts/noto-cjk/blob/main/Sans/Variable/TTF/Subset/NotoSansSC-VF.ttf

使用 fontTools 将可变字体实例化为 wght=400 的静态 TrueType，避免 PDF 阅读器对可变字体支持不一致。按随附 `OFL.txt`（SIL Open Font License 1.1）再分发。字体嵌入 PDF 并用于长图文字渲染，不依赖用户安装的系统字体。
不覆盖全部 emoji；遇到缺失字形，导出明确失败并建议保存 TXT，避免静默丢字。
