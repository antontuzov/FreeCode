// The transcript is where the host's events become rows and where the inspector and review panel
// read them back. These tests hold the two halves to their contracts: the reducer keeps the order a
// run produced and closes it exactly once, and the selectors report only what the rows themselves say.
import { beforeEach, describe, expect, it } from "vitest";

import type { AgentEvent, HostStep, RunReport } from "../lib/ipc";
import {
  builtPage,
  changedFiles,
  ctaLabel,
  drawnPictures,
  isPagePath,
  isTurn,
  liveRows,
  readSubjects,
  reviewRows,
  stepLabel,
  stepText,
  subagents,
  taskTitle,
  useTranscript,
  usedCapabilities,
  type Row,
  type Step,
} from "./transcript";

const REPORT: RunReport = {
  generatedTokens: 12,
  promptTokens: 30,
  cachedTokens: 0,
  generatedPerSecond: 4.5,
  promptPerSecond: 100,
  elapsedMs: 2_500,
};

function hostStep(patch: Partial<HostStep> & { id: string }): HostStep {
  return {
    kind: "readFile",
    params: {},
    status: "running",
    detail: "",
    diff: null,
    hunk: null,
    toolId: null,
    collapsedByDefault: false,
    ...patch,
  };
}

function at<T>(list: T[], index: number): T {
  const item = list[index];
  if (item === undefined) throw new Error(`no row at ${index}`);
  return item;
}

function apply(event: AgentEvent): void {
  useTranscript.getState().apply(event);
}

function rows(): Row[] {
  return useTranscript.getState().rows;
}

const STARTED: AgentEvent = {
  type: "runStarted",
  runId: "run-1",
  modelLabel: "Bonsai",
  local: true,
  engine: true,
  maxRounds: 4,
  plan: false,
};

beforeEach(() => {
  useTranscript.setState(useTranscript.getInitialState());
});

describe("labels and sentences", () => {
  it("names a batch by its count, and a batch of one as the single tool row it is", () => {
    const one: Step = { id: "b1", kind: "toolBatch", children: [{ id: "c1", kind: "readFile" }] };
    const two: Step = {
      id: "b2",
      kind: "toolBatch",
      children: [
        { id: "c2", kind: "readFile" },
        { id: "c3", kind: "terminal" },
      ],
    };
    expect(stepLabel("en", one)).toBe("Ran tool");
    expect(stepLabel("en", two)).toBe("Ran 2 tools");
  });

  it("counts the seconds a running row has waited", () => {
    expect(stepLabel("en", { id: "r", kind: "running", seconds: 7 })).toBe("Running… 7s");
  });

  it("names a seam by the place it marks rather than by the notice it carries", () => {
    const seam: Step = {
      id: "s",
      kind: "contextCompressed",
      notice: { code: "context-trimmed", params: {} },
    };
    expect(stepLabel("en", seam)).toBe("Context compressed");
  });

  it("lets a refusal replace the generic Blocked label, and a notice say Notice", () => {
    expect(stepLabel("en", { id: "n", kind: "blocked", notice: { code: "x", params: {} } })).toBe(
      "Notice",
    );
    expect(
      stepLabel("en", {
        id: "r",
        kind: "blocked",
        refusal: { code: "engine-not-installed", params: {} },
      }),
    ).toBe("The engine isn't on this machine yet");
  });

  it("words a notice and a refusal from the catalog, filling the host's measurements", () => {
    const notice = stepText("en", {
      id: "n",
      kind: "blocked",
      notice: { code: "prompt-too-long", params: { tokens: "5107", window: "4096" } },
    });
    expect(notice).toContain("5107");
    expect(notice).toContain("4096");

    const refusal = stepText("en", {
      id: "r",
      kind: "blocked",
      refusal: { code: "weights-not-downloaded", params: { file: "bonsai.gguf" } },
    });
    expect(refusal).toContain("bonsai.gguf");
  });

  it("prefers a known detail key over the host's English, and the English over a raw key", () => {
    expect(
      stepText("en", {
        id: "t",
        kind: "readFile",
        detail: "3 lines",
        detailKey: "transcript.summary.lines",
        detailParams: { count: 3 },
      }),
    ).toBe("3 lines");
    expect(
      stepText("en", {
        id: "t",
        kind: "readFile",
        detail: "a page's own title",
        detailKey: "page.title.not-in-this-build",
      }),
    ).toBe("a page's own title");
    expect(stepText("en", { id: "t", kind: "terminal", detail: "cargo test" })).toBe("cargo test");
    expect(stepText("en", { id: "t", kind: "terminal" })).toBe("");
  });

  it("labels a refusal's one action from the catalog", () => {
    expect(ctaLabel("en", "install-engine")).toBe("Install the PrismML engine");
  });

  it("tells a turn row from a step row", () => {
    expect(isTurn({ id: "t", kind: "turn", role: "user", text: "hi" })).toBe(true);
    expect(isTurn({ id: "s", kind: "readFile" })).toBe(false);
  });
});

describe("review scopes", () => {
  const conversation: Row[] = [
    { id: "u1", kind: "turn", role: "user", text: "first" },
    { id: "a1", kind: "turn", role: "assistant", text: "one" },
    { id: "s1", kind: "editFile", subject: "a.rs" },
    { id: "u2", kind: "turn", role: "user", text: "second" },
    { id: "a2", kind: "turn", role: "assistant", text: "two" },
    { id: "s2", kind: "editFile", subject: "b.rs" },
  ];

  it("reads the last turn from the newest prompt rather than from a run id", () => {
    expect(reviewRows(conversation, "lastTurn").map((row) => row.id)).toEqual(["a2", "s2"]);
  });

  it("shows the whole conversation when asked for all of it", () => {
    expect(reviewRows(conversation, "all")).toBe(conversation);
  });

  it("has no prompt to cut at in a transcript that never had one", () => {
    const steps: Row[] = [{ id: "s", kind: "readFile" }];
    expect(reviewRows(steps, "lastTurn")).toBe(steps);
  });
});

describe("derivations off the rows", () => {
  it("folds repeated writes into one file, summing both counts and keeping the hunks in order", () => {
    const written: Row[] = [
      {
        id: "batch",
        kind: "toolBatch",
        children: [
          {
            id: "e1",
            kind: "editFile",
            subject: "a.rs",
            diff: { added: 2, removed: 1 },
            hunk: { at: 1, lines: "+x" },
          },
          {
            id: "e2",
            kind: "editFile",
            subject: "a.rs",
            diff: { added: 3, removed: 1 },
            hunk: { at: 9, lines: "+y" },
          },
          { id: "e3", kind: "editFile", subject: "no-diff.rs" },
        ],
      },
      {
        id: "c1",
        kind: "createFile",
        subject: "b.html",
        diff: { added: 4, removed: 0 },
        hunk: { at: 1, lines: "+<p>" },
      },
    ];
    expect(changedFiles(written)).toEqual([
      {
        path: "a.rs",
        added: 5,
        removed: 2,
        hunks: [
          { at: 1, lines: "+x" },
          { at: 9, lines: "+y" },
        ],
        created: false,
      },
      {
        path: "b.html",
        added: 4,
        removed: 0,
        hunks: [{ at: 1, lines: "+<p>" }],
        created: true,
      },
    ]);
  });

  it("folds a call that changed several files per file instead of by its total", () => {
    const patch: Row[] = [
      {
        id: "p1",
        kind: "editFile",
        diff: { added: 7, removed: 2 },
        files: [
          {
            path: "a.rs",
            added: 4,
            removed: 0,
            hunks: [
              { at: 2, lines: "+x" },
              { at: 9, lines: "+y" },
            ],
          },
          { path: "b.rs", added: 3, removed: 2, hunks: [{ at: 1, lines: "-b\n+B" }] },
        ],
      },
      {
        id: "p2",
        kind: "editFile",
        subject: "a.rs",
        diff: { added: 1, removed: 1 },
        hunk: { at: 20, lines: "-z\n+Z" },
      },
    ];
    // The patch row's total is 7/2; folding it in beside the per-file entries would count every
    // changed line twice.
    expect(changedFiles(patch)).toEqual([
      {
        path: "a.rs",
        added: 5,
        removed: 1,
        hunks: [
          { at: 2, lines: "+x" },
          { at: 9, lines: "+y" },
          { at: 20, lines: "-z\n+Z" },
        ],
        created: false,
      },
      {
        path: "b.rs",
        added: 3,
        removed: 2,
        hunks: [{ at: 1, lines: "-b\n+B" }],
        created: false,
      },
    ]);
  });

  it("reports only the addresses a call actually read, one per address, in first-seen order", () => {
    const read: Row[] = [
      { id: "w1", kind: "webReading", subject: "https://a", status: "ok" },
      { id: "w2", kind: "webReading", subject: "https://a", status: "ok" },
      { id: "w3", kind: "webReading", subject: "https://refused", status: "error" },
      {
        id: "batch",
        kind: "toolBatch",
        children: [
          { id: "r1", kind: "readFile", subject: "x.rs", status: "ok" },
          { id: "r2", kind: "readFile", subject: "y.rs", status: "running" },
        ],
      },
    ];
    expect(readSubjects(read, "webReading")).toEqual(["https://a"]);
    expect(readSubjects(read, "readFile")).toEqual(["x.rs"]);
  });

  it("counts each capability once, off the first call that succeeded", () => {
    const used: Row[] = [
      { id: "r1", kind: "readFile", status: "ok" },
      { id: "r2", kind: "readFile", status: "ok" },
      { id: "t1", kind: "terminal", status: "ok" },
      { id: "e1", kind: "editFile", status: "running" },
      { id: "p", kind: "processed" },
      { id: "batch", kind: "toolBatch", children: [{ id: "tool1", kind: "tool", status: "ok" }] },
      { id: "b", kind: "blocked", status: "error" },
    ];
    expect(usedCapabilities(used).map((step) => step.id)).toEqual(["r1", "t1", "tool1"]);
  });

  it("opens the page a run built, preferring the entry page over the sheets beside it", () => {
    const built: Row[] = [
      { id: "u", kind: "turn", role: "user", text: "build me a snake game" },
      {
        id: "batch",
        kind: "toolBatch",
        children: [
          { id: "c1", kind: "createFile", subject: "snake/game.js", status: "ok" },
          { id: "c2", kind: "createFile", subject: "snake/style.css", status: "ok" },
          { id: "c3", kind: "createFile", subject: "snake/index.html", status: "ok" },
        ],
      },
    ];
    expect(builtPage(built)).toBe("snake/index.html");
  });

  it("takes the last page a run made when none of them is an index", () => {
    const built: Row[] = [
      { id: "u", kind: "turn", role: "user", text: "make a demo page" },
      { id: "c1", kind: "createFile", subject: "draft.html", status: "ok" },
      { id: "c2", kind: "createFile", subject: "final.htm", status: "ok" },
    ];
    expect(builtPage(built)).toBe("final.htm");
  });

  it("reads a patch that created several files as the pages among them", () => {
    const built: Row[] = [
      { id: "u", kind: "turn", role: "user", text: "build the site" },
      {
        id: "c1",
        kind: "createFile",
        status: "ok",
        files: [
          { path: "site/index.html", added: 10, removed: 0, hunks: [] },
          { path: "site/readme.md", added: 2, removed: 0, hunks: [] },
        ],
      },
    ];
    expect(builtPage(built)).toBe("site/index.html");
  });

  it("leaves the browser shut for an edit to a page an earlier turn made", () => {
    const rows: Row[] = [
      { id: "u1", kind: "turn", role: "user", text: "build me a game" },
      { id: "c1", kind: "createFile", subject: "index.html", status: "ok" },
      { id: "a1", kind: "turn", role: "assistant", text: "Done." },
      { id: "u2", kind: "turn", role: "user", text: "make the duck bigger" },
      { id: "e1", kind: "editFile", subject: "index.html", status: "ok" },
    ];
    // The panel would be answering a question nobody asked: the page is one the user has already seen.
    expect(builtPage(rows)).toBeNull();
  });

  it("ignores a page whose write failed or is still in flight", () => {
    const rows: Row[] = [
      { id: "u", kind: "turn", role: "user", text: "build a page" },
      { id: "c1", kind: "createFile", subject: "broken.html", status: "error" },
      { id: "c2", kind: "createFile", subject: "still-going.html", status: "running" },
    ];
    expect(builtPage(rows)).toBeNull();
  });

  it("treats a non-page it wrote as no page at all", () => {
    const rows: Row[] = [
      { id: "u", kind: "turn", role: "user", text: "write the notes" },
      { id: "c1", kind: "createFile", subject: "notes.md", status: "ok" },
    ];
    expect(builtPage(rows)).toBeNull();
    expect(isPagePath("a/b.HTM")).toBe(true);
    expect(isPagePath("a/b.md")).toBe(false);
  });

  it("lists the delegations that came back with an answer, even under a folded round", () => {
    const delegations: Row[] = [
      { id: "turn", kind: "turn", role: "user", text: "question" },
      {
        id: "batch",
        kind: "toolBatch",
        children: [
          {
            id: "s1",
            kind: "subagent",
            subject: "Find the entry point",
            status: "ok",
            detail: "2 rounds · 3 reads",
          },
          {
            id: "s2",
            kind: "subagent",
            subject: "A question that was refused",
            status: "error",
          },
          { id: "s3", kind: "subagent", subject: "Still reading", status: "running" },
        ],
      },
    ];
    // A refused or in-flight delegation is one the run asked for, not one that answered — the same
    // reading `readSubjects` and `usedCapabilities` use. The row keeps the subject and summary the
    // inspector shows.
    expect(subagents(delegations)).toEqual([
      {
        id: "s1",
        kind: "subagent",
        subject: "Find the entry point",
        status: "ok",
        detail: "2 rounds · 3 reads",
      },
    ]);
  });

  it("lists every frame the run drew, a repeated assertion as the two pictures it is", () => {
    const shot = "data:image/png;base64,AAAA";
    const drawn: Row[] = [
      { id: "turn", kind: "turn", role: "user", text: "check the page" },
      {
        id: "batch",
        kind: "toolBatch",
        children: [
          { id: "c1", kind: "browser", frame: shot, status: "ok" },
          // A check of the same page before and after a fix: one assertion, two frames, and both are
          // the report's to show — the pair is what says the change worked.
          { id: "k1", kind: "browser", frame: shot, status: "ok" },
          { id: "k2", kind: "browser", frame: shot, status: "ok" },
          // A browser call that read or clicked the page drew nothing, so it is not a picture.
          { id: "b1", kind: "browser", status: "ok" },
        ],
      },
      { id: "c2", kind: "browser", frame: shot, status: "ok" },
    ];
    expect(drawnPictures(drawn).map((step) => step.id)).toEqual(["c1", "k1", "k2", "c2"]);
  });
});

describe("the live round", () => {
  it("is the calls above the answer the run is still writing, not the earlier folded round", () => {
    const list: Row[] = [
      { id: "q", kind: "turn", role: "user", text: "build it" },
      { id: "a", kind: "turn", role: "assistant", text: "Reading first." },
      {
        id: "b",
        kind: "toolBatch",
        status: "ok",
        collapsed: true,
        children: [{ id: "old", kind: "readFile", status: "ok" }],
      },
      { id: "x", kind: "editFile", subject: "index.html", status: "ok" },
      { id: "y", kind: "terminal", status: "running" },
      { id: "open", kind: "turn", role: "assistant", text: "", streaming: true },
    ];
    expect(liveRows(list).map((step) => step.id)).toEqual(["x", "y"]);
  });

  it("keeps the route's reasoning inside the card, in the order it streamed", () => {
    // The OpenCode route streams a thought-only turn after every call, and the reference draws that
    // reasoning as one more row of the round: the card holds both, and the call above it is still
    // the round in flight rather than history.
    const list: Row[] = [
      { id: "q", kind: "turn", role: "user", text: "build it" },
      { id: "a", kind: "turn", role: "assistant", text: "Building it." },
      { id: "x", kind: "editFile", subject: "index.html", status: "ok" },
      {
        id: "t",
        kind: "turn",
        role: "assistant",
        text: "",
        thought: "Now check it.",
        streaming: true,
      },
    ];
    expect(liveRows(list).map((row) => row.id)).toEqual(["x", "t"]);
  });

  it("is nothing once the answer the round was writing has started", () => {
    const list: Row[] = [
      { id: "x", kind: "readFile", status: "ok" },
      { id: "a", kind: "turn", role: "assistant", text: "Done.", streaming: true },
    ];
    expect(liveRows(list)).toEqual([]);
  });

  it("stops at the plan and the refusal rows, which the card does not own", () => {
    const list: Row[] = [
      { id: "x", kind: "readFile", status: "ok" },
      { id: "todo", kind: "todo", status: "ok" },
      { id: "open", kind: "turn", role: "assistant", text: "", streaming: true },
    ];
    expect(liveRows(list)).toEqual([]);
  });
});

describe("the reducer", () => {
  it("opens a run under the host's own id and takes its round budget from the announcement", () => {
    useTranscript.setState({ runDiff: { files: 2, added: 1, removed: 1 } });
    apply(STARTED);

    const state = useTranscript.getState();
    expect(state.rows).toHaveLength(1);
    const turn = at(state.rows, 0);
    expect(turn).toMatchObject({
      id: "run-1",
      kind: "turn",
      role: "assistant",
      text: "",
      streaming: true,
      local: true,
    });
    expect(state.running).toBe(true);
    expect(typeof state.runStartedAt).toBe("number");
    expect(state.runRounds).toEqual({ done: 0, max: 4 });
    expect(state.runDiff).toBeNull();
  });

  it("draws no step pill for a run that was never offered a tool", () => {
    apply({ ...STARTED, maxRounds: 0 });
    expect(useTranscript.getState().runRounds).toBeNull();
  });

  it("keeps the plan mark on every answer a plan run opens, not only the first", () => {
    useTranscript.getState().pushPrompt("plan the change");
    apply({ ...STARTED, plan: true });
    apply({ type: "answerDelta", text: "I will read that." });
    apply({ type: "toolStarted", step: hostStep({ id: "p1" }) });

    // The tool round closes the spoken answer and opens a fresh one below the read; that answer is
    // still the plan run's, and losing the mark there would make the plan read as a built change.
    expect(at(rows(), 1)).toMatchObject({ kind: "turn", streaming: false, plan: true });
    expect(at(rows(), 3)).toMatchObject({ kind: "turn", streaming: true, plan: true });
    expect(at(rows(), 0)).toMatchObject({ kind: "turn", role: "user" });
    expect((at(rows(), 0) as { plan?: boolean }).plan).toBeUndefined();
  });

  it("appends deltas to the turn the run opened, and drops one that has no turn to join", () => {
    useTranscript.getState().pushPrompt("question");
    apply(STARTED);
    apply({ type: "thoughtDelta", text: "think " });
    apply({ type: "thoughtDelta", text: "twice" });
    apply({ type: "answerDelta", text: "par" });
    apply({ type: "answerDelta", text: "tial" });

    const turn = at(rows(), 1);
    expect(turn).toMatchObject({ text: "partial", thought: "think twice" });

    useTranscript.getState().setRows([]);
    apply({ type: "answerDelta", text: "orphan" });
    expect(rows()).toEqual([]);
  });

  it("puts a call above an answer that has not spoken yet, and opens a fresh answer below one that has", () => {
    useTranscript.getState().pushPrompt("question");
    apply(STARTED);
    apply({ type: "toolStarted", step: hostStep({ id: "s1" }) });
    expect(rows().map((row) => row.kind)).toEqual(["turn", "readFile", "turn"]);

    useTranscript.getState().setRows([]);
    useTranscript.getState().pushPrompt("question");
    apply(STARTED);
    apply({ type: "answerDelta", text: "I will read that." });
    apply({ type: "toolStarted", step: hostStep({ id: "s2" }) });

    const spoken = at(rows(), 1);
    expect(spoken).toMatchObject({ kind: "turn", text: "I will read that.", streaming: false });
    expect(at(rows(), 2).kind).toBe("readFile");
    expect(at(rows(), 3)).toMatchObject({ kind: "turn", text: "", streaming: true });
  });

  it("carries the host's hunk and collapsed flag onto the row it opens", () => {
    apply({
      type: "toolStarted",
      step: hostStep({
        id: "e1",
        kind: "editFile",
        detail: "src/a.rs",
        collapsedByDefault: true,
        hunk: { at: 4, lines: "+x" },
      }),
    });
    expect(at(rows(), 0)).toMatchObject({
      subject: "src/a.rs",
      collapsed: true,
      hunk: { at: 4, lines: "+x" },
    });
  });

  it("closes a call in place, with the catalog key and the measured diff under it", () => {
    useTranscript.getState().pushPrompt("question");
    apply(STARTED);
    apply({ type: "toolStarted", step: hostStep({ id: "s1" }) });
    apply({
      type: "toolFinished",
      step: hostStep({
        id: "s1",
        status: "done",
        detail: "a.rs",
        diff: { linesAdded: 1, linesRemoved: 2, filesChanged: 1 },
      }),
      outcome: {
        ok: true,
        summary: "3 lines",
        summaryKey: "transcript.summary.lines",
        summaryParams: { count: 3 },
        diff: { linesAdded: 1, linesRemoved: 2, filesChanged: 1 },
      },
    });

    const closed = at(rows(), 1);
    expect(closed).toMatchObject({
      id: "s1",
      status: "ok",
      subject: "a.rs",
      detail: "3 lines",
      detailKey: "transcript.summary.lines",
      detailParams: { count: 3 },
      diff: { added: 1, removed: 2 },
    });
    expect(rows()).toHaveLength(3);
  });

  it("keeps the frame a capture or a check drew on the row that took it", () => {
    const frame = "data:image/png;base64,AAAA";
    apply({
      type: "toolFinished",
      step: hostStep({ id: "shot", kind: "browser", status: "done", frame }),
      outcome: {
        ok: true,
        summary: "Captured the page (1440×900)",
        summaryKey: "transcript.summary.browserCaptured",
        summaryParams: { size: "1440×900" },
        diff: null,
      },
    });
    apply({
      type: "toolFinished",
      step: hostStep({ id: "click", kind: "browser", status: "done" }),
      outcome: { ok: true, summary: "clicked", diff: null },
    });

    expect(at(rows(), 0)).toMatchObject({ id: "shot", frame });
    // A browser call that drew nothing carries nothing: the report lists pictures, not calls, and a
    // row with a frame-shaped hole would be a thumbnail the window draws as a broken image.
    const click = rows().find((row): row is Step => !isTurn(row) && row.id === "click");
    expect(click).toBeDefined();
    expect(click?.frame).toBeUndefined();
  });

  it("appends a finished call that never opened a row rather than dropping it", () => {
    apply({
      type: "toolFinished",
      step: hostStep({ id: "late", status: "done" }),
      outcome: { ok: true, summary: "done", diff: null },
    });
    expect(rows().map((row) => row.id)).toEqual(["late"]);
  });

  it("folds a round's tail of tool rows under one finished header", () => {
    apply({ type: "toolStarted", step: hostStep({ id: "a" }) });
    apply({ type: "toolStarted", step: hostStep({ id: "b" }) });
    apply({
      type: "toolBatchClosed",
      step: hostStep({ id: "batch", kind: "toolBatch", collapsedByDefault: true }),
    });

    const header = at(rows(), 0);
    expect(header).toMatchObject({ kind: "toolBatch", status: "ok", collapsed: true });
    expect(rows()).toHaveLength(1);
    expect(!isTurn(header) && header.children?.map((child) => child.id)).toEqual(["a", "b"]);
  });

  it("carries a call's per-file breakdown onto its row", () => {
    const files = [
      { path: "a.rs", added: 1, removed: 1, hunks: [{ at: 2, lines: "-x\n+X" }] },
      { path: "b.rs", added: 1, removed: 0, hunks: [{ at: 5, lines: "+y" }] },
    ];
    apply({
      type: "toolFinished",
      step: hostStep({
        id: "p1",
        kind: "editFile",
        status: "done",
        diff: { linesAdded: 2, linesRemoved: 1, filesChanged: 2 },
        files,
      }),
      outcome: {
        ok: true,
        summary: "2 files patched",
        summaryKey: "transcript.summary.filesPatched",
        summaryParams: { count: 2 },
        diff: { linesAdded: 2, linesRemoved: 1, filesChanged: 2 },
      },
    });
    expect(at(rows(), 0)).toMatchObject({
      id: "p1",
      detailKey: "transcript.summary.filesPatched",
      diff: { added: 2, removed: 1 },
      files,
    });
  });

  it("leaves a notice where it stands rather than folding it into the batch that followed", () => {
    useTranscript.getState().setRows([
      { id: "a", kind: "readFile", status: "ok" },
      { id: "n", kind: "blocked", status: "error", notice: { code: "x", params: {} } },
      { id: "b", kind: "readFile", status: "ok" },
    ]);
    apply({ type: "toolBatchClosed", step: hostStep({ id: "batch", kind: "toolBatch" }) });

    expect(rows().map((row) => row.id)).toEqual(["a", "n", "batch"]);
    const header = at(rows(), 2);
    expect(!isTurn(header) && header.children?.map((child) => child.id)).toEqual(["b"]);
  });

  it("moves the diff pill without adding a row for it, and counts rounds as the host closes them", () => {
    apply(STARTED);
    apply({ type: "diffStaged", diff: { linesAdded: 5, linesRemoved: 2, filesChanged: 3 } });
    const state = useTranscript.getState();
    expect(state.rows).toHaveLength(1);
    expect(state.runDiff).toEqual({ files: 3, added: 5, removed: 2 });
    expect(state.runRounds).toEqual({ done: 0, max: 4 });

    apply({ type: "toolBatchClosed", step: hostStep({ id: "batch", kind: "toolBatch" }) });
    expect(useTranscript.getState().runRounds).toEqual({ done: 1, max: 4 });
  });

  it("draws a warning about the answer to come above it, and one about an answer in flight below it", () => {
    useTranscript.getState().pushPrompt("question");
    apply(STARTED);
    apply({
      type: "warning",
      code: "engine-restarted",
      params: { attempt: "1", max: "3", log: "" },
    });
    expect(rows().map((row) => row.kind)).toEqual(["turn", "blocked", "turn"]);
    expect(at(rows(), 1)).toMatchObject({
      notice: { code: "engine-restarted" },
      status: "error",
    });

    useTranscript.getState().setRows([]);
    useTranscript.getState().pushPrompt("question");
    apply(STARTED);
    apply({ type: "answerDelta", text: "already speaking" });
    apply({
      type: "warning",
      code: "engine-restarted",
      params: { attempt: "2", max: "3", log: "" },
    });
    expect(rows().map((row) => row.kind)).toEqual(["turn", "turn", "blocked"]);
  });

  it("draws the context trim as a seam rather than as a notice under a good answer", () => {
    apply({ type: "warning", code: "context-trimmed", params: {} });
    expect(at(rows(), 0)).toMatchObject({
      kind: "contextCompressed",
      notice: { code: "context-trimmed" },
    });
  });

  it("keeps a refusal's code, evidence and one action for the catalog to word", () => {
    apply({
      type: "engineBlocked",
      info: {
        code: "engine-not-installed",
        params: {},
        technical: "engine is not installed",
        ctaCode: "install-engine",
        ctaUrl: "https://example.test/pin",
      },
    });
    expect(at(rows(), 0)).toMatchObject({
      kind: "blocked",
      status: "error",
      refusal: { code: "engine-not-installed", params: {} },
      technical: "engine is not installed",
      cta: { code: "install-engine", url: "https://example.test/pin" },
    });
  });

  it("leaves one row for a refusal that arrives twice, and one for each different failure", () => {
    apply({
      type: "engineBlocked",
      info: {
        code: "engine-not-installed",
        params: {},
        technical: "T",
        ctaCode: "install-engine",
        ctaUrl: "https://example.test/pin",
      },
    });
    apply({ type: "failed", message: "engine refused: engine-not-installed — T" });
    expect(rows()).toHaveLength(1);

    apply({ type: "failed", message: "the stream ended early" });
    expect(rows()).toHaveLength(2);
    expect(at(rows(), 1)).toMatchObject({ kind: "blocked", detail: "the stream ended early" });
  });

  it("closes the turn and the clock when the run finishes, keeping the last measurement of the window", () => {
    useTranscript.getState().pushPrompt("question");
    apply(STARTED);
    apply({ type: "answerDelta", text: "answer" });
    apply({
      type: "runFinished",
      runId: "run-1",
      stopped: true,
      contextUsed: 42,
      totalDiff: null,
      run: REPORT,
    });

    const state = useTranscript.getState();
    expect(state.running).toBe(false);
    expect(state.runStartedAt).toBeNull();
    expect(at(state.rows, 1)).toMatchObject({ text: "answer", streaming: false, stopped: true });
    expect(state.contextUsed).toBe(42);
    expect(state.lastRun).toBe(REPORT);

    useTranscript.setState({ contextUsed: 55 });
    apply({
      type: "runFinished",
      runId: "run-1",
      stopped: false,
      contextUsed: null,
      totalDiff: null,
      run: REPORT,
    });
    expect(useTranscript.getState().contextUsed).toBe(55);
  });

  it("takes the closing diff when the run wrote something, and keeps the ledger when it did not", () => {
    useTranscript.setState({ runDiff: { files: 1, added: 1, removed: 0 } });
    apply({
      type: "runFinished",
      runId: "run-1",
      stopped: false,
      contextUsed: null,
      totalDiff: { linesAdded: 9, linesRemoved: 4, filesChanged: 2 },
      run: REPORT,
    });
    expect(useTranscript.getState().runDiff).toEqual({ files: 2, added: 9, removed: 4 });

    apply({
      type: "runFinished",
      runId: "run-1",
      stopped: false,
      contextUsed: null,
      totalDiff: null,
      run: REPORT,
    });
    expect(useTranscript.getState().runDiff).toEqual({ files: 2, added: 9, removed: 4 });
  });

  it("stops the clock when a run fails, because nothing is waiting any more", () => {
    useTranscript.setState({ running: true, runStartedAt: 123 });
    apply({ type: "failed", message: "send refused" });
    const state = useTranscript.getState();
    expect(state.running).toBe(false);
    expect(state.runStartedAt).toBeNull();
  });
});

describe("prompt rows", () => {
  it("adds the prompt on the click, with the picture it carried", () => {
    useTranscript.getState().pushPrompt("look", "data:image/png;base64,AA");
    expect(at(rows(), 0)).toMatchObject({
      kind: "turn",
      role: "user",
      text: "look",
      image: "data:image/png;base64,AA",
    });
  });

  it("stamps the wait on the click that started it, and clears it when the run closes", () => {
    useTranscript.getState().setRunning(true);
    expect(typeof useTranscript.getState().runStartedAt).toBe("number");
    useTranscript.getState().setRunning(false);
    expect(useTranscript.getState().runStartedAt).toBeNull();
  });
});

describe("task title", () => {
  // A step row ahead of the prompt, because the transcript can hold one before a turn: the name has
  // to come from the question, not from whatever row happens to be first.
  const asked = (text: string): Row[] => [
    { id: "s0", kind: "readFile" },
    { id: "u1", kind: "turn", role: "user", text },
    { id: "a1", kind: "turn", role: "assistant", text: "an answer" },
  ];

  it("names the task off its own first prompt", () => {
    expect(taskTitle(asked("rename the config loader"))).toBe("rename the config loader");
  });

  it("puts a prompt typed across lines on one line", () => {
    expect(taskTitle(asked("summarise\n\n  the attached   picture "))).toBe(
      "summarise the attached picture",
    );
  });

  // 60 is `sessions::TITLE_MAX`, the length the host files the same name at: the header and the
  // sidebar row are the same task, so they cut it the same way.
  it("cuts a prompt longer than the filed name at a word, and says so", () => {
    const title = taskTitle(asked("count the words here ".repeat(6)))!;
    expect([...title].length).toBeLessThanOrEqual(61);
    expect(title.endsWith("words…")).toBe(true);
  });

  it("has no name for a window nothing has been asked of", () => {
    expect(taskTitle([])).toBeNull();
    expect(
      taskTitle([{ id: "a1", kind: "turn", role: "assistant", text: "an answer" }]),
    ).toBeNull();
    expect(taskTitle(asked("   "))).toBeNull();
  });
});
