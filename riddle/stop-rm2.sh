#!/bin/sh
set -eu
cd "$(dirname "$0")"

if [ -f riddle.pid ]; then
    pid=$(cat riddle.pid)
    kill "$pid" 2>/dev/null || true
    rm -f riddle.pid
fi
echo "The Diary is closed. The stock reMarkable UI was not stopped."
