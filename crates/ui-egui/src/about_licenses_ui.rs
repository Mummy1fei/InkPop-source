//! Offline index of the implementations actually connected to this build.
//! Papers describe independently implemented algorithms, not imported libraries.
use crate::theme::palette::ScrollAreaPaletteExt;
use egui::{Label, RichText, Ui, vec2};

const ATTRIBUTION: &str = include_str!("../../../ATTRIBUTION.md");
const OWN_IMPLEMENTATION: &str = "本项目独立实现 · MIT OR Apache-2.0";
const SCOPE: &str = "主要算法、运行模型与组件索引；不代表全部传递依赖。论文方法由本项目独立实现。";

#[derive(Clone, Copy)]
struct Entry {
    name: &'static str,
    function: &'static str,
    author: &'static str,
    license: &'static str,
    source: &'static str,
}

struct Group {
    name: &'static str,
    entries: &'static [Entry],
}

const GROUPS: &[Group] = &[
    Group {
        name: "智能选择与本地模型",
        entries: &[
            Entry {
                name: "U²-Net / U2NETP",
                function: "主体与背景自动蒙版；本地前景分割",
                author: "Xuebin Qin 与 U²-Net 作者",
                license: "Apache-2.0",
                source: "https://github.com/xuebinqin/U-2-Net",
            },
            Entry {
                name: "SkySeg U2NETP",
                function: "天空自动选择；天空专训权重",
                author: "xiongzhu666；U²-Net 作者",
                license: "天空权重 MIT；U²-Net 架构 Apache-2.0",
                source: "https://github.com/xiongzhu666/Sky-Segmentation-and-Post-processing",
            },
            Entry {
                name: "MobileSAM / TinyViT",
                function: "智能对象选择：点击、轨迹与框提示",
                author: "MobileSAM 作者；Candle / lmz 转换",
                license: "Apache-2.0",
                source: "https://github.com/ChaoningZhang/MobileSAM",
            },
            Entry {
                name: "Candle",
                function: "Rust 本地神经网络推理",
                author: "Hugging Face 与 Candle 贡献者",
                license: "MIT OR Apache-2.0",
                source: "https://github.com/huggingface/candle",
            },
        ],
    },
    Group {
        name: "公开论文算法 · PhotoCraft 独立实现",
        entries: &[
            Entry {
                name: "GrabCut / Graph Cut",
                function: "经典对象选择、前景与背景图割",
                author: "Rother、Kolmogorov、Blake；Boykov 等",
                license: OWN_IMPLEMENTATION,
                source: "https://www.microsoft.com/en-us/research/publication/grabcut-interactive-foreground-extraction-using-iterated-graph-cuts/",
            },
            Entry {
                name: "SLIC / 显著性检测",
                function: "经典主体选择；超像素与显著性线索",
                author: "Achanta 等；Zhu、Liang、Wei、Sun",
                license: OWN_IMPLEMENTATION,
                source: "https://openaccess.thecvf.com/content_cvpr_2014/html/Zhu_Saliency_Optimization_from_2014_CVPR_paper.html",
            },
            Entry {
                name: "Guided Image Filtering",
                function: "图像引导蒙版边缘细化",
                author: "Kaiming He、Jian Sun、Xiaoou Tang",
                license: OWN_IMPLEMENTATION,
                source: "https://kaiminghe.com/eccv10/",
            },
            Entry {
                name: "Intelligent Scissors / Live Wire",
                function: "磁性套索：沿图像边缘寻找路径",
                author: "Eric N. Mortensen、William A. Barrett",
                license: OWN_IMPLEMENTATION,
                source: "https://doi.org/10.1145/218380.218442",
            },
            Entry {
                name: "PatchMatch / 图像补全",
                function: "内容识别填充、修复与纹理补全",
                author: "Barnes 等；Wexler、Shechtman、Irani",
                license: OWN_IMPLEMENTATION,
                source: "https://gfx.cs.princeton.edu/pubs/Barnes_2009_PAR/index.php",
            },
            Entry {
                name: "Harris / BRIEF / RANSAC",
                function: "图层自动对齐与全景配准",
                author: "Harris、Stephens；Calonder 等；Fischler、Bolles",
                license: OWN_IMPLEMENTATION,
                source: "https://doi.org/10.1145/358669.358692",
            },
        ],
    },
    Group {
        name: "项目自有图像实现",
        entries: &[
            Entry {
                name: "PhotoCraft 图像处理",
                function: "画笔、涂抹、Kubelka–Munk 颜料混色、Camera Raw、曲线、滤镜与选区",
                author: "PhotoCraft 贡献者；当前定制开发 Mummy-fei",
                license: "MIT OR Apache-2.0",
                source: "https://github.com/storytold/photocraft",
            },
            Entry {
                name: "photocraft-cms",
                function: "原生 Rust ICC 色彩管理；自有生成 ICC 配置为 CC0",
                author: "PhotoCraft 贡献者",
                license: "代码 MIT OR Apache-2.0；生成配置 CC0-1.0",
                source: "https://github.com/storytold/photocraft/tree/main/crates/cms",
            },
            Entry {
                name: "photocraft-psd / photocraft-raw",
                function: "原生 Rust PSD/PSB 编解码；相机 RAW 解码与显影",
                author: "PhotoCraft 贡献者",
                license: "MIT OR Apache-2.0",
                source: "https://github.com/storytold/photocraft/tree/main/crates",
            },
        ],
    },
    Group {
        name: "界面、GPU 与并行计算",
        entries: &[
            Entry {
                name: "egui / eframe",
                function: "原生界面、窗口与交互",
                author: "Emil Ernerfeldt 与 egui 贡献者",
                license: "MIT OR Apache-2.0",
                source: "https://github.com/emilk/egui",
            },
            Entry {
                name: "wgpu",
                function: "GPU 图像合成与界面渲染",
                author: "gfx-rs / wgpu 贡献者",
                license: "MIT OR Apache-2.0",
                source: "https://github.com/gfx-rs/wgpu",
            },
            Entry {
                name: "winit",
                function: "跨平台原生窗口与输入事件",
                author: "winit 贡献者",
                license: "Apache-2.0",
                source: "https://github.com/rust-windowing/winit",
            },
            Entry {
                name: "Rayon",
                function: "CPU 图像处理与分块并行",
                author: "Rayon 贡献者",
                license: "MIT OR Apache-2.0",
                source: "https://github.com/rayon-rs/rayon",
            },
            Entry {
                name: "tiny-skia / resvg",
                function: "矢量、SVG 与图标栅格化",
                author: "tiny-skia 与 resvg 贡献者",
                license: "tiny-skia BSD-3-Clause；resvg MIT OR Apache-2.0",
                source: "https://github.com/linebender/resvg",
            },
        ],
    },
    Group {
        name: "图像格式与压缩",
        entries: &[
            Entry {
                name: "image / png / image-webp",
                function: "常用图像格式、PNG 与 WebP",
                author: "image-rs 贡献者",
                license: "MIT OR Apache-2.0",
                source: "https://github.com/image-rs",
            },
            Entry {
                name: "tiff",
                function: "TIFF 编解码",
                author: "image-rs / image-tiff 贡献者",
                license: "MIT",
                source: "https://github.com/image-rs/image-tiff",
            },
            Entry {
                name: "exr",
                function: "OpenEXR 图像编解码",
                author: "Johannes Vollmer 与 exrs 贡献者",
                license: "BSD-3-Clause",
                source: "https://github.com/johannesvollmer/exrs",
            },
            Entry {
                name: "zune-jpeg",
                function: "JPEG 解码",
                author: "caleb / zune-image 贡献者",
                license: "MIT OR Apache-2.0 OR Zlib",
                source: "https://github.com/etemesi254/zune-image",
            },
            Entry {
                name: "jpeg-encoder",
                function: "JPEG 编码",
                author: "Volker Ströbel 与贡献者",
                license: "(MIT OR Apache-2.0) AND IJG",
                source: "https://github.com/vstroebel/jpeg-encoder",
            },
            Entry {
                name: "flate2",
                function: "DEFLATE / zlib / gzip 与大蒙版无损压缩",
                author: "flate2-rs 贡献者",
                license: "MIT OR Apache-2.0",
                source: "https://github.com/rust-lang/flate2-rs",
            },
            Entry {
                name: "base64",
                function: "压缩蒙版数据的文本封装",
                author: "base64 贡献者",
                license: "MIT OR Apache-2.0",
                source: "https://github.com/marshallpierce/rust-base64",
            },
        ],
    },
];

fn source_link(ui: &mut Ui, label: &str, url: &'static str) -> Option<&'static str> {
    let response = ui.link("来源").on_hover_text(url);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, ui.is_enabled(), format!("来源：{label}")));
    response.clicked().then_some(url)
}

fn entry(ui: &mut Ui, item: &Entry) -> Option<&'static str> {
    let t = crate::theme::Tokens::get(ui.ctx());
    let width = ui.available_width();
    let source_width = 42.0;
    let name_width = (width - source_width - 8.0).max(100.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(vec2(name_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(name_width);
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add(Label::new(RichText::new(item.name).strong().size(12.0)).wrap()).on_hover_text(format!("{}\n作者：{}", item.function, item.author));
            ui.add(Label::new(RichText::new(item.license).size(10.0).color(t.text_dim)).wrap());
        });
        source_link(ui, item.name, item.source)
    })
    .inner
}

/// Returns the viewport, content height and scroll offset for visual QA.
pub(crate) fn show(app: &mut crate::PhotocraftApp, ui: &mut Ui) -> (egui::Rect, f32, f32) {
    let (geometry, clicked) = content(ui);
    if let Some(url) = clicked {
        crate::links::open(app, ui.ctx(), url);
    }
    geometry
}

fn content(ui: &mut Ui) -> ((egui::Rect, f32, f32), Option<&'static str>) {
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(5.0);
    ui.strong("算法、模型与第三方组件");
    ui.add(Label::new(RichText::new(SCOPE).small().weak()).wrap());
    let used = (ui.cursor().top() - ui.min_rect().top()).max(0.0);
    let height = (ui.clip_rect().height() - used - 24.0).min((ui.ctx().content_rect().height() - 220.0).max(0.0)).max(0.0);
    let mut clicked = None;
    let result = egui::ScrollArea::vertical().id_salt("about-open-source-v66").max_height(height).auto_shrink([false, true]).show_palette(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        let mut groups = |ui: &mut Ui, indices: &[usize]| {
            for (index, group) in indices.iter().filter_map(|index| GROUPS.get(*index)).enumerate() {
                if index > 0 {
                    ui.add_space(4.0);
                    ui.separator();
                }
                ui.add(Label::new(RichText::new(group.name).strong().size(12.0)).wrap());
                for item in group.entries {
                    if let Some(url) = ui.push_id(item.name, |ui| entry(ui, item)).inner {
                        clicked = Some(url);
                    }
                }
            }
        };
        if ui.available_width() >= 900.0 {
            ui.columns(2, |columns| {
                if let Some(left) = columns.get_mut(0) {
                    groups(left, &[0, 1, 2]);
                }
                if let Some(right) = columns.get_mut(1) {
                    groups(right, &[3, 4]);
                }
            });
        } else {
            groups(ui, &[0, 1, 2, 3, 4]);
        }
        ui.add_space(5.0);
        egui::CollapsingHeader::new("第三方素材与来源索引（ATTRIBUTION.md）").id_salt("about-attribution-text").show(ui, |ui| {
            ui.add(Label::new(RichText::new("随源码编译的原文索引；包含模型、素材、图标与可选字体说明。").small().weak()).wrap());
            ui.horizontal(|ui| {
                ui.small("PhotoCraft 上游索引");
                if let Some(url) = source_link(ui, "ATTRIBUTION 上游索引", "https://github.com/storytold/photocraft/blob/main/ATTRIBUTION.md") {
                    clicked = Some(url);
                }
            });
            egui::ScrollArea::vertical().id_salt("about-attribution-scroll").max_height(180.0).show_palette(ui, |ui| {
                ui.add(Label::new(RichText::new(ATTRIBUTION).monospace().size(10.0)).wrap().selectable(true));
            });
        });
    });
    ui.add_space(4.0);
    ui.add(Label::new(RichText::new("保留各原作者的版权与署名；许可正文以对应文件及来源为准。").small().weak()).wrap());
    ((result.inner_rect, result.content_size.y, result.state.offset.y), clicked)
}

#[cfg(test)]
#[path = "about_licenses_tests_v66.rs"]
mod tests;
