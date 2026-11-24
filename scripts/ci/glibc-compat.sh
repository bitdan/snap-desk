#!/usr/bin/env bash
# 在 glibc 2.38 之前的构建机（Ubuntu 22.04）上链接 ort 预编译的 ONNX Runtime 静态库。
#
# ort 下载的 ONNX Runtime 在较新的系统上用 GCC 13+ 编译，引用了旧系统没有的两类符号：
# - glibc 2.38 的 C23 版 strtol 系列（__isoc23_strtol 等）：这里编译同名的隐藏符号转发到旧版函数，
#   行为只差 C23 新增的 0b 二进制前缀，ONNX Runtime 解析配置与数字不依赖它；
# - libstdc++ 13 的内部函数（如 basic_string::_M_replace_cold）：改为静态链接 GCC 13 的 libstdc++.a
#   （ubuntu-toolchain-r PPA 为 22.04 构建，只依赖 22.04 的 glibc），程序不再依赖系统 libstdc++ 的新版本。
# ort-sys 以 -lstdc++ 链接；把只含 libstdc++.a 的目录放在搜索路径最前面，链接器就会选用静态库。
#
# 用法：bash scripts/ci/glibc-compat.sh <输出目录>；输出应追加到 RUSTFLAGS 的参数。
set -euo pipefail

prefix="$(realpath -m "$1")"
gcc_version=13

if ! [ -f "/usr/lib/gcc/x86_64-linux-gnu/$gcc_version/libstdc++.a" ]; then
  sudo add-apt-repository -y ppa:ubuntu-toolchain-r/test >&2
  sudo apt-get -o Acquire::Retries=3 install -y --no-install-recommends "libstdc++-$gcc_version-dev" >&2
fi

rm -rf "$prefix"
mkdir -p "$prefix/lib"
cp "/usr/lib/gcc/x86_64-linux-gnu/$gcc_version/libstdc++.a" "$prefix/lib/"

cat > "$prefix/isoc23.c" <<'EOF'
#include <stdlib.h>
#define HIDDEN __attribute__((visibility("hidden")))
HIDDEN long __isoc23_strtol(const char *s, char **end, int base) { return strtol(s, end, base); }
HIDDEN unsigned long __isoc23_strtoul(const char *s, char **end, int base) { return strtoul(s, end, base); }
HIDDEN long long __isoc23_strtoll(const char *s, char **end, int base) { return strtoll(s, end, base); }
HIDDEN unsigned long long __isoc23_strtoull(const char *s, char **end, int base) { return strtoull(s, end, base); }
EOF
cc -O2 -fPIC -c "$prefix/isoc23.c" -o "$prefix/isoc23.o"

echo "-L native=$prefix/lib -C link-arg=$prefix/isoc23.o"
