#!/bin/sh
# Static resolv.conf for the container.
# Cloud images (ubuntu) ship etc/resolv.conf as a dangling symlink to
# ../run/systemd/resolve/stub-resolv.conf; rm removes the link itself,
# and a plain file works regardless (run.rs mounts a fresh tmpfs on /run,
# so anything placed under rootfs/run would be hidden at runtime anyway).
set -eu
ROOT="$1"
rm -f "$ROOT/etc/resolv.conf"
printf 'nameserver 8.8.8.8\nnameserver 1.1.1.1\n' > "$ROOT/etc/resolv.conf"
