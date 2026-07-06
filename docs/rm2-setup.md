# riddle on the reMarkable 2 — setup from zero

The rM2 needs no "developer mode": SSH as root is built into every unit.
You need: the tablet, its USB-C cable, and ~15 minutes.

## Quick path (one command)

1. On the tablet, read the root password: **Settings → Help → Copyrights and
   licenses → GPLv3 Compliance** (bottom of the page; the IP shown is
   `10.11.99.1`). Note the password somewhere safe.
2. Plug the tablet in over USB.
3. Build and install everything:

```sh
rustup target add armv7-unknown-linux-musleabihf   # once; needs zig + cargo-zigbuild
cd riddle && ./build-rm2.sh && cd ..
./scripts/install-rm2.sh
```

The installer connects over SSH (asking for that password once, then installing
your key), confirms the device is an rM2, installs
[xovi](https://github.com/asivery/xovi) +
[AppLoad](https://github.com/asivery/rm-appload) from their official arm32
releases, adds power-button persistence via
[xovi-tripletap](https://github.com/rmitchellscott/xovi-tripletap)
(triple-press = toggle xovi), copies the riddle bundle, prompts for your API
key, and verifies the oracle end-to-end.

Then on the tablet: open **AppLoad → The Diary**, write, rest the pen ~3 s.

> ⚠️ Everything here is reversible (`ssh root@10.11.99.1
> /home/root/xovi/stock` or a reboot returns the stock UI), but reMarkable OS
> updates can remove xovi/AppLoad/riddle — reinstallable by re-running the
> installer. Keep the SSH password: it is your escape hatch.

### If SSH won't connect

Two rM2 SSH quirks, both handled by the installer for its own connections.
For your own `ssh`/`scp` sessions:

- **`Connection closed by 10.11.99.1 port 22`** — the rM2's dropbear (OS 3.x)
  has a broken RSA host-key path: it hangs up whenever RSA is negotiated,
  though ed25519 works fine. A stale `ssh-rsa` entry for `10.11.99.1` in your
  `~/.ssh/known_hosts` (from an older device or firmware) forces your client
  to request RSA and triggers exactly this. Fix:
  `ssh-keygen -R 10.11.99.1`, then connect again.
- **`no matching host key type found. Their offer: ssh-rsa`** — older firmware
  offers only legacy `ssh-rsa`, which modern OpenSSH refuses by default.

This `~/.ssh/config` block covers both:

```
Host remarkable rm2 10.11.99.1
  HostName 10.11.99.1
  User root
  HostKeyAlgorithms ssh-ed25519,ssh-rsa
  PubkeyAcceptedAlgorithms +ssh-rsa
```

## The oracle key

Any OpenAI-compatible, vision-capable endpoint works. The installer writes
`oracle.env` for you; to change it later, edit
`/home/root/xovi/exthome/appload/riddle/oracle.env` on the tablet. OpenRouter
example:

```sh
RIDDLE_OPENAI_KEY=sk-or-v1-...
RIDDLE_OPENAI_BASE=https://openrouter.ai/api/v1
RIDDLE_OPENAI_MODEL=openai/gpt-4o-mini
```

Test it any time (tablet must be on Wi-Fi — USB gives it no internet):

```sh
ssh rm2 'cd /home/root/xovi/exthome/appload/riddle && \
  set -a && . ./oracle.env && set +a && ./riddle --oracle-test icon.png'
```

## Manual path (what the installer does, step by step)

If you prefer to run each step yourself:

1. **xovi** — grab `xovi-arm32.tar.gz` from
   [rm-xovi-extensions releases](https://github.com/asivery/rm-xovi-extensions/releases/latest)
   (it contains the loader, start/stop scripts, and qt-resource-rebuilder), and
   extract on the tablet: `tar -xzf xovi.tar.gz -C /home/root`.
2. **AppLoad** — grab `appload-arm32.zip` from
   [rm-appload releases](https://github.com/asivery/rm-appload/releases/latest).
   `appload.so` goes to `/home/root/xovi/extensions.d/`; the `shims/` folder
   goes to `/home/root/xovi/exthome/appload/shims/` (not extensions.d).
3. **Persistence** — on the tablet:
   `wget -qO- https://raw.githubusercontent.com/rmitchellscott/xovi-tripletap/main/install.sh | bash`
4. **riddle** — `scp -O -r dist/rm2/riddle root@10.11.99.1:/home/root/xovi/exthome/appload/`,
   then create `oracle.env` in that folder (see above).
5. **Start** — `/home/root/xovi/start` on the tablet (or triple-press power).

## Troubleshooting

- **Ink lands in the wrong place / mirrored** — the raw digitizer transform is
  off for your unit. The qtfb pen fallback (used automatically when the raw
  device can't be opened) is always correctly mapped — compare against it and
  open an issue with what you see.
- **"qtfb server rejected init"** — AppLoad missing or old; re-run the
  installer, then Reload in AppLoad.
- **No reply, ink blot pulses forever** — oracle problem: re-run
  `--oracle-test`; check Wi-Fi, key, and that the model supports images.
- **Tablet acting up** — `ssh rm2 'systemctl restart xochitl'` restores the
  stock UI; worst case hold power ~10 s to reboot. riddle in windowed mode
  never stops xochitl, so the blast radius is small.
