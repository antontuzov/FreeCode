import { useEffect, useState } from "react";

import { Inspector } from "./components/Inspector";
import { Knowledge } from "./components/Knowledge";
import { ModelStatus } from "./components/ModelStatus";
import { Palette } from "./components/Palette";
import { Settings } from "./components/Settings";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { Transcript } from "./components/Transcript";
import type { Locale } from "./i18n";
import { useShortcuts } from "./lib/shortcuts";
import { useAppearance } from "./state/appearance";
import { connectAgentEvents, useChat } from "./state/chat";
import { useSessions } from "./state/sessions";
import { FIXTURE_ROWS, useTranscript } from "./state/transcript";

export default function App() {
  const load = useAppearance((s) => s.load);
  const language = useAppearance((s) => s.language);
  const setRows = useTranscript((s) => s.setRows);
  const setContextUsed = useTranscript((s) => s.setContextUsed);
  const refreshEngine = useChat((s) => s.refresh);
  const send = useChat((s) => s.send);
  const stop = useChat((s) => s.stop);
  const newTask = useChat((s) => s.newTask);
  const [settingsPage, setSettingsPage] = useState<string | null>(query("settings"));
  // `?knowledge=1` opens the Knowledge Center and `?knowledge=<model id>` opens one model's page, for
  // the same reason as the other deep links: the rows that do that are clicks the native window
  // cannot make. An id the catalog does not have falls back to the machine page.
  const knowledgePage = query("knowledge");
  const [knowledge, setKnowledge] = useState(knowledgePage !== null);
  // `?search=<query>` opens the palette with that query already typed — the deep link the panel is
  // captured through, since the window takes no injected keystrokes and a search nobody can type
  // into is a search nobody can see.
  const paletteQuery = query("search");
  const [paletteOpen, setPaletteOpen] = useState(paletteQuery !== null);
  const fixture = fixtureMode();
  const ask = query("ask");
  const askRepeat = Math.max(1, Number(query("ask_repeat")) || 1);
  const askAgain = query("ask_again");
  const forgetBetween = query("new_task_between") === "1";
  const stopAfter = Math.max(0, Number(query("stop_after")) || 0);
  // `?coding=1` is the same press the composer's own control answers to, read here so a `?ask=` prompt
  // goes out with the workspace tools offered: the loop's rows are otherwise only reachable by a click
  // the native window cannot be given.
  const coding = query("coding") === "1";
  const reopen = query("reopen") === "1";
  const filed = useSessions((s) => s.items);
  const openFiled = useSessions((s) => s.open);
  const refreshFiled = useSessions((s) => s.refresh);

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
    // The sidebar's rows are conversations the host filed on a previous run, so the window has to ask
    // for them once it opens. The fixture does not: it draws its own workspace list to compare
    // against docs/reference-ui, and a real list arriving underneath it would be a second truth.
    if (!fixture) void refreshFiled();
    return () => {
      live = false;
      unlisten?.();
    };
  }, [fixture, refreshEngine, refreshFiled]);

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

  // `?ask=<prompt>` sends that prompt on mount; `?ask_repeat=<n>` sends it n times in a row. The
  // native window takes no injected clicks, so this is how a real answer from the local engine gets
  // verified in the real app — and how a second turn is verified, which is the one that has to reuse
  // the running server, or restart it after it died between the two.
  //
  // `?ask_again=<prompt>` sends it after the first one, and `&new_task_between=1` presses the
  // sidebar's New task between them — the only way to check that the control forgot anything, since
  // the second prompt asks what the first planted and an answer that remembers is a history the host
  // never dropped.
  useEffect(() => {
    if (!ask) return;
    void (async () => {
      for (let i = 0; i < askRepeat; i += 1) await send(ask, undefined, coding);
      if (!askAgain) return;
      if (forgetBetween) await newTask();
      await send(askAgain, undefined, coding);
    })();
  }, [ask, askRepeat, askAgain, forgetBetween, coding, newTask, send]);

  // `?reopen=1` opens the newest filed conversation once the list arrives — the same call the
  // sidebar's row makes, and the only way that row's behavior gets verified rather than assumed,
  // since the native window takes no injected clicks. Waiting for the list is the point: the row
  // cannot be opened before the host has said what it filed.
  useEffect(() => {
    if (!reopen) return;
    const newest = filed[0];
    if (newest) void openFiled(newest.id);
  }, [reopen, filed, openFiled]);

  // `?stop_after=<ms>` presses the composer's Stop that far into a run. Measured from the run
  // starting rather than from the window opening, because the engine spends seconds loading weights
  // and a timer from mount would press Stop before there was a reply to stop.
  const sending = useChat((s) => s.sending);
  useEffect(() => {
    if (!stopAfter || !sending) return;
    const timer = setTimeout(() => void stop(), stopAfter);
    return () => clearTimeout(timer);
  }, [sending, stopAfter, stop]);

  // The keys the hints on the controls name. Settings is a full-window surface rather than a modal,
  // so while it is up it owns the keyboard: Esc gives the window back and the chat keys hold until
  // it does, rather than firing something the user cannot see.
  const running = useTranscript((s) => s.running);
  useShortcuts({
    newTask: () => {
      // The palette owns the keyboard the same way Settings does: a New task fired from under it
      // would empty the transcript the rows in front of the user describe.
      if (settingsPage !== null || paletteOpen || knowledge) return;
      void newTask();
    },
    // Knowledge Center is a surface rather than a page of the shell, so ⌘, leaves it: staying on two
    // full-window pages at once would mean a back button that returns to a third.
    settings: () => {
      setKnowledge(false);
      setSettingsPage((page) => page ?? "appearance");
    },
    // Twice closes it, because the second press is the user changing their mind.
    palette: () => setPaletteOpen((open) => !open),
    escape: () => {
      if (settingsPage !== null) {
        setSettingsPage(null);
      } else if (knowledge) {
        setKnowledge(false);
      } else if (running) {
        void stop();
      }
    },
  });

  if (settingsPage !== null) {
    return <Settings locale={locale} page={settingsPage} onBack={() => setSettingsPage(null)} />;
  }

  if (knowledge) {
    return (
      <Knowledge
        locale={locale}
        initial={knowledgePage === "1" ? "" : (knowledgePage ?? "")}
        onBack={() => setKnowledge(false)}
      />
    );
  }

  return (
    <div className="flex h-full flex-col bg-bg text-fg">
      <div className="flex min-h-0 flex-1">
        <Sidebar
          locale={locale}
          fixture={fixture}
          onOpenSettings={() => setSettingsPage("appearance")}
          actions={{
            newTask: () => void newTask(),
            openPalette: () => setPaletteOpen(true),
            openKnowledge: () => setKnowledge(true),
          }}
        />
        <div className="flex min-w-0 flex-1 flex-col">
          <Transcript locale={locale} onPickModel={() => setSettingsPage("model")} />
          <StatusBar locale={locale} />
        </div>
        <div className="flex flex-col">
          <ModelStatus locale={locale} />
          <Inspector locale={locale} fixture={fixture} />
        </div>
      </div>
      {paletteOpen && (
        <Palette
          locale={locale}
          initial={paletteQuery ?? ""}
          onClose={() => setPaletteOpen(false)}
        />
      )}
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
