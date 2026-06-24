# App icons

The shipped apps already carry their icon — macOS bakes it into the `.app` at
bundle time and Windows embeds it into the `.exe` at link time, so end users do
nothing. This doc is maintainer reference for regenerating the icon when the
source art changes.

The canonical app icon is `assets/icon.png` (1024×1024). Platform variants are
generated from it and live alongside:

| Platform | File(s) | Wired by |
|----------|---------|----------|
| macOS    | `assets/icon.icns`              | `[package.metadata.bundle]` in `Cargo.toml` (read by `cargo bundle`) |
| Windows  | `assets/icon.ico` + `assets/notepad-studio.rc` | `build.rs` via [`embed-resource`](https://crates.io/crates/embed-resource); embedded into `notepad-studio.exe` at link time |
| Linux    | `assets/linux/icon_*.png` + `assets/notepad-studio.desktop` | Manual install (see below) — Wayland/X11 WM class is `notepad-studio`, set via GPUI's `app_id` |

## Regenerating from a new source PNG (macOS host)

```sh
# .icns (multi-resolution iconset)
sips -z 16  16   assets/icon.png --out assets/icon.iconset/icon_16x16.png
sips -z 32  32   assets/icon.png --out assets/icon.iconset/icon_16x16@2x.png
sips -z 32  32   assets/icon.png --out assets/icon.iconset/icon_32x32.png
sips -z 64  64   assets/icon.png --out assets/icon.iconset/icon_32x32@2x.png
sips -z 128 128  assets/icon.png --out assets/icon.iconset/icon_128x128.png
sips -z 256 256  assets/icon.png --out assets/icon.iconset/icon_128x128@2x.png
sips -z 256 256  assets/icon.png --out assets/icon.iconset/icon_256x256.png
sips -z 512 512  assets/icon.png --out assets/icon.iconset/icon_256x256@2x.png
sips -z 512 512  assets/icon.png --out assets/icon.iconset/icon_512x512.png
cp assets/icon.png assets/icon.iconset/icon_512x512@2x.png
iconutil -c icns assets/icon.iconset -o assets/icon.icns

# .ico (Windows; needs ImageMagick)
magick assets/icon.png -define icon:auto-resize=16,32,48,64,128,256 assets/icon.ico

# Linux PNG sizes
for s in 16 32 48 64 128 256 512; do
  sips -z $s $s assets/icon.png --out assets/linux/icon_${s}.png
done
```

## Linux install (per-user)

Linux has no self-contained app bundle, so the icon is installed into the
system icon theme rather than carried by the binary:

```sh
for s in 16 32 48 64 128 256 512; do
  xdg-icon-resource install --novendor --size $s assets/linux/icon_${s}.png notepad-studio
done
xdg-desktop-menu install --novendor assets/notepad-studio.desktop
```

## Dev-mode dock icon caveat

`cargo run` produces a bare binary, not a macOS `.app` bundle. The Dock falls
back to a generic terminal icon until you run `cargo bundle` (release
distribution) or until GPUI gains a runtime window-icon API. The Windows `.exe`
icon embeds at link time, so dev `cargo build` already shows the correct icon
in Explorer / Alt-Tab. Linux icon visibility depends on the WM picking up the
`.desktop` file via the install commands above.
