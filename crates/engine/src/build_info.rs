//! Version and build provenance shared by every frontend (About dialog, `--version`).
//!
//! The version is the workspace version (`[workspace.package] version` in the root
//! `Cargo.toml`, managed by `cargo xtask version`). The commit and build date are optional and
//! come from environment variables at compile time, set by CI and the packaging scripts:
//!
//! - `PHOTOCRAFT_BUILD_SHA`: the git commit (any length; shown shortened to 9 characters)
//! - `PHOTOCRAFT_BUILD_DATE`: the build date, conventionally `YYYY-MM-DD`
//! - `PHOTOCRAFT_CUSTOM_VERSION`: the custom distribution's release identifier, such as
//!   `V2.0-v66`. Packaging should set this once for both the build and the release directory.
//!
//! Without them (a plain `cargo build`) the build reports itself as a dev build. Nothing here
//! shells out to git, so builds work from a source tarball and for wasm.

/// The workspace version, such as `0.1.0` or `0.2.0-rc.1`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Product identity used by the customised desktop distribution. Keep the upstream crate
/// version above intact so file formats and dependency metadata continue to report PhotoCraft
/// 0.5.0, while the About page can show the application's own iteration number.
pub const CUSTOM_PRODUCT_NAME: &str = "InkPop";
/// The same distribution name is used in Chinese.
pub const CUSTOM_PRODUCT_NAME_ZH: &str = CUSTOM_PRODUCT_NAME;
/// Display name for window titles and the About dialog.
pub const CUSTOM_PRODUCT_TITLE: &str = CUSTOM_PRODUCT_NAME;
/// Source checkout's current custom release. Official packages override this at compile time
/// with `PHOTOCRAFT_CUSTOM_VERSION`, rather than maintaining a separate About-only version.
pub const CUSTOM_VERSION: &str = "V2.0-v121";

/// The actual custom release compiled by the distribution's packaging process.
pub fn custom_version() -> &'static str {
    custom_version_from(option_env!("PHOTOCRAFT_CUSTOM_VERSION"))
}

fn custom_version_from(packaged: Option<&'static str>) -> &'static str {
    packaged.map(str::trim).filter(|version| !version.is_empty()).unwrap_or(CUSTOM_VERSION)
}

/// The full git commit this build was made from, when the build environment recorded it.
pub fn git_sha() -> Option<&'static str> {
    option_env!("PHOTOCRAFT_BUILD_SHA").map(str::trim).filter(|s| !s.is_empty())
}

/// The build date (`YYYY-MM-DD`), when the build environment recorded it.
pub fn build_date() -> Option<&'static str> {
    option_env!("PHOTOCRAFT_BUILD_DATE").map(str::trim).filter(|s| !s.is_empty())
}

/// The commit shortened for display.
pub fn short_sha() -> Option<&'static str> {
    git_sha().map(|s| &s[..s.len().min(9)])
}

/// `0.1.0 (3f2a9c1d0, 2026-10-01)`, or `0.1.0 (dev build)` without provenance.
pub fn long_version() -> String {
    format_long(VERSION, short_sha(), build_date())
}

fn format_long(version: &str, sha: Option<&str>, date: Option<&str>) -> String {
    match (sha, date) {
        (Some(s), Some(d)) => format!("{version} ({s}, {d})"),
        (Some(s), None) => format!("{version} ({s})"),
        (None, Some(d)) => format!("{version} ({d})"),
        (None, None) => format!("{version} (dev build)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_version_formats() {
        assert_eq!(format_long("1.2.3", Some("abc"), Some("2026-10-01")), "1.2.3 (abc, 2026-10-01)");
        assert_eq!(format_long("1.2.3", Some("abc"), None), "1.2.3 (abc)");
        assert_eq!(format_long("1.2.3", None, Some("2026-10-01")), "1.2.3 (2026-10-01)");
        assert_eq!(format_long("1.2.3-rc.1", None, None), "1.2.3-rc.1 (dev build)");
    }

    #[test]
    fn version_is_the_workspace_version() {
        assert!(!VERSION.is_empty());
        assert!(long_version().starts_with(VERSION));
        if let Some(s) = short_sha() {
            assert!(s.len() <= 9);
        }
    }

    #[test]
    fn custom_distribution_identity_is_stable() {
        assert_eq!(CUSTOM_PRODUCT_NAME, "InkPop");
        assert_eq!(CUSTOM_PRODUCT_NAME_ZH, "InkPop");
        assert_eq!(CUSTOM_PRODUCT_TITLE, "InkPop");
        assert_eq!(CUSTOM_VERSION, "V2.0-v121");
        assert!(!custom_version().is_empty());
    }

    #[test]
    fn packaged_custom_version_overrides_the_source_default() {
        assert_eq!(custom_version_from(Some("V2.0-v66")), "V2.0-v66");
        assert_eq!(custom_version_from(Some("  V2.0-v66  ")), "V2.0-v66");
        assert_eq!(custom_version_from(Some("   ")), CUSTOM_VERSION);
        assert_eq!(custom_version_from(None), CUSTOM_VERSION);
    }
}
