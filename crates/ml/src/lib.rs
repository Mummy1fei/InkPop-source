//! Local promptable segmentation. No network access or document mutation.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(not(target_arch = "wasm32"))]
pub mod mobile_sam;

#[cfg(not(target_arch = "wasm32"))]
pub mod u2net;

#[cfg(all(not(target_arch = "wasm32"), feature = "onnx"))]
pub mod onnx_segment;

#[cfg(all(not(target_arch = "wasm32"), feature = "onnx"))]
pub mod sam2_onnx;
