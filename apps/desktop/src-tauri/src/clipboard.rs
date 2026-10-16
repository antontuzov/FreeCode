//! The one thing the UI is allowed to put on the system pasteboard.
//!
//! A code fence in an answer gets a copy control, and the write cannot live in the web page: the UI
//! runs in a WKWebView, which is not a place this app can count on the web Clipboard API existing
//! at all. So the transcript hands over the text it is already showing and the host writes exactly
//! that. The command takes a string and nothing else — it cannot name a file, a format or a command.
//!
//! It answers with an error rather than staying quiet, because a control that says "Copied" when the
//! pasteboard never took the text is the same lie as a status card claiming a server is running.

/// The fence's text, as the host's pasteboard now holds it.
#[tauri::command]
pub async fn copy_text(text: String) -> Result<(), String> {
    let mut board = arboard::Clipboard::new().map_err(|e| format!("no system pasteboard: {e}"))?;
    board
        .set_text(text)
        .map_err(|e| format!("the pasteboard refused the text: {e}"))
}

#[cfg(test)]
mod tests {
    /// The read side of the round trip, and only ever needed by the test. `None` is a pasteboard
    /// holding no text at all, which is a state this machine can be in.
    fn read_back() -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }

    /// The button's whole claim is that this write returned, so the round trip through the real
    /// pasteboard is what is worth proving on a real machine. What was there before is put back.
    #[test]
    fn a_copy_lands_on_the_pasteboard() {
        let previous = read_back();
        let fence = "fn main() {\n    println!(\"hi\");\n}";
        tauri::async_runtime::block_on(super::copy_text(fence.into())).unwrap();
        assert_eq!(read_back().as_deref(), Some(fence));
        if let Some(text) = previous {
            tauri::async_runtime::block_on(super::copy_text(text)).unwrap();
        }
    }
}
