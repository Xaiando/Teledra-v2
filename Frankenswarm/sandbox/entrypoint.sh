#!/bin/sh
set -eu
if [ "$#" -eq 0 ]; then
  echo "No command provided to sandbox." >&2
  exit 64
fi
exec /bin/sh -c "$1"
