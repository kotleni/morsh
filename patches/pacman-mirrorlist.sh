#!/bin/sh
# Arch bootstrap tweaks: single working mirror, no package signature
# checks, no sandboxing (rootfs is extracted unprivileged).
set -eu
ROOT="$1"
printf 'Server = https://geo.mirror.pkgbuild.com/$repo/os/$arch\n' > "$ROOT/etc/pacman.d/mirrorlist"
sed -i \
  -e 's/^[[:space:]]*SigLevel.*/SigLevel = Never/' \
  -e 's/#DisableSandboxFilesystem/DisableSandboxFilesystem/' \
  -e 's/#DisableSandboxSyscalls/DisableSandboxSyscalls/' \
  "$ROOT/etc/pacman.conf"
