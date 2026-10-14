#!/usr/bin/env bash
# FORK-ONLY: DC1 installer, run on the Dell from a checkout of the fork (no sudo).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$HOME/filmcraft-ci/bin" "$HOME/.config/systemd/user"
install -m 755 "$here/dell-ci.sh" "$HOME/filmcraft-ci/bin/dell-ci.sh"
install -m 644 "$here/dell-ci.service" "$here/dell-ci.timer" "$HOME/.config/systemd/user/"
systemctl --user daemon-reload
systemctl --user enable --now dell-ci.timer
loginctl show-user "$USER" -p Linger
