//! v90 regression against the original whole-float-collect implementation below.
//! Native output diagnostics track actual concurrent worker float buffers with shared
//! atomic counters and RAII. They are not process RAM, allocator or GPU peak memory.

use super::*;
use photocraft_color::{ColorMode, SampleType};
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Legacy metrics observe successful result vectors retained at whole-collect.
/// Native metrics count all live output float vectors, including discarded empty
/// results, from immediately before allocation until native conversion completes.
/// Neither measures source-footprint floats, allocator/process residency or GPU memory.
#[derive(Clone, Debug, Default)]
struct BatchStats {
    /// Native: output-buffer allocations. Legacy: one whole-collect observation.
    batches: usize,
    total_result_tiles: usize,
    /// Native: actual concurrent output float buffers. Legacy: retained result vectors.
    max_result_tiles: usize,
    peak_result_float_bytes: usize,
    total_result_float_bytes: usize,
    total_allocated_float_bytes: usize,
    live_float_buffers_after: usize,
    live_float_bytes_after: usize,
}

#[derive(Default)]
pub(super) struct OutputMeasurement {
    allocations: AtomicUsize,
    current_buffers: AtomicUsize,
    current_bytes: AtomicUsize,
    peak_buffers: AtomicUsize,
    peak_bytes: AtomicUsize,
    successful_buffers: AtomicUsize,
    successful_bytes: AtomicUsize,
    allocated_bytes: AtomicUsize,
}

pub(super) struct OutputFloatGuard {
    measurement: Arc<OutputMeasurement>,
    bytes: usize,
}

impl OutputMeasurement {
    /// Called before allocating outp. Guard is declared before outp, so reverse
    /// drop order frees its Vec before decreasing these test-only live counters.
    pub(super) fn allocate(self: &Arc<Self>, bytes: usize) -> OutputFloatGuard {
        self.allocations.fetch_add(1, Ordering::Relaxed);
        self.allocated_bytes.fetch_add(bytes, Ordering::Relaxed);
        let buffers = self.current_buffers.fetch_add(1, Ordering::Relaxed) + 1;
        let live = self.current_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.peak_buffers.fetch_max(buffers, Ordering::Relaxed);
        self.peak_bytes.fetch_max(live, Ordering::Relaxed);
        OutputFloatGuard { measurement: Arc::clone(self), bytes }
    }

    fn snapshot(&self) -> BatchStats {
        BatchStats {
            batches: self.allocations.load(Ordering::Relaxed),
            total_result_tiles: self.successful_buffers.load(Ordering::Relaxed),
            max_result_tiles: self.peak_buffers.load(Ordering::Relaxed),
            peak_result_float_bytes: self.peak_bytes.load(Ordering::Relaxed),
            total_result_float_bytes: self.successful_bytes.load(Ordering::Relaxed),
            total_allocated_float_bytes: self.allocated_bytes.load(Ordering::Relaxed),
            live_float_buffers_after: self.current_buffers.load(Ordering::Relaxed),
            live_float_bytes_after: self.current_bytes.load(Ordering::Relaxed),
        }
    }
}

impl OutputFloatGuard {
    pub(super) fn record_result(&self) {
        self.measurement.successful_buffers.fetch_add(1, Ordering::Relaxed);
        self.measurement.successful_bytes.fetch_add(self.bytes, Ordering::Relaxed);
    }
}

impl Drop for OutputFloatGuard {
    fn drop(&mut self) {
        self.measurement.current_bytes.fetch_sub(self.bytes, Ordering::Relaxed);
        self.measurement.current_buffers.fetch_sub(1, Ordering::Relaxed);
    }
}

thread_local! {
    static MEASURED_OUTPUT: RefCell<Option<Arc<OutputMeasurement>>> = const { RefCell::new(None) };
}

pub(super) fn current_measurement() -> Option<Arc<OutputMeasurement>> {
    MEASURED_OUTPUT.with_borrow(Clone::clone)
}

fn worker_limit() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rayon::current_num_threads()
    }
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
}

fn assert_worker_bound(stats: &BatchStats, channels: usize, label: &str) {
    let workers = worker_limit();
    assert!(stats.max_result_tiles <= workers, "{label}: output buffers exceed {workers} workers: {stats:?}");
    assert!(stats.peak_result_float_bytes <= workers * 256 * 256 * channels * std::mem::size_of::<f32>(), "{label}: {stats:?}");
    assert_eq!(stats.live_float_buffers_after, 0, "{label}: output guards survived the warp");
    assert_eq!(stats.live_float_bytes_after, 0, "{label}: output float bytes survived the warp");
}

fn statistics(done: &[(Rect, Vec<f32>)]) -> BatchStats {
    let bytes = done.iter().map(|(_, pixels)| std::mem::size_of_val(pixels.as_slice())).sum();
    BatchStats {
        batches: 1,
        total_result_tiles: done.len(),
        max_result_tiles: done.len(),
        peak_result_float_bytes: bytes,
        total_result_float_bytes: bytes,
        ..BatchStats::default()
    }
}

fn bounded_measured(src: &Surface, bounds: Rect, h: &Homography, interp: Interp) -> (Surface, BatchStats) {
    let measurement = Arc::new(OutputMeasurement::default());
    let previous = MEASURED_OUTPUT.with_borrow_mut(|slot| slot.replace(Arc::clone(&measurement)));
    let surface = warp_surface(src, bounds, h, interp);
    MEASURED_OUTPUT.with_borrow_mut(|slot| *slot = previous);
    (surface, measurement.snapshot())
}

fn patterned(format: PixelFormat, bounds: Rect) -> Surface {
    let mut surface = Surface::new(format);
    let n = format.channels();
    for y in bounds.y0..bounds.y1 {
        let mut row = Vec::with_capacity(bounds.width() as usize * n);
        for x in bounds.x0..bounds.x1 {
            let u = x - bounds.x0;
            let v = y - bounds.y0;
            for channel in 0..format.mode.color_channels() {
                row.push(((u * 17 + v * 11 + channel as i32 * 31).rem_euclid(251)) as f32 / 250.0);
            }
            if format.alpha {
                let alpha = if (u / 13 + v / 11) % 7 == 0 { 0.0 } else { ((u * 7 + v * 3).rem_euclid(19)) as f32 / 18.0 };
                row.push(alpha);
            }
        }
        surface.write_region(Rect::new(bounds.x0, y, bounds.x1, y + 1), &row);
    }
    surface.prune();
    surface
}

fn assert_native_equal(actual: &Surface, expected: &Surface, label: &str) {
    assert_eq!(actual.format(), expected.format(), "{label}: native format");
    assert_eq!(actual.default_pixel(), expected.default_pixel(), "{label}: default pixel");
    assert_eq!(actual.content_bounds(), expected.content_bounds(), "{label}: content bounds");
    let bounds = actual.content_bounds().union(&expected.content_bounds());
    let a = actual.to_interleaved(bounds);
    let e = expected.to_interleaved(bounds);
    assert_eq!(a.len(), e.len(), "{label}: byte count");
    let first_difference = a.iter().zip(&e).position(|(a, e)| a != e);
    assert_eq!(first_difference, None, "{label}: first unequal native byte");
    assert_eq!(actual.tile_count(), expected.tile_count(), "{label}: sparse tile count");
}

fn maps(bounds: Rect) -> Vec<(&'static str, Homography)> {
    let cx = (f64::from(bounds.x0) + f64::from(bounds.x1)) / 2.0;
    let cy = (f64::from(bounds.y0) + f64::from(bounds.y1)) / 2.0;
    let angle = 19.0f64.to_radians();
    let (s, c) = angle.sin_cos();
    vec![
        ("identity", Homography::IDENTITY),
        ("negative-cross-tile-translation", Homography([1.0, 0.0, -137.0, 0.0, 1.0, 79.0, 0.0, 0.0, 1.0])),
        ("rotate", Homography([c, -s, cx - c * cx + s * cy, s, c, cy - s * cx - c * cy, 0.0, 0.0, 1.0])),
        ("enlarge", Homography([1.3, 0.0, -31.0, 0.0, 1.3, 17.0, 0.0, 0.0, 1.0])),
        ("prefilter-shrink", Homography([0.37, 0.0, 17.0, 0.0, 0.37, -23.0, 0.0, 0.0, 1.0])),
    ]
}

#[test]
fn batched_warp_matches_legacy_native_bytes_all_depths_and_interpolations_v90() {
    // Keep the candidate3 large fixture (>64 source tiles) and every strict native
    // byte comparison. Workers now encode directly into independently owned native tiles.
    let bounds = Rect::new(-1025, -769, 1024, 769);
    for sample in SampleType::ALL {
        let source = patterned(PixelFormat::new(ColorMode::Rgb, sample, true), bounds);
        for (name, h) in maps(bounds) {
            for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
                let (expected, old_stats) = legacy_collect(&source, bounds, &h, interp);
                let (actual, stats) = bounded_measured(&source, bounds, &h, interp);
                let label = format!("{sample:?}/{name}/{interp:?}");
                assert_native_equal(&actual, &expected, &label);
                assert_eq!(stats.total_result_tiles, old_stats.total_result_tiles, "{label}: result tiles");
                assert_eq!(stats.total_result_float_bytes, old_stats.total_result_float_bytes, "{label}: all float results");
                assert_worker_bound(&stats, actual.channels(), &label);
                if name == "identity" {
                    assert!(stats.batches > 64, "{label}: fixture must allocate more than 64 output buffers");
                    assert!(old_stats.max_result_tiles > 64, "{label}: legacy fixture must retain more than 64 results");
                    // A core count larger than the complete fixture can legitimately
                    // allocate every result concurrently; do not invent a fixed MiB cap.
                    if old_stats.peak_result_float_bytes > worker_limit() * 256 * 256 * actual.channels() * 4 {
                        assert!(stats.peak_result_float_bytes < old_stats.peak_result_float_bytes, "{label}: {stats:?} versus {old_stats:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn batched_warp_preserves_alpha_insertion_modes_and_sparse_tiles_v90() {
    let bounds = Rect::new(-260, -130, 271, 377);
    for format in [
        PixelFormat::GRAY8,
        PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true),
        PixelFormat::new(ColorMode::Cmyk, SampleType::U16, true),
        PixelFormat::new(ColorMode::Rgb, SampleType::U16, false),
    ] {
        let mut source = patterned(format, bounds);
        // Large cleared holes, including a whole tile, exercise skipped result buffers.
        source.fill_rect(Rect::new(0, 0, 256, 256), &vec![0.0; format.channels()]);
        source.prune();
        let h = Homography([1.07, 0.08, -73.0, -0.08, 1.07, -29.0, 0.0, 0.0, 1.0]);
        let (expected, old_stats) = legacy_collect(&source, bounds, &h, Interp::Bicubic);
        let (actual, stats) = bounded_measured(&source, bounds, &h, Interp::Bicubic);
        assert_native_equal(&actual, &expected, &format!("{format:?}"));
        assert!(actual.format().alpha);
        assert_eq!(stats.total_result_tiles, old_stats.total_result_tiles);
        assert_eq!(stats.total_result_float_bytes, old_stats.total_result_float_bytes);
        assert_worker_bound(&stats, actual.channels(), &format!("{format:?}"));
    }
}

#[test]
fn batched_warp_keeps_empty_and_degenerate_results_v90() {
    let mut source = Surface::new(PixelFormat::RGBA16);
    source.fill_rect(Rect::new(-20, -10, 20, 10), &[0.3, 0.6, 0.9, 0.7]);
    for (bounds, h) in
        [(Rect::EMPTY, Homography::IDENTITY), (Rect::new(-20, -10, 20, 10), Homography([0.0; 9])), (Rect::new(1000, 1000, 2000, 2000), Homography::IDENTITY)]
    {
        let (expected, _) = legacy_collect(&source, bounds, &h, Interp::Bilinear);
        let (actual, stats) = bounded_measured(&source, bounds, &h, Interp::Bilinear);
        assert_native_equal(&actual, &expected, "empty/degenerate");
        assert_worker_bound(&stats, actual.channels(), "empty/degenerate");
    }
}

#[test]
fn batched_warp_output_counter_raii_counts_overlap_and_success_v90() {
    let counter = Arc::new(OutputMeasurement::default());
    {
        let first = counter.allocate(128);
        {
            let second = counter.allocate(256);
            second.record_result();
            let snapshot = counter.snapshot();
            assert_eq!(snapshot.max_result_tiles, 2);
            assert_eq!(snapshot.peak_result_float_bytes, 384);
            assert_eq!(snapshot.total_result_tiles, 1);
            assert_eq!(snapshot.total_result_float_bytes, 256);
        }
        assert_eq!(counter.snapshot().live_float_buffers_after, 1);
        first.record_result();
    }
    let stats = counter.snapshot();
    assert_eq!(stats.batches, 2);
    assert_eq!(stats.total_result_tiles, 2);
    assert_eq!(stats.total_result_float_bytes, 384);
    assert_eq!(stats.total_allocated_float_bytes, 384);
    assert_eq!(stats.live_float_buffers_after, 0);
    assert_eq!(stats.live_float_bytes_after, 0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn batched_warp_output_float_bound_uses_actual_rayon_worker_count_v90() {
    let bounds = Rect::new(-273, -151, 270, 270);
    let source = patterned(PixelFormat::RGBA16, bounds);
    for workers in [1, 3] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(workers).build().expect("small isolated diagnostic pool");
        pool.install(|| {
            assert_eq!(worker_limit(), workers);
            let (expected, old_stats) = legacy_collect(&source, bounds, &Homography::IDENTITY, Interp::Bilinear);
            let (actual, stats) = bounded_measured(&source, bounds, &Homography::IDENTITY, Interp::Bilinear);
            assert_native_equal(&actual, &expected, &format!("{workers} workers"));
            assert_worker_bound(&stats, actual.channels(), &format!("{workers} workers"));
            assert_eq!(stats.total_result_tiles, old_stats.total_result_tiles);
            assert_eq!(stats.total_result_float_bytes, old_stats.total_result_float_bytes);
            assert!(stats.peak_result_float_bytes < old_stats.peak_result_float_bytes, "{stats:?} vs {old_stats:?}");
        });
    }
}

fn surface_digest(surface: &Surface) -> u64 {
    let bounds = surface.content_bounds();
    let mut hash = 0xcbf29ce484222325u64;
    for y in (bounds.y0..bounds.y1).step_by(8) {
        let band = Rect::new(bounds.x0, y, bounds.x1, (y + 8).min(bounds.y1));
        for byte in surface.to_interleaved(band) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    hash
}

#[test]
#[ignore = "24MP release-only legacy/bounded diagnostic; run without competing workloads"]
fn warp_output_24mp_measurement_v90() {
    let bounds = Rect::new(0, 0, 6000, 4000);
    let mut source = Surface::new(PixelFormat::RGBA8);
    source.fill_rect(bounds, &[0.17, 0.41, 0.73, 0.63]);
    source.fill_rect(Rect::new(400, 300, 1800, 2500), &[0.91, 0.2, 0.09, 0.85]);
    source.fill_rect(Rect::new(2700, 1300, 4900, 3300), &[0.08, 0.75, 0.35, 0.28]);
    let h = Homography([1.1, 0.0, -17.0, 0.0, 1.1, 23.0, 0.0, 0.0, 1.0]);
    let variant = std::env::var("INKPOP_WARP_BENCH_VARIANT").unwrap_or_else(|_| "both".into());
    assert!(matches!(variant.as_str(), "legacy" | "bounded" | "both"), "unknown benchmark variant");
    let mut results = Vec::new();
    for name in ["legacy", "bounded"].into_iter().filter(|name| variant == "both" || variant == *name) {
        let started = std::time::Instant::now();
        let (surface, stats) =
            if name == "legacy" { legacy_collect(&source, bounds, &h, Interp::Bilinear) } else { bounded_measured(&source, bounds, &h, Interp::Bilinear) };
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let digest = surface_digest(&surface);
        println!(
            "{}",
            serde_json::json!({"variant": name, "inputPixels": 24000000,
            "elapsedMs": elapsed_ms, "outputBounds": [surface.content_bounds().x0, surface.content_bounds().y0,
                surface.content_bounds().x1, surface.content_bounds().y1],
            "resultTiles": stats.total_result_tiles, "observationCount": stats.batches,
            "maxRetainedResultTiles": stats.max_result_tiles, "peakRetainedOutputFloatBytes": stats.peak_result_float_bytes,
            "rayonWorkersOrWasmOne": worker_limit(), "totalAllocatedOutputFloatBytes": stats.total_allocated_float_bytes,
            "successfulOutputFloatBytes": stats.total_result_float_bytes,
            "liveOutputBuffersAfter": stats.live_float_buffers_after, "liveOutputFloatBytesAfter": stats.live_float_bytes_after,
            "metricKind": if name == "legacy" { "successful output floats retained after complete collect" } else { "actual concurrent output float buffers including discarded empty results, allocation through native encoding" },
            "digestFnv1a64": format!("{digest:016x}"),
            "scope": "Output Vec<f32> payload diagnostic only; not process peak RAM, sampling footprints, allocator residency, native destination, prefilter buffers or GPU memory"})
        );
        results.push((surface.content_bounds(), digest));
        if name == "bounded" {
            assert_worker_bound(&stats, surface.channels(), "24MP native worker output");
        }
    }
    if results.len() == 2 {
        assert_eq!(results[0], results[1], "legacy and bounded output bounds and diagnostic FNV digest");
    }
}

// Exact pre-batching production function, with only its return augmented to report
// the complete retained result buffer size before native writes.
fn legacy_collect(src: &Surface, src_rect: Rect, h: &Homography, interp: Interp) -> (Surface, BatchStats) {
    let mut fmt = src.format();
    let converted;
    let mut src = if fmt.alpha {
        src
    } else {
        fmt = PixelFormat::new(fmt.mode, fmt.sample, true);
        converted = src.convert(fmt);
        &converted
    };
    let mut out = Surface::new(fmt);
    if src_rect.is_empty() {
        return (out, BatchStats::default());
    }
    let mut h = *h;
    let mut src_rect = src_rect;
    // Pre-reduce for large downscales (bicubic alone aliases below ~50%).
    let (cx, cy) = ((src_rect.x0 + src_rect.x1) as f64 / 2.0, (src_rect.y0 + src_rect.y1) as f64 / 2.0);
    let scale = h.local_scale(cx, cy);
    let reduced;
    if interp != Interp::Nearest && scale < 0.5 && scale > 0.0 {
        let f = 2f64.powf(scale.log2().ceil()).min(1.0); // remaining scale lands in [0.5, 1)
        reduced = resize_surface(src, f, f, Resample::Bicubic);
        src = &reduced;
        h = h.mul(&Homography([1.0 / f, 0.0, 0.0, 0.0, 1.0 / f, 0.0, 0.0, 0.0, 1.0]));
        src_rect = crate::resample::scaled_rect(src_rect, f, f);
    }
    let Some(inv) = h.inverse() else { return (out, BatchStats::default()) };
    // Destination bounds: the warped corners (plus a pixel for filter support).
    let corners = [(src_rect.x0, src_rect.y0), (src_rect.x1, src_rect.y0), (src_rect.x1, src_rect.y1), (src_rect.x0, src_rect.y1)]
        .map(|(x, y)| h.apply(x as f64, y as f64));
    if corners.iter().any(|c| !c.0.is_finite() || !c.1.is_finite()) {
        return (out, BatchStats::default());
    }
    let lim = 1 << 20;
    let bx0 = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor().max(-(lim as f64)) as i32 - 1;
    let by0 = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor().max(-(lim as f64)) as i32 - 1;
    let bx1 = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil().min(lim as f64) as i32 + 1;
    let by1 = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil().min(lim as f64) as i32 + 1;
    let dst = Rect::new(bx0, by0, bx1, by1);
    let n = fmt.channels();
    let a = n - 1;
    let tiles: Vec<Rect> = dst.tiles().map(|tc| tc.rect().intersect(&dst)).filter(|r| !r.is_empty()).collect();
    let src_ref: &Surface = src;
    let work = |t: &Rect| -> Option<(Rect, Vec<f32>)> {
        // Source footprint of this tile (inverse-mapped corners), padded for the filter.
        let tc = [(t.x0, t.y0), (t.x1, t.y0), (t.x1, t.y1), (t.x0, t.y1)].map(|(x, y)| inv.apply(x as f64, y as f64));
        let fx0 = tc.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor() as i32 - 3;
        let fy0 = tc.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor() as i32 - 3;
        let fx1 = tc.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil() as i32 + 3;
        let fy1 = tc.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil() as i32 + 3;
        let foot = Rect::new(fx0, fy0, fx1, fy1).intersect(&src_rect);
        if foot.is_empty() || !src_ref.has_tiles_in(foot) {
            return None;
        }
        // Premultiplied source window.
        let mut px = src_ref.read_region(foot);
        for p in px.chunks_exact_mut(n) {
            let al = p[a];
            for v in &mut p[..a] {
                *v *= al;
            }
        }
        let fw = foot.width() as usize;
        let at = |x: i32, y: i32, c: usize| -> f64 {
            if x < foot.x0 || y < foot.y0 || x >= foot.x1 || y >= foot.y1 {
                0.0
            } else {
                px[((y - foot.y0) as usize * fw + (x - foot.x0) as usize) * n + c] as f64
            }
        };
        let w = t.width() as usize;
        let mut outp = vec![0.0f32; w * t.height() as usize * n];
        let mut any = false;
        let mut acc = [0.0f64; 8];
        for y in t.y0..t.y1 {
            for x in t.x0..t.x1 {
                let (u, v) = inv.apply(x as f64 + 0.5, y as f64 + 0.5);
                let (u, v) = (u - 0.5, v - 0.5);
                if u < src_rect.x0 as f64 - 1.0 || v < src_rect.y0 as f64 - 1.0 || u > src_rect.x1 as f64 || v > src_rect.y1 as f64 {
                    continue;
                }
                acc[..n].fill(0.0);
                match interp {
                    Interp::Nearest => {
                        let (ix, iy) = ((u + 0.5).floor() as i32, (v + 0.5).floor() as i32);
                        for (c, s) in acc.iter_mut().enumerate().take(n) {
                            *s = at(ix, iy, c);
                        }
                    }
                    Interp::Bilinear => {
                        let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                        let (fx, fy) = (u - ix as f64, v - iy as f64);
                        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                                for (c, s) in acc.iter_mut().enumerate().take(n) {
                                    *s += at(ix + dx, iy + dy, c) * wx * wy;
                                }
                            }
                        }
                    }
                    Interp::Bicubic => {
                        let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                        let (wx, wy) = (catmull_rom(u - ix as f64), catmull_rom(v - iy as f64));
                        for (j, wyj) in wy.iter().enumerate() {
                            for (i, wxi) in wx.iter().enumerate() {
                                let k = wxi * wyj;
                                for (c, s) in acc.iter_mut().enumerate().take(n) {
                                    *s += at(ix - 1 + i as i32, iy - 1 + j as i32, c) * k;
                                }
                            }
                        }
                    }
                }
                let al = acc[a].clamp(0.0, 1.0);
                if al <= 0.0 {
                    continue;
                }
                any = true;
                let o = ((y - t.y0) as usize * w + (x - t.x0) as usize) * n;
                for c in 0..a {
                    outp[o + c] = (acc[c] / al).clamp(0.0, 1.0) as f32;
                }
                outp[o + a] = al as f32;
            }
        }
        any.then_some((*t, outp))
    };
    #[cfg(not(target_arch = "wasm32"))]
    let done: Vec<(Rect, Vec<f32>)> = {
        use rayon::prelude::*;
        tiles.par_iter().filter_map(work).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let done: Vec<(Rect, Vec<f32>)> = tiles.iter().filter_map(work).collect();
    let stats = statistics(&done);
    for (r, v) in done {
        out.write_region(r, &v);
    }
    out.prune();
    (out, stats)
}
