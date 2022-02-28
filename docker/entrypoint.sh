#!/bin/sh

echo "Starting with UID: $UID, GID: $GID"
useradd -u "$UID" -o -m user
groupmod -g "$GID" user
export HOME=/home/user
chown -R "$UID":"$GID" /input /output

exec runuser -u user "$@"