#!/bin/sh
# Portage privilege-dropping breaks in a userns container: setgroups(2) is
# denied (EPERM), so fetching/building as the portage user fails with
# "PermissionError: [Errno 1] Operation not permitted" (os.setgroups).
# Same class of issue as apt-sandbox.sh. Containers are single-user;
# run everything as root instead.
set -eu
ROOT="$1"
CONF="$ROOT/etc/portage/make.conf"
mkdir -p "$ROOT/etc/portage"
touch "$CONF"
if ! grep -q 'dshell-userns' "$CONF"; then
    cat >> "$CONF" <<'EOF'
# dshell-userns: no setgroups in userns, cannot drop to the portage user
FEATURES="${FEATURES} -userfetch -userpriv -usersandbox -usersync"
EOF
fi
