#!/bin/sh
# LibriSync Server entrypoint (runs under tini).
#
# Started as root (the default): re-map the `librisync` user to PUID/PGID, apply UMASK,
# make /config ours, then drop privileges with setpriv. The library volume is NOT
# chowned recursively — it can be huge and may be shared with Audiobookshelf; only its
# top level is checked for writability.
#
# Started non-root (`docker run --user 1000:1000`): skip the remapping and run as is.
set -eu

umask "${UMASK:-022}"

if [ "$(id -u)" = "0" ]; then
    PUID="${PUID:-1000}"
    PGID="${PGID:-1000}"

    if [ "$(id -g librisync)" != "$PGID" ]; then
        groupmod --non-unique --gid "$PGID" librisync
    fi
    if [ "$(id -u librisync)" != "$PUID" ]; then
        usermod --non-unique --uid "$PUID" librisync
    fi

    mkdir -p "${LIBRISYNC_CONFIG_DIR}"
    chown -R librisync:librisync "${LIBRISYNC_CONFIG_DIR}"

    if ! setpriv --reuid="$PUID" --regid="$PGID" --init-groups \
            test -w "${LIBRISYNC_LIBRARY_DIR}"; then
        echo "WARNING: ${LIBRISYNC_LIBRARY_DIR} is not writable by ${PUID}:${PGID}; downloads will fail." >&2
    fi

    exec setpriv --reuid="$PUID" --regid="$PGID" --init-groups "$@"
fi

if [ ! -w "${LIBRISYNC_CONFIG_DIR}" ]; then
    echo "ERROR: ${LIBRISYNC_CONFIG_DIR} is not writable by uid $(id -u)." >&2
    exit 1
fi
exec "$@"
