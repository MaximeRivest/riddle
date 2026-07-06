# Riddle for iPad

An iPad port of [MaximeRivest/riddle](https://github.com/MaximeRivest/riddle) — Tom Riddle's enchanted diary, reimagined for Apple Pencil.

Write with Apple Pencil → the diary drinks your ink → Tom replies in handwriting that appears stroke by stroke.

## Screenshots

(Coming soon)

## Requirements

- iPad with Apple Pencil support
- iOS 17.0+
- Xcode 26.6+
- An API key for any OpenAI-compatible, vision-capable LLM

## Setup

1. Clone the repo
2. Open `Riddle.xcodeproj` in Xcode (use [XcodeGen](https://github.com/yonaskolb/XcodeGen) to regenerate from `project.yml` if needed)
3. Build and run on your iPad
4. On first launch, the Settings screen will appear — enter your API configuration:
   - **API Key**: Your API key
   - **Base URL**: The API endpoint (defaults to OpenAI)
   - **Model**: A vision-capable model name

## Supported APIs

Any OpenAI-compatible `/chat/completions` endpoint that supports image input works:

| Provider | Base URL | Model |
|----------|----------|-------|
| OpenAI | `https://api.openai.com/v1` | `gpt-4o-mini` |
| OpenRouter | `https://openrouter.ai/api/v1` | `openai/gpt-4o-mini` |
| Volcano Ark | `https://ark.cn-beijing.volces.com/api/plan/v3` | `doubao-seed-2-0-pro` |
| Groq | `https://api.groq.com/openai/v1` | `llama-3.2-90b-vision-preview` |

Presets are available in the Settings screen.

## How It Works

1. **Write** with Apple Pencil on the white page
2. After 1.5s idle, the diary **drinks the ink** — pixels dissolve using a per-pixel hash
3. The ink snapshot is sent as a PNG to your configured LLM **during** the dissolve (hides network latency)
4. The model **reads your handwriting** from the image and replies as Tom Riddle
5. The reply is rendered using **handwriting synthesis**: text is rasterized → Zhang-Suen skeletonized → stroke-traced → animated point by point
6. After lingering, the reply **dissolves** the same way the ink did

## Architecture

Direct port of the original project's approach:

- **Surface.swift** — Pixel buffer (like reMarkable's framebuffer). Direct `putPx`/`brushLine`/`stamp` operations.
- **Ink** — Stroke capture, PNG export, pixel dissolve (`dissolve_pass` with `px_hash`)
- **Oracle.swift** — HTTP client for OpenAI-compatible `/chat/completions`
- **HandwritingSynthesis.swift** — Zhang-Suen thinning + stroke tracing (port of `script.rs`)
- **DiaryCanvasView.swift** — Apple Pencil input via UIKit touches, draws directly to Surface
- **DiaryViewModel.swift** — State machine: Listening → Drinking → Thinking → Replying → Lingering → Fading

## Differences from Original

| | Original (reMarkable) | iPad Port |
|---|---|---|
| Platform | reMarkable Paper Pro | iPad + Apple Pencil |
| Language | Rust | Swift / SwiftUI |
| Pen Input | evdev `/dev/input/eventN` | UIKit `UITouch` (`.pencil` type only) |
| Display | Linux framebuffer | UIView `draw()` + Surface bitmap |
| API Config | `oracle.env` file | In-app Settings screen |
| LLM | OpenAI-compatible (env vars) | OpenAI-compatible (UserDefaults) |

## Credits

- Original project: [MaximeRivest/riddle](https://github.com/MaximeRivest/riddle)
- Persona prompt, state machine, handwriting synthesis, and dissolve algorithm are direct ports from the original Rust source

## License

Same license as the original project.
