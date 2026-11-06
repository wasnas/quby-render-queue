# Quby Render Queue 1.3

Batch render queue for Blender on Windows 10 / 11. Tested design target:
**Windows 11 + Blender 5.0+**, still compatible with Blender 3.x and 4.x.

---

## Building

```
cargo build --release
```

or just double-click `build_release.bat`.
The binary lands in `target\release\QubyRenderQueue.exe`.

**Important:** copy these files *over your existing 1.0 project folder* so that
`Cargo.lock` and `target\` are reused. The dependency list is **identical** to
1.0 (eframe/egui 0.27, serde, serde_json, rfd, winres), so the rebuild is fast
and needs no new crates downloaded.

Files in this archive:

```
Cargo.toml          version bumped to 1.3.0, dependencies unchanged
build.rs            unchanged
build_release.bat   unchanged
icon.png / icon.ico unchanged
src/main.rs         rewritten for 1.3
src/qrq_apply.py    NEW - embedded in the exe, applies the settings inside Blender
src/qrq_probe.py    NEW - embedded in the exe, asks Blender what it supports
```

The two Python files are embedded at compile time with `include_str!`, written
to `%APPDATA%\BlenderRenderQueue\` on first launch and handed to Blender with
`--python`. You do not ship them separately.

---

## What is new in 1.3

### Still images
Each job can be switched between **Animation** and **Single image**. In single
image mode you pick the frame; Blender renders exactly that one frame (using
`-s N -e N -a` so that retiming still applies correctly).

### Real file formats and codecs, read from your Blender
On first run (and whenever you press *Re-detect Blender features*) Quby launches
Blender once in the background and reads the **actual enum values** your build
exposes: file formats, colour modes and depths, EXR/TIFF codecs, FFMPEG
containers, video and audio codecs, CRF levels, encoding presets, render
engines, view transforms, looks, display devices, the full OCIO colour-space
list, denoisers and Cycles device types. Those lists then populate the dropdowns,
so you can never pick something your Blender does not have. If the probe fails,
a sensible built-in list is used instead.

Per format you get the right controls: PNG compression, JPEG/JPEG2000/WEBP
quality, EXR codec, TIFF codec, and for FFMPEG the container, video codec, audio
codec, quality (CRF) or a fixed bitrate, encoding speed, GOP size and audio
bitrate.

### Compositor / video editor only
A four-way switch: **Both**, **Compositor only**, **Video editor only**,
**Neither**. There is also **Skip the 3D render entirely**, which disables every
view layer so only the node tree or the timeline runs — ideal when your
compositor or VSE reads image sequences and movie files rather than Render
Layers.

### Resolution
One-click presets — 720p, 1080p, 1440p, 4K UHD, **DCI 4K (4096)**, 8K, square
1080 and vertical 1080×1920 — plus a free X/Y box and the percentage scale. The
final pixel size is shown live.

### Colour management
View transform, look, display device, exposure, gamma, output colour space and
sequencer colour space, all from the real OCIO config of your Blender.

### Image compression
PNG compression, JPEG/WEBP quality, EXR and TIFF codecs, plus the FFMPEG
quality/bitrate controls described above.

### Non-destructive fps change  ← the big one
Set any frame rate (presets from 12 to 120, including 23.976 / 29.97 / 59.94,
plus a custom value) and choose how timing is preserved:

* **Yes — Blender time remap** *(default)*
  Uses Blender's own `render.frame_map_old / frame_map_new`. Quby computes the
  exact integer ratio for you (24 → 60 becomes 2/5; 23.976 → 60 becomes
  199/498, an error of 0.0004 %). Nothing in your scene is touched, so this is
  the safest option and works for absolutely every kind of animation.

* **Yes — rescale keyframes**
  Deep retime: every f-curve (including Blender 4.4+/5.0 slotted actions), curve
  modifiers, drivers, NLA strips, grease pencil frames (legacy and GPv3),
  physics / particle / soft-body / rigid-body cache ranges, video-sequencer
  strips, timeline markers, the frame range and the motion-blur shutter are all
  scaled by the fps ratio. Use it when you need the retime baked into the
  session rather than remapped.

* **No — just change fps**
  Changes the number only; playback gets faster or slower. Quby warns you.

Either way the result is the same visual speed at the new frame rate, and
**your .blend file is never saved** — everything happens inside the background
render session.

### Other things 3D artists asked for
* **Drag & drop** .blend files straight onto the window, and multi-file add.
* **Scene / camera / view-layer / engine pickers** per job, read from the file.
* **Sampling overrides**: Cycles samples, adaptive noise threshold, denoise +
  denoiser, EEVEE samples, motion blur and transparent film (each as
  Keep / On / Off so you never clobber a setting by accident).
* **Performance**: persistent data, CPU thread count.
* **Frame step** — render every Nth frame for quick previews.
* **Resume a render**: turn off *Overwrite existing frames* and Blender skips
  frames already on disk. Placeholder files for multi-machine renders.
* **Burn metadata** (stamp) as Keep / On / Off.
* **Retries on failure**, 0–5 per job.
* **Presets**: save a whole settings block by name, re-apply it with one click,
  or push the current job's settings to every file in the queue.
* **Per-job enable / disable, duplicate, reorder, remove** via right-click.
* **Elapsed time and ETA** per frame, per job and for the whole queue.
* **Compact log** that filters Blender's per-sample spam, plus *Save log*.
* **Open output folder** button.
* **When finished**: do nothing / sleep / hibernate / shut down.
* Device list extended: Auto, CPU, CUDA, OptiX, HIP, oneAPI, Metal, and the
  CUDA+CPU / OptiX+CPU combinations.

### Fixes carried over from 1.0
* **STOP now really stops.** 1.0 only set a flag; the Blender process kept
  rendering in the background. Quby now stores the child PID and kills the whole
  process tree (`taskkill /T /F`).
* Reading a .blend's frame range no longer freezes the window — it happens on a
  background thread, and the probes are serialised so adding 20 files does not
  start 20 Blender processes at once.
* Old `config.json` and `queue.json` files keep loading: every struct uses
  `#[serde(default)]`, so 1.0 data is read and simply gains the new fields.

---

## How the overrides reach Blender

```
blender --background file.blend [-y] [-S scene] [-o out] -s A -e B
        --python qrq_apply.py -a -- --qrq-json job_<id>.json [--cycles-device X]
```

The settings are passed as a JSON file rather than on the command line, which
avoids every Windows quoting and command-length problem. The script applies
things in a safe order (scene → engine → resolution → colour → format → codec →
post-processing → sampling → output → **fps and retiming last**), each step in
its own `try/except`. If a setting does not exist in your Blender version the
script prints a `QRQ-WARN` line and carries on — it can never make a render
fail.

---

## Known limits, honestly

* The Rust code was written and reviewed by hand but **could not be compiled in
  the environment where it was prepared** (no Rust toolchain, no network there).
  Build it once locally; if the compiler complains about anything, the message
  will point straight at the line.
* The two Python scripts *were* executed and unit-tested (ratio maths, fps
  pairs, argument parsing).
* Delivered as `.zip` rather than `.rar` — no RAR compressor was available.
* `Cargo.lock` is deliberately **not** included so that your existing one, and
  therefore your existing `target\` cache, stay valid.
