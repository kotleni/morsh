#!/bin/sh
# apt privilege-dropping breaks in a userns container: setgroups(2) is
# denied (EPERM), so the http method dies with "Failed to setgroups".
# The sandbox only matters on multi-user systems; disable it here.
set -eu
ROOT="$1"
mkdir -p "$ROOT/etc/apt/apt.conf.d"
printf 'APT::Sandbox::User "root";\n' > "$ROOT/etc/apt/apt.conf.d/10-dshell-sandbox"
