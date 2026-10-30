// The chat store owns the fact the transcript cannot see — whether a reply is in flight — and it is
// the seam where a command failure becomes either a transcript row or a card error. These tests hold
// those boundaries, not the host's answers.
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ipc, type AgentEvent, type RunReport } from "../lib/ipc";
import { connectAgentEvents, useChat } from "./chat";
import { useSessions } from "./sessions";
import { useTranscript } from "./transcript";

vi.mock("../lib/ipc", () => ({
  ipc: {
    chatSend: vi.fn(),
    chatStop: vi.fn(),
    newTask: vi.fn(),
    fileRows: vi.fn(),
    onAgentEvent: vi.fn(),
    listSessions: vi.fn(),
    openSession: vi.fn(),
    deleteSession: vi.fn(),
  },
}));

const mock = vi.mocked(ipc);

const REPORT: RunReport = {
  generatedTokens: 12,
  promptTokens: 30,
  cachedTokens: 0,
  generatedPerSecond: 4.5,
  promptPerSecond: 100,
  elapsedMs: 2_500,
};

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
} {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function at<T>(list: T[], index: number): T {
  const item = list[index];
  if (item === undefined) throw new Error(`no entry at ${index}`);
  return item;
}

beforeEach(() => {
  vi.clearAllMocks();
  useChat.setState(useChat.getInitialState());
  useTranscript.setState(useTranscript.getInitialState());
  useSessions.setState(useSessions.getInitialState());
  mock.listSessions.mockResolvedValue([]);
  mock.fileRows.mockResolvedValue(undefined);
});

describe("sending", () => {
  it("refuses an empty prompt with no picture, and sends one that is only a picture", async () => {
    await useChat.getState().send("   ");
    expect(mock.chatSend).not.toHaveBeenCalled();
    expect(useTranscript.getState().rows).toEqual([]);

    mock.chatSend.mockResolvedValueOnce("ok");
    await useChat.getState().send("", "data:image/png;base64,AA");
    // Even a picture with no words is an agent prompt: the tools always go with it.
    expect(mock.chatSend).toHaveBeenCalledWith("", "data:image/png;base64,AA", true);
  });

  it("shows the bubble on the click, before the host has answered", async () => {
    mock.chatSend.mockResolvedValueOnce("ok");
    const send = useChat.getState().send("hello");
    expect(at(useTranscript.getState().rows, 0)).toMatchObject({
      kind: "turn",
      role: "user",
      text: "hello",
    });
    expect(useChat.getState().sending).toBe(true);
    await send;
  });

  it("sends one prompt at a time", async () => {
    const gate = deferred<string>();
    mock.chatSend.mockReturnValueOnce(gate.promise);
    const first = useChat.getState().send("one");
    await useChat.getState().send("two");

    expect(mock.chatSend).toHaveBeenCalledTimes(1);
    expect(useTranscript.getState().rows).toHaveLength(1);

    gate.resolve("done");
    await first;
  });

  it("closes the send out, refreshes the server and the filed list, and hands the rows over", async () => {
    mock.chatSend.mockResolvedValueOnce("answer");
    await useChat.getState().send("hello", "data:image/png;base64,AA");

    expect(mock.chatSend).toHaveBeenCalledWith("hello", "data:image/png;base64,AA", true);
    expect(useChat.getState().sending).toBe(false);
    expect(useTranscript.getState().running).toBe(false);
    expect(mock.listSessions).toHaveBeenCalled();
    expect(mock.fileRows).toHaveBeenCalledTimes(1);

    const filed = at(mock.fileRows.mock.calls, 0)[0];
    expect(at(filed, 0)).toMatchObject({ kind: "turn", role: "user", text: "hello" });
    expect(at(filed, 0)).not.toHaveProperty("image");
  });

  it("pushes the failure onto the transcript when the command refused before announcing anything", async () => {
    mock.chatSend.mockRejectedValueOnce(new Error("engine is not running"));
    await useChat.getState().send("hello");

    const state = useTranscript.getState();
    expect(state.rows).toHaveLength(2);
    expect(at(state.rows, 1)).toMatchObject({ kind: "blocked" });
    expect(at(state.rows, 1)).toHaveProperty(
      "detail",
      expect.stringContaining("engine is not running"),
    );
    expect(useChat.getState().sending).toBe(false);
  });

  it("leaves a failure that already announced itself to the row the event channel pushed", async () => {
    mock.chatSend.mockImplementationOnce(async () => {
      useChat.setState({ announced: true });
      throw new Error("the stream ended early");
    });
    await useChat.getState().send("hello");

    expect(useTranscript.getState().rows).toHaveLength(1);
    expect(at(useTranscript.getState().rows, 0)).toMatchObject({ role: "user" });
  });
});

describe("stopping", () => {
  it("reports a stop the host refused and sends one that succeeds", async () => {
    mock.chatStop.mockRejectedValueOnce(new Error("nothing is streaming"));
    await useChat.getState().stop();
    expect(useChat.getState().error).toContain("nothing is streaming");

    vi.clearAllMocks();
    mock.chatStop.mockResolvedValueOnce(undefined);
    await useChat.getState().stop();
    expect(mock.chatStop).toHaveBeenCalledTimes(1);
  });
});

describe("a new task", () => {
  it("throws the conversation away only after the host has forgotten its history", async () => {
    useTranscript.setState({
      rows: [{ id: "t", kind: "turn", role: "user", text: "old" }],
      contextUsed: 50,
      lastRun: REPORT,
      runDiff: { files: 1, added: 1, removed: 0 },
      runRounds: { done: 1, max: 2 },
    });
    useSessions.setState({ activeId: "s1" });
    mock.newTask.mockResolvedValueOnce(undefined);

    await useChat.getState().newTask();

    const transcript = useTranscript.getState();
    expect(transcript.rows).toEqual([]);
    expect(transcript.contextUsed).toBeNull();
    expect(transcript.lastRun).toBeNull();
    expect(transcript.runDiff).toBeNull();
    expect(transcript.runRounds).toBeNull();
    expect(useSessions.getState().activeId).toBeNull();
    expect(mock.listSessions).toHaveBeenCalled();
  });

  it("keeps the rows when the host refused to forget the history", async () => {
    const rows = [{ id: "t", kind: "turn" as const, role: "user" as const, text: "old" }];
    useTranscript.getState().setRows(rows);
    useSessions.setState({ activeId: "s1" });
    mock.newTask.mockRejectedValueOnce(new Error("a reply is streaming"));

    await useChat.getState().newTask();

    expect(useTranscript.getState().rows).toBe(rows);
    expect(useSessions.getState().activeId).toBe("s1");
    expect(useChat.getState().error).toContain("reply is streaming");
  });
});

describe("the event channel", () => {
  it("marks a send as announced and lets the event reach the transcript", async () => {
    let listener: ((event: AgentEvent) => void) | undefined;
    const unlisten = vi.fn();
    mock.onAgentEvent.mockImplementationOnce(async (callback) => {
      listener = callback;
      return unlisten;
    });

    const stop = await connectAgentEvents();
    if (!listener) throw new Error("the channel was never subscribed");
    listener({
      type: "runStarted",
      runId: "r1",
      modelLabel: "Bonsai",
      local: true,
      maxRounds: 2,
      plan: false,
    });

    expect(useChat.getState().announced).toBe(true);
    expect(useTranscript.getState().running).toBe(true);
    expect(useTranscript.getState().rows).toHaveLength(1);

    stop();
    expect(unlisten).toHaveBeenCalled();
  });
});
