# riddle on the reMarkable 2 — setup from zero

The rM2 needs no "developer mode": SSH as root is built into every unit.
You need: the tablet, its USB-C cable, and ~20 minutes.

## 1. Get SSH access

1. On the tablet: **Settings → Help → Copyrights and licenses → GPLv3 Compliance**.
   The password and IP (`10.11.99.1`) are shown at the bottom.
2. Plug the tablet into your computer over USB.
3. `ssh root@10.11.99.1` — enter that password.
4. Recommended: install your key so future steps don't prompt:
   `ssh-copy-id root@10.11.99.1`

> **Keep SSH working.** It is the escape hatch for everything below. Note the
> password somewhere safe. reMarkable OS updates can remove third-party
> software (xovi, AppLoad, riddle) — they are reinstallable, but keep this in
> mind before accepting an update.

## 2. Install xovi + AppLoad

riddle runs as an [AppLoad](https://github.com/asivery/rm-appload) app inside
xochitl via the [xovi](https://github.com/asivery/xovi) extension framework.
The maintained path is asivery's installer — follow the AppLoad README for the
one-command install appropriate to your OS version. After install, a launcher
chip appears in the tablet's UI; tap it to see the AppLoad screen.

## 3. Build and install riddle (from a computer)

```sh
rustup target add armv7-unknown-linux-musleabihf
cd riddle && ./build-rm2.sh
scp -O -r dist/rm2/riddle root@10.11.99.1:/home/root/xovi/exthome/appload/
```

## 4. Add the oracle key

```sh
ssh root@10.11.99.1
cd /home/root/xovi/exthome/appload/riddle
cp oracle.env.example oracle.env
vi oracle.env
```

For OpenRouter:

```sh
RIDDLE_OPENAI_KEY=sk-or-v1-...
RIDDLE_OPENAI_BASE=https://openrouter.ai/api/v1
RIDDLE_OPENAI_MODEL=openai/gpt-4o-mini    # any vision-capable model
```

Verify before ever opening the diary (prints the streamed reply, no display
needed) — run on the tablet:

```sh
cd /home/root/xovi/exthome/appload/riddle
set -a; . ./oracle.env; set +a
./riddle --oracle-test path/to/any-handwriting.png
```

The tablet must be on Wi-Fi (USB alone gives the tablet no internet).

## 5. Run

AppLoad → **Reload** → **The Diary**. Write something, rest the pen ~3 s,
and watch the page drink your ink.

## Troubleshooting

- **Ink lands in the wrong place / mirrored** — the raw digitizer transform
  is off for your unit. Relaunch with the qtfb fallback to compare:
  temporarily rename the wacom grab away by launching `riddle` with the pen
  device busy, or report it upstream; the qtfb pen path (used automatically
  when the raw device can't be opened) is always correctly mapped.
- **"qtfb server rejected init"** — AppLoad missing or old; reinstall/update
  AppLoad, then Reload.
- **No reply, blot pulses forever** — oracle problem: re-run `--oracle-test`;
  check Wi-Fi, key, and that the model chosen supports images.
- **Tablet acting up after experiments** — `ssh root@10.11.99.1 'systemctl
  restart xochitl'` restores the stock UI; worst case a reboot (hold power
  ~10 s) returns everything to normal. riddle in windowed mode never stops
  xochitl, so the blast radius is small.
