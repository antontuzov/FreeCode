// The panel's refusal line is the visible half of the promise that the shell is not a way around the
// dial. The host answers with a code, so the sentence is this app's to choose — and each code must
// reach the sentence that explains it, not the fallback that prints the key.
import { describe, expect, it, vi } from "vitest";

import en from "../i18n/en.json";
import { refusalText } from "./Terminal";

// The panel imports the shell store and the host boundary at module scope.
vi.mock("../lib/ipc", () => ({ ipc: {}, hostMascot: null }));

describe("refusalText", () => {
  it("names the command the dial refused", () => {
    const text = refusalText("en", { code: "blocked", command: "rm -rf ." });
    expect(text).toBe(en["terminal.refusal.blocked"].replace("{{command}}", "rm -rf ."));
  });

  it("says the shell tool itself was refused when there is no command", () => {
    expect(refusalText("en", { code: "blocked", command: "" })).toBe(
      en["terminal.refusal.blockedShell"],
    );
  });

  it("carries the host's reason when the shell could not start", () => {
    expect(refusalText("en", { code: "start", reason: "no pty" })).toContain("no pty");
  });

  it("answers every code in the language the window is in", () => {
    for (const refusal of [{ code: "off" }, { code: "plan" }, { code: "closed" }] as const) {
      expect(refusalText("en", refusal)).toBe(en[`terminal.refusal.${refusal.code}`]);
      expect(refusalText("ru", refusal)).not.toContain("terminal.refusal.");
    }
  });
});
