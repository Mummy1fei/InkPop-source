# InkPop desktop scroll policy (v70)

Unmodified source: egui 0.36.2, upstream commit 49682f8baa058bf49e011035cfbd6e825f88a5ef, crates/egui.
The existing registry source is copied byte-for-byte before the local change.
MIT and Apache license bytes were fetched from that exact upstream commit and
match the already bundled eframe license texts from the same repository commit.

The only Rust change is ScrollSource::default and its documentation: native
desktop drag scrolling is Never; wasm32 retains OnTouch. Wheel and scrollbar
handling, explicit widget sources, IDs, layout and image/canvas input are unchanged.
This covers dependency-internal ComboBox scroll areas as well as application forms.
The application wrappers additionally enforce their desktop policy on existing
explicit ScrollSource::ALL callers. No tablet/touch input is discarded or remapped.
