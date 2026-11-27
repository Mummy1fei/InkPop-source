use super::*;

fn geometry(side: f32) -> (Pos2, f32, f32, [Pos2; 3]) {
    let center = egui::pos2(side * 0.5, side * 0.5);
    let radius = side * 0.46;
    let inner = radius * 0.78;
    let tri = [center + vec2(inner * 0.90, 0.0), center + vec2(-inner * 0.45, -inner * 0.7794), center + vec2(-inner * 0.45, inner * 0.7794)];
    (center, radius, inner, tri)
}

fn sample(mesh: &egui::Mesh, p: Pos2) -> [f32; 4] {
    for indices in mesh.indices.as_chunks::<3>().0 {
        let vertices = indices.map(|i| mesh.vertices[i as usize]);
        if let Some(w) = barycentric(p, vertices.map(|v| v.pos)) {
            return std::array::from_fn(|channel| vertices.iter().zip(w).map(|(v, w)| f32::from(v.color.to_array()[channel]) * w).sum());
        }
    }
    [0.0; 4]
}

fn barycentric(p: Pos2, tri: [Pos2; 3]) -> Option<[f32; 3]> {
    let [a, b, c] = tri;
    let cross = |a: Vec2, b: Vec2| a.x * b.y - a.y * b.x;
    let det = cross(b - a, c - a);
    if det.abs() < 1e-8 {
        return None;
    }
    let wb = cross(p - a, c - a) / det;
    let wc = cross(b - a, p - a) / det;
    let wa = 1.0 - wb - wc;
    [wa, wb, wc].iter().all(|w| *w >= -1e-5).then_some([wa, wb, wc])
}

#[test]
fn ring_fringe_has_physical_pixel_coverage_on_both_edges_at_every_supported_panel_scale() {
    for side in [120.0, 180.0, 240.0] {
        for ppp in [0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0] {
            let (center, radius, inner, _) = geometry(side);
            let mesh = ring_mesh(center, radius, inner, ppp);
            for edge in [radius, inner] {
                let alphas = [-0.75, -0.25, 0.0, 0.25, 0.75].map(|offset| sample(&mesh, center + vec2(edge + offset / ppp, 0.0))[3]);
                let expected = if edge == radius { [255.0, 191.25, 127.5, 63.75, 0.0] } else { [0.0, 63.75, 127.5, 191.25, 255.0] };
                for (actual, expected) in alphas.iter().zip(expected) {
                    assert!((actual - expected).abs() < 0.1, "side={side}, ppp={ppp}, edge={edge}: {alphas:?}");
                }
            }
            let count = ring_segments(radius, ppp);
            let sagitta_pixels = radius * ppp * (1.0 - (std::f32::consts::PI / count as f32).cos());
            assert!(sagitta_pixels <= 0.101, "polygon edge error {sagitta_pixels} at {ppp}");
            assert_eq!(mesh.vertices.len(), (count as usize + 1) * 4);
            assert_eq!(mesh.indices.len(), count as usize * 18);
            assert!(
                mesh.vertices.iter().filter(|v| v.color.a() == 0).all(|v| v.color == Color32::TRANSPARENT),
                "fringe must be premultiplied transparent, without a white halo"
            );
        }
    }
}

#[test]
fn triangle_edges_feather_without_changing_the_interior_rgb_or_hsva_mapping() {
    for side in [120.0, 240.0] {
        for ppp in [1.0, 1.25, 2.0, 3.0] {
            let (_, _, _, tri) = geometry(side);
            let mesh = triangle_mesh(tri, 0.37, ppp);
            assert_eq!(mesh.vertices.len(), 6);
            assert_eq!(mesh.indices.len(), 21);
            // The vertical white-to-black edge has no hue contribution. Its nominal
            // boundary must have half coverage, rather than an opaque staircase.
            let midpoint = tri[1] + (tri[2] - tri[1]) * 0.5;
            assert!((sample(&mesh, midpoint)[3] - 127.5).abs() < 0.1);
            assert_eq!(sample(&mesh, midpoint - vec2(0.75 / ppp, 0.0))[3], 0.0);
            assert!((sample(&mesh, midpoint + vec2(0.75 / ppp, 0.0))[3] - 255.0).abs() < 0.1);
            let point = tri[0] + (tri[1] - tri[0]) * 0.3 + (tri[2] - tri[0]) * 0.2;
            let actual = sample(&mesh, point);
            let hue = Color32::from(Hsva::new(0.37, 1.0, 1.0, 1.0)).to_array();
            for channel in 0..3 {
                let original_rgb = f32::from(hue[channel]) * 0.5 + 255.0 * 0.3;
                assert!((actual[channel] - original_rgb).abs() <= 1.0, "RGB field shifted: {actual:?}");
            }
            assert!((actual[3] - 255.0).abs() < 0.1);
            let [h, w, _] = weights(point, tri);
            let value = h + w;
            let saturation = h / value;
            assert!((value - 0.8).abs() < 1e-5);
            assert!((saturation - 0.625).abs() < 1e-5);
        }
    }
}

#[test]
fn ring_hue_seam_and_invalid_scale_fallback_remain_bounded() {
    let (center, radius, inner, _) = geometry(240.0);
    let mesh = ring_mesh(center, radius, inner, 2.0);
    let first = &mesh.vertices[..4];
    let last = &mesh.vertices[mesh.vertices.len() - 4..];
    for (a, b) in first.iter().zip(last) {
        assert!(a.pos.distance(b.pos) < 0.001);
        assert_eq!(a.color, b.color);
    }
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(pixel_size(scale), 1.0);
        assert_eq!(ring_segments(radius, scale), ring_segments(radius, 1.0));
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "exports actual wheel mesh raster evidence; set INKPOP_WHEEL_V85_OUT to a new directory"]
fn export_antialiased_wheel_mesh_evidence_v85() {
    let out = std::path::PathBuf::from(std::env::var_os("INKPOP_WHEEL_V85_OUT").expect("set INKPOP_WHEEL_V85_OUT"));
    assert!(!out.exists(), "preserve previous evidence");
    std::fs::create_dir_all(&out).unwrap();
    for ppp in [1.0, 1.25, 2.0] {
        let side = 240.0;
        let (center, radius, inner, tri) = geometry(side);
        let meshes = [ring_mesh(center, radius, inner, ppp), triangle_mesh(tri, 0.01, ppp)];
        let size = (side * ppp).round() as u32;
        let mut rgba = vec![0u8; size as usize * size as usize * 4];
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[244, 241, 232, 255]);
        }
        // Rasterize the actual production mesh at physical pixel centers. This
        // isolates coverage math; final native GPU screenshots remain separate.
        for mesh in meshes {
            for indices in mesh.indices.as_chunks::<3>().0 {
                let vertices = indices.map(|i| mesh.vertices[i as usize]);
                let positions = vertices.map(|v| v.pos);
                let min_x = positions.iter().map(|p| (p.x * ppp).floor() as i32).min().unwrap().max(0) as u32;
                let min_y = positions.iter().map(|p| (p.y * ppp).floor() as i32).min().unwrap().max(0) as u32;
                let max_x = positions.iter().map(|p| (p.x * ppp).ceil() as u32).max().unwrap().min(size);
                let max_y = positions.iter().map(|p| (p.y * ppp).ceil() as u32).max().unwrap().min(size);
                for y in min_y..max_y {
                    for x in min_x..max_x {
                        if let Some(w) = barycentric(egui::pos2((x as f32 + 0.5) / ppp, (y as f32 + 0.5) / ppp), positions) {
                            let color: [f32; 4] =
                                std::array::from_fn(|channel| vertices.iter().zip(w).map(|(v, w)| f32::from(v.color.to_array()[channel]) * w).sum());
                            let pixel = &mut rgba[((y * size + x) * 4) as usize..][..4];
                            for channel in 0..3 {
                                // Both meshes are disjoint; always composite on the same
                                // background so a shared-edge tie cannot blend twice.
                                let background = [244.0, 241.0, 232.0][channel];
                                pixel[channel] = (color[channel] + background * (1.0 - color[3] / 255.0)).round().clamp(0.0, 255.0) as u8;
                            }
                        }
                    }
                }
            }
        }
        let image = photocraft_codecs::Image::from_u8(size, size, photocraft_codecs::ChannelLayout::Rgba, rgba.clone()).unwrap();
        let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        assert_eq!(photocraft_codecs::decode(&png).unwrap().to_rgba8(), rgba);
        std::fs::write(out.join(format!("wheel-aa-{}ppp.png", (ppp * 100.0) as u32)), png).unwrap();
    }
}
