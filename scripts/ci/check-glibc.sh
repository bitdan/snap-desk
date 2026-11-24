#!/usr/bin/env bash
# 检查 Linux 安装包要求的最高 glibc 版本：解开 AppImage，读取其中全部 ELF 文件引用的 GLIBC_x.y 符号版本。
# 超过上限时失败，防止构建机或内置库升级后旧发行版（以及 AppImage 目录的测试环境）无法启动。
# 用法：bash scripts/ci/check-glibc.sh <AppImage 路径> [上限，默认 2.35]
set -euo pipefail

appimage="$1"
limit="${2:-2.35}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cp "$appimage" "$work/app.AppImage"
chmod +x "$work/app.AppImage"
# --appimage-extract 只解包不运行程序，输出到当前目录的 squashfs-root。
(cd "$work" && ./app.AppImage --appimage-extract > /dev/null)

report="$work/report.txt"
while IFS= read -r -d '' file; do
  # 非 ELF 文件 objdump 会报错，直接跳过。
  version="$(objdump -T "$file" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -V | tail -n1 || true)"
  [ -n "$version" ] && echo "$version ${file#"$work/squashfs-root/"}" >> "$report"
done < <(find "$work/squashfs-root" -type f -print0)

if [ ! -s "$report" ]; then
  echo "未在 $appimage 中找到引用 glibc 的 ELF 文件。" >&2
  exit 1
fi

sort -V -r "$report" | head -n 10
max="$(sort -V "$report" | tail -n1 | cut -d' ' -f1)"
echo "最高 glibc 要求：GLIBC_$max（上限 GLIBC_$limit）"
# libstdc++ 由系统提供时同样不能高于 22.04（GLIBCXX_3.4.30）；这里只打印，便于排查。
glibcxx="$(find "$work/squashfs-root" -type f -exec objdump -T {} \; 2>/dev/null | grep -o 'GLIBCXX_[0-9.]*' | sort -V | tail -n1 || true)"
echo "最高 libstdc++ 要求：${glibcxx:-无}"
if [ "$(printf '%s\n%s\n' "$max" "$limit" | sort -V | tail -n1)" != "$limit" ]; then
  echo "安装包要求的 glibc 高于 $limit，旧发行版无法运行。" >&2
  exit 1
fi
