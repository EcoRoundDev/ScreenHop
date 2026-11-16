#[macro_use]
pub mod macros;

pub mod config;
pub mod monitor;
pub mod updater;

// Re-export platform 类型供外部使用
pub use screenhop_platform::{MonitorInfo, Point, Rect};

pub const fn is_dev_build() -> bool {
    cfg!(all(target_os = "macos", feature = "macos-dev"))
}

pub const fn app_id() -> &'static str {
    if is_dev_build() {
        "com.ecorounddev.screenhop.dev"
    } else {
        "com.ecorounddev.screenhop"
    }
}

pub const fn app_name() -> &'static str {
    if is_dev_build() {
        "ScreenHop Dev"
    } else {
        "ScreenHop"
    }
}
