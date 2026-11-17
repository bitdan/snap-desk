#!/usr/bin/env bash
# Linux 构建依赖：CI 与发布流水线共用，本地 Debian / Ubuntu 也可直接执行。
set -euo pipefail

sudo apt-get update
# 第一组是 Tauri（WebKitGTK、托盘、打包 AppImage）的要求；
# 第二组是截图库 xcap 的要求：X11 抓屏、Wayland / PipeWire 接口及其绑定生成所需的 libclang；
# 第三组是录屏：MIT-SHM 抓屏、XInput2 键鼠事件和 GStreamer 编码 / 声音采集的开发库。
sudo apt-get install -y --no-install-recommends \
  build-essential curl wget file pkg-config patchelf \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev \
  libclang-dev libxcb1-dev libxrandr-dev libdbus-1-dev libpipewire-0.3-dev libwayland-dev libegl-dev libgbm-dev \
  libxcb-shm0-dev libxcb-xinput-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
