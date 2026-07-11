#!/bin/sh
set -eu
cd "$(dirname "$0")"

if [ -f riddle.pid ] && kill -0 "$(cat riddle.pid)" 2>/dev/null; then
    echo "The Diary is already listening (PID $(cat riddle.pid))."
    exit 0
fi

if [ ! -f oracle.env ]; then
    echo "Missing oracle.env. Copy oracle.env.example and add RIDDLE_OPENAI_KEY." >&2
    exit 1
fi

set -a
. ./oracle.env
set +a
export RIDDLE_XOCHITL=1
export RIDDLE_MEMORY=${RIDDLE_MEMORY:-off}

nohup ./riddle-rm2 >>riddle.log 2>&1 &
echo $! >riddle.pid
echo "The Diary is listening. Open a blank notebook and write near the top."
