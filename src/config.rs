pub const APP_ID: &str = "io.github.sachesi.rill";
pub const RESOURCE_PATH: &str = "/io/github/sachesi/rill";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GETTEXT_PACKAGE: &str = "rill";
/// Where the installed catalogues are, `$prefix/share/locale`. Set `RILL_LOCALEDIR` when
/// building for a prefix other than `/usr/local`.
pub const LOCALEDIR: &str = match option_env!("RILL_LOCALEDIR") {
    Some(v) => v,
    None => "/usr/local/share/locale",
};
