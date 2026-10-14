// The status strip reads the same catalog + hardware choice the CLI verifier checks, so what the UI
// claims about the local model is the same thing CI asserts about the files on disk.
import { create } from "zustand";

import {
  ipc,
  type EngineInstallProgress,
  type ModelStatus,
  type WeightsProgress,
} from "../lib/ipc";

type ModelStore = {
  status: ModelStatus | null;
  error: string | null;
  loading: boolean;
  /** Set while the host is fetching the pinned engine; the Install control keys off this. */
  installing: boolean;
  /** The host's last progress event, or null when nothing is in flight. */
  progress: EngineInstallProgress | null;
  installError: string | null;
  /** Set while the host is downloading the chosen pack; the Download control keys off this. */
  downloading: boolean;
  downloadProgress: WeightsProgress | null;
  downloadError: string | null;
  refresh: () => Promise<void>;
  installEngine: () => Promise<void>;
  downloadWeights: () => Promise<void>;
};

export const useModel = create<ModelStore>((set, get) => ({
  status: null,
  error: null,
  loading: false,
  installing: false,
  progress: null,
  installError: null,
  downloading: false,
  downloadProgress: null,
  downloadError: null,
  refresh: async () => {
    set({ loading: true, error: null });
    try {
      set({ status: await ipc.modelStatus(), loading: false });
    } catch (error) {
      set({ error: String(error), loading: false });
    }
  },
  // The engine install and the weights download are deliberately parallel rather than shared: the
  // two fetches clear different refusals, and a change to one lifecycle has to be made to the other
  // too — the guard, the subscribe-before-invoke ordering and the post-fetch refresh are the contract.
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
  downloadWeights: async () => {
    if (get().downloading) return;
    set({ downloading: true, downloadError: null, downloadProgress: null });
    const unlisten = await ipc.onWeightsDownload((downloadProgress) => set({ downloadProgress }));
    try {
      await ipc.downloadWeights();
      // The verdict the button was pressed to clear comes from the host, not from the transfer's
      // own success, so the card re-reads it rather than assuming the guard now agrees.
      await get().refresh();
    } catch (error) {
      set({ downloadError: String(error) });
    } finally {
      unlisten();
      set({ downloading: false, downloadProgress: null });
    }
  },
}));
