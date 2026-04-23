pub const DISPLAY_VERSION: &str = match option_env!("AGTOP_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

pub fn display_version() -> &'static str {
    DISPLAY_VERSION
}
