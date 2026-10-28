// The terminal panel is a view of a command while it runs, so these tests hold the store to the two
// things that makes it: which rows open a tab, and that the bytes the host streams land in the right
// tab and the tab keeps the end of them. The last-run bound and the clear press are here too, because
// a panel that grows without end or empties a command that is still printing is the failure the
// bounds exist to prevent.
import { beforeEach, describe, expect, it } from "vitest";

import type { AgentEvent, HostStep } from "../lib/ipc";
import { FIXTURE_RUNS, MAX_OUTPUT, MAX_RUNS, useTerminal } from "./terminal";

function hostStep(patch: Partial<HostStep> & { id: string }): HostStep {
  return {
    kind: "terminal",
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

function started(
  id: string,
  kind: HostStep["kind"] = "terminal",
  detail = "pnpm test",
): AgentEvent {
  return { type: "toolStarted", step: hostStep({ id, kind, detail }) };
}

function output(stepId: string, chunk: string): AgentEvent {
  return { type: "terminalOutput", stepId, chunk };
}

function finished(id: string, ok = true): AgentEvent {
  return {
    type: "toolFinished",
    step: hostStep({ id, status: ok ? "done" : "blocked" }),
    outcome: { ok, summary: "", diff: null },
  };
}

function apply(event: AgentEvent): void {
  useTerminal.getState().apply(event);
}

beforeEach(() => {
  useTerminal.setState(useTerminal.getInitialState());
});

describe("a command opens a tab", () => {
  it("opens the panel on a command and labels the tab with the command", () => {
    apply(started("s1"));
    const state = useTerminal.getState();
    expect(state.open).toBe(true);
    expect(state.active).toBe("s1");
    expect(state.runs).toHaveLength(1);
    expect(state.runs[0]).toMatchObject({ id: "s1", label: "pnpm test", status: "running" });
  });

  it("opens on every kind of command, and on nothing else", () => {
    apply(started("t1", "tests"));
    apply(started("b1", "build"));
    apply(started("l1", "linter"));
    apply(started("r1", "readFile", "src/lib.rs"));
    apply(started("e1", "editFile", "src/lib.rs"));
    expect(useTerminal.getState().runs.map((run) => run.id)).toEqual(["t1", "b1", "l1"]);
    expect(useTerminal.getState().active).toBe("l1");
  });

  it("does not add the same row twice when the host restates it", () => {
    apply(started("s1"));
    apply(output("s1", "one"));
    apply(started("s1"));
    expect(useTerminal.getState().runs).toHaveLength(1);
    expect(useTerminal.getState().runs[0]?.output).toBe("one");
  });

  it("keeps the last runs and scrolls the oldest out", () => {
    for (let i = 0; i < MAX_RUNS + 3; i += 1) apply(started(`s${i}`));
    const runs = useTerminal.getState().runs;
    expect(runs).toHaveLength(MAX_RUNS);
    expect(runs[0]?.id).toBe("s3");
    expect(useTerminal.getState().active).toBe(`s${MAX_RUNS + 2}`);
  });
});

describe("output", () => {
  it("lands in the tab the host named and in no other", () => {
    apply(started("a"));
    apply(started("b"));
    apply(output("a", "first\n"));
    apply(output("b", "second\n"));
    apply(output("gone", "nobody\n"));
    const byId = Object.fromEntries(useTerminal.getState().runs.map((run) => [run.id, run.output]));
    expect(byId).toEqual({ a: "first\n", b: "second\n" });
  });

  it("keeps the end of a command that printed more than the panel holds", () => {
    apply(started("s1"));
    apply(output("s1", "x".repeat(MAX_OUTPUT) + "TAIL"));
    const kept = useTerminal.getState().runs[0]?.output ?? "";
    expect(kept).toHaveLength(MAX_OUTPUT);
    expect(kept.endsWith("TAIL")).toBe(true);
  });

  it("does not leave half a character when it trims the front", () => {
    apply(started("s1"));
    // An emoji is a surrogate pair; the cut has to move past it rather than keep its second half.
    apply(output("s1", "😀" + "x".repeat(MAX_OUTPUT)));
    const kept = useTerminal.getState().runs[0]?.output ?? "";
    expect(kept.charCodeAt(0)).not.toBe(0xde00);
    expect(kept.length).toBeLessThanOrEqual(MAX_OUTPUT);
  });

  it("marks a finished command by how it ended", () => {
    apply(started("ok"));
    apply(started("bad"));
    apply(finished("ok", true));
    apply(finished("bad", false));
    const byId = Object.fromEntries(useTerminal.getState().runs.map((run) => [run.id, run.status]));
    expect(byId).toEqual({ ok: "ok", bad: "error" });
  });
});

describe("the strip's controls", () => {
  it("clears what has finished and keeps what is still printing", () => {
    apply(started("done"));
    apply(finished("done"));
    apply(started("live"));
    useTerminal.getState().clear();
    expect(useTerminal.getState().runs.map((run) => run.id)).toEqual(["live"]);
    expect(useTerminal.getState().active).toBe("live");
  });

  it("closes one tab and hands the strip to its neighbour", () => {
    apply(started("a"));
    apply(started("b"));
    apply(started("c"));
    useTerminal.getState().focus("b");
    useTerminal.getState().dismiss("b");
    expect(useTerminal.getState().runs.map((run) => run.id)).toEqual(["a", "c"]);
    expect(useTerminal.getState().active).toBe("c");

    useTerminal.getState().dismiss("c");
    expect(useTerminal.getState().active).toBe("a");

    useTerminal.getState().dismiss("a");
    expect(useTerminal.getState().open).toBe(false);
    expect(useTerminal.getState().active).toBeNull();
  });

  it("shows a tab the panel already holds, and does nothing for one it does not", () => {
    apply(started("a"));
    useTerminal.getState().setOpen(false);
    useTerminal.getState().focus("a");
    expect(useTerminal.getState().open).toBe(true);

    useTerminal.getState().setOpen(false);
    useTerminal.getState().focus("gone");
    expect(useTerminal.getState().open).toBe(false);
  });
});

describe("fixture", () => {
  it("scripts a finished check, so the panel's chrome has a tab to draw", () => {
    expect(useTerminal.getState().runs).toHaveLength(0);
    useTerminal.getState().seed(FIXTURE_RUNS);
    const state = useTerminal.getState();
    expect(state.runs).toHaveLength(1);
    expect(state.active).toBe("f4a");
    expect(state.runs[0]).toMatchObject({ kind: "tests", status: "ok" });
    expect(state.runs[0]?.output).toContain("17 passed");
  });
});
