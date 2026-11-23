//! 富文本（HTML）贴图：读取剪贴板 CF_HTML 片段，按白名单解析并在 Rust 侧排版成图片。
//!
//! Tauri 版由主 WebView 排版（`features/pins/html-pin.ts`）；GPUI 版没有 WebView，用这里的简化排版器：
//! 与前端相同的标签/样式白名单——不加载外链与图片（只保留替代文本）、不执行脚本、不继承任意 CSS；
//! 支持粗体、下划线、删除线、颜色、字号、等宽、标题、段落、换行、列表、引用、预格式、分隔线与简单表格。
//! 斜体、背景色、对齐等不改变内容的样式会被忽略。宽 640 像素，高度超过 4096 像素报错。
use crate::annotations::raster::{advance, draw_text, em_scale, glyph_font, Fonts};
use ab_glyph::{Font, FontRef, ScaleFont};
use html5ever::{local_name, namespace_url, ns, tendril::TendrilSink, ParseOpts, QualName};
use image::{Rgba, RgbaImage};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

const WIDTH: u32 = 640;
const PADDING: f64 = 16.0;
const MAX_HEIGHT: f64 = 4096.0;
const MAX_HTML: usize = 1024 * 1024;
const TEXT: [u8; 3] = [0x24, 0x29, 0x2f];
const LINK: [u8; 3] = [0x25, 0x63, 0xeb];
const BORDER: [u8; 3] = [0xd1, 0xd5, 0xdb];
const QUOTE: [u8; 3] = [0xd0, 0xd7, 0xde];

const BLOCKED: [&str; 18] = [
    "script", "style", "iframe", "object", "embed", "svg", "math", "template", "link", "meta", "base", "input",
    "textarea", "select", "button", "audio", "video", "canvas",
];

/// CF_HTML 的偏移单位是 UTF-8 字节；只接受合法范围，缺失偏移时用标准片段标记。
pub fn fragment(bytes: &[u8]) -> Result<String, String> {
    let header = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let offset = |key: &str| {
        header
            .lines()
            .find_map(|line| line.strip_prefix(key)?.trim().parse::<usize>().ok())
    };
    let content = match (offset("StartFragment:"), offset("EndFragment:")) {
        (Some(start), Some(end)) if start < end && end <= bytes.len() => &bytes[start..end],
        _ => {
            let start_tag = b"<!--StartFragment-->";
            let end_tag = b"<!--EndFragment-->";
            let start = bytes
                .windows(start_tag.len())
                .position(|v| v == start_tag)
                .map(|i| i + start_tag.len())
                .ok_or("富文本片段头无效。")?;
            let end = bytes[start..]
                .windows(end_tag.len())
                .position(|v| v == end_tag)
                .map(|i| start + i)
                .ok_or("富文本片段尾无效。")?;
            &bytes[start..end]
        }
    };
    if content.len() > MAX_HTML {
        return Err("富文本贴图最多支持 1 MiB 内容。".into());
    }
    let html = std::str::from_utf8(content)
        .map_err(|_| "富文本编码无效。")?
        .trim_matches('\0')
        .to_string();
    if html.trim().is_empty() {
        return Err("剪贴板富文本为空。".into());
    }
    Ok(html)
}

/// 读取剪贴板中的 HTML 片段；没有富文本时返回 None。不写入剪贴板历史。
#[cfg(windows)]
pub fn read() -> Result<Option<String>, String> {
    use windows::{
        core::w,
        Win32::{
            Foundation::HGLOBAL,
            System::{DataExchange::*, Memory::*},
        },
    };
    let _clipboard =
        clipboard_win::Clipboard::new_attempts(5).map_err(|_| "剪贴板正在使用，请稍后重试。")?;
    unsafe {
        let format = RegisterClipboardFormatW(w!("HTML Format"));
        if format == 0 || IsClipboardFormatAvailable(format).is_err() {
            return Ok(None);
        }
        let handle = HGLOBAL(GetClipboardData(format).map_err(|e| e.to_string())?.0);
        let length = GlobalSize(handle);
        if length == 0 || length > MAX_HTML + 4096 {
            return Err("富文本贴图最多支持 1 MiB 内容。".into());
        }
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            return Err("富文本暂时不可读。".into());
        }
        let bytes = std::slice::from_raw_parts(pointer.cast::<u8>(), length).to_vec();
        let _ = GlobalUnlock(handle);
        fragment(&bytes).map(Some)
    }
}

#[cfg(not(windows))]
pub fn read() -> Result<Option<String>, String> {
    Ok(None)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Style {
    bold: bool,
    underline: bool,
    strike: bool,
    mono: bool,
    color: [u8; 3],
    size: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self { bold: false, underline: false, strike: false, mono: false, color: TEXT, size: 16.0 }
    }
}

#[derive(Clone, Debug)]
struct Run {
    text: String,
    style: Style,
}

#[derive(Debug)]
enum Block {
    Text {
        runs: Vec<Run>,
        indent: f64,
        prefix: Option<Run>,
        quote: bool,
        heading: bool,
    },
    Rule,
    Table(Vec<Vec<Vec<Run>>>),
}

/// 颜色：#RGB / #RRGGBB / #RRGGBBAA、rgb()/rgba() 与常用颜色名。
fn parse_color(value: &str) -> Option<[u8; 3]> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() > 40 {
        return None;
    }
    if let Some(hex) = value.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok();
        return match hex.len() {
            3 | 4 => Some([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17]),
            6 | 8 => Some([
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            ]),
            _ => None,
        };
    }
    if let Some(body) = value.strip_prefix("rgba(").or_else(|| value.strip_prefix("rgb(")) {
        let parts: Vec<f64> = body
            .trim_end_matches(')')
            .split(',')
            .take(3)
            .filter_map(|p| p.trim().trim_end_matches('%').parse().ok())
            .collect();
        let channel = |v: f64| v.clamp(0.0, 255.0) as u8;
        return (parts.len() == 3).then(|| [channel(parts[0]), channel(parts[1]), channel(parts[2])]);
    }
    Some(match value.as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "orange" => [255, 165, 0],
        "purple" => [128, 0, 128],
        "yellow" => [255, 255, 0],
        "navy" => [0, 0, 128],
        "maroon" => [128, 0, 0],
        "teal" => [0, 128, 128],
        _ => return None,
    })
}

/// 字号：px / pt / em，限制在 8～64 像素。
fn parse_size(value: &str, parent: f64) -> Option<f64> {
    let value = value.trim().to_ascii_lowercase();
    let number = |suffix: &str| value.strip_suffix(suffix).and_then(|v| v.trim().parse::<f64>().ok());
    let px = if let Some(v) = number("px") {
        v
    } else if let Some(v) = number("pt") {
        v * 4.0 / 3.0
    } else if let Some(v) = number("rem") {
        v * 16.0
    } else if let Some(v) = number("em") {
        v * parent
    } else {
        return None;
    };
    px.is_finite().then(|| px.clamp(8.0, 64.0))
}

/// 只读取白名单内的内联样式。
fn apply_css(style: &mut Style, css: &str) {
    for declaration in css.split(';').take(64) {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_end_matches("!important").trim();
        match property.trim().to_ascii_lowercase().as_str() {
            "color" => {
                if let Some(color) = parse_color(value) {
                    style.color = color;
                }
            }
            "font-weight" => style.bold = matches!(value, "bold" | "bolder" | "600" | "700" | "800" | "900"),
            "text-decoration" | "text-decoration-line" => {
                style.underline = value.contains("underline");
                style.strike = value.contains("line-through");
            }
            "font-size" => {
                if let Some(size) = parse_size(value, style.size) {
                    style.size = size;
                }
            }
            "font-family" => {
                let family = value.to_ascii_lowercase();
                if family.contains("mono") || family.contains("consolas") || family.contains("courier") {
                    style.mono = true;
                }
            }
            _ => {}
        }
    }
}

struct Builder {
    blocks: Vec<Block>,
    current: Vec<Run>,
    indent: f64,
    quote: u32,
    pre: u32,
    lists: Vec<Option<usize>>,
    prefix: Option<Run>,
    heading: bool,
    /// 表格：行、当前行、当前单元格。
    table: Option<Vec<Vec<Vec<Run>>>>,
    row: Option<Vec<Vec<Run>>>,
    cell: Option<Vec<Run>>,
    nodes: usize,
}

impl Builder {
    fn push_text(&mut self, text: &str, style: Style) {
        let text = if self.pre > 0 {
            text.replace("\r\n", "\n").replace('\t', "    ")
        } else {
            let mut out = String::with_capacity(text.len());
            let mut space = false;
            for c in text.chars() {
                if c.is_whitespace() {
                    space = true;
                } else {
                    if space {
                        out.push(' ');
                    }
                    space = false;
                    out.push(c);
                }
            }
            if space {
                out.push(' ');
            }
            out
        };
        if text.is_empty() {
            return;
        }
        let target = self.cell.as_mut().unwrap_or(&mut self.current);
        match target.last_mut() {
            Some(last) if last.style == style => last.text.push_str(&text),
            _ => target.push(Run { text, style }),
        }
    }

    fn flush(&mut self) {
        let mut runs = std::mem::take(&mut self.current);
        if self.pre == 0 {
            trim_runs(&mut runs);
        }
        if runs.iter().any(|r| !r.text.trim().is_empty()) {
            self.blocks.push(Block::Text {
                runs,
                indent: self.indent,
                prefix: self.prefix.take(),
                quote: self.quote > 0,
                heading: self.heading,
            });
        }
    }

    fn walk(&mut self, node: &Handle, style: Style, depth: usize) -> Result<(), String> {
        self.nodes += 1;
        if self.nodes > 4096 || depth > 40 {
            return Err("富文本结构过于复杂，请缩小选区。".into());
        }
        let (name, attrs) = match &node.data {
            NodeData::Text { contents } => {
                self.push_text(&contents.borrow(), style);
                return Ok(());
            }
            NodeData::Element { name, attrs, .. } => (name.local.to_ascii_lowercase(), attrs.borrow().clone()),
            NodeData::Document => return self.children(node, style, depth),
            _ => return Ok(()),
        };
        let tag: &str = &name;
        if BLOCKED.contains(&tag) || matches!(tag, "noscript" | "head" | "title") {
            return Ok(());
        }
        let attr = |key: &str| attrs.iter().find(|a| a.name.local.as_ref() == key).map(|a| a.value.to_string());
        if tag == "img" {
            if let Some(alt) = attr("alt").filter(|a| !a.trim().is_empty()) {
                self.push_text(&alt, style);
            }
            return Ok(());
        }
        let mut style = style;
        match tag {
            "b" | "strong" | "th" => style.bold = true,
            "u" | "ins" => style.underline = true,
            "s" | "del" | "strike" => style.strike = true,
            "code" | "pre" | "kbd" | "samp" => style.mono = true,
            "sub" | "sup" | "small" => style.size = (style.size * 0.8).max(8.0),
            "a" => {
                style.color = LINK;
                style.underline = true;
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                style.bold = true;
                style.size = match tag {
                    "h1" => 32.0,
                    "h2" => 24.0,
                    "h3" => 18.72,
                    "h4" => 16.0,
                    "h5" => 13.28,
                    _ => 10.72,
                };
            }
            _ => {}
        }
        if tag == "font" {
            if let Some(color) = attr("color").and_then(|c| parse_color(&c)) {
                style.color = color;
            }
        }
        if let Some(css) = attr("style") {
            apply_css(&mut style, &css);
        }
        match tag {
            "br" => self.newline(),
            "hr" => {
                self.flush();
                self.blocks.push(Block::Rule);
            }
            "table" => {
                self.flush();
                let outer = self.table.replace(Vec::new());
                self.children(node, style, depth)?;
                let rows = std::mem::replace(&mut self.table, outer).unwrap_or_default();
                if rows.iter().any(|r| r.iter().any(|c| c.iter().any(|run| !run.text.trim().is_empty()))) {
                    self.blocks.push(Block::Table(rows));
                }
            }
            "tr" if self.table.is_some() => {
                self.row = Some(Vec::new());
                self.children(node, style, depth)?;
                if let (Some(row), Some(table)) = (self.row.take(), self.table.as_mut()) {
                    if !row.is_empty() {
                        table.push(row);
                    }
                }
            }
            "td" | "th" if self.row.is_some() => {
                self.cell = Some(Vec::new());
                self.children(node, style, depth)?;
                let mut cell = self.cell.take().unwrap_or_default();
                trim_runs(&mut cell);
                if let Some(row) = self.row.as_mut() {
                    row.push(cell);
                }
            }
            _ if self.cell.is_some() => self.children(node, style, depth)?,
            "ul" | "ol" => {
                self.flush();
                self.lists.push(if tag == "ol" { Some(0) } else { None });
                self.indent += 24.0;
                self.children(node, style, depth)?;
                self.indent -= 24.0;
                self.lists.pop();
                self.flush();
            }
            "li" => {
                self.flush();
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{n}.")
                    }
                    _ => "•".to_owned(),
                };
                self.prefix = Some(Run { text: marker, style });
                self.children(node, style, depth)?;
                self.flush();
            }
            "blockquote" => {
                self.flush();
                self.quote += 1;
                self.indent += 16.0;
                self.children(node, style, depth)?;
                self.flush();
                self.indent -= 16.0;
                self.quote -= 1;
            }
            "pre" => {
                self.flush();
                self.pre += 1;
                self.children(node, style, depth)?;
                self.flush();
                self.pre -= 1;
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "dt" | "dd" | "figcaption" => {
                self.flush();
                self.heading = tag.starts_with('h') && tag.len() == 2;
                self.children(node, style, depth)?;
                self.flush();
                self.heading = false;
            }
            _ => self.children(node, style, depth)?,
        }
        Ok(())
    }

    fn children(&mut self, node: &Handle, style: Style, depth: usize) -> Result<(), String> {
        for child in node.children.borrow().iter() {
            self.walk(child, style, depth + 1)?;
        }
        Ok(())
    }
}

impl Builder {
    /// `<br>`：非预格式文本里也要保留为换行（push_text 会把空白折叠成空格）。
    fn newline(&mut self) {
        let target = self.cell.as_mut().unwrap_or(&mut self.current);
        if let Some(last) = target.last_mut() {
            if last.text.ends_with(' ') {
                last.text.pop();
            }
            last.text.push('\n');
        } else {
            target.push(Run { text: "\n".into(), style: Style::default() });
        }
    }
}

fn trim_runs(runs: &mut Vec<Run>) {
    if let Some(first) = runs.first_mut() {
        first.text = first.text.trim_start_matches(' ').to_owned();
    }
    if let Some(last) = runs.last_mut() {
        last.text = last.text.trim_end_matches(' ').to_owned();
    }
    runs.retain(|r| !r.text.is_empty());
}

/// 解析并清洗 HTML，得到排版块。
fn blocks(html: &str) -> Result<Vec<Block>, String> {
    if html.trim().is_empty() || html.len() > MAX_HTML {
        return Err("富文本为空或超过 1 MiB。".into());
    }
    let dom = html5ever::parse_fragment(
        RcDom::default(),
        ParseOpts::default(),
        QualName::new(None, ns!(html), local_name!("body")),
        vec![],
    )
    .one(html);
    let mut builder = Builder {
        blocks: Vec::new(),
        current: Vec::new(),
        indent: 0.0,
        quote: 0,
        pre: 0,
        lists: Vec::new(),
        prefix: None,
        heading: false,
        table: None,
        row: None,
        cell: None,
        nodes: 0,
    };
    builder.walk(&dom.document, Style::default(), 0)?;
    builder.flush();
    if builder.blocks.is_empty() {
        return Err("没有可贴图的富文本。".into());
    }
    Ok(builder.blocks)
}

/// 排版用的三套字体栈：常规、粗体、等宽（均回退中文字体）。
struct Stacks<'a> {
    regular: Vec<FontRef<'a>>,
    bold: Vec<FontRef<'a>>,
    mono: Vec<FontRef<'a>>,
}

impl<'a> Stacks<'a> {
    fn of(&self, style: &Style) -> &[FontRef<'a>] {
        if style.mono {
            &self.mono
        } else if style.bold {
            &self.bold
        } else {
            &self.regular
        }
    }
}

fn cjk(c: char) -> bool {
    matches!(c, '\u{2e80}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}' | '\u{ff00}'..='\u{ffef}')
}

/// 一行中的一个字符：字符、样式、宽度。
type Glyph = (char, Style, f64);

/// 按宽度折行：优先在空格处断开（中日韩文字可在任意字处断开），`\n` 强制换行。
fn wrap(runs: &[Run], width: f64, stacks: &Stacks) -> Result<Vec<Vec<Glyph>>, String> {
    let mut lines: Vec<Vec<Glyph>> = vec![Vec::new()];
    let mut used = 0.0;
    for run in runs {
        for c in run.text.chars() {
            if c == '\n' {
                lines.push(Vec::new());
                used = 0.0;
                continue;
            }
            let w = advance(stacks.of(&run.style), c, run.style.size)?;
            let line = lines.last_mut().expect("at least one line");
            if used + w > width && !line.is_empty() {
                // 在最近的空格处断开，把空格之后的内容移到下一行。
                let split = (!cjk(c)).then(|| line.iter().rposition(|g| g.0 == ' ')).flatten();
                let mut next = match split {
                    Some(at) if at + 1 < line.len() => {
                        let tail = line.split_off(at + 1);
                        line.pop();
                        tail
                    }
                    _ => Vec::new(),
                };
                if c != ' ' {
                    next.push((c, run.style, w));
                }
                used = next.iter().map(|g| g.2).sum();
                lines.push(next);
                continue;
            }
            line.push((c, run.style, w));
            used += w;
        }
    }
    Ok(lines)
}

enum Op {
    Text { x: f64, baseline: f64, text: String, style: Style },
    Rect { x: f64, y: f64, w: f64, h: f64, color: [u8; 3] },
}

fn line_height(line: &[Glyph], base: f64) -> f64 {
    line.iter().map(|g| g.1.size).fold(base, f64::max) * 1.5
}

/// 把一组折好的行放到 (x, y) 处，返回总高度。
fn place_lines(ops: &mut Vec<Op>, lines: &[Vec<Glyph>], x: f64, y: f64, base: f64) -> f64 {
    let mut top = y;
    for line in lines {
        let height = line_height(line, base);
        let max = line.iter().map(|g| g.1.size).fold(base, f64::max);
        let baseline = top + height / 2.0 + max * 0.35;
        let mut cursor = x;
        let mut segment = String::new();
        let mut segment_x = x;
        let mut segment_style: Option<Style> = None;
        for (c, style, w) in line {
            if segment_style != Some(*style) {
                if let Some(s) = segment_style.take() {
                    ops.push(Op::Text { x: segment_x, baseline, text: std::mem::take(&mut segment), style: s });
                }
                segment_style = Some(*style);
                segment_x = cursor;
            }
            segment.push(*c);
            cursor += w;
        }
        if let Some(s) = segment_style {
            ops.push(Op::Text { x: segment_x, baseline, text: segment, style: s });
        }
        top += height;
    }
    top - y
}

/// 排版并绘制：白底、宽 640。高度超过 4096 像素时报错（与前端一致）。
pub fn render(html: &str, fonts: &Fonts) -> Result<RgbaImage, String> {
    let blocks = blocks(html)?;
    let stacks = Stacks { regular: fonts.stack("sans")?, bold: fonts.stack("bold")?, mono: fonts.stack("mono")? };
    let content = f64::from(WIDTH) - PADDING * 2.0;
    let mut ops = Vec::new();
    let mut y = PADDING;
    let mut first = true;
    for block in &blocks {
        match block {
            Block::Text { runs, indent, prefix, quote, heading } => {
                if !first {
                    y += if *heading { 12.0 } else { 6.0 };
                }
                let x = PADDING + indent;
                let lines = wrap(runs, (content - indent).max(40.0), &stacks)?;
                let base = runs.first().map_or(16.0, |r| r.style.size);
                if let Some(prefix) = prefix {
                    let width: f64 = prefix
                        .text
                        .chars()
                        .map(|c| advance(stacks.of(&prefix.style), c, prefix.style.size))
                        .sum::<Result<f64, _>>()?;
                    let first_line = lines.first().map_or(base * 1.5, |l| line_height(l, base));
                    let max = lines.first().map_or(base, |l| l.iter().map(|g| g.1.size).fold(base, f64::max));
                    ops.push(Op::Text {
                        x: x - width - 6.0,
                        baseline: y + first_line / 2.0 + max * 0.35,
                        text: prefix.text.clone(),
                        style: Style { underline: false, strike: false, ..prefix.style },
                    });
                }
                let height = place_lines(&mut ops, &lines, x, y, base);
                if *quote {
                    ops.push(Op::Rect { x: x - 10.0, y, w: 3.0, h: height, color: QUOTE });
                }
                y += height;
            }
            Block::Rule => {
                y += 8.0;
                ops.push(Op::Rect { x: PADDING, y, w: content, h: 1.0, color: QUOTE });
                y += 9.0;
            }
            Block::Table(rows) => {
                if !first {
                    y += 6.0;
                }
                let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                let column = content / columns as f64;
                for row in rows {
                    let mut laid = Vec::new();
                    let mut height: f64 = 16.0 * 1.5;
                    for cell in row {
                        let lines = wrap(cell, (column - 8.0).max(20.0), &stacks)?;
                        let base = cell.first().map_or(16.0, |r| r.style.size);
                        let h: f64 = lines.iter().map(|l| line_height(l, base)).sum();
                        height = height.max(h);
                        laid.push((lines, base));
                    }
                    height += 8.0;
                    for (index, (lines, base)) in laid.iter().enumerate() {
                        place_lines(&mut ops, lines, PADDING + column * index as f64 + 4.0, y + 4.0, *base);
                    }
                    for index in 0..columns {
                        let x = PADDING + column * index as f64;
                        ops.push(Op::Rect { x, y, w: column, h: 1.0, color: BORDER });
                        ops.push(Op::Rect { x, y: y + height, w: column, h: 1.0, color: BORDER });
                        ops.push(Op::Rect { x, y, w: 1.0, h: height, color: BORDER });
                    }
                    ops.push(Op::Rect { x: PADDING + content - 1.0, y, w: 1.0, h: height, color: BORDER });
                    y += height;
                    if y > MAX_HEIGHT {
                        return Err("富文本尺寸过大，请缩小选区。".into());
                    }
                }
                y += 1.0;
            }
        }
        first = false;
        if y > MAX_HEIGHT {
            return Err("富文本尺寸过大，请缩小选区。".into());
        }
    }
    let height = (y + PADDING).ceil().min(MAX_HEIGHT) as u32;
    let mut canvas = RgbaImage::from_pixel(WIDTH, height, Rgba([255, 255, 255, 255]));
    let mut layer = RgbaImage::new(WIDTH, height);
    for op in &ops {
        match op {
            Op::Rect { x, y, w, h, color } => {
                let (x0, y0) = (x.round().max(0.0) as u32, y.round().max(0.0) as u32);
                let (x1, y1) = (((x + w).round() as u32).min(WIDTH), ((y + h).round() as u32).min(height));
                for py in y0..y1.max(y0 + 1).min(height) {
                    for px in x0..x1.max(x0 + 1).min(WIDTH) {
                        canvas.put_pixel(px, py, Rgba([color[0], color[1], color[2], 255]));
                    }
                }
            }
            Op::Text { x, baseline, text, style } => {
                let stack = stacks.of(style);
                let first = text.chars().next().unwrap_or(' ');
                let font = glyph_font(stack, first).unwrap_or(&stack[0]);
                let ascent = f64::from(font.as_scaled(em_scale(font, style.size)).ascent());
                let end = draw_text(&mut layer, stack, text, style.size, (*x, baseline - ascent), style.color)?;
                let thickness = (style.size / 16.0).max(1.0);
                let mut line = |offset: f64| {
                    let top = (baseline + offset).round().max(0.0) as u32;
                    for py in top..(top + thickness.round() as u32).min(height) {
                        for px in (x.round().max(0.0) as u32)..(end.round() as u32).min(WIDTH) {
                            layer.put_pixel(px, py, Rgba([style.color[0], style.color[1], style.color[2], 255]));
                        }
                    }
                };
                if style.underline {
                    line(style.size * 0.12);
                }
                if style.strike {
                    line(-style.size * 0.3);
                }
            }
        }
    }
    for (base, glyph) in canvas.pixels_mut().zip(layer.pixels()) {
        let a = f64::from(glyph[3]) / 255.0;
        if a > 0.0 {
            for channel in 0..3 {
                base[channel] = (f64::from(glyph[channel]) * a + f64::from(base[channel]) * (1.0 - a)).round() as u8;
            }
        }
    }
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    const FONT: &[u8] = include_bytes!("../../tests/fixtures/export-test.ttf");

    fn fonts() -> Fonts {
        Fonts { sans: FONT.to_vec(), serif: None, mono: None, bold: None }
    }

    #[test]
    fn fragment_uses_byte_offsets_and_rejects_invalid_ranges() {
        let body = "<b>中文</b>";
        let header = format!("StartFragment:{:010}\r\nEndFragment:{:010}\r\n", 60, 60 + body.len());
        let mut bytes = header.into_bytes();
        bytes.resize(60, b' ');
        bytes.extend_from_slice(body.as_bytes());
        assert_eq!(fragment(&bytes).unwrap(), body);
        assert!(fragment(b"StartFragment:900\r\nEndFragment:1000\r\n").is_err());
        assert_eq!(fragment(b"<!--StartFragment--><i>ok</i><!--EndFragment-->").unwrap(), "<i>ok</i>");
    }

    #[test]
    fn sanitizes_blocked_content_and_keeps_structure() {
        let parsed = blocks(
            "<script>alert(1)</script><style>p{}</style><h1>Title</h1><p>a <b>bold</b><br>next</p>\
             <img src=\"http://x/a.png\" alt=\"alt text\"><ul><li>one</li><li>two</li></ul>\
             <table><tr><th>h</th><td>v</td></tr></table><hr>",
        )
        .unwrap();
        let text: Vec<String> = parsed
            .iter()
            .filter_map(|b| match b {
                Block::Text { runs, .. } => Some(runs.iter().map(|r| r.text.as_str()).collect()),
                _ => None,
            })
            .collect();
        assert!(!text.iter().any(|t| t.contains("alert") || t.contains("p{}")));
        assert_eq!(text[0], "Title");
        assert_eq!(text[1], "a bold\nnext");
        assert!(text.iter().any(|t| t.contains("alt text")));
        assert!(parsed.iter().any(|b| matches!(b, Block::Text { prefix: Some(p), .. } if p.text == "•")));
        assert!(parsed.iter().any(|b| matches!(b, Block::Table(rows) if rows[0].len() == 2)));
        assert!(matches!(parsed.last(), Some(Block::Rule)));
        assert!(blocks("<script>x</script>").is_err());
    }

    #[test]
    fn styles_follow_whitelist() {
        let mut style = Style::default();
        apply_css(&mut style, "color:#f00; font-weight:700; text-decoration:underline; font-size:200px; background:url(x)");
        assert_eq!(style.color, [255, 0, 0]);
        assert!(style.bold && style.underline);
        assert_eq!(style.size, 64.0);
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some([1, 2, 3]));
        assert_eq!(parse_color("expression(alert(1))"), None);
        assert_eq!(parse_size("12pt", 16.0), Some(16.0));
    }

    #[test]
    fn renders_white_page_with_text_and_rejects_huge_content() {
        let image = render("<p>Snap <b>Desk</b> <u>English</u></p><blockquote>中文</blockquote>", &fonts()).unwrap();
        assert_eq!(image.width(), 640);
        assert_eq!(image.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert!(image.pixels().any(|p| p[0] < 128));
        let long = "<p>line</p>".repeat(400);
        assert!(render(&long, &fonts()).is_err());
    }

    /// 依赖本机字体，CI 不跑；设置 SNAP_HTML_DUMP=<目录> 可导出 PNG 查看效果。
    #[test]
    #[ignore]
    fn renders_sample_with_system_fonts() {
        let fonts = Fonts::system(None, Some("mono"), true).unwrap();
        let html = r#"<h2>富文本贴图 Rich text</h2><p>普通文字 <b>粗体</b> <u>下划线</u> <s>删除线</s>
            <span style="color:#e11d48">红色</span> <a href="https://example.com">链接</a> <code>code()</code></p>
            <ul><li>第一项 first item that is long enough to wrap onto the next line for testing purposes</li><li>第二项</li></ul>
            <ol><li>one</li><li>two</li></ol><blockquote>引用的段落</blockquote>
            <table><tr><th>名称</th><th>值</th></tr><tr><td>a</td><td>1</td></tr></table><hr><pre>fn main() {
    println!("hi");
}</pre>"#;
        let image = render(html, &fonts).unwrap();
        if let Ok(dir) = std::env::var("SNAP_HTML_DUMP") {
            image.save(format!("{dir}/html-pin.png")).unwrap();
        }
    }
}
