import React from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { connectAgentEvents } from "./state/chat";
import "./styles/global.css";

// The host's event channel is wired once per window, at module scope rather than inside the React
// tree: StrictMode's dev double-mount would register a second listener before the first unlisten
// could land, and a run's opening events — the ones that arrive inside that gap — would each be
// applied twice by the two listeners that were both live.
void connectAgentEvents();

const container = document.getElementById("root");
if (!container) throw new Error("#root is missing from index.html");

createRoot(container).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
