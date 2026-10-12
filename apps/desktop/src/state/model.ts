// The status strip reads the same catalog + hardware choice the CLI verifier checks, so what the UI
// claims about the local model is the same thing CI asserts about the files on disk.
import { create } from "zustand";

import { ipc, type ModelStatus } from "../lib/ipc";

type ModelStore = {
  status: ModelStatus | null;
  error: string | null;
  loading: boolean;
  refresh: () => Promise<void>;
};

export const useModel = create<ModelStore>((set) => ({
  status: null,
  error: null,
  loading: false,
  refresh: async () => {
    set({ loading: true, error: null });
    try {
      set({ status: await ipc.modelStatus(), loading: false });
    } catch (error) {
      set({ error: String(error), loading: false });
    }
  },
}));
