import { useEffect, useState } from "react";

import { Inspector } from "./components/Inspector";
import { ModelStatus } from "./components/ModelStatus";
import { Settings } from "./components/Settings";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { Transcript } from "./components/Transcript";
import type { Locale } from "./i18n";
import { useAppearance } from "./state/appearance";
import { FIXTURE_STEPS, useTranscript } from "./state/transcript";

export default function App() {
  const load = useAppearance((s) => s.load);
  const language = useAppearance((s) => s.language);
  const setSteps = useTranscript((s) => s.setSteps);
  const [settingsPage, setSettingsPage] = useState<string | null>(query("settings"));
  const fixture = fixtureMode();

  // The catalog language is the setting, so switching it in Settings repaints every label.
  const locale: Locale = language;

  useEffect(() => {
    void load();
  }, [load]);

  // `?fixture=1` renders the shell with a scripted run so the layout can be compared against
  // docs/reference-ui. Nothing sets it by default: an empty transcript is honest, an invented one
  // is not.
  useEffect(() => {
    if (fixture) setSteps(FIXTURE_STEPS);
  }, [fixture, setSteps]);

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
