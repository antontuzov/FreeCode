// The status strip reads the same catalog + hardware choice the CLI verifier checks, so what the UI
// claims about the local model is the same thing CI asserts about the files on disk.
import { create } from "zustand";

import { ipc, type EngineInstallProgress, type ModelStatus } from "../lib/ipc";

type ModelStore = {
  status: ModelStatus | null;
  error: string | null;
  loading: boolean;
  /** Set while the host is fetching the pinned engine; the Install control keys off this. */
  installing: boolean;
  /** The host's last progress event, or null when nothing is in flight. */
  progress: EngineInstallProgress | null;
  installError: string | null;
  refresh: () => Promise<void>;
  installEngine: () => Promise<void>;
};

export const useModel = create<ModelStore>((set, get) => ({
  status: null,
  error: null,
  loading: false,
  installing: false,
  progress: null,
  installError: null,
  refresh: async () => {
    set({ loading: true, error: null });
    try {
      set({ status: await ipc.modelStatus(), loading: false });
    } catch (error) {
      set({ error: String(error), loading: false });
    }
  },
  installEngine: async () => {
    if (get().installing) return;
    set({ installing: true, installError: null, progress: null });
    // Subscribed before the command is invoked, so no early progress event is lost.
    const unlisten = await ipc.onEngineInstall((progress) => set({ progress }));
    try {
      await ipc.installEngine();
      await get().refresh();
    } catch (error) {
      set({ installError: String(error) });
    } finally {
      unlisten();
      set({ installing: false, progress: null });
    }
  },
}));
