// The Settings surface edits the whole document the host owns. It never rebuilds the object: the
// loaded value is cloned, one field is assigned, and the result goes straight back, so a setting
// with no control yet cannot be reset by saving the page.
import { create } from "zustand";

import { ipc, type ReasoningEffort, type SettingsDoc } from "../lib/ipc";
import { useAppearance } from "./appearance";

/**
 * The effort dial's own order, the one Settings lists. The header's bolt walks it, so a press on the
 * glyph and a pick on the page are the same five values in the same sequence — and the last one wraps
 * back to the model's own choice rather than pinning the widest effort with no way home.
 */
export const EFFORT_ORDER: ReasoningEffort[] = ["auto", "low", "medium", "xhigh", "none"];

export function nextEffort(current: ReasoningEffort): ReasoningEffort {
  const at = EFFORT_ORDER.indexOf(current);
  return EFFORT_ORDER[(at + 1) % EFFORT_ORDER.length] ?? "auto";
}

/**
 * Whether the saved effort reaches the route the next run will take.
 *
 * Neither route takes an effort field: the free service has no switch for one, and a remote
 * provider's vocabulary is not measured. Read here so the bolt and Settings agree before a run
 * starts.
 */
export function effortReachesRoute(_doc: SettingsDoc): boolean {
  return false;
}

type SettingsStore = {
  doc: SettingsDoc | null;
  loaded: boolean;
  /** Set when a save failed. The control still shows the attempt — the banner says it did not stick. */
  error: string | null;
  load: () => Promise<void>;
  update: (mutate: (doc: SettingsDoc) => void) => Promise<void>;
};

export const useSettings = create<SettingsStore>((set, get) => ({
  doc: null,
  loaded: false,
  error: null,
  load: async () => {
    try {
      set({ doc: await ipc.loadSettings(), loaded: true, error: null });
    } catch (e) {
      set({ loaded: true, error: String(e) });
    }
  },
  update: async (mutate) => {
    const current = get().doc;
    if (!current) return;
    const next = structuredClone(current);
    mutate(next);
    set({ doc: next });
    try {
      set({ doc: await ipc.saveSettings(next), error: null });
    } catch (e) {
      set({ error: String(e) });
    }
    // The shell paints from its own narrow view of the same file, so re-read it: a language,
    // accent or font-size change made here has to reach the window chrome immediately.
    await useAppearance.getState().load();
  },
}));
