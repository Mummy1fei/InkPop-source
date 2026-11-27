//! Cached native tool pointers: OS movement does not wait for document presentation.
//! Unsupported backends and oversized footprints retain the existing software overlay.
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    sync::Arc,
};

use egui::{CustomCursorImage, Vec2};
use photocraft_engine::BrushSettings;
use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

const MAX_SIDE: u16 = 512;
const CACHE_LIMIT: usize = 32;

#[derive(Clone, Default)]
struct Cache(VecDeque<(u64, CustomCursorImage)>);

pub(crate) fn enabled(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new("photocraft-native-tool-cursors"))).unwrap_or(false)
}

fn signature(value: impl Hash) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hash);
    hash.finish()
}

fn install(ctx: &egui::Context, key: u64, build: impl FnOnce() -> Option<CustomCursorImage>) -> bool {
    let id = egui::Id::new("photocraft-native-cursor-images");
    let hit = ctx.data_mut(|data| {
        let cache = data.get_temp_mut_or_default::<Cache>(id);
        let at = cache.0.iter().position(|(k, _)| *k == key)?;
        let entry = cache.0.remove(at)?;
        let image = entry.1.clone();
        cache.0.push_back(entry);
        Some(image)
    });
    let image = match hit {
        Some(image) => image,
        None => {
            let Some(image) = build() else { return false };
            ctx.data_mut(|data| {
                let cache = data.get_temp_mut_or_default::<Cache>(id);
                while cache.0.len() >= CACHE_LIMIT {
                    cache.0.pop_front();
                }
                cache.0.push_back((key, image.clone()));
            });
            image
        }
    };
    ctx.set_cursor_image(Some(image));
    true
}

fn valid_scale(ppp: f32) -> bool {
    ppp.is_finite() && ppp > 0.0 && ppp <= 16.0
}

fn from_pixmap(pixmap: &Pixmap, hotspot: [u16; 2]) -> CustomCursorImage {
    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let p = p.demultiply();
            [p.red(), p.green(), p.blue(), p.alpha()]
        })
        .collect();
    CustomCursorImage { rgba: rgba.into(), size: [pixmap.width() as u16, pixmap.height() as u16], hotspot }
}

fn paint_paths(pixmap: &mut Pixmap, paths: &[Vec<Vec2>], center: Vec2, ppp: f32) {
    stroke_paths(pixmap, paths, center, ppp, &[(2.0, [0, 0, 0, 160]), (1.0, [255, 255, 255, 230])]);
}

fn stroke_paths(pixmap: &mut Pixmap, paths: &[Vec<Vec2>], center: Vec2, ppp: f32, styles: &[(f32, [u8; 4])]) {
    for &(width, rgba) in styles {
        let mut paint = Paint::default();
        paint.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
        paint.anti_alias = true;
        let stroke = Stroke { width: width * ppp, ..Default::default() };
        for points in paths {
            let Some(first) = points.first() else { continue };
            let mut path = PathBuilder::new();
            path.move_to(center.x + first.x, center.y + first.y);
            for p in points.iter().skip(1) {
                path.line_to(center.x + p.x, center.y + p.y);
            }
            if let Some(path) = path.finish() {
                pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
        }
    }
}

fn cross_paths(len: f32) -> [Vec<Vec2>; 2] {
    [vec![egui::vec2(-len, 0.0), egui::vec2(len, 0.0)], vec![egui::vec2(0.0, -len), egui::vec2(0.0, len)]]
}

fn raster_paths(paths: &[Vec<Vec2>], half_extent: f32, ppp: f32) -> Option<CustomCursorImage> {
    if !half_extent.is_finite() || half_extent < 0.0 || !valid_scale(ppp) {
        return None;
    }
    let half = (half_extent + 2.0 * ppp).ceil();
    let side = half * 2.0 + 1.0;
    if side > f32::from(MAX_SIDE) {
        return None;
    }
    let mut pixmap = Pixmap::new(side as u32, side as u32)?;
    // A bitmap hotspot identifies a pixel, whose centre is x + 0.5. Centre odd-width
    // strokes there instead of splitting a 1 px white core equally across two pixels.
    paint_paths(&mut pixmap, paths, Vec2::splat(half + 0.5), ppp);
    Some(from_pixmap(&pixmap, [half as u16; 2]))
}

pub(crate) fn clipping(ctx: &egui::Context) -> bool {
    let ppp = ctx.pixels_per_point();
    if !enabled(ctx) || !valid_scale(ppp) {
        return false;
    }
    install(ctx, signature(("layer-clipping", ppp.to_bits())), || {
        let paths = crate::layer_row_ui::clipping_arrow_paths().map(|path| path.map(|point| point * ppp).into());
        raster_paths(&paths, 11.0 * ppp, ppp)
    })
}

pub(crate) fn crosshair(ctx: &egui::Context, len: f32) -> bool {
    let ppp = ctx.pixels_per_point();
    if !enabled(ctx) || !valid_scale(ppp) || !len.is_finite() || len <= 0.0 {
        return false;
    }
    install(ctx, signature(("cross", len.to_bits(), ppp.to_bits())), || raster_paths(&cross_paths(len * ppp), len * ppp, ppp))
}

/// Quick Selection samples a circular seed radius, not the current painting tip.
/// Keep its bitmap independent of sampled brush data, hardness, angle and view rotation.
pub(crate) fn selection_circle(ctx: &egui::Context, radius: f32) -> bool {
    area_circle(ctx, radius, None)
}

/// A circular area-tool footprint, independent of the saved painting tip.
pub(crate) fn area_circle(ctx: &egui::Context, radius: f32, crosshair: Option<f32>) -> bool {
    let ppp = ctx.pixels_per_point();
    if !enabled(ctx) || !valid_scale(ppp) || !radius.is_finite() || radius <= 0.0 || crosshair.is_some_and(|len| !len.is_finite() || len <= 0.0) {
        return false;
    }
    let radius_px = radius * ppp;
    let half = radius_px.max(crosshair.unwrap_or(0.0) * ppp);
    if half * 2.0 + 4.0 * ppp + 1.0 > f32::from(MAX_SIDE) {
        return false;
    }
    install(ctx, signature(("area-circle", radius.to_bits(), ppp.to_bits(), crosshair.map(f32::to_bits))), || {
        let circle = (0..=128)
            .map(|i| {
                let angle = i as f32 * std::f32::consts::TAU / 128.0;
                egui::vec2(angle.cos(), angle.sin()) * radius_px
            })
            .collect();
        let mut paths = vec![circle];
        if let Some(len) = crosshair {
            paths.extend(cross_paths(len * ppp));
        }
        raster_paths(&paths, half, ppp)
    })
}

pub(crate) fn brush(ctx: &egui::Context, brush: &BrushSettings, xf: &crate::canvas::ViewXform, normal: bool, crosshair: Option<f32>) -> bool {
    let ppp = ctx.pixels_per_point();
    let size = brush.size * xf.zoom;
    if !enabled(ctx)
        || !valid_scale(ppp)
        || !size.is_finite()
        || size <= 0.0
        || !xf.rotation.is_finite()
        || crosshair.is_some_and(|len| !len.is_finite() || len <= 0.0)
    {
        return false;
    }
    let resolution = crate::brush_cursor_ui::RESOLUTION as f32;
    let size = size.clamp(1.0, 100000.0) * resolution / (resolution - 2.0) * ppp;
    // Conservative square bound also covers the footprint after arbitrary view rotation.
    if size * std::f32::consts::SQRT_2 + 4.0 * ppp + 1.0 > f32::from(MAX_SIDE) {
        return false;
    }
    let contour = crate::brush_cursor_ui::contour(ctx, brush, normal);
    if contour.paths.is_empty() {
        return false;
    }
    let key = signature(("brush", contour.signature, size.to_bits(), xf.rotation.to_bits(), xf.flip, ppp.to_bits(), crosshair.map(f32::to_bits)));
    install(ctx, key, || {
        let rotation = egui::emath::Rot2::from_angle(xf.rotation.to_radians());
        let mut paths: Vec<Vec<Vec2>> =
            contour.paths.iter().map(|path| path.iter().map(|p| rotation * egui::vec2(if xf.flip { -p.x } else { p.x }, p.y) * size).collect()).collect();
        if let Some(len) = crosshair {
            paths.extend(cross_paths(len * ppp));
        }
        let half = paths.iter().flatten().fold(0.0_f32, |extent, p| extent.max(p.x.abs()).max(p.y.abs()));
        raster_paths(&paths, half, ppp)
    })
}

fn glyph_image(svg: &[u8], hot: Vec2, size: f32, ppp: f32) -> Option<CustomCursorImage> {
    if !valid_scale(ppp) || !size.is_finite() || size <= 0.0 || !hot.is_finite() || hot.x < 0.0 || hot.y < 0.0 || hot.x > 1.0 || hot.y > 1.0 {
        return None;
    }
    let n = (size * ppp).round().max(1.0);
    let pad = ppp.ceil().max(1.0) as usize;
    if n + 2.0 * pad as f32 > f32::from(MAX_SIDE) {
        return None;
    }
    let n = n as usize;
    let hint = egui::load::SizeHint::Size { width: n as u32, height: n as u32, maintain_aspect_ratio: false };
    let image = egui_extras::image::load_svg_bytes_with_size(svg, hint, &Default::default()).ok()?;
    let side = n + 2 * pad;
    let mut rgba = vec![0_u8; side * side * 4];
    let alpha = |x: isize, y: isize| -> f32 {
        if x < 0 || y < 0 || x >= n as isize || y >= n as isize {
            return 0.0;
        }
        image.pixels.get(y as usize * n + x as usize).map_or(0.0, |p| f32::from(p.a()) / 255.0)
    };
    for y in 0..side {
        for x in 0..side {
            let (sx, sy) = (x as isize - pad as isize, y as isize - pad as isize);
            let white = alpha(sx, sy);
            let mut outline = 0.0_f32;
            for dy in -(pad as isize)..=pad as isize {
                for dx in -(pad as isize)..=pad as isize {
                    outline = outline.max(alpha(sx + dx, sy + dy));
                }
            }
            let total = white + outline * (200.0 / 255.0) * (1.0 - white);
            if let Some(pixel) = rgba.get_mut((y * side + x) * 4..(y * side + x + 1) * 4) {
                let c = if total > 0.0 { (white / total * 255.0).round() as u8 } else { 0 };
                pixel.copy_from_slice(&[c, c, c, (total * 255.0).round() as u8]);
            }
        }
    }
    Some(CustomCursorImage {
        rgba: Arc::from(rgba),
        size: [side as u16; 2],
        hotspot: [(pad as f32 + hot.x * n as f32).round().min((side - 1) as f32) as u16, (pad as f32 + hot.y * n as f32).round().min((side - 1) as f32) as u16],
    })
}

pub(crate) fn glyph(ctx: &egui::Context, name: &str, svg: &[u8], hot: Vec2, size: f32) -> bool {
    if !enabled(ctx) {
        return false;
    }
    let ppp = ctx.pixels_per_point();
    install(ctx, signature(("glyph", name, hot.x.to_bits(), hot.y.to_bits(), size.to_bits(), ppp.to_bits())), || glyph_image(svg, hot, size, ppp))
}

#[derive(Clone)]
struct BadgedImage {
    // Retain the source Arc: a freed bitmap address must never alias a cache key.
    source: CustomCursorImage,
    badge: crate::tool_feedback::Badge,
    centred: bool,
    ppp: f32,
    image: CustomCursorImage,
}

#[derive(Clone, Default)]
struct BadgeCache(VecDeque<BadgedImage>);

fn badge_image(source: &CustomCursorImage, badge: crate::tool_feedback::Badge, centred: bool, ppp: f32) -> Option<CustomCursorImage> {
    if !valid_scale(ppp) {
        return None;
    }
    let hotspot = egui::vec2(f32::from(source.hotspot[0]), f32::from(source.hotspot[1]));
    let center = hotspot + if centred { Vec2::ZERO } else { egui::vec2(30.0, 11.0) * ppp };
    let half = 5.5 * ppp;
    let min = (center - Vec2::splat(half)).min(Vec2::ZERO).floor();
    let max = (center + Vec2::splat(half)).max(egui::vec2(f32::from(source.size[0]), f32::from(source.size[1]))).ceil();
    let size = max - min;
    if size.x > f32::from(MAX_SIDE) || size.y > f32::from(MAX_SIDE) {
        return None;
    }
    let mut pixmap = Pixmap::new(size.x as u32, size.y as u32)?;
    let paths: Vec<Vec<Vec2>> = crate::tool_feedback::badge_glyph(badge, egui::Pos2::ZERO, 3.5 * ppp)
        .into_iter()
        .map(|segment| segment.into_iter().map(|p| p.to_vec2()).collect())
        .collect();
    stroke_paths(&mut pixmap, &paths, center - min + Vec2::splat(0.5), ppp, &[(3.0, [0, 0, 0, 200]), (1.2, [255, 255, 255, 255])]);
    let target_hotspot = [(hotspot.x - min.x) as u16, (hotspot.y - min.y) as u16];
    let overlay = from_pixmap(&pixmap, target_hotspot);
    let width = usize::from(overlay.size[0]);
    let height = usize::from(overlay.size[1]);
    let mut rgba = overlay.rgba.to_vec();
    let (offset_x, offset_y) = ((-min.x) as usize, (-min.y) as usize);
    for y in 0..usize::from(source.size[1]) {
        for x in 0..usize::from(source.size[0]) {
            if y + offset_y >= height || x + offset_x >= width {
                continue;
            }
            let from = (y * usize::from(source.size[0]) + x) * 4;
            let to = ((y + offset_y) * width + x + offset_x) * 4;
            let (Some(src), Some(dst)) = (source.rgba.get(from..from + 4), rgba.get_mut(to..to + 4)) else { continue };
            let sa = f32::from(src.get(3).copied().unwrap_or(0)) / 255.0;
            let da = f32::from(dst.get(3).copied().unwrap_or(0)) / 255.0;
            let alpha = da + sa * (1.0 - da);
            for (out, input) in dst.iter_mut().take(3).zip(src.iter().take(3)) {
                *out = if alpha > 0.0 { ((f32::from(*out) * da + f32::from(*input) * sa * (1.0 - da)) / alpha).round() as u8 } else { 0 };
            }
            if let Some(out) = dst.get_mut(3) {
                *out = (alpha * 255.0).round() as u8;
            }
        }
    }
    Some(CustomCursorImage { rgba: rgba.into(), ..overlay })
}

pub(crate) fn badge(ctx: &egui::Context, badge: crate::tool_feedback::Badge, centred: bool) -> bool {
    if !enabled(ctx) {
        return false;
    }
    let Some(source) = ctx.output(|o| o.cursor_image.clone()) else { return false };
    let ppp = ctx.pixels_per_point();
    let id = egui::Id::new("photocraft-native-badged-cursors");
    let hit = ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<BadgeCache>(id);
        let at = cache.0.iter().position(|entry| {
            Arc::ptr_eq(&entry.source.rgba, &source.rgba)
                && entry.source.size == source.size
                && entry.source.hotspot == source.hotspot
                && entry.badge == badge
                && entry.centred == centred
                && entry.ppp == ppp
        })?;
        let entry = cache.0.remove(at)?;
        let image = entry.image.clone();
        cache.0.push_back(entry);
        Some(image)
    });
    let image = match hit {
        Some(image) => image,
        None => {
            let Some(image) = badge_image(&source, badge, centred, ppp) else { return false };
            ctx.data_mut(|d| {
                let cache = d.get_temp_mut_or_default::<BadgeCache>(id);
                while cache.0.len() >= CACHE_LIMIT {
                    cache.0.pop_front();
                }
                cache.0.push_back(BadgedImage { source, badge, centred, ppp, image: image.clone() });
            });
            image
        }
    };
    ctx.set_cursor_image(Some(image));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("photocraft-native-tool-cursors"), true));
        ctx
    }

    fn current(ctx: &egui::Context) -> CustomCursorImage {
        ctx.output(|o| o.cursor_image.clone()).expect("native cursor")
    }

    fn valid(image: &CustomCursorImage) {
        assert!(image.size.iter().all(|n| *n > 0 && *n <= MAX_SIDE));
        assert_eq!(image.rgba.len(), usize::from(image.size[0]) * usize::from(image.size[1]) * 4);
        assert!(image.hotspot[0] < image.size[0] && image.hotspot[1] < image.size[1]);
        // Straight RGBA alone says nothing about visible contrast: antialiased white
        // over a translucent black halo can be grey. Check the actual composited pixel
        // on both extreme document backgrounds. Transparent pixels cannot pass either.
        let (bright_on_black, dark_on_white) = contrast(image);
        assert!(bright_on_black >= 96.0, "white core too faint on black: {bright_on_black}");
        assert!(dark_on_white <= 200.0, "dark outline too faint on white: {dark_on_white}");
        assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 0));
    }

    fn contrast(image: &CustomCursorImage) -> (f32, f32) {
        image.rgba.as_chunks::<4>().0.iter().fold((0.0_f32, 255.0_f32), |(bright, dark), p| {
            let alpha = f32::from(p[3]) / 255.0;
            let luminance = (f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2])) / 3.0;
            (bright.max(luminance * alpha), dark.min(luminance * alpha + 255.0 * (1.0 - alpha)))
        })
    }

    #[test]
    fn cursor_cache_reuses_bitmap_and_is_bounded() {
        let ctx = context();
        assert!(crosshair(&ctx, 6.0));
        let a = current(&ctx);
        assert!(crosshair(&ctx, 6.0));
        assert!(Arc::ptr_eq(&a.rgba, &current(&ctx).rgba));
        valid(&a);
        for len in 1..=80 {
            assert!(crosshair(&ctx, len as f32));
        }
        assert_eq!(ctx.data(|d| d.get_temp::<Cache>(egui::Id::new("photocraft-native-cursor-images"))).unwrap().0.len(), CACHE_LIMIT);
        assert!(!crosshair(&egui::Context::default(), 6.0));
    }

    #[test]
    fn glyph_raster_is_dpi_scaled_straight_rgba_with_valid_hotspot() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><path d="M4 4L20 20M4 20L20 4" stroke="white" stroke-width="2"/></svg>"##;
        for ppp in [1.0, 1.25, 1.5, 2.0] {
            let image = glyph_image(svg, egui::vec2(2.0, 22.0) / 24.0, 22.0, ppp).unwrap();
            valid(&image);
            let expected = (22.0 * ppp).round() as u16 + 2 * ppp.ceil() as u16;
            assert_eq!(image.size, [expected; 2]);
        }
        assert!(glyph_image(svg, Vec2::ZERO, f32::INFINITY, 1.0).is_none());
        assert!(glyph_image(svg, Vec2::ZERO, 600.0, 1.0).is_none());
    }

    #[test]
    fn brush_native_rotation_and_oversize_fallback_preserve_geometry() {
        let ctx = context();
        let brush_settings = BrushSettings { size: 80.0, roundness: 0.3, angle: 12.0, ..Default::default() };
        let mut xf = crate::canvas::ViewXform { rect: egui::Rect::NOTHING, center: [0.0; 2], zoom: 1.0, rotation: 0.0, flip: false };
        assert!(brush(&ctx, &brush_settings, &xf, true, Some(3.0)));
        let first = current(&ctx);
        valid(&first);
        assert!(brush(&ctx, &brush_settings, &xf, true, Some(3.0)));
        assert!(Arc::ptr_eq(&first.rgba, &current(&ctx).rgba));
        xf.rotation = 35.0;
        xf.flip = true;
        assert!(brush(&ctx, &brush_settings, &xf, true, Some(3.0)));
        let rotated = current(&ctx);
        valid(&rotated);
        assert_ne!(first.rgba, rotated.rgba);
        xf.zoom = 1000.0;
        assert!(!brush(&ctx, &brush_settings, &xf, true, Some(3.0)));
    }

    #[test]
    fn selection_badges_follow_native_cursor_and_reuse_composite() {
        let ctx = context();
        assert!(!badge(&ctx, crate::tool_feedback::Badge::Add, false));
        for kind in [crate::tool_feedback::Badge::Add, crate::tool_feedback::Badge::Subtract, crate::tool_feedback::Badge::Intersect] {
            for centred in [false, true] {
                assert!(crosshair(&ctx, 6.0));
                assert!(badge(&ctx, kind, centred));
                let first = current(&ctx);
                valid(&first);
                assert!(crosshair(&ctx, 6.0));
                assert!(badge(&ctx, kind, centred));
                assert!(Arc::ptr_eq(&first.rgba, &current(&ctx).rgba));
            }
        }
    }

    #[test]
    fn clipping_cursor_has_a_downward_tip_hotspot_and_reuses_its_bitmap() {
        let evidence = std::env::var_os("PHOTOCRAFT_V104_CURSOR_EVIDENCE");
        let mut images = Vec::new();
        for ppp in [1.0_f32, 1.25, 1.5, 2.0, 3.0] {
            let ctx = context();
            ctx.set_pixels_per_point(ppp);
            let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
            output.textures_delta.clear();
            assert!(clipping(&ctx));
            let image = current(&ctx);
            valid(&image);
            assert!(clipping(&ctx));
            assert!(Arc::ptr_eq(&image.rgba, &current(&ctx).rgba));
            assert!(image.size[0] >= (22.0 * ppp) as u16);
            let [x, y] = image.hotspot.map(usize::from);
            let width = usize::from(image.size[0]);
            assert!(image.rgba[(y * width + x) * 4 + 3] > 0, "the tip covers the click location");
            let below_outline = (y + (ppp * 2.0).ceil() as usize + 1) * width * 4;
            assert!(image.rgba[below_outline..].iter().all(|&byte| byte == 0), "the arrow is the only cursor art");
            if evidence.is_some() && matches!(ppp, 1.0 | 2.0) {
                images.push(serde_json::json!({"pixelsPerPoint":ppp,"size":image.size,"hotspot":image.hotspot,"rgba":image.rgba.as_ref()}));
            }
        }
        assert!(!clipping(&egui::Context::default()));
        if let Some(path) = evidence {
            std::fs::write(path, serde_json::to_vec(&images).unwrap()).unwrap();
        }
    }

    #[test]
    #[ignore = "exports real native pointer bitmaps for visual inspection; not hardware latency"]
    fn export_native_cursor_assets() {
        let ctx = context();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v44/cursor-assets");
        std::fs::create_dir_all(&root).unwrap();
        let mut records = Vec::new();
        let mut save = |name: &str, image: CustomCursorImage| {
            let mut pixmap = Pixmap::new(u32::from(image.size[0]), u32::from(image.size[1])).unwrap();
            for (src, dst) in image.rgba.as_chunks::<4>().0.iter().zip(pixmap.data_mut().as_chunks_mut::<4>().0.iter_mut()) {
                let a = u16::from(src[3]);
                for i in 0..3 {
                    dst[i] = ((u16::from(src[i]) * a + 127) / 255) as u8;
                }
                dst[3] = src[3];
            }
            pixmap.save_png(root.join(format!("{name}.png"))).unwrap();
            let (bright_on_black, dark_on_white) = contrast(&image);
            records.push(serde_json::json!({"name": name, "size": image.size, "hotspot": image.hotspot,
                "brightOnBlack": bright_on_black, "darkOnWhite": dark_on_white}));
        };
        for len in [3.0, 6.0, 8.0] {
            assert!(crosshair(&ctx, len));
            save(&format!("crosshair-{len}"), current(&ctx));
        }
        let mut xf = crate::canvas::ViewXform { rect: egui::Rect::NOTHING, center: [0.0; 2], zoom: 1.0, rotation: 0.0, flip: false };
        for size in [2.0, 8.0, 40.0, 180.0] {
            let settings = BrushSettings { size, ..Default::default() };
            assert!(brush(&ctx, &settings, &xf, true, None));
            save(&format!("round-{size}"), current(&ctx));
        }
        let settings = BrushSettings { size: 80.0, roundness: 0.3, angle: 12.0, ..Default::default() };
        for rotation in [0.0, 35.0, 90.0] {
            xf.rotation = rotation;
            xf.flip = true;
            assert!(brush(&ctx, &settings, &xf, true, Some(3.0)));
            save(&format!("oval-rotated-{rotation}"), current(&ctx));
        }
        let tip = photocraft_engine::paint::GrayTile::from_fn(48, 48, |x, y| {
            let r = (x as f32 - 23.5).hypot(y as f32 - 23.5);
            if (10.0..20.0).contains(&r) && !(x > 27 && y < 15) { 1.0 } else { 0.0 }
        });
        let sampled = BrushSettings { tip: photocraft_engine::paint::TipShape::Sampled(tip), size: 100.0, ..Default::default() };
        xf.rotation = 25.0;
        assert!(brush(&ctx, &sampled, &xf, true, None));
        save("sampled-hollow-rotated", current(&ctx));
        for (name, hot) in [("pipette", egui::vec2(2.0, 22.0) / 24.0), ("rotate-cw", Vec2::splat(0.5)), ("square-dashed", Vec2::splat(0.5))] {
            crate::icons::cursor(&ctx, name, egui::pos2(100.0, 100.0), hot, 22.0);
            save(name, current(&ctx));
        }
        for (kind, name) in [
            (crate::tool_feedback::Badge::Add, "add"),
            (crate::tool_feedback::Badge::Subtract, "subtract"),
            (crate::tool_feedback::Badge::Intersect, "intersect"),
        ] {
            for centred in [true, false] {
                assert!(crosshair(&ctx, 6.0));
                assert!(badge(&ctx, kind, centred));
                save(&format!("badge-{name}-centred-{centred}"), current(&ctx));
            }
        }
        std::fs::write(root.join("metrics.json"), serde_json::to_vec_pretty(&records).unwrap()).unwrap();
    }
}
