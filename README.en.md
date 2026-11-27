# InkPop

[中文](README.md) · English

InkPop is built on [PhotoCraft v0.5.0](https://github.com/storytold/photocraft/releases/tag/v0.5.0), released on October 8, 2026. Development of InkPop began the following day.

InkPop is a Windows painting and image editor with familiar Photoshop workflows, from sketching and layer compositing to Camera Raw adjustments and PSD editing.

**Bring your brushes.** Import Photoshop ABR brushes and TPL painting tool presets with supported sampled tips, textures, dual brushes, spacing and mixing settings. Selecting large presets is more responsive, and strokes made with long, narrow tips finish sooner.

**Mixer Brush.** We compared strokes from the same wet-paint preset in Photoshop and InkPop to refine how the brush picks up and carries color, blends on a return stroke and tapers into transparency.

**Make the workspace yours.** Choose from four palettes in the Studio Light theme, adjust title bar, menu and panel colors, and export your palette to share it. Detach or dock panels and save the arrangement as a named workspace.

## Run on Windows

Download the Windows x64 portable ZIP from this repository’s Releases page. Extract the entire archive and run InkPop.exe. For pen pressure, manually add that executable to the application settings in your tablet driver, then start InkPop and test a brush stroke.

## Source and use

Buildable source is included. Artists may use InkPop for paid commissions, commercial illustration and the sale of their own work. Software containing contributions covered by the [InkPop distribution terms](LICENSE-InkPop.md) may be shared free of charge. Sale and paid redistribution are prohibited. Upstream PhotoCraft code is licensed under MIT or Apache-2.0. Third-party components, models and assets retain their respective terms. See [attribution](ATTRIBUTION.md) and [notices](NOTICE).
## Build on Windows

Install Rust 1.95 or newer with the MSVC toolchain, Visual Studio C++ Build Tools and a Windows SDK. From the directory containing the workspace Cargo.toml, run

```powershell
cargo build --locked --release -p photocraft -p photocraft-cli
.\target\release\photocraft.exe
```

The checkout includes the patched windowing crates, UI assets, translations, fonts and their license notices. Optional local segmentation needs the model weights and inference runtime from the matching Windows portable release. Keep its models, native libraries and notices together. InkPop reads the current Windows user's preferences and creates defaults on first use.

## Assemble the portable release

Extract the InkPop v121 Windows portable release to a directory such as `runtime`. From the source checkout, use Python 3.11 or newer to combine the freshly built desktop and CLI executables with the verified runtime files

```powershell
python packaging/windows/build-public-zip.py --release-dir target/release --runtime-dir runtime --output InkPop-Windows-x64.zip
```

The packager checks runtime files against the SHA-256 manifest. It copies only the named shipping files. User preferences, recent files, custom presets and crash logs belong to the local user profile and installation.

[Download the InkPop poster](docs/media/InkPop-v121-poster-v1.png)
