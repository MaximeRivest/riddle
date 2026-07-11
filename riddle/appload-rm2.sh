#!/bin/sh
set -eu
cd "$(dirname "$0")"
if [ -f oracle.env ]; then
    set -a
    . ./oracle.env
    set +a
fi
exec ./riddle-rm2
