#!/usr/bin/env bash
# 为旧发行版（Ubuntu 22.04 自带 PipeWire 0.3.48）准备 PipeWire 0.3.65 的头文件。
#
# 截图库 xcap 依赖的 libspa 0.10 使用了 0.3.65 头文件里的内联函数和结构体字段，旧头文件无法编译。
# 这些内联函数只编进程序本身；动态库仍链接系统自带的 libpipewire-0.3.so（0.3.48）。
# 只换头文件、不换库：若代码用到旧库没有的导出函数，链接阶段就会报错，不会产生在旧系统上启动失败的程序。
#
# 头文件取自 Debian 归档中固定不变的上游源码包，校验 SHA-256 后只复制公开头文件，并生成
# 指向这些头文件的 pkg-config 描述；调用方把输出的目录加入 PKG_CONFIG_PATH。
# 用法：bash scripts/ci/pipewire-headers.sh <安装目录>，输出 pkg-config 目录路径。
set -euo pipefail

version=0.3.65
url="https://deb.debian.org/debian/pool/main/p/pipewire/pipewire_${version}.orig.tar.bz2"
sha256=c78c95cdff2774418f0a9468e94a1b2800761d322f13104df6501e1a0dc92558

prefix="$(realpath -m "$1")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

curl -fsSL --retry 3 -o "$work/pipewire.tar.bz2" "$url"
echo "$sha256  $work/pipewire.tar.bz2" | sha256sum -c - >&2
tar -xjf "$work/pipewire.tar.bz2" -C "$work"
source="$work/pipewire-$version"

# 与 meson 安装布局一致：include/spa-0.2/spa、include/pipewire-0.3/pipewire（含 extensions）。
rm -rf "$prefix"
mkdir -p "$prefix/include/spa-0.2" "$prefix/include/pipewire-0.3/pipewire" "$prefix/lib/pkgconfig"
cp -r "$source/spa/include/spa" "$prefix/include/spa-0.2/"
(cd "$source/src/pipewire" && find . -name '*.h' -exec cp --parents {} "$prefix/include/pipewire-0.3/pipewire/" \;)

# version.h 由 meson 生成，这里按同样的占位符替换。
IFS=. read -r major minor micro <<< "$version"
sed -e "s/@PIPEWIRE_VERSION_MAJOR@/$major/g" -e "s/@PIPEWIRE_VERSION_MINOR@/$minor/g" \
  -e "s/@PIPEWIRE_VERSION_MICRO@/$micro/g" -e 's/@PIPEWIRE_API_VERSION@/"0.3"/g' \
  "$source/src/pipewire/version.h.in" > "$prefix/include/pipewire-0.3/pipewire/version.h"

# Libs 不写 -L：链接器在系统库目录找到发行版自带的 libpipewire-0.3.so。
cat > "$prefix/lib/pkgconfig/libspa-0.2.pc" <<EOF
prefix=$prefix
includedir=\${prefix}/include

Name: libspa
Description: Simple Plugin API (headers only)
Version: $version
Cflags: -I\${includedir}/spa-0.2 -D_REENTRANT
EOF
cat > "$prefix/lib/pkgconfig/libpipewire-0.3.pc" <<EOF
prefix=$prefix
includedir=\${prefix}/include

Name: libpipewire
Description: PipeWire Interface (newer headers, system library)
Version: $version
Requires: libspa-0.2
Libs: -lpipewire-0.3
Cflags: -I\${includedir}/pipewire-0.3 -D_REENTRANT
EOF

echo "$prefix/lib/pkgconfig"
