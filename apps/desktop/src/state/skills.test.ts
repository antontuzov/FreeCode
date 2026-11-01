// A draft is either naming a pack or writing a prompt, and the two halves of the composer that read it
// — the strip over the box and the press of Send — must not disagree. These hold the reading of the
// command and the store that fills it; the host's reading of the files is its own tests' business.
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ipc, type SkillInventory, type SkillPack } from "../lib/ipc";
import { invocation, suggestions, useSkills } from "./skills";

vi.mock("../lib/ipc", () => ({
  ipc: {
    skillsList: vi.fn(),
  },
}));

const mock = vi.mocked(ipc);

function pack(id: string, over: Partial<SkillPack> = {}): SkillPack {
  return {
    id,
    name: id,
    description: `what ${id} teaches`,
    source: "machine",
    chars: 120,
    cut: false,
    path: `/Users/me/.freecode/skills/${id}/SKILL.md`,
    ...over,
  };
}

const INVENTORY: SkillInventory = {
  skills: [pack("tidy-commit"), pack("review-diff")],
  machineDir: "/Users/me/.freecode/skills",
  projectDir: "/work/proj/.freecode/skills",
};

beforeEach(() => {
  vi.clearAllMocks();
  useSkills.setState(useSkills.getInitialState());
});

describe("reading a command", () => {
  it("splits the pack name from the prompt to work on", () => {
    expect(invocation("/tidy-commit rewrite this")).toEqual({
      skill: "tidy-commit",
      prompt: "rewrite this",
    });
    // The command is the front of the line, so a name with the padding a draft always has still reads.
    expect(invocation("  /review-diff   is the hunk right?  ")).toEqual({
      skill: "review-diff",
      prompt: "is the hunk right?",
    });
  });

  it("keeps a prompt that opens with a path as the prose it is", () => {
    // A second slash means the word is not one folder's name, and `/etc/hosts looks like this` is a
    // question the user meant to ask, not a pack they meant to invoke.
    expect(invocation("/etc/hosts looks like this")).toEqual({
      skill: null,
      prompt: "/etc/hosts looks like this",
    });
    expect(invocation("read /config.toml please")).toEqual({
      skill: null,
      prompt: "read /config.toml please",
    });
  });

  it("hands a bare command to the host with no prompt, which is the case that refuses", () => {
    expect(invocation("/tidy-commit")).toEqual({ skill: "tidy-commit", prompt: "" });
    // An unknown name is not quietly dropped here: it reaches the host and comes back in words, so the
    // reading of the command has to be the same whether or not a pack exists.
    expect(invocation("/no-such-pack do the thing")).toEqual({
      skill: "no-such-pack",
      prompt: "do the thing",
    });
  });
});

describe("the strip", () => {
  const skills = [pack("review-diff"), pack("review-pr"), pack("tidy-commit")];
  /** The ids a draft is offered, or `undefined` when the strip would not draw at all. */
  const listed = (draft: string) => suggestions(skills, draft)?.map((p) => p.id);

  it("offers the packs a name still being typed could be", () => {
    expect(listed("/re")).toEqual(["review-diff", "review-pr"]);
    expect(listed("/")).toEqual(skills.map((p) => p.id));
    expect(listed("/TID")).toEqual(["tidy-commit"]);
  });

  it("goes away once the command is settled, and never appears over prose", () => {
    // The prompt is what is being written next, and a list of names over it is an obstruction.
    expect(suggestions(skills, "/review-diff write it")).toBeNull();
    expect(suggestions(skills, "fix the test")).toBeNull();
    expect(suggestions(skills, "/review-diff\nthen commit")).toBeNull();
    // A path is not a name, so the strip has nothing to say about it either.
    expect(suggestions(skills, "/etc/")).toBeNull();
  });

  it("says there is nothing to offer rather than showing an empty list", () => {
    expect(suggestions([], "/re")).toEqual([]);
  });

  it("keeps the list short enough to read over the box it covers", () => {
    const many = Array.from({ length: 20 }, (_, i) => pack(`pack-${i}`));
    expect(suggestions(many, "/pack-")).toHaveLength(6);
  });
});

describe("the store", () => {
  it("reads the packs once and keeps the two directories the empty states name", async () => {
    mock.skillsList.mockResolvedValue(INVENTORY);
    await useSkills.getState().load();

    expect(mock.skillsList).toHaveBeenCalledTimes(1);
    expect(useSkills.getState().skills.map((p) => p.id)).toEqual(["tidy-commit", "review-diff"]);
    expect(useSkills.getState().projectDir).toBe("/work/proj/.freecode/skills");
    expect(useSkills.getState().loaded).toBe(true);
    expect(useSkills.getState().error).toBeNull();

    // A second `/` in the same window is not a second walk of the disk.
    await useSkills.getState().load();
    expect(mock.skillsList).toHaveBeenCalledTimes(1);
  });

  it("re-reads when a page asks for the folder as it is now", async () => {
    mock.skillsList.mockResolvedValue(INVENTORY);
    await useSkills.getState().load();
    await useSkills.getState().load(true);
    expect(mock.skillsList).toHaveBeenCalledTimes(2);
  });

  it("keeps the error visible and the list empty, rather than showing a vocabulary that is not there", async () => {
    mock.skillsList.mockRejectedValue(new Error("settings could not be read"));
    await useSkills.getState().load();

    expect(useSkills.getState().skills).toEqual([]);
    expect(useSkills.getState().loaded).toBe(false);
    expect(useSkills.getState().error).toContain("settings could not be read");
  });

  it("does not start a second read while one is in flight", async () => {
    const gate = deferred<SkillInventory>();
    mock.skillsList.mockReturnValue(gate.promise);
    const first = useSkills.getState().load();
    await useSkills.getState().load();
    expect(mock.skillsList).toHaveBeenCalledTimes(1);

    gate.resolve(INVENTORY);
    await first;
    expect(useSkills.getState().loaded).toBe(true);
  });
});

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}
