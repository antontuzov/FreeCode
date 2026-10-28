// The panel's own shell is a log the host admits one line at a time. These tests hold it to the two
// promises that make it safe to put in the window: a refused line never appears as run (so the log
// cannot show a command the dial turned down), and the text a session prints is only ever applied to
// the session that printed it — a restarted shell must not inherit the old one's output.
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ipc, type ShellEvent } from "../lib/ipc";
import { useShell } from "./shell";

vi.mock("../lib/ipc", () => ({
  ipc: {
    shellOpen: vi.fn(),
    shellSend: vi.fn(),
    shellInterrupt: vi.fn(),
    shellResize: vi.fn(),
    shellClose: vi.fn(),
    onShellEvent: vi.fn(),
  },
}));

const mock = vi.mocked(ipc);

const INFO = { id: 7, cwd: "/work/project" };

function chunk(id: number, text: string): ShellEvent {
  return { kind: "chunk", id, text };
}

beforeEach(() => {
  vi.clearAllMocks();
  useShell.setState(useShell.getInitialState());
});

describe("opening and closing", () => {
  it("opens a session and empties the log it starts on", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    useShell.setState({ text: "from an earlier session" });
    await expect(useShell.getState().open()).resolves.toBe(true);
    const state = useShell.getState();
    expect(state.status).toBe("live");
    expect(state.info).toEqual(INFO);
    expect(state.text).toBe("");
    expect(state.refusal).toBeNull();
  });

  it("says which rule refused the start rather than going quiet", async () => {
    mock.shellOpen.mockRejectedValue({ code: "off" });
    await expect(useShell.getState().open()).resolves.toBe(false);
    const state = useShell.getState();
    expect(state.status).toBe("closed");
    expect(state.refusal).toEqual({ code: "off" });
  });

  it("reports a host that could not be reached as a start failure", async () => {
    mock.shellOpen.mockRejectedValue("shell_open is not a function");
    await useShell.getState().open();
    expect(useShell.getState().refusal).toEqual({
      code: "start",
      reason: "shell_open is not a function",
    });
  });

  it("does not open a second process over a live one", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    await useShell.getState().open();
    await useShell.getState().open();
    expect(mock.shellOpen).toHaveBeenCalledTimes(1);
  });

  it("kills the session and forgets it", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    mock.shellClose.mockResolvedValue(undefined);
    await useShell.getState().open();
    await useShell.getState().close();
    expect(mock.shellClose).toHaveBeenCalled();
    expect(useShell.getState().status).toBe("closed");
    expect(useShell.getState().info).toBeNull();
  });
});

describe("sending a line", () => {
  it("sends the command and keeps the refusal of the last line from lingering", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    mock.shellSend.mockResolvedValue(undefined);
    await useShell.getState().open();
    useShell.setState({ refusal: { code: "blocked", command: "rm -rf ." } });
    await expect(useShell.getState().send("cargo test")).resolves.toBe(true);
    expect(mock.shellSend).toHaveBeenCalledWith("cargo test");
    expect(useShell.getState().refusal).toBeNull();
  });

  it("leaves the log untouched when the dial refuses the line", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    mock.shellSend.mockRejectedValue({ code: "blocked", command: "rm -rf ." });
    await useShell.getState().open();
    useShell.getState().apply(chunk(7, "ready\n"));
    await expect(useShell.getState().send("rm -rf .")).resolves.toBe(false);
    const state = useShell.getState();
    expect(state.text).toBe("ready\n");
    expect(state.refusal).toEqual({ code: "blocked", command: "rm -rf ." });
    // The session is still there: one refused line is not an ended shell.
    expect(state.status).toBe("live");
  });

  it("treats a send to an ended shell as the shell having ended", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    mock.shellSend.mockRejectedValue({ code: "closed" });
    await useShell.getState().open();
    await useShell.getState().send("cargo test");
    expect(useShell.getState().status).toBe("ended");
    expect(useShell.getState().refusal).toEqual({ code: "closed" });
  });

  it("does not send an empty line", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    await useShell.getState().open();
    await expect(useShell.getState().send("   ")).resolves.toBe(false);
    expect(mock.shellSend).not.toHaveBeenCalled();
  });
});

describe("the host's events", () => {
  it("writes the admitted line above the output it produced", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    await useShell.getState().open();
    useShell.getState().apply({ kind: "sent", id: 7, command: "pwd" });
    useShell.getState().apply(chunk(7, "/work/project\n"));
    expect(useShell.getState().text).toBe("$ pwd\n/work/project\n");
  });

  it("ignores a session this window has already replaced", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    await useShell.getState().open();
    useShell.getState().apply(chunk(6, "stale\n"));
    expect(useShell.getState().text).toBe("");
    expect(useShell.getState().status).toBe("live");
  });

  it("ends the shell on its exit code", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    await useShell.getState().open();
    useShell.getState().apply({ kind: "exit", id: 7, code: 130 });
    expect(useShell.getState().status).toBe("ended");
    expect(useShell.getState().exitCode).toBe(130);
  });

  it("keeps the end of a very long log rather than all of it", async () => {
    mock.shellOpen.mockResolvedValue(INFO);
    await useShell.getState().open();
    useShell.getState().apply(chunk(7, "a".repeat(300 * 1024)));
    useShell.getState().apply(chunk(7, "tail"));
    const text = useShell.getState().text;
    expect(text.length).toBeLessThanOrEqual(256 * 1024);
    expect(text.endsWith("tail")).toBe(true);
  });
});
