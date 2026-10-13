import { useEffect, useState } from "react";

import { Inspector } from "./components/Inspector";
import { ModelStatus } from "./components/ModelStatus";
import { Settings } from "./components/Settings";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { Transcript } from "./components/Transcript";
import type { Locale } from "./i18n";
import { useAppearance } from "./state/appearance";
import { connectAgentEvents, useChat } from "./state/chat";
import { FIXTURE_ROWS, useTranscript } from "./state/transcript";

export default function App() {
  const load = useAppearance((s) => s.load);
  const language = useAppearance((s) => s.language);
  const setRows = useTranscript((s) => s.setRows);
  const setContextUsed = useTranscript((s) => s.setContextUsed);
  const refreshEngine = useChat((s) => s.refresh);
  const send = useChat((s) => s.send);
  const [settingsPage, setSettingsPage] = useState<string | null>(query("settings"));
  const fixture = fixtureMode();
  const ask = query("ask");

  // The catalog language is the setting, so switching it in Settings repaints every label.
  const locale: Locale = language;

  useEffect(() => {
    void load();
  }, [load]);

  // One subscription for the window's lifetime: a run's events keep arriving whether or not this
  // component re-rendered, and the engine row has to agree with the server that is actually up.
  useEffect(() => {
    let live = true;
    let unlisten: (() => void) | null = null;
    void connectAgentEvents().then((off) => {
      if (live) unlisten = off;
      else off();
    });
    void refreshEngine();
    return () => {
      live = false;
      unlisten?.();
    };
  }, [refreshEngine]);

  // `?fixture=1` renders the shell with a scripted run so the layout can be compared against
  // docs/reference-ui. Nothing sets it by default: an empty transcript is honest, an invented one
  // is not.
  useEffect(() => {
    if (!fixture) return;
    setRows(FIXTURE_ROWS);
    // The shot this is compared against has a context meter part-filled. Only the fixture can claim
    // a number; a real run reports none until the engine measures it.
    setContextUsed(40);
  }, [fixture, setRows, setContextUsed]);

  // `?ask=<prompt>` sends that prompt on mount. The native window takes no injected clicks, so this
  // is how a real answer from the local engine gets verified in the real app.
  useEffect(() => {
    if (ask) void send(ask);
  }, [ask, send]);

  if (settingsPage !== null) {
    return <Settings locale={locale} page={settingsPage} onBack={() => setSettingsPage(null)} />;
  }

  return (
    <div className="flex h-full flex-col bg-bg text-fg">
      <div className="flex min-h-0 flex-1">
        <Sidebar
          locale={locale}
          fixture={fixture}
          onOpenSettings={() => setSettingsPage("appearance")}
        />
        <div className="flex min-w-0 flex-1 flex-col">
          <Transcript locale={locale} />
          <StatusBar locale={locale} />
        </div>
        <div className="flex flex-col">
          <ModelStatus locale={locale} />
          <Inspector locale={locale} fixture={fixture} />
        </div>
      </div>
    </div>
  );
}

function fixtureMode(): boolean {
  return query("fixture") === "1";
}

/** `?settings=<page>` opens the Settings surface on that page — the dev-only deep link the
 * reference screenshots are captured from, since the native window takes no injected clicks. */
function query(name: string): string | null {
  return new URLSearchParams(window.location.search).get(name);
}
