#!/bin/sh
# Runs Scoutline as PUID:PGID so files in /config belong to the host user.
set -eu

if [ "$(id -u)" = "0" ]; then
    mkdir -p /config
    chown -R "$PUID:$PGID" /config
    exec setpriv --reuid="$PUID" --regid="$PGID" --clear-groups scoutline "$@"
fi
exec scoutline "$@"
