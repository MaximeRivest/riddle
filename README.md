# riddle — the diary of Tom Riddle, for the reMarkable Paper Pro

Write on the page with your pen. After a pause, the diary **drinks your ink** —
your words fade into the paper — the page thinks for a moment, and an answer
writes itself back in a flowing hand, stroke by stroke, then fades away.

No screen glow, no keyboard, no chat UI. Just ink appearing on paper.

_This is the diary from [the demo](https://x.com/MaximeRivest)._

### 🪄 New to this? Start here

You need a **reMarkable Paper Pro** in developer mode with a launcher installed.
If that sounds like a lot, it isn't — **[remagic](https://github.com/maximerivest/remagic)**
walks you through turning on developer mode and sets up everything with one
command. Come back here, drop riddle in, and start writing to Tom.

Already have xovi + AppLoad? **[Download the latest release](https://github.com/MaximeRivest/riddle/releases/latest)** — a ready-to-drop bundle, no compiler needed — or [build from source](#building).

### Install the prebuilt bundle

1. Grab `riddle-appload-aarch64.zip` from the [latest release](https://github.com/MaximeRivest/riddle/releases/latest) and unzip it.
2. Copy the folder to your tablet:
   `scp -O -r riddle root@10.11.99.1:/home/root/xovi/exthome/appload/`
3. Add an API key: `cp oracle.env.example oracle.env` in that folder and put your `RIDDLE_OPENAI_KEY` in it (any OpenAI-compatible key). Or skip it to use [pi](#option-b--pi-the-power-path).
4. In **AppLoad**: tap **Reload**, then **The Diary**. Write, and rest your pen.

> ⚠️ **This modifies your device.** It runs as root, stops the vendor UI
> (in takeover mode), and drives the e-ink engine directly. It has only been
> tested on a **reMarkable Paper Pro** (ferrari, aarch64, OS 3.26–3.27). It may
> not work on other models or OS versions, and you use it entirely at your own
> risk. Not affiliated with reMarkable AS. Keep SSH access working before you
> install anything — that is your escape hatch.

## How it works

```
 pen (raw evdev, full 4096-level pressure, hardware event rate)
   │ strokes
   ▼
 riddle ── idle 2.8s → commit page → PNG ──► oracle (resident LLM process,
   │                                          streams reply sentence-by-sentence)
   ▼ strokes (Dancing Script → skeletonized to single-pixel pen paths)
 display backend
   ├── qtfb        — windowed, inside xochitl (AppLoad app)
   └── quill       — full takeover: xochitl stopped, vendor e-ink engine
                     driven directly for instant ink (lowest latency there is)
```

- **`riddle/`** — the app (Rust). Pen input, ink surface, handwriting
  synthesis (rasterize → Zhang-Suen thinning → stroke tracing → animated
  replay), the oracle process manager, and both display backends.
- **`quill/`** — the takeover display host (C/C++). An
  [epfb-re](https://github.com/asivery)-style QImage-constructor interposition
  shim over the vendor `libqsgepaper.so` waveform engine, exposed as a small
  C ABI (`quill_init` / `quill_buffer` / `quill_swap`) that riddle links
  against with `--features takeover`. Includes `scribble`, a minimal
  pen-to-glass latency demo.

## Gestures

| Do this | And |
|---------|-----|
| Write, then rest the pen | The diary drinks your ink and Tom replies |
| Flip the marker | Erase |
| Draw a large **?** | Summon the built-in guide |
| Tap five fingers at once | Leave the diary |
| Power button | The page turns to *"The diary sleeps."*, then the tablet suspends; press again to wake exactly where you were |

## The oracle (the "spirit" in the diary)

The diary's replies come from a vision LLM that reads your handwriting from the
committed page (sent as an inline PNG). There are **two backends**, chosen at
startup — pick whichever you have:

### Option A — any OpenAI-compatible API (easiest, zero setup)

Set an API key and riddle talks straight to an OpenAI-compatible
`/chat/completions` endpoint. Works with OpenAI, OpenRouter, Groq, a local
server — anything that speaks the format. No extra software on the tablet.

```sh
export RIDDLE_OPENAI_KEY="sk-..."                       # required
export RIDDLE_OPENAI_BASE="https://api.openai.com/v1"   # optional (default)
export RIDDLE_OPENAI_MODEL="gpt-4o-mini"                # optional; must see images
```

Any vision-capable model works. Example with OpenRouter:

```sh
export RIDDLE_OPENAI_KEY="$OPENROUTER_API_KEY"
export RIDDLE_OPENAI_BASE="https://openrouter.ai/api/v1"
export RIDDLE_OPENAI_MODEL="openai/gpt-4o-mini"
```

Verify your setup before launching the diary:

```sh
riddle --oracle-test path/to/handwriting.png   # prints the streamed reply
```

Measured ~0.9–1.1 s to first ink on-device. The HTTPS is built into riddle
(pure-Rust, no extra libraries).

### Option B — pi (the power path)

If you already run [`pi`](https://github.com/badlogic/pi-mono), riddle will use
a resident `pi --mode rpc` process kept warm (Node + your subscription auth
loaded once), so each turn pays only model latency. Used automatically when
`RIDDLE_OPENAI_KEY` is **not** set.

Both stream the reply sentence-by-sentence, so the quill starts writing seconds
before the model finishes. The persona prompt lives in `riddle/src/oracle.rs`.

## Building

Cross-compiled from x86_64. Two flavours:

### Windowed (AppLoad/qtfb) — easiest

Requires [xovi + AppLoad](https://github.com/asivery/rm-appload) on the device.

```sh
cd riddle
cargo build --release --target aarch64-unknown-linux-gnu
```

Install to `/home/root/xovi/exthome/appload/riddle/` with
`external.manifest.json`, `appload-launch.sh`, and the binary.

### Takeover (instant ink) — the one from the demo

Requires the reMarkable SDK toolchain (`~/rm-sdk-3.26`) because the linked
vendor Qt libs need its glibc, **and** `libqsgepaper.so` pulled from *your own
device* (it is proprietary and not distributed here):

```sh
cd quill && ./build.sh          # pulls libqsgepaper.so from the device over ssh,
                                # builds libquill.so + scribble
cd ../riddle && ./build-takeover.sh
```

Deploy `libquill.so` to `/home/root/quill/` and `riddle-takeover` to
`/home/root/riddle/riddle`, plus `scripts/riddle-takeover.sh`. Launching via
AppLoad (`appload-launch.sh`) detaches into a transient systemd unit, stops
xochitl, runs the diary, and **always restores xochitl on exit** — exit with
the power button, a 5-finger tap, or SIGTERM. If anything wedges:
`ssh root@10.11.99.1 'systemctl start xochitl'`.

### reMarkable 2 (windowed only)

The rM2 needs **no developer mode** — SSH is built in (password under
Settings → Help → Copyrights → GPLv3 Compliance). One command installs
everything: xovi, AppLoad, persistence, riddle, and your oracle key:

```sh
rustup target add armv7-unknown-linux-musleabihf   # once; needs zig + cargo-zigbuild
cd riddle && ./build-rm2.sh && cd ..
./scripts/install-rm2.sh        # USB; or RM_HOST=<tablet-ip> for Wi-Fi
```

Full walkthrough, manual steps, and troubleshooting:
**[docs/rm2-setup.md](docs/rm2-setup.md)**. The rM2 runs in windowed
(AppLoad/qtfb) mode only — the takeover engine is Paper Pro-specific.

## Fonts

The reply hand is [Dancing Script](https://github.com/googlefonts/DancingScript)
(SIL OFL 1.1 — see `riddle/fonts/OFL.txt`).

## License

MIT for everything in this repository (see `LICENSE`). The vendor libraries it
interposes (`libqsgepaper.so`, Qt) are **not** included and must come from
your own device/SDK.
