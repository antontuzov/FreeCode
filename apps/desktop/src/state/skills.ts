// The skill packs a prompt can be worked under, and the one shape that invokes one: a draft that
// opens with `/name `.
//
// The list comes from the host because a pack is a file on disk, in one of two places, and a window
// that guessed at the vocabulary would be a second registry to keep in step. What lives here is the
// other half: the reading of the command itself, which has to be one function so the suggestion strip
// and the press of Send cannot disagree about which draft was a command and which was prose.
import { create } from "zustand";

import { ipc, type SkillPack } from "../lib/ipc";

/**
 * A draft opening with `/word` is naming a pack; everything after the name is the prompt to work on.
 *
 * The name is one word of the shapes a folder is ever given — no second slash, no space — which is what
 * keeps a prompt that begins with a path (`/etc/hosts looks like this`) as the prose it is. A name that
 * matches no pack is not quietly dropped: it goes to the host, which refuses it in words, because a
 * prompt the user believes is being worked under a pack is worse than one that says the pack was not
 * found.
 */
const COMMAND = /^\/([\w][\w.-]*)(?=\s|$)\s*/;

/** What a draft means: the pack to work under, if any, and the prompt left over. */
export function invocation(draft: string): { skill: string | null; prompt: string } {
  const trimmed = draft.trim();
  const found = COMMAND.exec(trimmed);
  const name = found?.[1];
  const matched = found?.[0];
  if (name === undefined || matched === undefined) return { skill: null, prompt: trimmed };
  return { skill: name, prompt: trimmed.slice(matched.length).trim() };
}

/** How many rows the strip draws at once. A machine with thirty packs is a list to filter, not to
 * scroll, and the strip sits over the box the prompt is being typed into. */
const SUGGESTIONS = 6;

/**
 * The packs a draft could resolve to, while it is still naming one.
 *
 * `null` is the answer for anything that is not a command — a draft with a space after the name has
 * finished naming, and the strip that would obscure the prompt has nothing left to say.
 */
export function suggestions(skills: SkillPack[], draft: string): SkillPack[] | null {
  const trimmed = draft.trimStart();
  if (!trimmed.startsWith("/") || trimmed.includes("\n")) return null;
  const found = /^\/([\w][\w.-]*)?$/.exec(trimmed);
  // Once the name is followed by a space the command is settled, and a strip over a prompt being
  // written is an obstruction rather than a suggestion. A bare `/` is the whole list: it is the
  // keystroke that asks what is available, and on this machine the answer can be nothing at all,
  // which is the one reply the user can act on.
  if (!found) return null;
  const needle = (found[1] ?? "").toLowerCase();
  return skills.filter((pack) => pack.id.toLowerCase().startsWith(needle)).slice(0, SUGGESTIONS);
}

type SkillsStore = {
  skills: SkillPack[];
  /** The two places a pack is looked for, which the empty states have to name. */
  machineDir: string;
  projectDir: string;
  loaded: boolean;
  loading: boolean;
  error: string | null;
  load: (force?: boolean) => Promise<void>;
};

/** The packs on this machine and in this folder. Read when a surface asks, never polled. */
export const useSkills = create<SkillsStore>((set, get) => ({
  skills: [],
  machineDir: "",
  projectDir: "",
  loaded: false,
  loading: false,
  error: null,
  load: async (force = false) => {
    const state = get();
    // A pack is a file, so the only reason to re-read is a page asking for the folder as it is now:
    // the composer's first `/` takes the answer the window already has.
    if (state.loaded && !force) return;
    if (state.loading) return;
    set({ loading: true });
    try {
      const found = await ipc.skillsList();
      set({
        skills: found.skills,
        machineDir: found.machineDir,
        projectDir: found.projectDir,
        loaded: true,
        loading: false,
        error: null,
      });
    } catch (error) {
      set({ loading: false, error: String(error) });
    }
  },
}));
