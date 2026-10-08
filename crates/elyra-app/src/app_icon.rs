//! The application icon: the Lyra constellation in Elyra yellow.

/// Brand yellow sampled from the icon.
pub const BRAND_YELLOW: u32 = 0xfbd22d;
/// Darker amber for text on light backgrounds.
pub const BRAND_AMBER: u32 = 0xa16207;

/// 256×256 PNG for in-app use (About).
pub const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon/icon-256.png");
/// 1024×1024 PNG for the Dock.
const DOCK_PNG: &[u8] = include_bytes!("../../../assets/icon/icon.png");

/// Show the icon in the Dock and app switcher. A bundled app gets it from its
/// `.icns`; this also covers `cargo run`, where there is no bundle.
#[cfg(target_os = "macos")]
pub fn install_dock_icon() {
    use objc2::AllocAnyThread as _;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(DOCK_PNG);
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        let app = NSApplication::sharedApplication(mtm);
        // SAFETY: called on the main thread with a valid image.
        unsafe { app.setApplicationIconImage(Some(&image)) };
    }
}

/// Show `count` on the Dock icon (hidden when zero).
#[cfg(target_os = "macos")]
pub fn set_badge(count: usize) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
    NSApplication::sharedApplication(mtm)
        .dockTile()
        .setBadgeLabel(label.as_deref());
}

/// No Dock icon or menu bar: for headless test runs.
#[cfg(target_os = "macos")]
pub fn hide_from_dock() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn hide_from_dock() {}

#[cfg(not(target_os = "macos"))]
pub fn set_badge(_count: usize) {}

#[cfg(not(target_os = "macos"))]
pub fn install_dock_icon() {
    let _ = DOCK_PNG;
}
