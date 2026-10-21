// The transcript row model. `StepKind` mirrors `smollm_core::transcript::StepKind` one-for-one and
// its i18n keys match Rust's `StepKind::i18n_key`; `verify-transcript-i18n` fails the build if the
// two lists or the catalogs drift, because a row that renders a raw key mid-sentence is a bug the
// user sees in every language.
//
// A transcript is one ordered list of two row shapes: labelled steps (what a tool did) and turns
// (what the user asked and what the model answered). The reference shot interleaves them — the
// prompt bubble, "Processed", the answer paragraph, "Ran 8 tools" — so they share one order rather
// than two lists the UI would have to guess between.
import { create } from "zustand";

import { has, translate, type Locale } from "../i18n";
import type { AgentEvent, HostStep, HostStepStatus, HostToolOutcome, RunReport } from "../lib/ipc";

export type StepKind =
  | "processed"
  | "toolBatch"
  | "running"
  | "thought"
  | "terminal"
  | "tests"
  | "build"
  | "linter"
  | "git"
  | "editFile"
  | "createFile"
  | "readFile"
  | "searchContent"
  | "tool"
  | "contextCompressed"
  | "subagent"
  | "artifact"
  | "webSearch"
  | "webReading"
  | "source"
  | "blocked";

const I18N_KEYS: Record<StepKind, string> = {
  processed: "transcript.step.processed",
  toolBatch: "transcript.step.ranTools",
  running: "transcript.step.running",
  thought: "transcript.step.thought",
  terminal: "transcript.step.terminal",
  tests: "transcript.step.tests",
  build: "transcript.step.build",
  linter: "transcript.step.linter",
  git: "transcript.step.git",
  editFile: "transcript.step.editFile",
  createFile: "transcript.step.createFile",
  readFile: "transcript.step.readFile",
  searchContent: "transcript.step.searchContent",
  tool: "transcript.step.ranTool",
  contextCompressed: "transcript.step.contextCompressed",
  subagent: "transcript.step.subagent",
  artifact: "transcript.step.artifact",
  webSearch: "transcript.step.webSearch",
  webReading: "transcript.step.webReading",
  source: "transcript.step.source",
  blocked: "transcript.step.blocked",
};

export type StepStatus = "ok" | "running" | "stopped" | "error";

/**
 * A notice's code and the fields its sentence interpolates, exactly as the host sent them. Stored on
 * the row rather than turned into text when it arrives, because the wording belongs to the locale
 * catalog: a run that streamed under English and is read under Chinese says its notice in Chinese.
 */
export type Notice = { code: string; params: Record<string, string> };

/**
 * A refusal's code and the fields its sentences interpolate, exactly as the host sent them. Like a
 * notice, the row keeps the code rather than the wording: `blocker.<code>.title` and
 * `blocker.<code>.detail` come out of the catalog for the language the interface is in, so switching
 * language re-words a refusal that is already on screen.
 */
export type Refusal = { code: string; params: Record<string, string> };

export type DiffStat = { added: number; removed: number };

/**
 * `smollm_core::transcript::Hunk`: one block of a change as the tool that made it wrote it. `lines` is
 * the diff text — `+`, `-`, and the two context lines on either side — and `at` is the line of the
 * file *before* the write that the first of them sits on, which is what lets the review number its
 * gutter rather than guess where the block fell.
 */
export type Hunk = { at: number; lines: string };

/**
 * `smollm_core::transcript::EditedFile`: one file a call changed, with the lines the call cost it and
 * each block it wrote there. A row carries a list of these when one [`Hunk`] cannot describe the call:
 * a patch that named two files, or one file changed in two places.
 */
export type EditedFile = { path: string; added: number; removed: number; hunks: Hunk[] };

/**
 * What a run has changed so far, as the pill above the composer reads it. `files` is the host's count
 * of *distinct* paths, not the number of edit rows: a model that fixes one file four times changed one
 * file, and saying "4 files" would be a number nobody can check.
 */
export type RunDiff = { files: number; added: number; removed: number };

/**
 * How far into its budget a run has got. The code calls the unit a round — one ask, the tools it
 * called, one replay — because `Step` already names a tool row here and two different "steps" in one
 * file is a confusion nobody catches. The pill says "Step", which is the word the reference uses and
 * the word a user reaches for.
 */
export type RunRounds = { done: number; max: number };

export type Step = {
  id: string;
  kind: StepKind;
  /** The command, path or snippet the row ran against, truncated by the UI. */
  detail?: string;
  /**
   * For a tool row: which catalog sentence [`detail`] is, and the fields it names. The host sends both
   * beside its own English, because the same sentence goes back into the model's prompt and a prompt is
   * not in the interface's language.
   */
  detailKey?: string;
  detailParams?: Record<string, string | number>;
  /** Shown as a monospace chip after the label, e.g. the file an edit touched. */
  subject?: string;
  status?: StepStatus;
  diff?: DiffStat;
  /**
   * On a row that wrote a file, whether it made one (`createFile`) or changed one (`editFile`): the
   * change as diff lines, exactly as the host handed them to the model. The review panel draws the
   * run's writes from these rather than from the `DiffStat`, which counts lines but cannot say which
   * ones.
   */
  hunk?: Hunk;
  /**
   * A call that made more than one change lists its files here, one entry per file with that file's
   * own hunks. A single change in a single file stays in [`hunk`]; the host sends one shape or the
   * other, never both. The review folds these per path exactly as it folds rows.
   */
  files?: EditedFile[];
  /** For `toolBatch` and `running`: the rows nested underneath the header. */
  children?: Step[];
  /**
   * `TranscriptStep::collapsed_by_default`: the host's word for a row that starts folded over its
   * children. A finished round is the case that matters — its calls are on screen while they run and
   * fold under "Ran N tools" the moment they are all in, which is how the reference keeps a long run
   * skimmable instead of leaving every read standing.
   */
  collapsed?: boolean;
  /** Seconds elapsed, only meaningful on a `running` header. */
  seconds?: number;
  /** For a `blocked` row: the single action that clears the refusal, as its catalog key and its URL. */
  cta?: { code: string; url: string };
  /**
   * For a `blocked` row: which of the load-guard's refusals this is. The code replaces the generic
   * "Blocked" label and its sentence, because both are the catalog's to write — see [`Refusal`].
   */
  refusal?: Refusal;
  /** For a `blocked` row: the evidence — paths, pins, metadata keys — folded away under the headline. */
  technical?: string;
  /**
   * A `blocked` row that reports something the run did anyway — an effort the model substituted, a
   * server that was restarted — rather than one that refuses it. It reads as a notice because a row
   * headed "Blocked" under an answer that streamed fine tells the user the answer is not to be
   * trusted, which is the opposite of what happened.
   *
   * What it holds is the host's code and the fields it measured, not a sentence: the wording is the
   * locale catalog's, so a different language re-renders the notices already on screen instead of
   * keeping the English the run happened to be described in.
   */
  notice?: Notice;
};

/** One side of the conversation. The reference renders the prompt as a right-aligned bubble and the
 * answer as plain text, so neither is a labelled step row. */
export type Turn = {
  id: string;
  kind: "turn";
  role: "user" | "assistant";
  text: string;
  /**
   * The attached picture, as a `data:` URL, on the prompt that carried it. Display only to this row —
   * the host holds the history and replays the picture itself, so a reopened conversation shows what
   * the answer was given to instead of a sentence about a photo that is gone.
   */
  image?: string;
  /**
   * The reasoning that streamed before the answer. Display only: the host replays the answer text
   * into the next request and never this, because re-rendered reasoning makes the fork's prompt
   * cache miss on every turn (Bonsai-demo#183).
   */
  thought?: string;
  /** Deltas are still arriving for this turn. */
  streaming?: boolean;
  /** Cut short by the stop button. The text is what streamed before the click, and saying so is the
   * difference between a truncated answer and an answer the model chose to keep short. */
  stopped?: boolean;
  /** The refusal badge on an answer that never came: the "Local" badge says what the run was. */
  local?: boolean;
};

export type Row = Step | Turn;

export function isTurn(row: Row): row is Turn {
  return row.kind === "turn";
}

export function stepLabel(locale: Locale, step: Step): string {
  if (step.kind === "toolBatch") {
    // A batch of one would read "Ran 1 tools". The count only earns its place above one, and the
    // singular the catalogs already own for a lone tool row is the one this header borrows.
    const count = step.children?.length ?? 0;
    return count === 1
      ? translate(locale, I18N_KEYS.tool)
      : translate(locale, I18N_KEYS.toolBatch, { count });
  }
  if (step.kind === "running") {
    return translate(locale, I18N_KEYS.running, { seconds: step.seconds ?? 0 });
  }
  // A seam is named by the place it marks, not by the notice it carries: a line reading "Notice" over
  // the rule where the history was shortened says a problem happened there, and none did.
  if (step.kind === "contextCompressed") return translate(locale, I18N_KEYS.contextCompressed);
  if (step.notice) return translate(locale, "transcript.notice");
  // "Blocked" is the name of the row shape, not of what happened. A refusal says which problem it is.
  if (step.refusal) return translate(locale, `blocker.${step.refusal.code}.title`);
  return translate(locale, I18N_KEYS[step.kind]);
}

/**
 * What a row says, as opposed to what it is called. A notice's and a refusal's sentences come out of
 * the catalog for their code, filled with the measurements the host sent: switching language re-words
 * the rows of a run that already finished, and a catalog entry can name the Settings control in the
 * same language it labels the control with.
 */
export function stepText(locale: Locale, step: Step): string {
  if (step.notice) return translate(locale, `notice.${step.notice.code}`, step.notice.params);
  if (step.refusal)
    return translate(locale, `blocker.${step.refusal.code}.detail`, step.refusal.params);
  // A tool's line is the app's own sentence about a measurement, so the catalog words it — unless the
  // host named a key this build's catalogs do not hold, which is the one case where its English is the
  // truer line. Printing the key there would be a raw identifier mid-transcript.
  if (step.detailKey && has(step.detailKey))
    return translate(locale, step.detailKey, step.detailParams ?? {});
  return step.detail ?? "";
}

/** The one action a refusal offers, as its catalog label. */
export function ctaLabel(locale: Locale, code: string): string {
  return translate(locale, `cta.${code}`);
}

let sequence = 0;
function nextId(prefix: string): string {
  sequence += 1;
  return `${prefix}-${sequence}`;
}

/** The row a refusal gets: a code the catalog words into a headline and an instruction, plus the evidence. */
function blockedRow(refusal: {
  refusal?: Refusal;
  technical?: string;
  cta?: { code: string; url: string };
  detail?: string;
}): Step {
  return {
    id: nextId("blocked"),
    kind: "blocked",
    refusal: refusal.refusal,
    detail: refusal.detail,
    technical: refusal.technical,
    status: "error",
    cta: refusal.cta,
  };
}

/**
 * Whether this row already announces that problem. A refusal arrives twice over — once as
 * `engineBlocked`, and once as the `failed` message the host composes out of the same blocker's code
 * and evidence — so the row has to be read back the way the host wrote it, or every blocked launch
 * would leave two rows. Both halves are host text, which is why the comparison is against the code
 * rather than the sentence the user sees.
 */
function carries(step: Step, message: string): boolean {
  if (step.refusal)
    return message === `engine refused: ${step.refusal.code} — ${step.technical ?? ""}`;
  return step.detail === message;
}

/** Something the run worked around, on the same quiet footing as the answer it did not stop. */
function noticeRow(notice: Notice): Step {
  // One notice is not a thing the run had to forgive but a place the transcript has: where the server
  // ran out of window, every row above the seam was written from a prompt that no longer held the
  // conversation's start. So it draws the reference's rule across the column and keeps the host's
  // sentence — the one that names the two controls that could undo it — folded under it.
  if (notice.code === "context-trimmed")
    return { id: nextId("seam"), kind: "contextCompressed", notice };
  return { id: nextId("notice"), kind: "blocked", status: "error", notice };
}

/** The turn the next delta belongs to: the assistant turn the current run opened. */
function openTurn(rows: Row[]): Turn | null {
  for (let i = rows.length - 1; i >= 0; i -= 1) {
    const row = rows[i];
    if (row && isTurn(row) && row.role === "assistant" && row.streaming) return row;
  }
  return null;
}

/**
 * Apply a delta to the turn the run opened. A delta that arrives with no open turn is dropped
 * rather than invented into a row: the host streams inside one run, so an orphan means the window
 * re-rendered mid-run, and duplicating text would be worse than losing a fragment.
 */
function onOpenTurn(rows: Row[], mutate: (turn: Turn) => Turn): Row[] {
  const turn = openTurn(rows);
  return turn ? rows.map((row) => (row === turn ? mutate(turn) : row)) : rows;
}

/** The host's four states in the row's three. A call that failed and one that was refused both read as
 * a problem line, and what tells them apart is the sentence under the label rather than its colour. */
function hostStatus(status: HostStepStatus): StepStatus {
  if (status === "done") return "ok";
  if (status === "running") return "running";
  return "error";
}

/**
 * A tool's one-line result, as the host sent it: the catalog sentence and its fields when the line is
 * the app's own wording, and the English it was written in either way.
 */
type ToolSummary = {
  text: string;
  key?: string;
  params?: Record<string, string | number>;
};

function toolSummary(outcome: HostToolOutcome): ToolSummary {
  return {
    text: outcome.summary,
    key: outcome.summaryKey ?? undefined,
    params: outcome.summaryParams ?? undefined,
  };
}

/**
 * A host step as a row. `detail` is the path or pattern the call acted on — for an edit, the file it
 * wrote as the sandbox resolved it — which the reference draws as the chip beside the label, and the
 * outcome's one-line summary is the sentence under it. Neither is the app's own wording by accident:
 * "Read file" and "3 lines" both come out of the catalog, which is why the summary arrives as a key and
 * its fields rather than as a sentence, and why `verify-transcript-i18n` holds both sides to each other.
 */
function hostRow(step: HostStep, summary?: ToolSummary): Step {
  return {
    id: step.id,
    kind: step.kind,
    subject: step.detail || undefined,
    detail: summary?.text,
    detailKey: summary?.key,
    detailParams: summary?.params,
    diff: step.diff ? { added: step.diff.linesAdded, removed: step.diff.linesRemoved } : undefined,
    hunk: step.hunk ?? undefined,
    files: step.files?.length ? step.files : undefined,
    collapsed: step.collapsedByDefault || undefined,
    status: hostStatus(step.status),
  };
}

/**
 * Where a step row goes: above the answer the run is still writing.
 *
 * A round that said something before it asked for a tool leaves that sentence in the answer row, which
 * would then sit *under* the read it promised. So a row that has already spoken closes, and the next
 * round writes into a fresh one below the tool rows: the transcript keeps the order the run produced,
 * and each row still holds one continuous piece of prose.
 */
function pushStep(rows: Row[], step: Step): Row[] {
  const turn = openTurn(rows);
  if (!turn) return [...rows, step];
  const at = rows.indexOf(turn);
  if (!turn.text && !turn.thought) return [...rows.slice(0, at), step, ...rows.slice(at)];
  return [
    ...rows.slice(0, at),
    { ...turn, streaming: false },
    step,
    ...rows.slice(at + 1),
    {
      id: nextId("turn"),
      kind: "turn",
      role: "assistant",
      text: "",
      streaming: true,
      local: turn.local,
    },
  ];
}

/** A row a tool opened, which is what a batch header may claim. Notices and refusals are `blocked`
 * rows that a run produced alongside those tools, and folding one under "Ran 2 tools" would say the
 * batch ran it. */
function isToolStep(row: Row): boolean {
  return !isTurn(row) && row.kind !== "toolBatch" && row.kind !== "blocked";
}

/**
 * The round's header, taking the tool rows it counts as its children. They are already the run's tail —
 * each opened as its call ran — so the header folds them rather than repeating them, which is how the
 * reference draws one collapsible "Ran N tools" line per round.
 */
function closeBatch(rows: Row[], header: Step): Row[] {
  const turn = openTurn(rows);
  const end = turn ? rows.indexOf(turn) : rows.length;
  let start = end;
  while (start > 0) {
    const row = rows[start - 1];
    if (!row || !isToolStep(row)) break;
    start -= 1;
  }
  const children = rows.slice(start, end) as Step[];
  if (children.length === 0) return rows;
  return [...rows.slice(0, start), { ...header, children }, ...rows.slice(end)];
}

/** The row a finished call leaves: replaced in place, so its position in the run does not move when
 * its outcome arrives. A step that never opened a row (a window that re-rendered mid-round) is appended
 * rather than dropped, because a call that ran and shows nothing is the worse failure. */
function finishStep(rows: Row[], step: HostStep, summary: ToolSummary): Row[] {
  if (!rows.some((row) => row.id === step.id)) return pushStep(rows, hostRow(step, summary));
  return rows.map((row) => (row.id === step.id ? hostRow(step, summary) : row));
}

/**
 * What the review panel counts: the last prompt's change, or everything the conversation wrote.
 * The reference's header carries this as its first control because the two answers are different
 * questions, and a panel that quietly picks one reports a change the reader did not ask about.
 */
export type ReviewScope = "lastTurn" | "all";

/**
 * The rows a scope reviews.
 *
 * "Last turn" is read off the conversation rather than off the run: the rows after the newest prompt,
 * which is one run's worth while that run is the newest thing that happened, and stays so when the
 * window re-renders mid-round or the run is still streaming. Without this the panel's file sections
 * would walk every turn the conversation holds while its headline counted the last run alone, and the
 * two would disagree on any conversation with two prompts in it.
 */
export function reviewRows(rows: Row[], scope: ReviewScope): Row[] {
  if (scope === "all") return rows;
  for (let i = rows.length - 1; i >= 0; i -= 1) {
    const row = rows[i];
    if (row && isTurn(row) && row.role === "user") return rows.slice(i + 1);
  }
  return rows;
}

/**
 * Every step in the span, one level under a closed round. A run's calls live under their "Ran N
 * tools" header once the batch folds, and a page the run read while folded is still a page it read —
 * so anything that asks what a run *did* walks this rather than the list it was given.
 */
function everyStep(rows: Row[]): Step[] {
  return rows.flatMap((row) => (isTurn(row) ? [] : row.children ? [row, ...row.children] : [row]));
}

/**
 * The files this conversation's rows say were written, newest write folded into the file it touched.
 *
 * Read off the rows rather than off the run's total because the inspector names *which* files, and a
 * `DiffStat` carries a count of them, not their paths. The two agree by construction: the host writes
 * the row's numbers and its ledger off the same `Edit`, one path at a time. A file the run made counts
 * as one it touched, so a run that only created a page still shows up here.
 *
 * Each entry also carries the file's hunks, in the order the run wrote them: two edits to one file
 * make two blocks of diff lines under one path rather than a merge no diff tool could produce. A row
 * that changed several files at once lists them in `files`, and its `diff` is their total — so the
 * list is folded in place of the row's number, not beside it. And each entry says whether the run
 * *made* the file, which a create row can only be reporting about a path that was free — the executor
 * refuses one that already stands — so the mark is the host's fact, not a guess the panel makes from a
 * hunk that happens to hold no removed lines.
 */
export function changedFiles(
  rows: Row[],
): (DiffStat & { path: string; hunks: Hunk[]; created: boolean })[] {
  const byPath = new Map<string, { diff: DiffStat; hunks: Hunk[]; created: boolean }>();
  const add = (path: string, diff: DiffStat, hunks: Hunk[], created: boolean) => {
    const seen = byPath.get(path);
    byPath.set(path, {
      diff: {
        added: (seen?.diff.added ?? 0) + diff.added,
        removed: (seen?.diff.removed ?? 0) + diff.removed,
      },
      hunks: [...(seen?.hunks ?? []), ...hunks],
      created: (seen?.created ?? false) || created,
    });
  };
  for (const row of everyStep(rows)) {
    if (row.kind !== "editFile" && row.kind !== "createFile") continue;
    const created = row.kind === "createFile";
    if (row.files?.length) {
      for (const file of row.files) add(file.path, file, file.hunks, created);
      continue;
    }
    if (!row.diff || !row.subject) continue;
    add(row.subject, row.diff, row.hunk ? [row.hunk] : [], created);
  }
  return [...byPath].map(([path, entry]) => ({
    path,
    ...entry.diff,
    hunks: entry.hunks,
    created: entry.created,
  }));
}

/**
 * The addresses this span's rows say the run read something from, in the order it first read them:
 * the pages a `fetch_url` opened, the files a `read_file` brought into the next round.
 *
 * Both of the inspector's read groups are derived this way for the reason `changedFiles` is: the host
 * writes the address into the row's own subject, so a panel that listed anything else would disagree
 * with the transcript above it — including by advertising a page nobody fetched. Hence the status
 * test: a call the network policy refused, or one still in flight, is a row that says the run *asked*,
 * not one that says it read. Repeats fold to one entry, because a model that re-reads a file opened
 * one file.
 */
export function readSubjects(rows: Row[], kind: "webReading" | "readFile"): string[] {
  const seen = new Set<string>();
  for (const row of everyStep(rows)) {
    if (row.kind !== kind || row.status !== "ok" || !row.subject) continue;
    seen.add(row.subject);
  }
  return [...seen];
}

/**
 * The delegations this span's rows say the run made, in the order it made them: one entry per
 * `spawn_subagent` row that came back with an answer. A refused delegation is excluded on the same
 * reading as a refused read — the run asked, and asking is not an answer — and the host's one-per-run
 * budget keeps the list to a single row without the panel needing to enforce it.
 *
 * The row's subject is the task, clipped by the host, so the panel can name what was delegated rather
 * than repeat the group's own title; the row's summary supplies the rounds and reads.
 */
export function subagents(rows: Row[]): Step[] {
  return everyStep(rows).filter((row) => row.kind === "subagent" && row.status === "ok");
}

/**
 * The kinds that name a capability the loop can be handed, as opposed to a row the run leaves behind.
 * The rest of the model is narration (`thought`, `processed`, a batch's own header) or a seam
 * (`contextCompressed`), and a list of what a run *used* that counted its own commentary would be
 * padding the number out.
 */
const CAPABILITIES = new Set<StepKind>([
  "readFile",
  "searchContent",
  "terminal",
  "tests",
  "build",
  "linter",
  "git",
  "editFile",
  "createFile",
  "webSearch",
  "webReading",
  "tool",
]);

/**
 * One step per capability this span's rows say the run called on, in the order it first reached for
 * one — the representative step rather than the kind, so the panel can name it with [`stepLabel`] and
 * word the row exactly as the transcript row above words itself.
 *
 * The same derivation as the other three groups, for the same reason: the host writes a row per call,
 * so a panel that listed capabilities from anywhere else would advertise one the run never used. A
 * call that was refused or is still in flight is excluded on the same reading as `readSubjects` — the
 * run asked, and asking is not using. Repeats fold to one entry because a model that read nine files
 * had one capability, which is the question this group answers: what was at its hands, not how busy
 * it was.
 */
export function usedCapabilities(rows: Row[]): Step[] {
  const first = new Map<StepKind, Step>();
  for (const row of everyStep(rows)) {
    if (!CAPABILITIES.has(row.kind) || row.status !== "ok") continue;
    if (!first.has(row.kind)) first.set(row.kind, row);
  }
  return [...first.values()];
}

function reduce(rows: Row[], event: AgentEvent): Row[] {
  switch (event.type) {
    case "runStarted":
      return [
        ...rows,
        {
          id: event.runId,
          kind: "turn",
          role: "assistant",
          text: "",
          streaming: true,
          local: event.local,
        },
      ];
    case "thoughtDelta":
      return onOpenTurn(rows, (turn) => ({
        ...turn,
        thought: `${turn.thought ?? ""}${event.text}`,
      }));
    case "answerDelta":
      return onOpenTurn(rows, (turn) => ({ ...turn, text: `${turn.text}${event.text}` }));
    case "toolStarted":
      return pushStep(rows, hostRow(event.step));
    case "toolFinished":
      return finishStep(rows, event.step, toolSummary(event.outcome));
    case "toolBatchClosed":
      // The header arrives when the round is over, so it never renders the live dot its own rows had
      // while they ran — the host's step is still in its opening state, and the batch is not.
      return closeBatch(rows, { ...hostRow(event.step), status: "ok" });
    case "diffStaged":
      // The pill is store state rather than a row: a run that changed six files already says so in six
      // rows, and a seventh line repeating the count would be a summary nobody asked for.
      return rows;
    case "runFinished":
      return onOpenTurn(rows, (turn) => ({
        ...turn,
        streaming: false,
        stopped: event.stopped || undefined,
      }));
    case "engineBlocked":
      return [
        ...rows,
        blockedRow({
          refusal: { code: event.info.code, params: event.info.params },
          technical: event.info.technical,
          cta: { code: event.info.ctaCode, url: event.info.ctaUrl },
        }),
      ];
    case "warning": {
      // Warnings are what the run adapted to, not what it refused: `failed` and `engineBlocked`
      // carry the refusals, and this case takes the substitutions and relaunches that left the
      // answer standing.
      const notice = noticeRow({ code: event.code, params: event.params });
      const turn = openTurn(rows);
      // The run's row opens the moment the host announces it, so an appended notice would otherwise
      // land under an answer that has not said a word yet. A warning sent before the first token is
      // about the answer to come, and belongs above it; one that lands on an answer already in
      // flight — or after the run closed — stays below the row it describes.
      if (turn && !turn.text && !turn.thought) {
        const at = rows.indexOf(turn);
        return [...rows.slice(0, at), notice, ...rows.slice(at)];
      }
      return [...rows, notice];
    }
    case "failed": {
      const last = rows[rows.length - 1];
      // The host both emits `failed` and rejects the command with the same text, and a refusal
      // arrives as `engineBlocked` first. One row per problem.
      if (last && !isTurn(last) && last.kind === "blocked" && carries(last, event.message)) {
        return rows;
      }
      return [...rows, blockedRow({ detail: event.message })];
    }
  }
}

type TranscriptStore = {
  rows: Row[];
  running: boolean;
  /**
   * Percent of the loaded context in use, or null while nothing has measured it. The meter renders
   * an em dash rather than a made-up number, because a real run is now the thing being shown.
   */
  contextUsed: number | null;
  /** What the last finished run cost, or null while this window has had none — which is what a new
   * task or a thrown-away conversation leaves behind. A run that was cut short still reports its
   * wait, with the engine's own numbers absent from its report rather than the whole report gone. */
  lastRun: RunReport | null;
  /**
   * When the wait for the current run opened, as `Date.now()`, or null while no run is open. The
   * transcript's running line counts the seconds off this, which is why the stamp is taken from the
   * click that sent the prompt rather than from the host's answer.
   */
  runStartedAt: number | null;
  /**
   * What the run in flight — or the one that just closed — has changed in the workspace, or null while
   * it has written nothing. This is the "+N −M files changed" pill above the composer, and it is the
   * host's ledger rather than an addition the interface makes: the same numbers arrive on `diffStaged`
   * while the run goes and on `runFinished` when it ends.
   */
  runDiff: RunDiff | null;
  /**
   * How far into its budget the run in flight — or the one that just closed — has got, or null when
   * there is no budget to be partway into. `max` is the host's own round cap, carried on the
   * announcement rather than copied here, and `done` counts the batches the host has closed, so the
   * figure moves at the end of a round rather than at the start of one. This is the composer's step
   * pill; the transcript's numbered steps are a different thing and are not counted by it.
   */
  runRounds: RunRounds | null;
  setRows: (rows: Row[]) => void;
  setRunning: (running: boolean) => void;
  /** Presets the meter for the fixture, and clears it when the conversation is thrown away. */
  setContextUsed: (percent: number | null) => void;
  /** Presets the run report for the fixture; a real run fills it from its own timings. */
  setLastRun: (run: RunReport | null) => void;
  /** Presets the pill for the fixture; a real run fills it from its own writes. */
  setRunDiff: (diff: RunDiff | null) => void;
  /** Presets the step pill for the fixture; a real run fills it from its own closed rounds. */
  setRunRounds: (rounds: RunRounds | null) => void;
  /** The prompt's own row, added on send so the bubble appears before the engine answers. */
  pushPrompt: (text: string, image?: string) => void;
  apply: (event: AgentEvent) => void;
};

export const useTranscript = create<TranscriptStore>((set) => ({
  rows: [],
  running: false,
  contextUsed: null,
  lastRun: null,
  runDiff: null,
  runRounds: null,
  runStartedAt: null,
  setRows: (rows) => set({ rows }),
  setRunning: (running) => set({ running, runStartedAt: running ? Date.now() : null }),
  setContextUsed: (contextUsed) => set({ contextUsed }),
  setLastRun: (lastRun) => set({ lastRun }),
  setRunDiff: (runDiff) => set({ runDiff }),
  setRunRounds: (runRounds) => set({ runRounds }),
  pushPrompt: (text, image) =>
    set((s) => ({
      rows: [...s.rows, { id: nextId("turn"), kind: "turn", role: "user", text, image }],
    })),
  apply: (event) =>
    set((s) => {
      const running =
        event.type === "runStarted"
          ? true
          : event.type === "runFinished" || event.type === "failed"
            ? false
            : s.running;
      return {
        rows: reduce(s.rows, event),
        running,
        // `send` already stamped the click, and the run's own announcement is inside the wait that
        // started there, so the origin stands. A window that re-rendered mid-run never saw the click
        // and takes the announcement instead of showing a clock that guessed an origin.
        runStartedAt: running ? (s.runStartedAt ?? Date.now()) : null,
        // The server counted the tokens; this is the one place that learns them. A run that reported
        // no usage leaves the last measured figure standing rather than blanking a meter that was
        // already telling the truth about the same window.
        contextUsed:
          event.type === "runFinished" ? (event.contextUsed ?? s.contextUsed) : s.contextUsed,
        // Unlike the meter, the report is about one run rather than about the window, so a run that
        // never timed itself replaces the last one's numbers instead of leaving them up.
        lastRun: event.type === "runFinished" ? event.run : s.lastRun,
        // The pill is the run's own ledger, and the host clears its copy when a send opens, so this
        // does too: a prompt that changes nothing shows a pill that has gone, not the last one's count
        // sitting above a composer it no longer describes.
        runDiff:
          event.type === "runStarted"
            ? null
            : event.type === "diffStaged"
              ? {
                  files: event.diff.filesChanged,
                  added: event.diff.linesAdded,
                  removed: event.diff.linesRemoved,
                }
              : event.type === "runFinished"
                ? event.totalDiff
                  ? {
                      files: event.totalDiff.filesChanged,
                      added: event.totalDiff.linesAdded,
                      removed: event.totalDiff.linesRemoved,
                    }
                  : s.runDiff
                : s.runDiff,
        // The step figure counts closed rounds, so it moves when the host says a round is over rather
        // than when the next one starts: a pill that read `Step 1 / 4` while the first round's tools
        // were still running would claim a round had finished that had not. A run with no budget has
        // nothing to count, and the pill stays out of the way.
        runRounds:
          event.type === "runStarted"
            ? event.maxRounds > 0
              ? { done: 0, max: event.maxRounds }
              : null
            : event.type === "toolBatchClosed" && s.runRounds
              ? { ...s.runRounds, done: s.runRounds.done + 1 }
              : s.runRounds,
      };
    }),
}));

/**
 * A scripted run used only to render the shell when there is nothing to show yet: reached through
 * `?fixture=1`, never by default. A transcript that invented its own tool results would be worse
 * than an empty one, so the real rows arrive with the agent loop and this stays opt-in.
 */
export const FIXTURE_ROWS: Row[] = [
  // A picture on the prompt, because the native window takes no injected clicks and the file panel a
  // pick would open is not something a capture can drive: this row is how the attached-picture bubble
  // gets looked at on the real surface. It is a stand-in for the JPEG bytes the composer produces, not
  // a second format the app accepts.
  {
    id: "fq",
    kind: "turn",
    role: "user",
    text: "continue",
    image:
      "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxNjAiIGhlaWdodD0iMTAwIj48cmVjdCB3aWR0aD0iMTYwIiBoZWlnaHQ9IjEwMCIgZmlsbD0iIzNmN2Y2YSIvPjx0ZXh0IHg9IjEyIiB5PSI1OCIgZm9udC1mYW1pbHk9InNhbnMtc2VyaWYiIGZvbnQtc2l6ZT0iMjAiIGZpbGw9IiNmZmZmZmY+cGljdHVyZTwvdGV4dD48L3N2Zz4=",
  },
  { id: "f1", kind: "processed" },
  {
    id: "f1t",
    kind: "turn",
    role: "assistant",
    // Markdown and LaTeX on purpose: this is also the row that proves the answer parser runs on
    // real model output rather than on a paragraph that happened to need no formatting. The link is
    // there for the same reason — an answer's address is a control now, not a URL to read out. The
    // fence has a comment, a blank line and a line longer than the column because those are the three
    // shapes the gutter and the wrap toggle are judged on: a numbered blank row, no phantom row from
    // the fence's trailing newline, and a continuation that either scrolls or breaks.
    text: "Picking up the next phase rather than waiting: wiring the loop, since that is what makes everything so far actually runnable.\n\nThe budget per turn is $O(n^2)$ in the replayed history, so the cache does most of the work:\n\n```rust\nlet ratio = cached as f32 / prompt as f32;\n// A line this long is what the wrap toggle is for: either it runs past the column and the block scrolls, or it breaks inside itself.\nif ratio > 0.6 { reuse(ratio) } else { replay_from_turn(0) }\n\nreplay_from_turn(0)\n```\n\nThe build that reads these weights is [the PrismML fork](https://github.com/PrismML-Eng/llama.cpp).",
  },
  {
    id: "f2",
    kind: "toolBatch",
    // The host declares a finished batch folded, and this is the row that shows what a run of the loop
    // looks like once it is over: one line naming the round, with the calls under it on demand.
    collapsed: true,
    children: [
      { id: "f2a", kind: "thought", detail: "The catalog pins a fork-only engine." },
      { id: "f2b", kind: "readFile", subject: "agent.rs", status: "ok" },
      {
        id: "f2c",
        kind: "editFile",
        subject: "apps/desktop/src-tauri/src/agent.rs",
        status: "ok",
        // The row's `diff` and its `hunk` are the same change counted and written, so the pill above
        // the composer and the review panel it opens cannot disagree. `?fixture=1` shows a shortened
        // edit rather than a 29-line one for the same reason the host shortens one: a hunk is cut to
        // the result budget, and the cut is what the panel has to survive.
        diff: { added: 6, removed: 3 },
        hunk: {
          at: 214,
          lines: [
            " let mut rows = Vec::new();",
            " let mut total: Option<DiffStat> = None;",
            "-run_round(&call, &policy).await;",
            "-rows.push(host_row(&outcome));",
            "-state.note(&path);",
            "+let outcome = dispatch(&call, &policy, &state).await;",
            "+rows.push(host_row(&outcome).with_detail(&call.path));",
            "+if let Some(edit) = outcome.edit {",
            "+    total = Some(state.stage(&edit.path, edit.added, edit.removed));",
            "+}",
            "+close_batch(&app, &mut rows, total);",
            " }",
            " broadcast(&app, AgentEvent::ToolBatchClosed);",
          ].join("\n"),
        },
      },
    ],
  },
  // A second prompt, so the review panel's two scopes have something to disagree about: the edit above
  // this row belongs to the previous turn, the two below it to the last one. A conversation with one
  // prompt cannot show the selector doing anything, and a selector that does nothing is not one.
  { id: "fq2", kind: "turn", role: "user", text: "now the round counter" },
  // The seam the server's trim leaves: the rule across the column, with the host's sentence about what
  // fell above it folded under it. No conversation on this machine has filled a window on purpose, so
  // this is how the seam and its copy get reviewed in either language.
  { id: "f3", kind: "contextCompressed", notice: { code: "context-trimmed", params: {} } },
  // A notice, on the footing of the answer it did not stop. Its sentence is the catalog's, so this row
  // is also how that copy gets looked at in either language without filling a context window on
  // purpose; the two numbers are the shape a real one arrives with, not this machine's measurement.
  {
    id: "f3n",
    kind: "blocked",
    status: "error",
    notice: { code: "prompt-too-long", params: { tokens: "5107", window: "4096" } },
  },
  {
    id: "f4",
    kind: "running",
    seconds: 10,
    children: [
      { id: "f4a", kind: "terminal", detail: "cargo test -p smollm-core", status: "ok" },
      { id: "f4b", kind: "searchContent", detail: "\\.run\\(", status: "ok" },
      // The read that found the line this batch then rewrote, naming a file the round also edits:
      // opening a file is a source and changing it is a change, so the same path belongs in both of
      // the inspector's lists, and this is the pair that shows the panel keeping them apart.
      {
        id: "f4b2",
        kind: "readFile",
        subject: "crates/smollm-core/src/transcript.rs",
        status: "ok",
      },
      // The round's one create, and the only hunk shape an edit cannot produce: every line of a new
      // file is an addition, with no context above or below it. The review panel has to hold that
      // without a removed line to anchor its gutter on.
      {
        id: "f4b3",
        kind: "createFile",
        subject: "demo/index.html",
        diff: { added: 6, removed: 0 },
        hunk: {
          at: 1,
          lines: [
            "<!doctype html>",
            "<meta charset=utf-8>",
            "<title>What the run made</title>",
            "<main>",
            "  <h1>A page written from nothing</h1>",
            "</main>",
          ]
            .map((line) => `+${line}`)
            .join("\n"),
        },
        status: "ok",
      },
      { id: "f4c", kind: "editFile", subject: "crates/smollm-core/src/lib.rs", status: "running" },
      {
        id: "f4d",
        kind: "editFile",
        subject: "crates/smollm-core/src/events.rs",
        detail: "1 copy replaced",
        detailKey: "transcript.summary.copyReplaced",
        diff: { added: 7, removed: 2 },
        hunk: {
          at: 41,
          lines: [
            " pub enum AgentEvent {",
            "     DiffStaged { diff: DiffStat },",
            "-    ToolFinished { step: TranscriptStep, ok: bool },",
            "-    RunFinished { elapsed_ms: u64 },",
            "+    ToolFinished {",
            "+        step: TranscriptStep,",
            "+        outcome: ToolOutcome,",
            "+    },",
            "+    RunFinished {",
            "+        elapsed_ms: u64,",
            "+        total_diff: Option<DiffStat>,",
            " }",
          ].join("\n"),
        },
        status: "ok",
      },
      {
        id: "f4e",
        kind: "editFile",
        subject: "apps/desktop/src-tauri/src/agent.rs",
        detail: "1 copy replaced",
        detailKey: "transcript.summary.copyReplaced",
        // The same file written a second time in one run, as shot 06's batch does: one section with two
        // hunks, and the stretch of the file between them named — the number a reader checks the
        // gutter against.
        diff: { added: 2, removed: 1 },
        hunk: {
          at: 262,
          lines: [
            " fn close_batch(app: &AppHandle, rows: &mut Vec<HostRow>, total: Option<DiffStat>) {",
            "     rows.push(HostRow::batch_closed(total));",
            '-    let _ = app.emit("run://round", rows.len());',
            "+    let round = u32::try_from(rows.len()).unwrap_or(u32::MAX);",
            '+    let _ = app.emit("run://round", round);',
            " }",
          ].join("\n"),
        },
        status: "ok",
      },
      {
        id: "f4f",
        kind: "editFile",
        subject: "crates/smollm-core/src/transcript.rs",
        detail: "1 copy replaced",
        detailKey: "transcript.summary.copyReplaced",
        // The third file of the round, and the one that gives the panel a section below the section a
        // jump lands on: a diff with two short sections cannot show a scroll arriving anywhere, because
        // there is nothing under it to scroll past.
        diff: { added: 12, removed: 4 },
        hunk: {
          at: 88,
          lines: [
            " pub struct TranscriptStep {",
            "     pub kind: StepKind,",
            "-    pub detail: Option<String>,",
            "-    pub subject: Option<String>,",
            "-    pub diff: Option<DiffStat>,",
            "-    pub hunk: Option<String>,",
            "+    /// The host's line for this row, already measured: the catalog never re-counts it.",
            "+    pub detail: Option<String>,",
            "+    /// The path the row touched, so a reader can jump from the run to the file.",
            "+    pub subject: Option<String>,",
            "+    /// The change counted, and the same change written out.",
            "+    pub diff: Option<DiffStat>,",
            "+    pub hunk: Option<String>,",
            "+    /// Whether the row starts folded over its children.",
            "+    pub collapsed: bool,",
            "+    /// What the run worked around, on the footing of the answer it did not stop.",
            "+    pub notice: Option<Notice>,",
            "+    /// The engine's own words, behind a refusal's disclosure.",
            "+    pub technical: Option<String>,",
            "     pub status: StepStatus,",
            " }",
            "",
            " impl TranscriptStep {",
            "     pub fn notice(code: impl Into<String>, params: Params) -> Self {",
            "         Self {",
            "             kind: StepKind::Blocked,",
            "             notice: Some(Notice {",
          ].join("\n"),
        },
        status: "ok",
      },
      // The calls that leave the machine. A real one of these needs the privacy switch on and a
      // model that decides to ask, so this is how a row's label and its subject chip get looked at
      // in either language — the chip carries what was searched for, and the address that was read.
      // The addresses keep the scheme the host writes them with, since that is what `fetch_url` was
      // handed; the inspector is the surface that cuts it back off, so the two are shown disagreeing
      // on purpose. The re-read is the same reason: three rows here are the two pages the run looked
      // at, which is the count the panel has to arrive at.
      { id: "f4g", kind: "webSearch", subject: "ternary quantization", status: "ok" },
      {
        id: "f4h",
        kind: "webReading",
        subject: "https://en.wikipedia.org/wiki/Quantization_(signal_processing)",
        status: "ok",
      },
      {
        id: "f4i",
        kind: "webReading",
        subject: "https://huggingface.co/prism-ml/Ternary-Bonsai-1.7B-gguf",
        status: "ok",
      },
      {
        id: "f4j",
        kind: "webReading",
        subject: "https://en.wikipedia.org/wiki/Quantization_(signal_processing)",
        status: "ok",
      },
    ],
  },
  // A refusal, as the load guard reports it, so the card can be looked at without uninstalling the
  // engine on a machine that already has one. It carries its code rather than its sentences: the
  // headline, the instruction and the button are the catalog's, which is what makes this row the way
  // the copy gets reviewed in either language. The evidence line is the shape a real one arrives with.
  {
    id: "f5",
    kind: "blocked",
    status: "error",
    refusal: { code: "engine-not-installed", params: {} },
    technical:
      "engine prism-b10787-73bfe6c is not installed (expected " +
      "~/Library/Application Support/studio.smollm.desktop/engines/llama-server from " +
      "llama-server-aarch64-apple-darwin-metal.tar.gz)",
    cta: {
      code: "install-engine",
      url: "https://github.com/PrismML-Eng/llama.cpp/releases/tag/prism-b10787-73bfe6c",
    },
  },
];

/**
 * The pill `?fixture=1` shows: the writes the last turn's rows report — one file made, three changed —
 * added up. It is written out rather than derived, so the number here is the shape a real `DiffStaged`
 * arrives with instead of a second source of truth computed from the rows — and it is *that* shape the
 * rows of one turn add up to, so the pill, the review panel's last-turn scope and the transcript all
 * describe one change. The panel's other scope counts the earlier turn's edit as well, which is the
 * point of having two.
 */
export const FIXTURE_DIFF: RunDiff = { files: 4, added: 27, removed: 7 };

/** The step pill `?fixture=1` shows beside it: one round closed of the four the host budgets. The
 * scripted rows carry one "Ran 3 tools" header, and this is the figure that header would have left
 * standing — the pill and the transcript agree rather than each inventing its own. */
export const FIXTURE_ROUNDS: RunRounds = { done: 1, max: 4 };
