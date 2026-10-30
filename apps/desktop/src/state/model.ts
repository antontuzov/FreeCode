// The status strip's store. There is one route to describe — the free tier answered by the service
// on this Mac, or the remote provider standing in front of it — so the store reads the host's verdict
// and holds nothing else: no engine to fetch, no weights to download.
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
