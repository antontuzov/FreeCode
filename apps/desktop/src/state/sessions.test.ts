// A filed conversation is the host's file read back into this window's row model. These tests hold
// the join to its contract: ids are re-keyed so they cannot collide with a new run, pictures come
// back from `turns` and only onto the row that earned them, and a delete that succeeds is the one
// thing that empties the screen.
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ipc, type RunReport, type Session, type SessionSummary } from "../lib/ipc";
import { fileable, useSessions } from "./sessions";
import { isTurn, useTranscript, type Row } from "./transcript";

vi.mock("../lib/ipc", () => ({
  ipc: {
    listSessions: vi.fn(),
    openSession: vi.fn(),
    deleteSession: vi.fn(),
  },
}));

const mock = vi.mocked(ipc);

const REPORT: RunReport = {
  generatedTokens: 1,
  promptTokens: 2,
  cachedTokens: 0,
  generatedPerSecond: 1,
  promptPerSecond: 1,
  elapsedMs: 1,
};

function summary(id: string): SessionSummary {
  return { id, workspace: "/work", title: id, model: "bonsai", updatedAtMs: 1, turnCount: 1 };
}

function filed(patch: Partial<Session> & { id: string }): Session {
  return {
    schemaVersion: 1,
    workspace: "/work",
    title: patch.id,
    model: "bonsai",
    updatedAtMs: 1,
    turns: [],
    ...patch,
  };
}

function at<T>(list: T[], index: number): T {
  const item = list[index];
  if (item === undefined) throw new Error(`no row at ${index}`);
  return item;
}

beforeEach(() => {
  useSessions.setState(useSessions.getInitialState());
  useTranscript.setState(useTranscript.getInitialState());
  vi.clearAllMocks();
});

describe("the list", () => {
  it("takes the host's order, and empties itself rather than keeping rows that may no longer exist", async () => {
    mock.listSessions.mockResolvedValueOnce([summary("a"), summary("b")]);
    await useSessions.getState().refresh();
    expect(useSessions.getState().items.map((item) => item.id)).toEqual(["a", "b"]);
    expect(useSessions.getState().error).toBeNull();

    useSessions.setState({ items: [summary("stale")] });
    mock.listSessions.mockRejectedValueOnce(new Error("store unreadable"));
    await useSessions.getState().refresh();
    expect(useSessions.getState().items).toEqual([]);
    expect(useSessions.getState().error).toContain("store unreadable");
  });
});

describe("opening a conversation", () => {
  it("re-keys the filed rows, including the calls under a folded batch", async () => {
    const rows: Row[] = [
      { id: "turn-4", kind: "turn", role: "user", text: "hi" },
      {
        id: "b1",
        kind: "toolBatch",
        children: [{ id: "read-1", kind: "readFile", subject: "a.rs", status: "ok" }],
      },
      { id: "turn-5", kind: "turn", role: "assistant", text: "yo" },
    ];
    mock.openSession.mockResolvedValueOnce(filed({ id: "s1", title: "hi", rows }));

    await useSessions.getState().open("s1");

    const written = useTranscript.getState().rows;
    expect(written.map((row) => row.id)).toEqual(["s1-0", "s1-1", "s1-2"]);
    const batch = at(written, 1);
    if (isTurn(batch)) throw new Error("expected a batch");
    expect(batch.children?.map((child) => child.id)).toEqual(["s1-1-child-0"]);
    expect(useSessions.getState().activeId).toBe("s1");
  });

  it("puts each picture back on the turn it was sent with, and nowhere else", async () => {
    const rows: Row[] = [
      // A prompt the host refused is a row but never became a turn: the picture after it belongs to
      // the prompt that did, not to this one.
      { id: "turn-1", kind: "turn", role: "user", text: "a refused prompt" },
      { id: "turn-2", kind: "turn", role: "user", text: "look" },
      { id: "turn-3", kind: "turn", role: "assistant", text: "a duck" },
    ];
    mock.openSession.mockResolvedValueOnce(
      filed({
        id: "s1",
        rows,
        turns: [
          { role: "user", text: "look", image: "data:image/png;base64,AA" },
          { role: "assistant", text: "a duck" },
        ],
      }),
    );

    await useSessions.getState().open("s1");

    const written = useTranscript.getState().rows;
    expect(written[0]).not.toHaveProperty("image");
    expect(at(written, 1)).toMatchObject({ image: "data:image/png;base64,AA" });
    expect(written[2]).not.toHaveProperty("image");
  });

  it("rebuilds a conversation an older build filed from its turns alone", async () => {
    mock.openSession.mockResolvedValueOnce(
      filed({
        id: "s2",
        turns: [
          { role: "user", text: "hi", image: "data:image/png;base64,AA" },
          { role: "assistant", text: "yo" },
        ],
      }),
    );

    await useSessions.getState().open("s2");

    expect(useTranscript.getState().rows).toEqual([
      { id: "s2-0", kind: "turn", role: "user", text: "hi", image: "data:image/png;base64,AA" },
      { id: "s2-1", kind: "turn", role: "assistant", text: "yo", image: undefined },
    ]);
  });

  it("forgets the previous conversation's run figures when a new one comes on screen", async () => {
    useTranscript.setState({
      contextUsed: 51,
      lastRun: REPORT,
      runDiff: { files: 2, added: 3, removed: 1 },
      runRounds: { done: 1, max: 4 },
    });
    mock.openSession.mockResolvedValueOnce(filed({ id: "s3" }));

    await useSessions.getState().open("s3");

    const state = useTranscript.getState();
    expect(state.contextUsed).toBeNull();
    expect(state.lastRun).toBeNull();
    expect(state.runDiff).toBeNull();
    expect(state.runRounds).toBeNull();
  });

  it("keeps what the window was showing when the file cannot be read", async () => {
    const showing: Row[] = [{ id: "t", kind: "turn", role: "user", text: "still here" }];
    useTranscript.getState().setRows(showing);
    useSessions.setState({ activeId: "old" });
    mock.openSession.mockRejectedValueOnce(new Error("conversation gone"));

    await useSessions.getState().open("missing");

    expect(useTranscript.getState().rows).toBe(showing);
    expect(useSessions.getState().activeId).toBe("old");
    expect(useSessions.getState().error).toContain("conversation gone");
  });
});

describe("deleting a conversation", () => {
  it("drops an inactive row without touching the conversation on screen", async () => {
    useSessions.setState({ items: [summary("a"), summary("b")], activeId: "a" });
    const showing: Row[] = [{ id: "t", kind: "turn", role: "user", text: "still here" }];
    useTranscript.getState().setRows(showing);
    mock.deleteSession.mockResolvedValueOnce(undefined);

    await useSessions.getState().remove("b");

    expect(useSessions.getState().items.map((item) => item.id)).toEqual(["a"]);
    expect(useSessions.getState().activeId).toBe("a");
    expect(useTranscript.getState().rows).toBe(showing);
  });

  it("empties the screen when the row deleted is the conversation on it", async () => {
    useSessions.setState({ items: [summary("a"), summary("b")], activeId: "a" });
    useTranscript.setState({
      rows: [{ id: "t", kind: "turn", role: "user", text: "still here" }],
      contextUsed: 51,
      lastRun: REPORT,
      runDiff: { files: 1, added: 1, removed: 0 },
      runRounds: { done: 1, max: 2 },
    });
    mock.deleteSession.mockResolvedValueOnce(undefined);

    await useSessions.getState().remove("a");

    expect(useSessions.getState().items.map((item) => item.id)).toEqual(["b"]);
    expect(useSessions.getState().activeId).toBeNull();
    const state = useTranscript.getState();
    expect(state.rows).toEqual([]);
    expect(state.contextUsed).toBeNull();
    expect(state.lastRun).toBeNull();
    expect(state.runDiff).toBeNull();
    expect(state.runRounds).toBeNull();
  });

  it("keeps the row when the host refuses the delete, because a conversation is still on disk", async () => {
    useSessions.setState({ items: [summary("a")], activeId: "a" });
    const showing: Row[] = [{ id: "t", kind: "turn", role: "user", text: "still here" }];
    useTranscript.getState().setRows(showing);
    mock.deleteSession.mockRejectedValueOnce(new Error("a reply is streaming"));

    await useSessions.getState().remove("a");

    expect(useSessions.getState().items.map((item) => item.id)).toEqual(["a"]);
    expect(useSessions.getState().activeId).toBe("a");
    expect(useTranscript.getState().rows).toBe(showing);
    expect(useSessions.getState().error).toContain("reply is streaming");
  });
});

describe("what is handed back to the host", () => {
  it("files a turn without its picture or its reasoning, and a step exactly as drawn", () => {
    const turn = {
      id: "t",
      kind: "turn" as const,
      role: "user" as const,
      text: "hi",
      image: "data:image/png;base64,AA",
      thought: "a private thought",
      streaming: false,
    };
    const step = { id: "s", kind: "terminal" as const, detail: "cargo test" };

    const handed = fileable([turn, step]);

    expect(handed[0]).toMatchObject({ id: "t", text: "hi", streaming: false });
    expect(handed[0]).not.toHaveProperty("image");
    expect(handed[0]).not.toHaveProperty("thought");
    expect(handed[1]).toBe(step);
    // The window's own row is not mutated by filing a copy of it.
    expect(turn.image).toBe("data:image/png;base64,AA");
    expect(turn.thought).toBe("a private thought");
  });
});
