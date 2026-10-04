#!/usr/bin/env bash
# Build the optional ChatGPT companion. Requires Go 1.25+.
set -euo pipefail
cd "$(dirname "$0")/../tools/chatgpt"
mkdir -p dist
go test ./...
CGO_ENABLED=0 GOOS=linux GOARCH=arm64 go build -trimpath -o dist/riddle-chatgpt-linux-arm64 .
# Use this native helper for the one-time browser sign-in on the computer.
CGO_ENABLED=0 go build -trimpath -o dist/riddle-chatgpt-login .
