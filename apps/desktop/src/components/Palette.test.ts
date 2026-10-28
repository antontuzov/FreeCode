// The palette is now two lists — the window's own open commands, and the conversations the host filed —
// and the half a test can hold without a window is which commands it offers and what its query leaves
// standing. The rows themselves are checked against the reference by eye, in the native window.
import { describe, expect, it, vi } from "vitest";

import en from "../i18n/en.json";
import ru from "../i18n/ru.json";
import { matchingCommands, paletteCommands } from "./Palette";

// The palette imports the host boundary at module scope; the boundary is replaced whole rather than
// left to reach for a Tauri runtime.
vi.mock("../lib/ipc", () => ({ ipc: {}, hostMascot: null }));

describe("the commands the panel opens with", () => {
  it("carries the reference's five, in its order, each with a binding", () => {
    const commands = paletteCommands("en");
    expect(commands.map((command) => command.id)).toEqual([
      "files",
      "newTask",
      "browser",
      "review",
      "terminal",
    ]);
    for (const command of commands) expect(command.shortcut).toBe(command.id);
  });

  it("speaks the catalog it was handed", () => {
    expect(paletteCommands("en").find((command) => command.id === "files")?.title).toBe(
      en["palette.cmd.files"],
    );
    expect(paletteCommands("ru").find((command) => command.id === "files")?.title).toBe(
      ru["palette.cmd.files"],
    );
  });
});

describe("what a query leaves standing", () => {
  const commands = paletteCommands("en");

  it("keeps everything until something is typed", () => {
    expect(matchingCommands(commands, "")).toEqual(commands);
    expect(matchingCommands(commands, "   ")).toEqual(commands);
  });

  it("matches a label, whatever the case", () => {
    expect(matchingCommands(commands, "term").map((command) => command.id)).toEqual(["terminal"]);
    expect(matchingCommands(commands, "new task").map((command) => command.id)).toEqual([
      "newTask",
    ]);
    expect(matchingCommands(commands, "BROWSER").map((command) => command.id)).toEqual(["browser"]);
  });

  it("says nothing when nothing matches", () => {
    expect(matchingCommands(commands, "zzzz")).toEqual([]);
  });
});
