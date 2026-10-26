// The conversation row's menu was checked against the reference by eye, in the native window. What an
// eye cannot hold is the judgement underneath it: four of the nine presses do nothing, and which four
// is the honest half of the feature — the layout is the reference's, the refusals are this build's. So
// the builder is a plain function, and this is where its ledger is kept.
//
// `t` hands back the key it was given rather than English, so a change to a sentence cannot break a
// test that was really asking which sentence a row prints.
import { describe, expect, it, vi } from "vitest";

import { taskPresses, type Conversation, type PressInput } from "./Sidebar";

// Importing the sidebar pulls the brand mark in with it, and that reads the host's mascot number at
// module scope — so the boundary is replaced whole rather than left to reach for a Tauri runtime.
vi.mock("../lib/ipc", () => ({ ipc: {}, hostMascot: null }));

const t = (key: string) => key;
const NOT_IN_BUILD = "notInBuild";

function presses(patch: Partial<PressInput> = {}) {
  const conversation: Conversation = { id: "s1", title: "how are you", pinned: false };
  const calls: string[] = [];
  const input: PressInput = {
    conversation,
    t,
    notInBuild: NOT_IN_BUILD,
    busy: false,
    armed: false,
    copied: null,
    onCopy: (id) => calls.push(`copy:${id}`),
    onRename: (id, from) => calls.push(`rename:${id}:${from}`),
    onExport: (id) => calls.push(`export:${id}`),
    onPin: (id) => calls.push(`pin:${id}`),
    onDelete: (id) => calls.push(`delete:${id}`),
    ...patch,
  };
  return Object.assign(taskPresses(input), { calls });
}

function byId(list: ReturnType<typeof presses>, id: string) {
  const press = list.find((p) => p.id === id);
  if (!press) throw new Error(`no press named ${id}`);
  return press;
}

describe("the panel the reference draws", () => {
  it("carries its nine rows in its order", () => {
    expect(presses().map((p) => p.id)).toEqual([
      "window",
      "copy",
      "rename",
      "export",
      "pin",
      "group",
      "unread",
      "side",
      "delete",
    ]);
  });

  it("keeps the one submenu arrow on the row that promises a column", () => {
    const list = presses();
    expect(list.filter((p) => p.chevron).map((p) => p.id)).toEqual(["group"]);
  });

  it("draws the separator above the row that costs something, and above nothing else", () => {
    expect(
      presses()
        .filter((p) => p.danger)
        .map((p) => p.id),
    ).toEqual(["delete"]);
  });
});

describe("what this build refuses", () => {
  it("says no to the four rows it has nothing behind, and gives them nothing to run", () => {
    const list = presses();
    const refusing = list.filter((p) => p.refuse !== undefined);
    expect(refusing.map((p) => p.id)).toEqual(["window", "group", "unread", "side"]);
    expect(refusing.map((p) => p.refuse)).toEqual([
      NOT_IN_BUILD,
      NOT_IN_BUILD,
      NOT_IN_BUILD,
      NOT_IN_BUILD,
    ]);
    // `refuse` is what the panel reads to mark the row out and swallow the click; the missing `run` is
    // the other half — a press with a sentence explaining itself has no handler behind it at all.
    expect(refusing.map((p) => p.run)).toEqual([undefined, undefined, undefined, undefined]);
    expect(list.filter((p) => p.run).map((p) => p.id)).toEqual([
      "copy",
      "rename",
      "export",
      "pin",
      "delete",
    ]);
  });

  it("refuses to delete the conversation a reply is still writing", () => {
    const list = presses({ busy: true });
    const del = byId(list, "delete");
    expect(del.label).toBe("sidebar.delete.busy");
    // The press keeps its handler here, unlike the four with nothing behind them: this one would run,
    // and it is the panel's check on `refuse` that holds it back until the reply files its turn.
    expect(del.refuse).toBe("sidebar.delete.busy");
  });
});

describe("what this build runs", () => {
  it("hands each press the conversation the menu was opened on", () => {
    const list = presses();
    for (const id of ["copy", "rename", "export", "pin", "delete"]) byId(list, id).run?.();
    expect(list.calls).toEqual([
      "copy:s1",
      // Rename starts the field on the name the row is carrying, not on an empty box.
      "rename:s1:how are you",
      "export:s1",
      "pin:s1",
      "delete:s1",
    ]);
  });

  it("says which way the copy went, and says nothing before it was pressed", () => {
    expect(byId(presses(), "copy").value).toBeUndefined();
    expect(byId(presses({ copied: true }), "copy").value).toBe("task.copied");
    expect(byId(presses({ copied: false }), "copy").value).toBe("task.copyFailed");
  });

  it("names the tack by which way it is about to move the row", () => {
    const unpinned = byId(presses(), "pin");
    expect(unpinned.label).toBe("task.pin");
    // No note on a row that is not held: it is not yet doing the thing the note would describe.
    expect(unpinned.value).toBeUndefined();

    const pinned = byId(presses({ conversation: { id: "s1", title: "x", pinned: true } }), "pin");
    expect(pinned.label).toBe("task.unpin");
    expect(pinned.value).toBe("task.pinned");
  });

  it("asks for the second press that deletes, and only on the row that was armed", () => {
    expect(byId(presses(), "delete").label).toBe("sidebar.delete");
    expect(byId(presses({ armed: true }), "delete").label).toBe("sidebar.deleteAgain");
  });
});
