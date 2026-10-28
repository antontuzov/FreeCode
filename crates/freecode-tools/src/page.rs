//! Driving the page a run built, from the run itself.
//!
//! The in-app browser frames the `preview` scheme, so the page is a document inside the window
//! rather than a file this crate can read: `innerText` of a rendered page is not in its source, and a
//! button that only works after its own script ran cannot be clicked by editing the HTML. The
//! executors therefore name an operation and its fields, and the window answers it from inside the
//! document — [`PageHost`] is the one door between the two.
//!
//! The operation list is deliberately small and each one is a thing a person does with a browser:
//! open a page, read what it shows, click, type, press a key, wait for something to appear, read what
//! the console said, take a picture of what is on screen, and measure that picture against what the
//! page was supposed to draw. Nothing here reaches outside the panel:
//! `PageOp::Open` takes a workspace path the executor has already resolved through the sandbox, and
//! the rest act on whatever document the panel is showing.

use std::time::Duration;

use serde_json::Value;

/// One thing the panel can be asked to do to the page it is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageOp {
    /// Show a workspace page in the panel; answers once the address is committed.
    Open,
    /// The page's title, address and visible text, or one element's.
    Read,
    /// A click at an element's centre, or at an offset inside it.
    Click,
    /// Text put into a field the way typing puts it there.
    Type,
    /// One key press, as a keyboard sends it.
    Press,
    /// Wait for a selector to appear.
    Wait,
    /// The errors and warnings the page has logged since it loaded.
    Console,
    /// The page as it is drawn, saved as a PNG the run keeps.
    Capture,
    /// The page as it is drawn, measured against one assertion without a model looking at it.
    Check,
}

impl PageOp {
    /// The word on the wire between the executors and the window.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Read => "read",
            Self::Click => "click",
            Self::Type => "type",
            Self::Press => "press",
            Self::Wait => "wait",
            Self::Console => "console",
            Self::Capture => "capture",
            Self::Check => "check",
        }
    }

    /// How long the panel gets to answer. A page still loading is the ordinary case, so this is
    /// longer than a disk read; a wait carries its own deadline and gets that plus the travel, and a
    /// capture — or a check, which takes one too — is the webview rendering a frame of a page that
    /// may still be animating.
    pub fn patience(self) -> Duration {
        match self {
            Self::Wait => Duration::from_secs(17),
            Self::Capture | Self::Check => Duration::from_secs(12),
            _ => Duration::from_secs(8),
        }
    }
}

/// The window's page, as the executors see it. One implementation lives in the host, which routes
/// the operation through the panel's frame; the trait is here so [`crate::exec`] stays testable
/// without a window.
pub trait PageHost: Send + Sync {
    /// Perform one operation and return the panel's answer: the fields each executor reads are the
    /// ones its own arm of `page_tool` names. `Err` is the sentence the model and the row both get.
    fn ask(&self, op: PageOp, arguments: &Value, patience: Duration) -> Result<Value, String>;
}

/// Every tool that drives the page, in the order the registry declares them.
pub const PAGE_TOOLS: &[&str] = &[
    "browser_open",
    "browser_read",
    "browser_click",
    "browser_type",
    "browser_press",
    "browser_wait",
    "browser_console",
    "browser_capture",
    "browser_check",
];

/// The operation a tool id means, or `None` for every other call.
pub fn page_op(tool_id: &str) -> Option<PageOp> {
    match tool_id {
        "browser_open" => Some(PageOp::Open),
        "browser_read" => Some(PageOp::Read),
        "browser_click" => Some(PageOp::Click),
        "browser_type" => Some(PageOp::Type),
        "browser_press" => Some(PageOp::Press),
        "browser_wait" => Some(PageOp::Wait),
        "browser_console" => Some(PageOp::Console),
        "browser_capture" => Some(PageOp::Capture),
        "browser_check" => Some(PageOp::Check),
        _ => None,
    }
}

pub fn is_page_tool(tool_id: &str) -> bool {
    PAGE_TOOLS.contains(&tool_id)
}

/// The most page text one read hands back, in characters. A page's `innerText` is unbounded — a game
/// that appends a line per frame has megabytes of it after a minute — and this is a result that goes
/// back into the prompt.
pub const MAX_PAGE_TEXT: usize = 6000;

/// The most console lines one read hands back. A missing function inside a loop can log thousands.
pub const MAX_CONSOLE_LINES: usize = 40;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::registry;

    #[test]
    fn every_page_tool_is_declared_and_answers_an_op() {
        let declared: Vec<&str> = registry()
            .iter()
            .map(|tool| tool.id)
            .filter(|id| is_page_tool(id))
            .collect();
        assert_eq!(declared, PAGE_TOOLS, "the two lists must move together");
        for id in PAGE_TOOLS {
            assert!(page_op(id).is_some(), "{id} names no operation");
        }
    }

    #[test]
    fn other_tools_are_not_page_operations() {
        assert!(page_op("read_file").is_none());
        assert!(!is_page_tool("terminal"));
    }

    #[test]
    fn a_wait_is_patience_plus_the_travel() {
        assert!(PageOp::Wait.patience() > PageOp::Read.patience());
    }
}
