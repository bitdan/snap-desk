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

if [ -n "$archives" ]; then
  # 删除镜像上已不存在的旧版本，避免缓存随包升级不断变大；
  # apt 以 root 写入，交还给当前用户，缓存步骤才能读取并打包。
  sudo apt-get "${options[@]}" autoclean
  sudo rm -f "$archives/lock"
  sudo chown -R "$(id -u):$(id -g)" "$archives"
fi
