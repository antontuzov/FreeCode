//! The duck this launch wears, decided once by the host and read by everyone else.
//!
//! The mascot already picked one of the twelve at random, but only inside the webview, so the Dock
//! tile stayed the build-time bonsai in `icons/icon.icns`: the running app showed a duck and its own
//! tile showed a tree. The pick therefore moved here, and the window asks the host for the number
//! rather than rolling its own.
//!
//! Two things this cannot promise, said rather than quietly skipped:
//!
//! * Tauri 2.12 has no `set_dock_icon`; the only way to repaint the tile is AppKit's
//!   `setApplicationIconImage`, so it happens on macOS and nowhere else. Linux and Windows keep the
//!   built icon.
//! * A tile chosen at every launch cannot be baked into a bundle, so Finder's icon for the app on
//!   disk stays the drawn bonsai. Only the running app's Dock tile is the mascot.
//!
//! The tiles are 256px round PNGs on transparent margin — the Dock clips nothing, so a square tile
//! would show black corners among the app's own icons — and they are compiled into the binary rather
//! than bundled as resources: `NSImage` decodes the bytes it is handed, so no path has to be resolved
//! at runtime. Round costs alpha and alpha costs PNG: the twelve are 1.1 MB here where the same
//! crops as opaque JPEG weighed 151 KB.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The twelve tiles, numbered the way `?mascot=<1..12>` names them, cut from the sheet by
/// `scripts/render-mascot-logos.py` alongside the webp set the webview loads.
const TILES: &[&[u8]] = &[
    include_bytes!("../mascots/mascot-01.png"),
    include_bytes!("../mascots/mascot-02.png"),
    include_bytes!("../mascots/mascot-03.png"),
    include_bytes!("../mascots/mascot-04.png"),
    include_bytes!("../mascots/mascot-05.png"),
    include_bytes!("../mascots/mascot-06.png"),
    include_bytes!("../mascots/mascot-07.png"),
    include_bytes!("../mascots/mascot-08.png"),
    include_bytes!("../mascots/mascot-09.png"),
    include_bytes!("../mascots/mascot-10.png"),
    include_bytes!("../mascots/mascot-11.png"),
    include_bytes!("../mascots/mascot-12.png"),
];

/// The mascot for this process, one-based so the number the Dock was drawn from is the number the
/// window asks for. Fixed on first use: a face that changes while you are looking at it is a glitch.
pub fn tile() -> usize {
    static PICK: OnceLock<usize> = OnceLock::new();
    *PICK.get_or_init(|| {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let seed = since_epoch.as_nanos() as u64
            ^ (u64::from(std::process::id())).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        // A wall clock's low bits track the time of day, so the value is mixed before the modulo —
        // otherwise launches taken a fraction apart would keep landing on the same duck.
        let mut x = seed;
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        (x % TILES.len() as u64) as usize + 1
    })
}

/// Run before the page's own scripts, so the window's mascot and the Dock's tile are the same
/// picture on the first frame. An IPC call the UI awaited would paint some other duck for a moment
/// before the answer arrived.
pub fn init_script() -> String {
    format!("window.smollmMascot = {};", tile())
}

/// Repaint the Dock tile. Never fails the launch: the built icon is a perfectly good face, and a
/// mascot that cannot be drawn is not worth an app that does not open.
#[cfg(target_os = "macos")]
pub fn set_dock_tile(app: &tauri::AppHandle) {
    let index = tile();
    let apply = move || {
        // The marker is AppKit's own proof that this is the thread it owns; the Dock answers to no
        // other, and nothing here may assume which thread `setup` ran on.
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        // The number came out of `tile()`, which only ever names a tile that ships.
        let data = objc2_foundation::NSData::with_bytes(TILES[index - 1]);
        let Some(image) =
            objc2_app_kit::NSImage::initWithData(mtm.alloc::<objc2_app_kit::NSImage>(), &data)
        else {
            return;
        };
        unsafe {
            objc2_app_kit::NSApplication::sharedApplication(mtm)
                .setApplicationIconImage(Some(&image));
        }
    };
    if objc2::MainThreadMarker::new().is_some() {
        apply();
    } else if app.run_on_main_thread(apply).is_err() {
        // The event loop is gone already; there is no tile left to set.
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_dock_tile(_: &tauri::AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_wears_one_of_the_twelve_and_never_changes_its_mind() {
        let first = tile();
        assert!(
            (1..=TILES.len()).contains(&first),
            "the pick must name a tile that ships: {first}"
        );
        assert_eq!(first, tile(), "the face is chosen per launch, not per look");
    }

    #[test]
    fn the_script_names_the_same_duck_the_dock_does() {
        assert_eq!(init_script(), format!("window.smollmMascot = {};", tile()));
    }
}
