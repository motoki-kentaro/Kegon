# Kegon application icon asset set

This directory contains the application icon asset set for Kegon across supported desktop platforms.

## Provenance

The Kegon application icon was generated specifically for Kegon using Gemini, and the project owner adopted it as Kegon's official application icon. It is not derived from Codicons or any other third-party icon set.

The project owner permits redistribution of the icon with this repository and with Kegon distributions.

The canonical raster source is `png/icon_1024x1024.png`; every other file in this directory is derived from it.

## Canonical source / Master artwork

- `png/icon_1024x1024.png`: Canonical source master asset (1024x1024 transparent PNG).

## Platform assets & Roles

| Path | Format / Description | Role / Usage |
|---|---|---|
| `kegon.ico` | Windows ICO (16, 24, 32, 48, 64, 128, 256 px) | Formal Windows app icon. Used by `build.rs` (PE binary resource) and `src/app_icon.rs` (runtime window title bar, taskbar, Alt+Tab). |
| `windows/kegon.ico` | Windows ICO | Windows platform asset directory copy of `kegon.ico`. |
| `macos/kegon.icns` | macOS ICNS | Apple icon format. Reserved for future macOS application bundling. |
| `linux_hicolor/` | PNG icon theme directory structure | Freedesktop hicolor icon theme hierarchy (16x16 to 512x512). Reserved for future Linux packaging (.desktop, AppImage, deb, rpm). |
| `png/` | Transparent PNG variants | Pre-rendered transparent PNG assets in sizes 16x16 through 1024x1024. |
| `png_solid/` | Solid background PNG variants | Pre-rendered solid square background PNG assets in sizes 16x16 through 1024x1024. |

## Legacy assets & tools/icon-gen

The previous SVG-based generation workflow (`kegon.svg`, `kegon-app-icon.svg`, and `tools/icon-gen`) is obsolete and removed. Pre-rendered platform assets are now maintained directly in this repository.

## Note on Windows icon cache

Windows Explorer and taskbar may cache executable icons. If the icon does not refresh immediately in Explorer, restarting `explorer.exe` or clearing the Windows shell icon cache may be required.

## Updating the application icon

To update the Kegon application icon in the future:

1. Replace the canonical source asset `png/icon_1024x1024.png`.
2. Generate/update all derived PNG variants (`png/` and `png_solid/`).
3. Generate/update platform assets (`kegon.ico` / `windows/kegon.ico`, `macos/kegon.icns`, and `linux_hicolor/`).
4. Ensure `kegon.ico` contains all required icon sizes (16x16, 24x24, 32x32, 48x48, 64x64, 128x128, 256x256).
5. Run tests and verify the build (`cargo test`, `cargo build --release`).
