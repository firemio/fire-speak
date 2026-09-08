# packaging/

Distribution packaging that lives outside the Tauri bundler.

| Path | Status | What it is |
|---|---|---|
| `flatpak/jp.firemio.fire-speak.yml` | **Phase 2 — draft, does not build yet** | flatpak-builder manifest |
| `flatpak/jp.firemio.fire-speak.metainfo.xml` | **Phase 2 — draft** | AppStream metadata required by Flathub |

Everything the release workflow actually ships today (`.exe`, `.msi`, `.deb`,
`.rpm`, `.AppImage`) is produced by the Tauri bundler and configured in
`src-tauri/tauri.conf.json` → `bundle`. Nothing in this directory is wired into
`.github/workflows/release.yml`, and it must not be until the blockers below are
resolved.

---

## Why Flatpak at all

RustDesk ships a `.flatpak` alongside its `.deb`/`.rpm`/AppImage, which covers
distros whose system webkit2gtk is too old or too new for a
distro-package build. The goal here is the same: one extra release asset, and
eventually a Flathub listing.

## Building it locally

Prerequisites:

```sh
sudo apt install flatpak flatpak-builder   # or: sudo dnf install flatpak flatpak-builder
flatpak remote-add --if-not-exists --user flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user flathub org.gnome.Platform//48 org.gnome.Sdk//48
flatpak install --user flathub org.freedesktop.Sdk.Extension.node20//24.08
flatpak install --user flathub org.freedesktop.Sdk.Extension.rust-stable//24.08
```

> Verify the runtime version before running the above — `runtime-version` in the
> manifest is pinned to GNOME `48` and the SDK extension branch (`24.08`) must
> match whatever base that GNOME runtime is built on. `flatpak search
> org.freedesktop.Sdk.Extension.node` lists the Node extensions that actually
> exist for your Flathub remote; bump `node20` to a newer one if available and
> update `build-options.append-path` to match.

flatpak-builder builds with **no network access**, so every crate and every npm
package has to be materialised into the manifest first:

```sh
git clone https://github.com/flatpak/flatpak-builder-tools.git
pipx install ./flatpak-builder-tools/node/flatpak_node_generator

# npm dependencies  ->  node-sources.json
flatpak-node-generator --no-requests-cache -o packaging/flatpak/node-sources.json npm package-lock.json

# cargo dependencies -> cargo-sources.json
python3 flatpak-builder-tools/cargo/flatpak-cargo-generator.py \
  -o packaging/flatpak/cargo-sources.json src-tauri/Cargo.lock

flatpak-builder --user --install --force-clean \
  build-dir packaging/flatpak/jp.firemio.fire-speak.yml

flatpak run jp.firemio.fire-speak
```

To produce the single-file release asset:

```sh
flatpak build-bundle ~/.local/share/flatpak/repo \
  fire-speak.flatpak jp.firemio.fire-speak
```

---

## What still needs solving before this can ship

### 1. There is no `package-lock.json` in the repo

`flatpak-node-generator ... npm package-lock.json` needs one, and the manifest
runs `npm ci --offline`, which also needs one. Right now the repo has no npm
lockfile at all (`.github/workflows/*.yml` therefore use `npm install`). Commit a
lockfile first; that unblocks Flatpak *and* lets CI switch to `npm ci` with
dependency caching.

### 2. Vendored source manifests are not checked in

`cargo-sources.json` is several thousand lines and both files are release-specific,
so they are generated, not committed. That means the Flatpak build is not
reproducible from a clean checkout without running the two generator commands
above. Before wiring Flatpak into CI, decide one of:

- generate both files in a workflow step (needs network *before* flatpak-builder
  starts, which is fine — only the builder itself is offline), or
- commit them and refresh them with a bot on every dependency bump.

### 3. `finish-args` are best-effort and unverified at runtime

The permissions in the manifest are reasoned from what the app does, not from a
successful sandboxed run:

- `--socket=x11` + `--share=ipc` — required for the window, for the global
  hotkey, and for synthesising `Ctrl+V`. Deliberately **not**
  `--socket=fallback-x11`: fallback-x11 lets GTK pick native Wayland when
  available, and neither global hotkey capture nor input injection works there.
- `--socket=pulseaudio` — microphone capture via cpal. This is the one to check
  first if recording silently produces nothing inside the sandbox.
- `--share=network` — cloud STT/LLM calls, the GitHub update check, model
  downloads, and the loopback connection to the local whisper-server.
- `--talk-name=org.kde.StatusNotifierWatcher` — the tray icon. Whether
  libayatana-appindicator can *own* its `org.kde.StatusNotifierItem-PID-N` name
  through Flatpak's session bus proxy has not been tested; if the tray never
  appears, this is the first thing to look at.

Clipboard read/write needs no permission (it goes through the compositor/X11
connection already granted), but that has not been confirmed under the sandbox
either.

### 4. Local speech-to-text inside the sandbox is an open question

The app downloads a whisper.cpp server binary at runtime and executes it. Inside
Flatpak that binary lands in `~/.var/app/jp.firemio.fire-speak/data/`, is linked
against the host's glibc rather than the runtime's, and may simply not run. The
realistic options are to bundle whisper.cpp as an extra module in this manifest,
or to ship the Flatpak as cloud-STT-only. Note that as of this writing the
downloader in `src-tauri/src/setup.rs` fetches the Windows `bin-x64` asset, so
local STT needs Linux support in the backend before this question even becomes
answerable.

### 5. Flathub-specific requirements

- The app ID `jp.firemio.fire-speak` implies control of the `firemio.jp` domain.
  If that domain is not owned, Flathub will require `io.github.firemio.*` instead
  (and the manifest, metainfo, desktop file and icon names all have to follow).
- `metainfo.xml` needs a real `<screenshots>` block with hosted images and must
  pass `appstreamcli validate`.
- The repository has no `LICENSE` file; `metadata_license` / `project_license` in
  the metainfo are placeholders until it does.
