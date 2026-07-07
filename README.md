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

Already have xovi + AppLoad? Install from the [remagic](https://github.com/maximerivest/remagic)
catalog, [grab the prebuilt bundle](#install-the-prebuilt-bundle), or
[build from source](#building).

### Install with remagic (easiest)

```sh
remagic install riddle     # checksum-verified download → AppLoad
remagic config riddle      # settings form in your browser (+ QR for phone)
```

Then in **AppLoad**: tap **Reload**, then **The Diary**. Write, and rest your
pen. (Or install it from the **Store** app right on the tablet.)

### Install the prebuilt bundle

1. Grab `riddle-<version>.zip` from the [latest release](https://github.com/MaximeRivest/riddle/releases/latest)
   and unzip it into a folder: `unzip riddle-*.zip -d riddle`
2. Copy the folder to your tablet:
   `scp -O -r riddle root@10.11.99.1:/home/root/xovi/exthome/appload/`
3. Add an API key: `cp oracle.env.example oracle.env` in that folder and put your `RIDDLE_OPENAI_KEY` in it (any OpenAI-compatible key). Or skip it to use [pi](#option-b--pi-the-power-path).
4. In **AppLoad**: tap **Reload**, then **The Diary**. Write, and rest your pen.

> ⚠️ **This modifies your device.** The prebuilt bundle and the catalog build
> run in **takeover mode**: tapping The Diary stops the whole reMarkable UI
> and takes the screen. Leave with a **5-finger tap** — xochitl restarts
> automatically. It runs as root and drives the e-ink engine directly. It has
> only been tested on a **reMarkable Paper Pro** (ferrari, aarch64,
> OS 3.26–3.27). It may not work on other models or OS versions, and you use
> it entirely at your own risk. Not affiliated with reMarkable AS. Keep SSH
> access working before you install anything — if anything ever wedges:
> `ssh root@10.11.99.1 'systemctl start xochitl'`.

## How it works

```
 pen (raw evdev, full 4096-level pressure, hardware event rate)
   │ strokes
   ▼
 riddle ── idle 2.8s → commit page → PNG ──► oracle (resident LLM process,
   │                                          streams reply sentence-by-sentence)
   ▼ strokes (Dancing Script → skeletonized to single-pixel pen paths)
 display backend
   ├── qtfb        — windowed, inside xochitl (build-from-source flavour)
   └── quill       — full takeover: xochitl stopped, vendor e-ink engine
                     driven directly for instant ink (lowest latency there
                     is; what the prebuilt bundle runs)
```

- **`riddle/`** — the app (Rust). Pen input, ink surface, handwriting
  synthesis (rasterize → Zhang-Suen thinning → stroke tracing → animated
  replay), the oracle process manager, and both display backends.
- **`quill/`** — the takeover display host (C/C++). An
  [epfb-re](https://github.com/asivery/epfb-re)-style QImage-constructor
  interposition shim over the vendor `libqsgepaper.so` waveform engine,
  exposed as a small C ABI (`quill_init` / `quill_buffer` / `quill_swap`)
  that riddle links against with `--features takeover`. Also carries a small
  family of demos (`scribble`, a pen-to-glass latency test, plus map, image,
  and GIF renderers).

## Gestures

| Do this | And |
|---------|-----|
| Write, then rest the pen | The diary drinks your ink and Tom replies |
| Write *"show me what I wrote about…"* | The remembered page **rises through the paper**: the date, your own handwriting rewriting itself stroke by stroke, Tom's old reply — all in faded ink. Touch the pen anywhere and today's page returns |
| Write *"what do you remember?"* | Tom answers with a handwritten list of remembered moments |
| Write *"draw me…"* | Tom **draws back**: a sketch inks itself onto the page, stroke by stroke, in his own hand |
| Flip the marker | Erase |
| Draw a large **?** | Summon the built-in guide |
| Tap five fingers at once | Leave the diary *(takeover mode)* |
| Power button | The page turns to *"The diary sleeps."*, then the tablet suspends; press again to wake exactly where you were *(takeover mode)* |

In the windowed (qtfb) flavour, xochitl keeps the touchscreen and the power
button: close the diary from AppLoad instead.

## The diary remembers

Every finished page is kept — your actual pen strokes, a transcription, and
Tom's reply — so the diary can do three things:

- **Follow the conversation.** Recent pages ride along with each request, so
  Tom remembers what you wrote yesterday (both backends, same behavior).
- **Conjure the past.** Ask in ink — *"show me the page about the garden"*,
  *"find what I wrote on Tuesday"* — and the diary rewrites that page in
  front of you, in your own hand, dated, in faded ink. No buttons, no lists,
  no chrome: the pen is the only interface.
- **Answer from memory.** *"What do you remember?"* gets a handwritten index.

Memories live only on the tablet, in plain files under
`/home/root/riddle-data/memories` (delete the folder and the diary forgets;
the last ~400 pages are kept). `RIDDLE_MEMORY=off` in `oracle.env` turns all
of it off — no storage, and nothing extra sent with requests. Set
`RIDDLE_TZ_OFFSET` (hours from UTC) so memory dates read right.

## Tom draws back

Ask in ink — *"draw me a map of the grounds"*, *"sketch me a cat"* — and the
reply is a **drawing**: pen strokes inking themselves onto the page, placed
among whatever words come with them. It also plays: draw a noughts-and-crosses
grid and make your move — your ink fades when the diary drinks it, so Tom
redraws the whole board with his move added.

Under the hood the oracle answers with a block of stroke coordinates that
riddle scales onto the page and draws with the small unsteadiness of a real
hand. Works with both oracle backends; how well Tom draws depends entirely on
the model behind the diary — stronger models sketch better.

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
export RIDDLE_OPENAI_REASONING="low"                    # thinking models only
export RIDDLE_OPENAI_MAX_TOKENS="2000"                  # runaway guard
```

Any vision-capable model works. On the tablet these live in `oracle.env`
next to the binary (see `oracle.env.example`, or just run
`remagic config riddle` — it has one-tap presets for OpenAI, OpenRouter,
and Gemini). Example with OpenRouter:

```sh
export RIDDLE_OPENAI_KEY="$OPENROUTER_API_KEY"
export RIDDLE_OPENAI_BASE="https://openrouter.ai/api/v1"
export RIDDLE_OPENAI_MODEL="openai/gpt-4o-mini"
```

Two gotchas with thinking models (Gemini 3.x, o-series): set
`RIDDLE_OPENAI_REASONING=low` for faster first ink (some providers reject
the field on non-thinking models — leave it unset there), and keep
`RIDDLE_OPENAI_MAX_TOKENS` roomy — hidden reasoning tokens count against it,
and a tight cap starves the visible reply.

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
`RIDDLE_OPENAI_KEY` is **not** set. Defaults (override in `oracle.env`):
pi at `/home/root/node/bin` (`RIDDLE_PI_BIN_DIR`), provider `openai-codex`
(`RIDDLE_PI_PROVIDER`), model `gpt-5.4-mini` (`RIDDLE_PI_MODEL`).

Both stream the reply sentence-by-sentence, so the quill starts writing seconds
before the model finishes. The persona prompt lives in `riddle/src/oracle.rs`.

A note on Tom's memory: with the HTTP backend every page is a fresh
conversation — Tom does not remember your previous page. With pi, the warm
session remembers everything since the diary was opened (and pi persists
that session in its own data dir on the tablet).

If the oracle can't answer — missing key, refused key, no Wi-Fi — Tom writes
the reason on the page instead of a reply, and the full error goes to the
journal (`journalctl -u riddle-takeover`).

## Building

Cross-compiled from x86_64. Two flavours:

### Windowed (AppLoad/qtfb) — build from source

The bundles above are the takeover flavour; the windowed flavour must be
built. Requires [xovi + AppLoad](https://github.com/asivery/rm-appload) on
the device.

```sh
cd riddle
cargo build --release --target aarch64-unknown-linux-gnu
```

Install the binary to `/home/root/xovi/exthome/appload/riddle/` with an
`external.manifest.json` that sets `"qtfb": true` and points `"application"`
at the binary itself (the manifest in this repo is the takeover one — AppLoad
only hands riddle a window, via `QTFB_KEY`, when `qtfb` is true).

### Takeover (instant ink) — the one from the demo

Requires the reMarkable SDK toolchain (`~/rm-sdk-3.26`) because the linked
vendor Qt libs need its glibc, **and** `libqsgepaper.so` pulled from *your own
device* (it is proprietary and not distributed here):

```sh
cd quill && ./build.sh              # pulls libqsgepaper.so from the device over
                                    # ssh, builds libquill.so + the demos
cd ../riddle && ./build-takeover.sh
./scripts/make-bundle.sh            # stages the AppLoad bundle in dist/riddle/
```

The staged `dist/riddle/` is self-contained (binary, `libquill.so`, launch
scripts, manifest) — copy it to
`/home/root/xovi/exthome/appload/riddle/`, or publish it to the catalog with
`remagic publish dist/riddle`. Launching via AppLoad (`appload-launch.sh`)
detaches into a transient systemd unit, stops xochitl, runs the diary, and
**always restores xochitl on exit** — leave with a 5-finger tap or SIGTERM
(`systemctl stop riddle-takeover`); the power button sleeps and wakes the
diary without leaving it. The unit's stop hook restarts xochitl even if
riddle dies uncleanly. If anything wedges:
`ssh root@10.11.99.1 'systemctl start xochitl'`.

## What leaves the device

- Each committed page is rasterized to a small grayscale PNG and sent to the
  oracle **you** configured — nothing else ever leaves the tablet, and there
  is no telemetry.
- The PNG (`/tmp/riddle-page.png`) is deleted as soon as the oracle has read
  it; set `RIDDLE_KEEP_PAGE=1` to keep the last page around for debugging.
- riddle never writes replies to disk. The pi backend, however, keeps its own
  session history in its data dir — the HTTP backend keeps nothing.
- Tom stays in character by design: the persona prompt (see
  `riddle/src/oracle.rs`) tells the model it is the diary and nothing else.

## Fonts

The reply hand is [Dancing Script](https://github.com/googlefonts/DancingScript)
(SIL OFL 1.1 — see `riddle/fonts/OFL.txt`).

## License

MIT for everything in this repository (see `LICENSE`). The vendor libraries it
interposes (`libqsgepaper.so`, Qt) are **not** included and must come from
your own device/SDK.
