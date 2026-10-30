// The status strip's store has one job now: read the host's verdict about the route the next prompt
// takes, and never claim one of its own when the host cannot answer.
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ipc, type ModelStatus } from "../lib/ipc";
import { useModel } from "./model";

vi.mock("../lib/ipc", () => ({
  ipc: { modelStatus: vi.fn() },
}));

const mock = vi.mocked(ipc);

const STATUS: ModelStatus = {
  remote: null,
  free: { id: "big-pickle", label: "Big Pickle", provider: "OpenCode" },
  opencode: { available: true, version: "2.2.0" },
};

describe("the model store", () => {
  beforeEach(() => {
    useModel.setState({ status: null, error: null, loading: false });
    mock.modelStatus.mockReset();
  });

  it("holds the host's verdict", async () => {
    mock.modelStatus.mockResolvedValueOnce(STATUS);
    await useModel.getState().refresh();
    expect(useModel.getState().status).toEqual(STATUS);
    expect(useModel.getState().error).toBeNull();
    expect(useModel.getState().loading).toBe(false);
  });

  it("keeps no verdict when the host refuses to answer", async () => {
    mock.modelStatus.mockRejectedValueOnce(new Error("no catalog"));
    await useModel.getState().refresh();
    expect(useModel.getState().status).toBeNull();
    expect(useModel.getState().error).toContain("no catalog");
  });
});
