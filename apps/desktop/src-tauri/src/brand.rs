//! The duck this launch wears, decided once by the host and read by everyone else.
//!
//! The mascot already picked one of the twelve at random, but only inside the webview, so the Dock
//! tile stayed the one duck baked into the bundle at `icons/icon.icns`: the running app could show
//! any of the twelve and its own tile always showed the forest one. The pick therefore moved here,
//! and the window asks the host for the number rather than rolling its own.
//!
//! Settings › Appearance can pin one of the set instead of rolling, which is the same number stored
//! in the settings document; the roll is what happens when nothing is pinned.
//!
//! Two things this cannot promise, said rather than quietly skipped:
//!
//! * Tauri 2.12 has no `set_dock_icon`; the only way to repaint the tile is AppKit's
//!   `setApplicationIconImage`, so it happens on macOS and nowhere else. Linux and Windows keep the
//!   built icon.
//! * A tile chosen at every launch cannot be baked into a bundle, so Finder's icon for the app on
//!   disk is the forest duck, not the tile this launch rolled. Only the running app's Dock tile is
//!   the rolled mascot.
//!
//! The ducks are 256px round PNGs on transparent margin — the Dock clips nothing, so a square tile
//! would show black corners among the app's own icons — and they are compiled into the binary rather
//! than bundled as resources: `NSImage` decodes the bytes it is handed, so no path has to be resolved
//! at runtime. Round costs alpha and alpha costs PNG: the twelve are 1.1 MB here where the same
//! crops as opaque JPEG weighed 151 KB. The two wave tiles are cut at the same measure, so the set
//! stays one set, and weigh about 60 KB between them because each is one mark on one flat ground
//! rather than a painted head.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The tiles, numbered the way `?mascot=<1..14>` names them: the twelve ducks, cut from the sheet by
/// `scripts/render-mascot-logos.py` alongside the webp set the webview loads, then the wave mark on
/// the two grounds a Dock can have, cut by `scripts/render-wave-marks.py`.
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
    include_bytes!("../mascots/mascot-13.png"),
    include_bytes!("../mascots/mascot-14.png"),
];

/// The mascot for this process, one-based so the number the Dock was drawn from is the number the
/// window asks for. Fixed on first use: a face that changes while you are looking at it is a glitch.
///
/// A tile the user pinned wins over the roll, which is why this reads live state rather than being a
/// `OnceLock` itself: the pin is set from disk before the window is built, and can change mid-run
/// when the Settings page saves.
pub fn tile() -> usize {
    let pinned = PINNED.load(Ordering::Relaxed);
    if pinned > 0 {
        return pinned;
    }
    rolled()
}

/// The rolled face for this launch, drawn once and kept. Kept separately from the pin so that
/// un-pinning partway through a run brings back the duck this launch rolled, rather than producing a
/// new one the moment the checkbox is cleared.
fn rolled() -> usize {
    static ROLL: OnceLock<usize> = OnceLock::new();
    *ROLL.get_or_init(|| {
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

/// `0` means "no pin, roll one". See [`prefer`].
static PINNED: AtomicUsize = AtomicUsize::new(0);

/// Say what the stored settings pin, before [`init_script`] runs and again after every save. A
/// number outside the set is discarded rather than trusted — a settings file edited by hand must not
/// be able to name a tile that does not ship, and the same rule keeps a future document with more
/// tiles than this build from crashing one that has fewer.
pub fn prefer(pinned: Option<usize>) {
    PINNED.store(
        pinned
            .filter(|index| *index > 0 && *index <= TILES.len())
            .unwrap_or(0),
        Ordering::Relaxed,
    );
    rolled();
}

/// Run before the page's own scripts, so the window's mascot and the Dock's tile are the same
/// picture on the first frame. An IPC call the UI awaited would paint some other duck for a moment
/// before the answer arrived.
pub fn init_script() -> String {
    format!("window.freecodeMascot = {};", tile())
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

    /// The pin is process-global, so the cases that move it cannot run beside the ones that read it.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn a_launch_wears_one_of_the_tiles_and_never_changes_its_mind() {
        let _serial = lock();
        prefer(None);
        let first = tile();
        assert!(
            (1..=TILES.len()).contains(&first),
            "the pick must name a tile that ships: {first}"
        );
        assert_eq!(first, tile(), "the face is chosen per launch, not per look");
    }

    #[test]
    fn the_script_names_the_same_duck_the_dock_does() {
        let _serial = lock();
        prefer(None);
        assert_eq!(
            init_script(),
            format!("window.freecodeMascot = {};", tile())
        );
    }

    #[test]
    fn a_pinned_tile_wins_and_a_cleared_pin_goes_back_to_this_launchs_duck() {
        let _serial = lock();
        prefer(None);
        let rolled = tile();
        for index in [1, TILES.len()] {
            prefer(Some(index));
            assert_eq!(tile(), index, "{index} is the tile the user chose");
            assert_eq!(
                init_script(),
                format!("window.freecodeMascot = {index};"),
                "the window and the Dock have to agree on the pinned tile"
            );
        }
        prefer(None);
        assert_eq!(tile(), rolled, "unpinning mid-run must not roll a new face");
    }

    #[test]
    fn a_number_outside_the_set_is_not_a_tile() {
        let _serial = lock();
        prefer(None);
        let rolled = tile();
        for bogus in [0, TILES.len() + 1, 4096] {
            prefer(Some(bogus));
            assert_eq!(tile(), rolled, "{bogus} names nothing that ships");
        }
    }

    /// The page's set is a glob of `../src/assets/mascots/*.webp` and this one is the list above, so
    /// the two are kept in step by nothing but this. A tile added on one side is a Dock that wears a
    /// face the window cannot show, and a number that means two different pictures depending on which
    /// side of the bridge it is read on — which is the exact thing `tile()` exists to prevent.
    #[test]
    fn the_page_globs_the_same_tiles_the_host_ships() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/assets/mascots");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap_or_else(|why| panic!("the page's tiles are the directory {dir:?}: {why}"))
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().to_string_lossy().into_owned();
                name.ends_with(".webp").then_some(name)
            })
            .collect();
        names.sort();
        assert_eq!(
            names.len(),
            TILES.len(),
            "the page globs {} tiles where the host ships {}",
            names.len(),
            TILES.len()
        );
        for (index, name) in names.iter().enumerate() {
            assert_eq!(
                name,
                &format!("mascot-{:02}.webp", index + 1),
                "the sorted glob has to number the way TILES is written"
            );
        }
    }
}
