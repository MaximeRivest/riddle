# Use your ChatGPT plan

The optional `riddle-chatgpt` companion adds the official **Sign in with ChatGPT** flow. It registers Riddle as an open-source app and requests permission to use your ChatGPT plan. It does not reuse Codex credentials or require an OpenAI API key. Account eligibility and model availability come from OpenAI; request usage counts toward your ChatGPT plan's app limits.

The one-time browser sign-in runs on your computer. The companion then runs on the tablet, refreshes its own credentials, and streams replies to the diary through a loopback connection. The computer is not needed while writing.

## Build and install

Install Go 1.25 or newer on your computer and run:

```sh
./scripts/build-chatgpt.sh
```

This tests the companion and builds a tablet binary plus a native sign-in helper. If you build a new AppLoad bundle afterward, `scripts/make-bundle.sh` includes the companion automatically. To add it to an existing Riddle installation:

Close The Diary and stop its takeover service before updating. Back up your existing Riddle folder before running the copy commands below. The companion does not fix display compatibility; use a display library that supports your firmware.

```sh
scp -O tools/chatgpt/dist/riddle-chatgpt-linux-arm64 root@10.11.99.1:/home/root/xovi/exthome/appload/riddle/riddle-chatgpt
scp -O scripts/riddle-takeover.sh root@10.11.99.1:/home/root/xovi/exthome/appload/riddle/riddle-takeover.sh.new
ssh root@10.11.99.1 'chmod 755 /home/root/xovi/exthome/appload/riddle/riddle-takeover.sh.new; mv /home/root/xovi/exthome/appload/riddle/riddle-takeover.sh.new /home/root/xovi/exthome/appload/riddle/riddle-takeover.sh'
ssh root@10.11.99.1 'chmod 755 /home/root/xovi/exthome/appload/riddle/riddle-chatgpt'
```

## Continue with ChatGPT

Create a private directory on your computer for this tablet's registration, outside the repository:

```sh
mkdir -m 700 "$HOME/.config/riddle-chatgpt-tablet"
tools/chatgpt/dist/riddle-chatgpt-login login --state-dir "$HOME/.config/riddle-chatgpt-tablet"
```

Open the printed **Continue with ChatGPT** link in the same computer's browser. Sign in, review the requested permissions, and enable ChatGPT plan usage. The helper validates the returned identity before saving tokens. Do not share the credentials or commit them to Git.

Copy this tablet's private registration to its app folder:

```sh
ssh root@10.11.99.1 'umask 077; mkdir -p /home/root/xovi/exthome/appload/riddle/chatgpt-auth'
scp -O "$HOME/.config/riddle-chatgpt-tablet/host-id" "$HOME/.config/riddle-chatgpt-tablet/credentials.json" root@10.11.99.1:/home/root/xovi/exthome/appload/riddle/chatgpt-auth/
ssh root@10.11.99.1 'chmod 700 /home/root/xovi/exthome/appload/riddle/chatgpt-auth; chmod 600 /home/root/xovi/exthome/appload/riddle/chatgpt-auth/*'
```

Add the following to the tablet's `oracle.env` beside the Riddle binary:

```sh
RIDDLE_CHATGPT=1
```

Close and reopen **The Diary**. The launcher starts and stops the companion with the app. The diary keeps its usual handwriting and memory behavior.

The companion selects the first visible model in your account's current catalog by default. To select another model, set `RIDDLE_CHATGPT_MODEL` to its catalog slug in `oracle.env`. The model must support images. A completed request confirms access; merely appearing in a catalog does not.

## Reauthorize or disconnect

The tablet holds the current rotating refresh token. Before signing in again on the computer, close the diary and copy the tablet's current `chatgpt-auth` folder back to your private computer directory. Reuse the saved host ID and issued client ID; do not create a new registration for every launch. Repeat the sign-in and upload steps afterward.

To use your previous API-key or pi backend again, remove `RIDDLE_CHATGPT=1` and reopen the diary. To disconnect completely, revoke Riddle in ChatGPT settings and remove its private `chatgpt-auth` directories from the tablet and computer.

## Implementation

OAuth uses PKCE, state and nonce validation, and signature verification through OpenAI's published JWKS. Credentials are stored atomically with owner-only permissions. The companion calls the public Responses API with `store=false` and `stream=true`; it does not execute model tools. Only the companion handles OAuth tokens. Riddle receives a separate local token that grants access to the loopback relay.

Official references: [registration and sign-in](https://developers.openai.com/siwc/token-sharing-open-source/sign-in), [models and inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference).
