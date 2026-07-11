# riddle for reMarkable 2 — no XOVI or AppLoad

This build runs beside the stock reMarkable UI on RM2 firmware 3.28. It reads
real Wacom strokes without grabbing the device, sends a private reconstruction
of the writing to the configured vision model, and replays the reply through
the Wacom input device. xochitl records Tom's handwriting in the open notebook
as ordinary ink.

## Install

Copy the extracted `riddle-rm2` directory to `/home/root/riddle-rm2`, then:

```sh
cd /home/root/riddle-rm2
cp oracle.env.example oracle.env
vi oracle.env                 # set RIDDLE_OPENAI_KEY
chmod +x riddle-rm2 start-rm2.sh stop-rm2.sh
```

No root filesystem files, startup hooks, XOVI, AppLoad, or systemd units are
installed.

## Use

Open a blank notebook in the stock UI and write in its upper third. From SSH:

```sh
/home/root/riddle-rm2/start-rm2.sh
```

After a 2.8 second pause, Tom writes back through xochitl. Stop it before using
other parts of the UI:

```sh
/home/root/riddle-rm2/stop-rm2.sh
```

This companion build intentionally does not erase/fade existing notebook ink
and disables the takeover-only help and memory-page animations. The original
notebook remains a normal reMarkable document and syncs normally.

## Remove

```sh
/home/root/riddle-rm2/stop-rm2.sh
rm -rf /home/root/riddle-rm2 /home/root/riddle-data
```

Rebooting also stops it because no autostart component is installed.
