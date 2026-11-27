# Attribution

Every non-code asset in this repository, with its author, source and license. PhotoCraft's own
code and original assets are MIT OR Apache-2.0 (see [`LICENSE-MIT`](LICENSE-MIT),
[`LICENSE-APACHE`](LICENSE-APACHE) and [`NOTICE`](NOTICE)). When you add an asset, add a row here
in the same change; third-party assets must be permissively licensed and keep their license file
next to them.

## Bundled in the app

| Path | Title | Author | Source | License |
|---|---|---|---|---|
| `crates/ui-egui/src/i18n/de.tsv` | German UI translations | PhotoCraft contributors | Original translations of PhotoCraft's English labels | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |
| `crates/ui-egui/src/i18n/it.tsv` | Italian UI translations | PhotoCraft contributors | Original translations of PhotoCraft's English labels | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |
| `crates/ui-egui/src/i18n/ko.tsv` | Korean UI translations | PhotoCraft contributors | Original translations of PhotoCraft's English labels | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |
| `crates/ui-egui/src/i18n/pt-br.tsv` | Brazilian Portuguese UI translations | PhotoCraft contributors | Original translations of PhotoCraft's English labels | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |
| `assets/fonts/Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf` | Inter 4.001 (UI font) | The Inter Project Authors (Rasmus Andersson) | <https://github.com/rsms/inter> | SIL OFL 1.1, [`assets/fonts/OFL-Inter.txt`](assets/fonts/OFL-Inter.txt) |
| `assets/fonts/JetBrainsMono-Regular.ttf` | JetBrains Mono 2.305 (numeric font) | The JetBrains Mono Project Authors | <https://github.com/JetBrains/JetBrainsMono> | SIL OFL 1.1, [`assets/fonts/OFL-JetBrainsMono.txt`](assets/fonts/OFL-JetBrainsMono.txt) |
| `assets/icons/*.svg` except `slice-knife.svg`, `eraser-background.svg`, `eraser-magic.svg`, `lasso-magnetic.svg`, `direct-select.svg`, `cursor-lasso.svg`, `cursor-polygon-lasso.svg` and `quick-selection.svg` (109 files) | Lucide icons | Lucide Icons and Contributors | <https://github.com/lucide-icons/lucide> (`icons/<name>.svg`; `align-*` are now named `text-align-*` upstream) | ISC, [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `assets/icons/cursor-lasso.svg`, `assets/icons/cursor-polygon-lasso.svg` | Small lasso pointers with explicit arrow hotspots | PhotoCraft contributors | Original work, drawn for PhotoCraft | MIT, [`assets/icons/LICENSE-cursor-lasso.txt`](assets/icons/LICENSE-cursor-lasso.txt) |
| `assets/icons/{check,chevron-down,chevron-right,chevron-up,chevrons-left,chevrons-right,circle,clock,compass,info,link,lock,minus,moon,move,navigation,plus,search,square,trash,triangle,type,x,zoom-in}.svg` | Lucide icons derived from Feather (subset of the row above) | Cole Bemis (Feather), Lucide Contributors | <https://github.com/feathericons/feather> via Lucide | MIT (Feather) and ISC (Lucide), [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `assets/icons/slice-knife.svg` | Slice tool glyph | PhotoCraft contributors | Original work | MIT OR Apache-2.0 |
| `assets/icons/eraser-background.svg`, `assets/icons/eraser-magic.svg` | Background Eraser and Magic Eraser tool glyphs: Lucide `eraser` plus added marks | Lucide Icons and Contributors; marks by PhotoCraft contributors | Derived from <https://github.com/lucide-icons/lucide> `icons/eraser.svg` | ISC, [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `assets/icons/direct-select.svg` | Direct Selection tool glyph: Lucide `mouse-pointer-2` plus an anchor square | Lucide Icons and Contributors; square by PhotoCraft contributors | Derived from <https://github.com/lucide-icons/lucide> `icons/mouse-pointer-2.svg` | ISC, [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `assets/icons/quick-selection.svg` | Quick Selection tool glyph: Lucide `brush` with a dashed selection footprint | Lucide Icons and Contributors; composition by PhotoCraft contributors | Derived from existing Lucide `icons/brush.svg`; no Photoshop asset used | ISC, [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `assets/icons/lasso-magnetic.svg` | Magnetic Lasso tool glyph, adapted from "Magnetic Lasso Tool" (redrawn on the 24 px toolbar grid) | Lil' Seal (Noun Project); adapted by PhotoCraft contributors | <https://thenounproject.com/icon/magnetic-lasso-tool-177513/> | CC BY 3.0, [`assets/icons/LICENSE-noun-magnetic-lasso.txt`](assets/icons/LICENSE-noun-magnetic-lasso.txt) |
| `assets/dict/en_US-scowl-50.txt.gz` | English word list for Check Spelling (SCOWL 2020.12.07, size 50, en_US; 72,403 words) | Kevin Atkinson and the SCOWL contributors | <http://wordlist.aspell.net/> | SCOWL license (permissive, keep the notice), [`assets/dict/LICENSE-SCOWL.txt`](assets/dict/LICENSE-SCOWL.txt), which also has the build recipe |
| `crates/cms/profiles/photocraft-coated-cmyk.icc` | PhotoCraft Coated CMYK (synthetic) ICC profile | PhotoCraft contributors (generated by `photocraft-cms`, `src/synth.rs`) | Original work, see [`crates/cms/README.md`](crates/cms/README.md) | CC0-1.0 |
| `assets/app-icon/` (all files) | PhotoCraft app icon (nine-tailed kitsune) | The project owner (drawn in ArtCraft, vectorised) | Original work, see [`assets/app-icon/README.md`](assets/app-icon/README.md) | MIT OR Apache-2.0, [`assets/app-icon/LICENSE.txt`](assets/app-icon/LICENSE.txt) |

### Optional build input: craft-fonts (not files in this repo)

Builds made with `CRAFT_FONTS_DIR` (all official releases) embed fonts from
[storytold/craft-fonts](https://github.com/storytold/craft-fonts) at a pinned commit; no font file
from it is committed here. Per-file authors, sources and licences:
[craft-fonts `ATTRIBUTION.md`](https://github.com/storytold/craft-fonts/blob/main/ATTRIBUTION.md).

| Font (embedded at build time) | Use | License |
|---|---|---|
| BIZ UDPGothic Regular, Bold (Morisawa) | Japanese UI and sans Type fallback (desktop) | SIL OFL 1.1, shipped as `OFL-biz-ud-pgothic.txt` |
| Shippori Mincho Regular (FONTDASU) | Japanese serif Type fallback (desktop) | SIL OFL 1.1, shipped as `OFL-shippori-mincho.txt` |
| BIZ UDMincho Regular (Morisawa) | Japanese serif Type fallback (desktop) | SIL OFL 1.1, shipped as `OFL-biz-ud-mincho.txt` |

The other built-in ICC profiles and the generated LUT looks are produced by code at runtime
(`crates/cms/src/builtin.rs`, `crates/cms/src/lutfile.rs`) and dedicated to the public domain
(CC0-1.0); they are not separate files.

## Documentation and README

| Path | Title | Author | Source | License |
|---|---|---|---|---|
| `docs/brand/` (all files) | ArtCraft name, wordmark and mark | ArtCraft Team | getartcraft.com | Not open source; trademarks of the ArtCraft Team, [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt) |
| `docs/images/photocraft-*.jpg` | PhotoCraft screenshots | PhotoCraft contributors (UI) | Rendered offscreen with the `snapshot` example | MIT OR Apache-2.0 (UI); the artwork in each is public domain, listed below |
| `docs/images/preferences-apply-*.png` | Preferences before and after adding Apply (no artwork) | PhotoCraft contributors | PhotoCraft control-channel capture and offscreen `snapshot` example | MIT OR Apache-2.0 |

Artwork shown in the screenshots (all public domain, via Wikimedia Commons; details in
[`docs/images/SOURCES.md`](docs/images/SOURCES.md)):

| Screenshot | Artwork | Source |
|---|---|---|
| `photocraft-demo.jpg` | *The Great Wave off Kanagawa*, Katsushika Hokusai, c. 1831 | [Commons](https://commons.wikimedia.org/wiki/File:Tsunami_by_hokusai_19th_century.jpg) |
| `photocraft-layer-styles.jpg` | *Earthrise*, William Anders, Apollo 8, 1968 (NASA) | [Commons](https://commons.wikimedia.org/wiki/File:NASA-Apollo8-Dec24-Earthrise.jpg) |
| `photocraft-masks.jpg` | *Girl with a Pearl Earring*, Johannes Vermeer, c. 1665 | [Commons](https://commons.wikimedia.org/wiki/File:1665_Girl_with_a_Pearl_Earring.jpg) |
| `photocraft-vector.jpg` | *Water Lilies*, Claude Monet, 1906 | [Commons](https://commons.wikimedia.org/wiki/File:Claude_Monet_-_Water_Lilies_-_1906,_Ryerson.jpg) |
| `photocraft-filters.jpg` | *The Starry Night*, Vincent van Gogh, 1889 | [Commons](https://commons.wikimedia.org/wiki/File:Van_Gogh_-_Starry_Night_-_Google_Art_Project.jpg) |
| `photocraft-adjustments.jpg` | *Impression, Sunrise*, Claude Monet, 1872 | [Commons](https://commons.wikimedia.org/wiki/File:Monet_-_Impression,_Sunrise.jpg) |
| `photocraft-transform.jpg` | *The Tetons and the Snake River*, Ansel Adams, 1942 (U.S. National Archives) | [Commons](https://commons.wikimedia.org/wiki/File:Adams_The_Tetons_and_the_Snake_River.jpg) |
| `photocraft-type.jpg` | *Among the Sierra Nevada, California*, Albert Bierstadt, 1868 | [Commons](https://commons.wikimedia.org/wiki/File:Albert_Bierstadt_-_Among_the_Sierra_Nevada,_California_-_Google_Art_Project.jpg) |
| `photocraft-export-light.jpg` | *The Kiss*, Gustav Klimt, 1907–1908 | [Commons](https://commons.wikimedia.org/wiki/File:Gustav_Klimt_016.jpg) |

## Test data (not committed, not shipped)

`corpus/` is gitignored and no test fixtures are committed to the repository.
`cargo xtask corpus --all` fetches every corpus at the pinned commits in `xtask/src/corpus_pins.rs`
and verifies each file against the sha256 lists in `xtask/*.sha256`, with the upstream licence
next to the files:

| Path (fetched) | Title | Author | Source | License |
|---|---|---|---|---|
| `corpus/photoshop/` (256 PSDs) | Photoshop oracle corpus: smart filters, layer-style effects, type, adjustments in every mode and depth | PhotoCraft contributors (authored with Adobe Photoshop 2026 by a script) | [https://github.com/storytold/photocraft-corpus](https://github.com/storytold/photocraft-corpus) (`photoshop/`, with its generator and README) | MIT OR Apache-2.0 |
| `corpus/psd-tools/` (309 files) | psd-tools test set | Kota Yamaguchi and contributors | [psd-tools `tests/psd_files`](https://github.com/psd-tools/psd-tools/tree/main/tests/psd_files) | MIT, Copyright (c) 2019 Kota Yamaguchi |
| `corpus/psd/` (170 files) | Small selection of the psd-tools and ag-psd test files | Kota Yamaguchi; Agamnentzar | psd-tools (above) and [ag-psd `test/`](https://github.com/Agamnentzar/ag-psd/tree/master/test) | MIT (both) |
| `corpus/heif/` (9 files, 0.1 MB) | HEIC/HEIF test files: `heic-rs/` checkerboards, RGB strips and a grid-tiled photo with EXIF and XMP (synthetic pixels encoded by macOS `sips`, each `.ref.png` Apple's decode); `pillow-heif/` the 10-bit RGBA `RGBA_10__29x100.heif` and its source `RGBA_16__29x100.png` | Thomas Braun (heic-rs); Pillow-Heif contributors | [heic-rs `tests/fixtures`](https://github.com/tbraun96/heic-rs/tree/main/tests/fixtures), [pillow-heif `tests/images`](https://github.com/bigcat88/pillow_heif/tree/master/tests/images) | MIT OR Apache-2.0 (heic-rs); BSD-3-Clause (pillow-heif) |
| `corpus/pngsuite/` | PngSuite | Willem van Schaik | <http://www.schaik.com/pngsuite/> | Public domain |

Files copied into `corpus/` by hand (tiff, exr, raw) must be MIT, BSD or CC0.

## InkPop components and test assets

| Path | Title | Author | Source | License |
|---|---|---|---|---|
| Portable `models/mobile_sam-tiny-vitt.safetensors` | MobileSAM TinyViT weights, safetensors conversion | MobileSAM authors; Candle/lmz conversion | https://huggingface.co/lmz/candle-sam/tree/8b4cb7c743f3b3cb8afd212a86ae15b1bbfdac97 | Apache-2.0; `models/LICENSE-MobileSAM.txt` |
| Local QA `v12-astronaut.png` and derived mask/overlay evidence | NASA astronaut photograph via scikit-image | NASA | https://scikit-image.org/docs/stable/api/skimage.data.html#skimage.data.astronaut | Public domain; no known copyright restrictions |
| `crates/ml` dependency: Candle 0.9.1 | Rust neural inference and MobileSAM components | Hugging Face and Candle contributors | https://github.com/huggingface/candle/tree/0.9.1 | MIT OR Apache-2.0; bundled in `models/LICENSE-Candle-*` |
| `vendor/winit/` | winit 0.30.13, PhotoCraft native contact coordinates and transparent owned frame patches | winit contributors | https://github.com/rust-windowing/winit/tree/v0.30.13 | Apache-2.0; LICENSE in directory |
| `vendor/eframe/` | eframe 0.36.2, PhotoCraft owned window, native frame and screenshot patches | egui contributors | https://github.com/emilk/egui/tree/0.36.2 | MIT OR Apache-2.0; LICENSE-MIT and LICENSE-APACHE in directory |
| `models/u2netp.safetensors`, `crates/ml/src/u2net.rs`, `evidence/v66/u2net-*` | U²-Net small salient-object checkpoint, published residual U-block architecture, upstream numerical oracle | Xuebin Qin and U²-Net authors | https://github.com/xuebinqin/U-2-Net/tree/ac7e1c817ecab7c7dff5ce6b1abba61cd213ff29 | Apache-2.0; original license beside model (`LICENSE-U2Net.txt`), checkpoint converted losslessly to safetensors |
| `models/skyseg-u2netp.safetensors`, `evidence/v66/skyseg*` | Public small sky segmentation model (U2NETP); folded F16 weights promoted exactly to F32 | xiongzhu666 | https://github.com/xiongzhu666/Sky-Segmentation-and-Post-processing/tree/1f7811b32b64ddc957269defff84bc87a3f0b74f | MIT; original `LICENSE-SkySeg-small.txt` beside model; shared U²-Net architecture Apache-2.0 |
| `evidence/v66/quality-inputs/astronaut.png` and derivatives | NASA astronaut sample (previously sourced v12 original) | NASA | https://scikit-image.org/docs/stable/api/skimage.data.html#skimage.data.astronaut | Public domain; source original and SHA retained |
| `evidence/v66/quality-inputs/rocket.jpg` and derivatives | DSCOVR Falcon 9 launch photograph | SpaceX | https://scikit-image.org/docs/stable/api/skimage.data.html#skimage.data.rocket | Public domain as documented by scikit-image; source original and SHA retained |
| `evidence/v66/quality-inputs/coffee.png` and derivatives | Coffee cup sample | Rachel Michetti | https://scikit-image.org/docs/stable/api/skimage.data.html#skimage.data.coffee | CC0; source original and SHA retained |
| `evidence/v66/quality-inputs/camera.png` and derivatives | Camera sample | Lav Varshney | https://scikit-image.org/docs/stable/api/skimage.data.html#skimage.data.camera | CC0; source original and SHA retained |

## Original InkPop interface palettes

| Path | Title | Author | Source | License |
|---|---|---|---|---|
| crates/engine/src/prefs/palette/builtin.rs | Four original Studio Light UI palettes | InkPop local contributors; Mummy-fei colour direction | User-provided colour references; photographs retained only as local review evidence, not embedded in the product | MIT OR Apache-2.0 (original palette code/data only; no licence inferred for reference photographs) |

| vendor/egui/ | egui 0.36.2 desktop scrollbar policy patch | egui contributors / Emil Ernerfeldt | https://github.com/emilk/egui/tree/49682f8baa058bf49e011035cfbd6e825f88a5ef/crates/egui | MIT OR Apache-2.0; exact upstream licenses in vendor/egui, identical texts already bundled as LICENSE-eframe-*; native default content-drag disabled, WebAssembly touch default preserved |


## User-supplied InkPop brand artwork

| Path | Title | Author | Source | License / authorization |
|---|---|---|---|---|
| assets/app-icon/inkpop/ (derived PNG/ICO/ICNS/SVG/hicolor icons) | InkPop gradient splash logo | User supplied; original authorship not separately established | InkPop brand artwork supplied by the project owner | Explicit user authorization for InkPop software branding; see assets/app-icon/inkpop/PROVENANCE.txt. No separate general MIT/Apache asset grant or trademark license inferred. Upstream PhotoCraft icon attribution remains separate. |

## Vendored cursor integration

| Path | Title | Author | Source | License |
|---|---|---|---|---|
| `vendor/egui-winit/` | egui-winit 0.36.2 (InkPop cursor application cache patch) | Emil Ernerfeldt and contributors | [upstream](https://github.com/emilk/egui/tree/49682f8baa058bf49e011035cfbd6e825f88a5ef/crates/egui-winit), [crate](https://crates.io/crates/egui-winit/0.36.2) | MIT OR Apache-2.0; both license files alongside source |

## Local segmentation models and inference runtime (v93)

| Packaged path | Component | Author/source | License and scope |
|---|---|---|---|
| models/sam2/* | SAM 2.1 Hiera Small, FP32 ONNX encoder/decoder and external weights | Meta; ONNX Community conversion at a7df49d8de14b9d2e4504d1687b0d568f905fd8d, https://huggingface.co/onnx-community/sam2.1-hiera-small-ONNX | SAM 2 checkpoints Apache-2.0, models/LICENSE-SAM2.txt; exact four file hashes in assets/segmentation/PROVENANCE.json. No SAM 3 assets included. |
| models/birefnet-lite.onnx | BiRefNet general Swin Tiny epoch 232, InkPop local 1024 ONNX export | Peng Zheng et al., https://github.com/ZhengPeng7/BiRefNet/releases/tag/v1; pinned source f7079632d681ecf12161fcea8905682934e59e51 | MIT, models/LICENSE-BiRefNet.txt. Strict official PTH weights; deformable convolution exported as sequential standard GridSample/Conv to reduce transient memory. Local conversion and exact SHA/contract are recorded in assets/segmentation/PROVENANCE.json and models/EXPORT-CONTRACT-v93.txt. Explicit CPU precise mode; BiRefNet DirectML disabled. |
| onnxruntime.dll; onnxruntime_providers_shared.dll | ONNX Runtime 1.22.0, native libraries only | Microsoft onnxruntime-directml 1.22.0 wheel | MIT plus retained third-party notices, licenses/ORT-LICENSE and licenses/ORT-ThirdPartyNotices.txt. Python QA environment is not distributed. Telemetry disabled by application initialization. |
| DirectML.dll | Microsoft.AI.DirectML 1.15.4 x64 native redistributable | Official Microsoft Nuget, https://www.nuget.org/packages/Microsoft.AI.DirectML/1.15.4 | Microsoft Software License Terms for the binary (licenses/DirectML-LICENSE.txt), with independent ThirdPartyNotices. The code MIT license does not replace the binary license; Windows redistribution only. |
| Rust dependencies ort / ort-sys 2.0.0-rc.10 | Safe Rust ONNX adapter and native API bindings | https://github.com/pykeio/ort | MIT OR Apache-2.0, exact licenses retained in assets/segmentation/licenses and copied with the package. Native dependency behind ml onnx feature; excluded from wasm target dependencies. |

The local QA photographs and all user images remain outside this runtime asset set. Model preprocessing/export contracts, package provenance and validation limitations are recorded separately; these licenses do not establish Photoshop pixel parity.
