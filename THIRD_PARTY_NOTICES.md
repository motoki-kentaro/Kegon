# Third-Party Notices

Kegon itself is licensed under the [MIT License](LICENSE). It bundles third-party assets and depends on third-party Rust crates that are distributed under their own licenses, listed below.

Versions match `Cargo.lock`. Copyright lines are quoted only where the crate's own license file states one. The complete license texts ship with each crate's source, which Cargo downloads from crates.io. This repository does not vendor dependency sources.

---

## Bundled assets

### Codicons

- **Files**: `assets/icons/codicons/` (see [its README](assets/icons/codicons/README.md) for the file list)
- **License**: Creative Commons Attribution 4.0 International (CC BY 4.0); full text in [`assets/icons/codicons/LICENSE`](assets/icons/codicons/LICENSE)
- **Copyright**: Microsoft Corporation and contributors
- **Source**: <https://github.com/microsoft/vscode-codicons>
- **Changes**: none; the SVG files are used as published. Kegon sets their color at runtime.

---

## Direct dependencies

Every direct dependency uses a permissive license. Crates offered under a choice of licenses are listed with all of their options.

### alacritty_terminal

- **Version**: 0.26.0
- **License**: Apache-2.0
- **Source**: <https://github.com/alacritty/alacritty>

### arboard

- **Version**: 3.6.1
- **License**: MIT OR Apache-2.0
- **Copyright**: Copyright (c) 2022 The Arboard contributors
- **Source**: <https://github.com/1Password/arboard>

### dirs

- **Version**: 5.0.1
- **License**: MIT OR Apache-2.0
- **Copyright**: Copyright (c) 2018-2019 dirs-rs contributors
- **Source**: <https://github.com/soc/dirs-rs>

### fluent-bundle

- **Version**: 0.16.0
- **License**: Apache-2.0 OR MIT
- **Copyright**: Copyright 2017 Mozilla
- **Source**: <https://github.com/projectfluent/fluent-rs>

### fontdb

- **Version**: 0.23.0
- **License**: MIT
- **Copyright**: Copyright (c) 2020 Yevhenii Reizner
- **Source**: <https://github.com/RazrFalcon/fontdb>

### iced

- **Version**: 0.14.0
- **License**: MIT
- **Copyright**: Copyright 2019 Héctor Ramón, Iced contributors
- **Source**: <https://github.com/iced-rs/iced>

### ico

- **Version**: 0.4.0
- **License**: MIT
- **Copyright**: Copyright (c) 2018 Matthew D. Steele
- **Source**: <https://github.com/mdsteele/rust-ico>

### serde

- **Version**: 1.0.229
- **License**: MIT OR Apache-2.0
- **Source**: <https://github.com/serde-rs/serde>

### sys-locale

- **Version**: 0.3.2
- **License**: MIT OR Apache-2.0
- **Copyright**: Copyright (c) 2021 1Password
- **Source**: <https://github.com/1Password/sys-locale>

### toml

- **Version**: 0.8.23
- **License**: MIT OR Apache-2.0
- **Copyright**: Copyright (c) Individual contributors
- **Source**: <https://github.com/toml-rs/toml>

### ttf-parser

- **Version**: 0.25.1
- **License**: MIT OR Apache-2.0
- **Copyright**: Copyright (c) 2018 Yevhenii Reizner
- **Source**: <https://github.com/harfbuzz/ttf-parser>

### unic-langid

- **Version**: 0.9.6
- **License**: MIT OR Apache-2.0
- **Source**: <https://github.com/zbraniecki/unic-locale>

---

## Build and development dependencies

These are used only to build or test Kegon and are not part of the distributed program.

### winresource (build)

- **Version**: 0.1.31
- **License**: MIT
- **Copyright**: Copyright 2016 Max Resch
- **Source**: <https://github.com/BenjaminRi/winresource>

### fluent-syntax (tests)

- **Version**: 0.12.0
- **License**: Apache-2.0 OR MIT
- **Copyright**: Copyright 2017 Mozilla
- **Source**: <https://github.com/projectfluent/fluent-rs>

---

## Transitive dependencies

The full dependency graph (`cargo metadata`) contains about 480 crates. All of them are available under permissive licenses (MIT, Apache-2.0, BSD, ISC, Zlib, BSL-1.0, Unicode-3.0, CC0, Unlicense, or a choice among these). A few crates offer GPL or LGPL only as one alternative among permissive options. The single MPL-2.0 crate (`option-ext`, used unmodified through `dirs`) is a file-level license.

No direct or transitive dependency compiled into Kegon ships an Apache-2.0 `NOTICE` file. People who distribute Kegon binaries should include the license texts of the crates they ship. `cargo metadata` or a license tool such as `cargo-about` can collect them.
