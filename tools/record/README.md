# Recording the feature clips

The clips in [docs/FEATURES.md](../../docs/FEATURES.md), and the README's tour and
screenshots, are recorded from a running C1 Studio by scripts, so they can be recorded
again when the app changes, or encoded as MP4 instead of GIF.

## Needs

- a release build: `cd app; pnpm tauri build --no-bundle` (`target\release\c1studio.exe`);
- Node 22 or later (for its built-in `WebSocket`), Python with Pillow;
- for MP4 only, ffmpeg on PATH: `winget install Gyan.FFmpeg`.

## Record

```powershell
tools\record\record.ps1                    # every clip → docs\media\features\<name>.gif
tools\record\record.ps1 -Only map,search   # just these
tools\record\record.ps1 -Mp4               # MP4 as well as GIF
tools\record\record.ps1 -FromFrames -Mp4   # encode the last recording again, without recording
```

`record.ps1` starts the recording instance (`start.ps1`) when it isn't running, runs each
scenario in `scenarios\`, keeps its frames in `%TEMP%\c1studio-record\frames\<name>\`
(`frame-NNNN.jpg` and `times.json`, when each frame was shown), and encodes them:
`assemble.py` to a GIF (960 px, 10 fps, one palette per clip), `to_mp4.py` to H.264 with
each frame held as long as it was on screen. `sheet.py in.gif out.png` makes a contact
sheet for checking a clip. The `demo` scenario is the README's tour: it goes to
`docs\media\demo.gif`, at 3 frames a second.

## Screenshots

```powershell
node tools\record\drive.mjs tools\record\stills.mjs   # docs\media\*.jpg, from the running instance
```

`stills.mjs` takes the README's screenshots, 1440×900 without the pointer. Its last one,
`folders.jpg`, opens the photo folders source the `folders` scenario made, so record that
one first.

## Photo folders

The `folders` scenario shows a photo folders source. The first time, it makes one on
`$C1_RECORD_FOLDERS` (default `G:\My Drive\foto`), named *Photos on Drive*, in the
instance's own data. It opens that source off camera (the window reloads), and goes back
to the catalog at the end. The first scan reads every file, and the thumbnails are made in
the background while the source is open: on a cloud drive, open it once and let them
finish before recording. Nothing is written to the folders (XMP export stays off).

## The recording instance

`start.ps1` copies the newest backup of the fuji catalog (from
`%LOCALAPPDATA%\c1studio\fiuji-*\work\backups\`) into `%TEMP%\c1studio-record\cat`, adds
a junction to the live catalog's `Cache` (Capture One's thumbnails and previews), and
starts the app on the copy with its own index, settings and WebView data, and the
DevTools protocol on port 9223. So:

- the live catalog is never opened; edits the scenarios make go to the copy;
- the instance is separate from the one you use, which can keep running;
- the work dir is kept: the first start indexes and measures the whole catalog (minutes),
  later ones only sync. `start.ps1 -NewCopy` takes the newest backup again.

To delete the work dir, remove the junction first, so nothing follows it into the live
`Cache`: `cmd /c rmdir %TEMP%\c1studio-record\cat\Cache`, then delete the folder.

## Writing a scenario

A scenario is a module in `scenarios\` whose default export is `{ name, title, run(d) }`.
`run` resets the app (`d.reset()`), sets the scene, and wraps what the clip shows in
`d.record(async () => …)`. `drive.mjs` gives `d`:

| | |
| --- | --- |
| `click(selector, text?)`, `rightClick(…)`, `clickAt(x, y)`, `move(x, y)`, `drag(x0, y0, x1, y1)`, `wheel(x, y, dy)` | the mouse, gliding so it reads in the clip |
| `type(text)`, `key(name, { modifiers })` | the keyboard (modifiers: 1 Alt, 2 Ctrl, 8 Shift) |
| `press(selector, text?)`, `choose(selector, value)` | click through the DOM, set a `<select>`: for setting a scene, or a native list the clip can't show |
| `locate(selector, text?)` | the centre of the first visible match, or null |
| `waitFor(selector, text?)`, `settle()` | wait for an element; wait for the images on screen to load |
| `cut(async () => …)` | leave a wait out of the clip, such as the backup before a write |
| `caption(text)` | a caption band at the bottom until the next one (`""` hides it) |
| `reset({ timeline })` | the whole library, Photos view, details on, nothing open |
| `invoke(command, args)` | one of the app's commands, as the page calls it |
| `open(spec)` | open another source, off camera: waits until it is read and shown, and draws the overlay again |
| `evaluate(js)`, `send(method, params)`, `shot(file)` | anything else over CDP |

Scenarios that edit undo their edits at the end, so the copy stays as it was (`hidden`
shows again what it hid). Only `history` writes, to the copy. `reset()` also puts the
sort back to *Date taken, newest first* and turns *Stack* and *Hidden* off, so each clip
starts from the same view.

`text` is a substring, or a RegExp tested against the element's trimmed text. Run a
scenario without recording while writing it: `node tools\record\drive.mjs
tools\record\scenarios\map.mjs`. The page is fixed at 1440×900 whatever the window's size,
and `overlay.js` draws the pointer (the screencast has none) and the captions.
