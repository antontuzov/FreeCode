import React from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { startPageBridge } from "./lib/page";
import { connectAgentEvents } from "./state/chat";
import { connectShellEvents, seedShellSession } from "./state/shell";
import "./styles/global.css";

// The host's event channel is wired once per window, at module scope rather than inside the React
// tree: StrictMode's dev double-mount would register a second listener before the first unlisten
// could land, and a run's opening events — the ones that arrive inside that gap — would each be
// applied twice by the two listeners that were both live.
void connectAgentEvents();
void connectShellEvents();
// The dev-only seed behind `?session=shell|python`: it opens the panel's own session so a native
// capture can reach a surface that is otherwise behind a click.
void seedShellSession();
// The agent's page calls arrive on their own channel and are answered from the same window, so the
// bridge is wired here for the same reason: one listener, installed before any page can be opened.
startPageBridge();

const container = document.getElementById("root");
if (!container) throw new Error("#root is missing from index.html");

createRoot(container).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
