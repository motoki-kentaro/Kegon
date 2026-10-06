# Kegon icon

| File | Role |
|---|---|
| `kegon.svg` | Master artwork. Edit this file. |
| `kegon-app-icon.svg` | Generated. The artwork on a light rounded plate, as used for the app icon. |
| `kegon.ico` | Generated. Windows icon: 16, 24, 32, 48, 64, 128, and 256 px. |

The master artwork is a near-black silhouette on a transparent background,
which is almost invisible on dark taskbars and in dark-mode Explorer. The app
icon therefore puts it on a light rounded plate, cropped close to the artwork.
Sizes up to 32 px use less padding, so the artwork keeps as many pixels as
possible.

`kegon.ico` is used in two places:

- `build.rs` embeds it in the Windows executable, for Explorer.
- `src/app_icon.rs` sets one of its sizes as the window icon, for the title
  bar, taskbar, and Alt+Tab.

## Regenerating

After changing `kegon.svg`, run this from the repository root:

```sh
cargo run --manifest-path tools/icon-gen/Cargo.toml
```

Commit the regenerated `kegon-app-icon.svg` and `kegon.ico` together with the
SVG change.
