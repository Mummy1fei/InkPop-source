# InkPop local egui-winit 0.36.2 patch (v84)

Upstream: Emil Ernerfeldt and contributors, egui monorepo,
<https://github.com/emilk/egui/tree/49682f8baa058bf49e011035cfbd6e825f88a5ef/crates/egui-winit>.
License: MIT OR Apache-2.0; license texts retained alongside the source.

The starting source is byte-identical to the installed cached crates.io archive
whose SHA-256 matches the pre-v84 Cargo.lock checksum:
`98466000559d66ba4db786a30448b01483946add4188e7f10f522a27b4fb89cc`.
No registry source was modified. Full source and license hashes are recorded in
`evidence/v84/pointer-backend/source-provenance.json`.

Local changes:

- Cache native bitmap application as well as creation. Retain the source Arc and
  compare image dimensions and hotspot, preventing pointer-address reuse errors.
- Keep each cache scoped to its own State/window. Reapply after lifecycle events,
  named/hidden icon transitions, and explicit external visibility invalidation.
- Expose State::invalidate_cursor for eframe's separate viewport-command path.
- Add deterministic cache and lifecycle tests. They measure decisions/call counts,
  not actual OS execution time or Wacom device-to-display latency.
- Cargo's license include entries now name the license files in this directory.

eframe's associated patch submits immediate-view platform output before rendering,
matching its ROOT path. Geometry, focus, screenshot and viewport-command timing
are otherwise retained. Native Windows WM_SETCURSOR still restores each window's
selected cursor on returning from native menus/nonclient resize regions.
