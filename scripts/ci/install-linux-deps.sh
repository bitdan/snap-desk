#!/usr/bin/env bash
# Linux 构建依赖：CI 与发布流水线共用，本地 Debian / Ubuntu 也可直接执行。
# 设置 SNAP_DESK_APT_ARCHIVES 时，安装包下载到该目录并保留，供 CI 缓存复用；
# runner 访问 Ubuntu 镜像有时只有几十 KB/s，命中缓存后 100 MB 依赖不必重新下载。
set -euo pipefail

options=(-o Acquire::Retries=3)
archives="${SNAP_DESK_APT_ARCHIVES:-}"
if [ -n "$archives" ]; then
  mkdir -p "$archives/partial"
  options+=(-o "Dir::Cache::Archives=$archives" -o APT::Keep-Downloaded-Packages=true)
fi

sudo apt-get "${options[@]}" update
# GitHub 的 ubuntu-22.04 镜像预装 clang 带来的 libunwind-14-dev，它与 GStreamer 开发包依赖的
# libunwind-dev 冲突（apt 报 held broken packages）；构建不需要它，安装前先卸载。
if dpkg -s libunwind-14-dev > /dev/null 2>&1; then
  sudo apt-get "${options[@]}" remove -y libunwind-14-dev
fi
# 第一组是 Tauri（WebKitGTK、托盘、打包 AppImage）的要求；
# 第二组是截图库 xcap 的要求：X11 抓屏、Wayland / PipeWire 接口及其绑定生成所需的 libclang；
# 第三组是录屏：MIT-SHM 抓屏、XInput2 键鼠事件和 GStreamer 编码 / 声音采集的开发库；
# 第四组是 GPUI 外壳：键盘布局（xkbcommon）、字体发现、X11-XCB 与 zstd（GPUI 资源解压）。
sudo apt-get "${options[@]}" install -y --no-install-recommends \
  build-essential curl wget file pkg-config patchelf \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev \
  libclang-dev libxcb1-dev libxrandr-dev libdbus-1-dev libpipewire-0.3-dev libwayland-dev libegl-dev libgbm-dev \
  libxcb-shm0-dev libxcb-xinput-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev   libxkbcommon-dev libxkbcommon-x11-dev libfontconfig-dev libfreetype-dev libx11-xcb-dev libzstd-dev libvulkan-dev

# Ubuntu 22.04 自带的 PipeWire 头文件早于 0.3.65，libspa 0.10 无法编译：改用 0.3.65 头文件，
# 仍链接系统库（详见 pipewire-headers.sh）。CI 中通过 GITHUB_ENV 让后续步骤的 pkg-config 先找到它。
if ! pkg-config --atleast-version=0.3.65 libpipewire-0.3; then
  pkgconfig="$(bash "$(dirname "$0")/pipewire-headers.sh" "$HOME/.cache/snap-desk-pipewire-headers")"
  if [ -n "${GITHUB_ENV:-}" ]; then
    echo "PKG_CONFIG_PATH=$pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}" >> "$GITHUB_ENV"
  else
    echo "构建前执行：export PKG_CONFIG_PATH=$pkgconfig\${PKG_CONFIG_PATH:+:\$PKG_CONFIG_PATH}"
  fi
fi

# glibc 2.38 之前的系统链接 ort 预编译库时缺少新符号：静态链接 GCC 13 的 libstdc++ 并补上 C23 转发函数
# （详见 glibc-compat.sh），通过 RUSTFLAGS 交给后续的 cargo 步骤。
glibc="$(ldd --version | head -n1 | grep -o '[0-9]*\.[0-9]*$')"
if [ "$(printf '%s\n2.38\n' "$glibc" | sort -V | head -n1)" != 2.38 ]; then
  rustflags="$(bash "$(dirname "$0")/glibc-compat.sh" "$HOME/.cache/snap-desk-glibc-compat")"
  if [ -n "${GITHUB_ENV:-}" ]; then
    echo "RUSTFLAGS=${RUSTFLAGS:+$RUSTFLAGS }$rustflags" >> "$GITHUB_ENV"
  else
    echo "构建前执行：export RUSTFLAGS=\"$rustflags\""
  fi
fi

if [ -n "$archives" ]; then
  # 删除镜像上已不存在的旧版本，避免缓存随包升级不断变大；
  # apt 以 root 写入，交还给当前用户，缓存步骤才能读取并打包。
  sudo apt-get "${options[@]}" autoclean
  sudo rm -f "$archives/lock"
  sudo chown -R "$(id -u):$(id -g)" "$archives"
fi
