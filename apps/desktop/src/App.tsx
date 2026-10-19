import { useEffect, useState } from "react";

import { Extensions } from "./components/Extensions";
import { Inspector } from "./components/Inspector";
import { Knowledge } from "./components/Knowledge";
import { ModelStatus } from "./components/ModelStatus";
import { Palette } from "./components/Palette";
import { Review } from "./components/Review";
import { Settings } from "./components/Settings";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { Transcript } from "./components/Transcript";
import type { Locale } from "./i18n";
import { useShortcuts } from "./lib/shortcuts";
import { useAppearance } from "./state/appearance";
import { connectAgentEvents, useChat } from "./state/chat";
import { currentView, useNav } from "./state/nav";
import { useSessions } from "./state/sessions";
import { FIXTURE_DIFF, FIXTURE_ROUNDS, FIXTURE_ROWS, useTranscript } from "./state/transcript";

export default function App() {
  const load = useAppearance((s) => s.load);
  const language = useAppearance((s) => s.language);
  const setRows = useTranscript((s) => s.setRows);
  const setContextUsed = useTranscript((s) => s.setContextUsed);
  const setRunDiff = useTranscript((s) => s.setRunDiff);
  const setRunRounds = useTranscript((s) => s.setRunRounds);
  const refreshEngine = useChat((s) => s.refresh);
  const send = useChat((s) => s.send);
  const stop = useChat((s) => s.stop);
  const newTask = useChat((s) => s.newTask);
  // Which surface the window shows is the top of the navigation stack rather than a set of booleans:
  // the header's arrows move along it, a surface's own sub-page travels with its entry, and Esc leaves
  // one step at a time. The deep links the captures go through (`?settings=model`, `?knowledge=<id>`,
  // `?extensions=1`) seed the same stack in `state/nav.ts`, with the conversation underneath them.
  const view = useNav(currentView);
  const push = useNav((s) => s.push);
  const back = useNav((s) => s.back);
  // Which of the two reports the right column is holding, and the press that gives it back. The review
  // is not on the navigation stack: it is the same column answering a different question, and a Back
  // arrow that un-opened it would leave the conversation exactly as it was anyway.
  const panel = useNav((s) => s.panel);
  const showInspector = useNav((s) => s.showInspector);
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
    // …and a "+N −M files changed" pill, which is the row a run leaves when it edits a file.
    setRunDiff(FIXTURE_DIFF);
    // …and the step pill beside it, which the scripted "Ran 3 tools" header earned.
    setRunRounds(FIXTURE_ROUNDS);
  }, [fixture, setRows, setContextUsed, setRunDiff, setRunRounds]);

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
      // The palette and the surfaces each own the keyboard the same way: a New task fired from under
      // one would empty the transcript the thing in front of the user describes.
      if (view.kind !== "chat" || paletteOpen) return;
      void newTask();
    },
    // ⌘, opens Settings over whatever is under it, and a second press does not bury the page the user
    // is reading under a fresh copy of Appearance.
    settings: () => {
      if (view.kind !== "settings") push({ kind: "settings", page: "appearance" });
    },
    // Twice closes it, because the second press is the user changing their mind.
    palette: () => setPaletteOpen((open) => !open),
    // Esc leaves a step of the history when there is one to leave and stops the run when the window is
    // already back at the conversation. The one control that means two things takes three in hand: a
    // review panel the user opened is the nearest thing to close, and it is not on the stack.
    escape: () => {
      if (panel === "review") {
        showInspector();
        return;
      }
      if (!back() && running) void stop();
    },
  });

  if (view.kind === "settings") {
    return (
      <Settings
        locale={locale}
        page={view.page}
        onOpen={(page) => push({ kind: "settings", page })}
        onBack={back}
      />
    );
  }

  if (view.kind === "knowledge") {
    return (
      <Knowledge
        locale={locale}
        page={view.model}
        onOpen={(model) => push({ kind: "knowledge", model })}
        onBack={back}
      />
    );
  }

  if (view.kind === "extensions") {
    return <Extensions locale={locale} onBack={back} />;
  }

  return (
    <div className="flex h-full flex-col bg-bg text-fg">
      <div className="flex min-h-0 flex-1">
        <Sidebar
          locale={locale}
          fixture={fixture}
          onOpenSettings={() => push({ kind: "settings", page: "appearance" })}
          actions={{
            newTask: () => void newTask(),
            openPalette: () => setPaletteOpen(true),
            openKnowledge: () => push({ kind: "knowledge", model: "" }),
            openExtensions: () => push({ kind: "extensions" }),
          }}
        />
        <div className="flex min-w-0 flex-1 flex-col">
          <Transcript
            locale={locale}
            onPickModel={() => push({ kind: "settings", page: "model" })}
            onOpenPalette={() => setPaletteOpen(true)}
          />
          <StatusBar locale={locale} />
        </div>
        {/* The column owns its width, not either report inside it: both are the same column's turn, and
            a height the pair has to share is one constraint the reports should not each re-declare. */}
        <div
          className={`flex min-h-0 shrink-0 flex-col ${
            panel === "review" ? "w-[var(--review-width)]" : "w-[var(--inspector-width)]"
          }`}
        >
          <ModelStatus locale={locale} />
          {/* One column, two reports: the run's numbers, or the change those numbers name. Shot 06 has
              the diff here rather than under the answer, because a code line needs the width the
              transcript would have to give up to show it. */}
          {panel === "review" ? (
            <Review locale={locale} />
          ) : (
            <Inspector locale={locale} fixture={fixture} />
          )}
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
