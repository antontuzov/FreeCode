// The status strip's store owns two fetches that clear two different refusals. Each has the same
// contract — single-flight, subscribe before invoke, refresh the host's verdict afterwards, and let
// go of the listener either way — and the two run in parallel rather than sharing a guard.
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  ipc,
  type EngineInstallProgress,
  type InstallOutcome,
  type ModelStatus,
  type WeightsDownloaded,
  type WeightsProgress,
} from "../lib/ipc";
import { useModel } from "./model";

vi.mock("../lib/ipc", () => ({
  ipc: {
    modelStatus: vi.fn(),
    onEngineInstall: vi.fn(),
    installEngine: vi.fn(),
    onWeightsDownload: vi.fn(),
    downloadWeights: vi.fn(),
  },
}));

const mock = vi.mocked(ipc);

const STATUS: ModelStatus = {
  catalogId: "ternary-bonsai-1-7b",
  displayName: "Ternary Bonsai 1.7B",
  repoId: "prism-ml/Ternary-Bonsai-1.7B-gguf",
  backend: "Metal",
  engineTag: "prism-b10787-73bfe6c",
  engineInstalled: true,
  weightsPresent: true,
  local: true,
  remote: null,
  packing: "PQ2_0",
  sizeGb: 0.46,
  warnings: [],
  uncheckedWarnings: [],
  packs: [],
  blocked: null,
};

const INSTALLED: InstallOutcome = {
  tag: "prism-b10787-73bfe6c",
  targetKey: "macos-arm64-metal",
  asset: "llama-server-aarch64-apple-darwin-metal.tar.gz",
  sha256: "ab",
  serverPath: "/engines/prism-b10787-73bfe6c/macos-arm64/llama-server",
};

const DOWNLOADED: WeightsDownloaded = {
  catalogId: "ternary-bonsai-1-7b",
  packing: "PQ2_0",
  file: "Ternary-Bonsai-1.7B-PQ2_0.gguf",
  sha256: "cd",
  path: "/models/Ternary-Bonsai-1.7B-PQ2_0.gguf",
  sizeBytes: 442_000_000,
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
  useModel.setState(useModel.getInitialState());
});

describe("refreshing the status", () => {
  it("holds loading while the host is read, then takes its verdict", async () => {
    const gate = deferred<ModelStatus>();
    mock.modelStatus.mockReturnValueOnce(gate.promise);

    const pending = useModel.getState().refresh();
    expect(useModel.getState().loading).toBe(true);

    gate.resolve(STATUS);
    await pending;
    expect(useModel.getState().status).toBe(STATUS);
    expect(useModel.getState().loading).toBe(false);
    expect(useModel.getState().error).toBeNull();
  });

  it("reports a status it could not read rather than claiming the old one", async () => {
    mock.modelStatus.mockRejectedValueOnce(new Error("catalog is missing"));
    await useModel.getState().refresh();
    expect(useModel.getState().status).toBeNull();
    expect(useModel.getState().loading).toBe(false);
    expect(useModel.getState().error).toContain("catalog is missing");
  });
});

describe("installing the engine", () => {
  it("subscribes before invoking, shows progress, then re-reads the host's verdict", async () => {
    const order: string[] = [];
    const listeners: ((progress: EngineInstallProgress) => void)[] = [];
    const unlisten = vi.fn();
    mock.onEngineInstall.mockImplementationOnce(async (callback) => {
      order.push("listen");
      listeners.push(callback);
      return unlisten;
    });
    const gate = deferred<InstallOutcome>();
    mock.installEngine.mockImplementationOnce(async () => {
      order.push("install");
      return gate.promise;
    });
    mock.modelStatus.mockResolvedValueOnce(STATUS);

    const pending = useModel.getState().installEngine();

    expect(useModel.getState().installing).toBe(true);
    expect(order).toEqual(["listen"]);

    await Promise.resolve();
    expect(order).toEqual(["listen", "install"]);

    at(listeners, 0)({ phase: "downloading", transferredBytes: 10, totalBytes: 20 });
    expect(useModel.getState().progress).toEqual({
      phase: "downloading",
      transferredBytes: 10,
      totalBytes: 20,
    });

    gate.resolve(INSTALLED);
    await pending;

    const state = useModel.getState();
    expect(state.installing).toBe(false);
    expect(state.progress).toBeNull();
    expect(state.status).toBe(STATUS);
    expect(state.installError).toBeNull();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("ignores a second press while the first fetch is still in flight", async () => {
    const gate = deferred<InstallOutcome>();
    mock.onEngineInstall.mockResolvedValueOnce(vi.fn());
    mock.installEngine.mockReturnValueOnce(gate.promise);

    const first = useModel.getState().installEngine();
    await Promise.resolve();
    await useModel.getState().installEngine();

    expect(mock.installEngine).toHaveBeenCalledTimes(1);
    gate.resolve(INSTALLED);
    await first;
  });

  it("puts a refused install on the card, releases the listener, and does not claim a verdict", async () => {
    const unlisten = vi.fn();
    mock.onEngineInstall.mockResolvedValueOnce(unlisten);
    mock.installEngine.mockRejectedValueOnce(new Error("release asset is gone"));

    await useModel.getState().installEngine();

    const state = useModel.getState();
    expect(state.installError).toContain("release asset is gone");
    expect(state.installing).toBe(false);
    expect(state.progress).toBeNull();
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(mock.modelStatus).not.toHaveBeenCalled();
  });
});

describe("downloading the weights", () => {
  it("follows the same contract as the install", async () => {
    const order: string[] = [];
    const listeners: ((progress: WeightsProgress) => void)[] = [];
    const unlisten = vi.fn();
    mock.onWeightsDownload.mockImplementationOnce(async (callback) => {
      order.push("listen");
      listeners.push(callback);
      return unlisten;
    });
    mock.downloadWeights.mockImplementationOnce(async () => {
      order.push("download");
      return DOWNLOADED;
    });
    mock.modelStatus.mockResolvedValueOnce(STATUS);

    const pending = useModel.getState().downloadWeights();
    expect(useModel.getState().downloading).toBe(true);
    expect(order).toEqual(["listen"]);

    at(listeners, 0)({ phase: "downloading", transferredBytes: 1, totalBytes: 2 });
    await pending;

    expect(order).toEqual(["listen", "download"]);
    const state = useModel.getState();
    expect(state.downloading).toBe(false);
    expect(state.downloadProgress).toBeNull();
    expect(state.status).toBe(STATUS);
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("ignores a second press and reports a failed download without refreshing", async () => {
    const gate = deferred<WeightsDownloaded>();
    mock.onWeightsDownload.mockResolvedValueOnce(vi.fn());
    mock.downloadWeights.mockReturnValueOnce(gate.promise);

    const first = useModel.getState().downloadWeights();
    await Promise.resolve();
    await useModel.getState().downloadWeights();
    expect(mock.downloadWeights).toHaveBeenCalledTimes(1);
    gate.resolve(DOWNLOADED);
    await first;
    mock.modelStatus.mockClear();

    mock.onWeightsDownload.mockResolvedValueOnce(vi.fn());
    mock.downloadWeights.mockRejectedValueOnce(new Error("disk is full"));
    await useModel.getState().downloadWeights();

    expect(useModel.getState().downloadError).toContain("disk is full");
    expect(mock.modelStatus).not.toHaveBeenCalled();
  });

  it("does not wait for an engine install that is still running", async () => {
    const installGate = deferred<InstallOutcome>();
    mock.onEngineInstall.mockResolvedValueOnce(vi.fn());
    mock.installEngine.mockReturnValueOnce(installGate.promise);
    const installing = useModel.getState().installEngine();
    await Promise.resolve();

    const downloadGate = deferred<WeightsDownloaded>();
    mock.onWeightsDownload.mockResolvedValueOnce(vi.fn());
    mock.downloadWeights.mockReturnValueOnce(downloadGate.promise);
    const downloading = useModel.getState().downloadWeights();
    await Promise.resolve();

    expect(useModel.getState().installing).toBe(true);
    expect(useModel.getState().downloading).toBe(true);
    expect(mock.downloadWeights).toHaveBeenCalledTimes(1);

    installGate.resolve(INSTALLED);
    downloadGate.resolve(DOWNLOADED);
    await Promise.all([installing, downloading]);
  });
});
