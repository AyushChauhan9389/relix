#!/usr/bin/env bash
# System libraries a Linux build needs: libmpv for playback, plus what Freya (Skia, windowing) links.
set -euo pipefail
sudo apt-get update -qq
sudo apt-get install -y -qq build-essential pkg-config cmake libclang-dev libssl-dev \
  libgtk-3-dev libxkbcommon-dev libwayland-dev libfontconfig1-dev libmpv-dev
