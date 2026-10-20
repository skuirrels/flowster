#!/usr/bin/env bash
# Installs what the Linux build needs in the WSL distro it runs in (`make linux` on Windows).
# Ubuntu 22.04, because packages built against its older glibc also run on newer distributions.
#
# Once, on the Windows PC:
#   wsl --install -d Ubuntu-22.04      (create the user it asks for, then exit)
#   make linux-setup                    (runs this script; asks for that user's sudo password)
set -euo pipefail

. /etc/os-release
if [ "${VERSION_ID:-}" != "22.04" ]; then
  echo "provision-wsl: this is ${PRETTY_NAME:-unknown}; Linux releases are built on Ubuntu 22.04" >&2
  exit 1
fi

sudo apt-get update
sudo apt-get install -y build-essential curl wget file git pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev patchelf rpm xdg-utils

if ! command -v cargo >/dev/null && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi

if ! command -v node >/dev/null || [ "$(node -p 'process.versions.node.split(".")[0]')" -lt 22 ]; then
  curl -fsSL https://deb.nodesource.com/setup_24.x | sudo -E bash -
  sudo apt-get install -y nodejs
fi
# pnpm, for apps that use it (the version in their package.json "packageManager").
command -v pnpm >/dev/null || sudo corepack enable

# The build clones the Windows checkout (/mnt/c/…), whose files belong to another user as far as
# git is concerned.
git config --global --add safe.directory '*'

. "$HOME/.cargo/env"
echo "provision-wsl: base tools ready ($(rustc --version), node $(node --version))"
