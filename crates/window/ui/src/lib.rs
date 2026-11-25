use std::borrow::Cow;
use std::process::Command;

use gpui_kit::App;

pub mod colour;
pub mod fonts;
pub mod metrics;
pub mod pieces;
pub mod themes;
pub mod tokens;
pub mod typography;

/// Registers the built-in fonts and checks the system's reduced-motion setting off the main
/// thread. The theme is applied by whoever owns the appearance.
pub fn init(cx: &mut App) {
    let built_in = fonts::BUILT_IN.iter().map(|bytes| Cow::Borrowed(*bytes));
    if let Err(error) = fonts::register(cx, built_in.collect()) {
        eprintln!("yaar-review: {error}");
    }
    if env_requests_reduced_motion() {
        cx.set_reduce_motion(true);
        return;
    }
    let check = cx
        .background_executor()
        .spawn(async move { system_reduces_motion() });
    cx.spawn(async move |cx| {
        if check.await {
            cx.update(|cx| cx.set_reduce_motion(true));
        }
    })
    .detach();
}

fn env_requests_reduced_motion() -> bool {
    std::env::var("YAAR_REDUCE_MOTION").is_ok_and(|value| parse_macos_reduce_motion(&value))
        || std::env::var("GTK_ENABLE_ANIMATIONS").is_ok_and(|value| value.trim() == "0")
}

fn system_reduces_motion() -> bool {
    #[cfg(target_os = "macos")]
    {
        Command::new("defaults")
            .args(["read", "com.apple.universalaccess", "reduceMotion"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_some_and(|output| {
                parse_macos_reduce_motion(&String::from_utf8_lossy(&output.stdout))
            })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_some_and(|output| {
                parse_linux_animations_disabled(&String::from_utf8_lossy(&output.stdout))
            })
    }
}

fn parse_macos_reduce_motion(output: &str) -> bool {
    matches!(output.trim(), "1" | "true" | "TRUE" | "yes" | "YES")
}

#[cfg(any(test, not(target_os = "macos")))]
fn parse_linux_animations_disabled(output: &str) -> bool {
    output.trim() == "false"
}

#[cfg(test)]
mod tests {
    use super::{parse_linux_animations_disabled, parse_macos_reduce_motion};

    #[test]
    fn system_reduced_motion_outputs_parse_for_macos_and_linux() {
        assert!(parse_macos_reduce_motion("1\n"));
        assert!(parse_macos_reduce_motion("true"));
        assert!(!parse_macos_reduce_motion("0\n"));
        assert!(!parse_macos_reduce_motion(""));

        assert!(parse_linux_animations_disabled("false\n"));
        assert!(!parse_linux_animations_disabled("true\n"));
        assert!(!parse_linux_animations_disabled(""));
    }
}
