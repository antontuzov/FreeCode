import { describe, expect, it } from "vitest";

import { updateDescription } from "./update";
import type { UpdateState } from "./ipc";

const state = (over: Partial<UpdateState> = {}): UpdateState => ({
  current: "0.1.0",
  latest: "0.1.2",
  available: true,
  url: "https://github.com/antontuzov/FreeCode/releases/tag/v0.1.2",
  publishedAt: null,
  ...over,
});

/** The catalog says what the sentence is; this test is about which sentence is chosen. */
const t = (key: string, params?: Record<string, string | number>) => {
  const table: Record<string, string> = {
    "update.newer": "{{latest}} is newer than the {{current}} this build carries.",
    "update.same": "{{latest}} — the published build and this one are the same.",
    "update.publishedOn": "Published {{date}}.",
  };
  const template = table[key] ?? key;
  return template.replace(/{{(\w+)}}/g, (_all, name: string) => String(params?.[name] ?? ""));
};

describe("updateDescription", () => {
  it("names the published build when it is newer than this one", () => {
    expect(updateDescription(state(), t, "en")).toBe(
      "0.1.2 is newer than the 0.1.0 this build carries.",
    );
  });

  it("says the build is current when the published version is this one", () => {
    expect(updateDescription(state({ available: false, latest: "0.1.0" }), t, "en")).toBe(
      "0.1.0 — the published build and this one are the same.",
    );
  });

  it("appends the day the release went out when the feed named one", () => {
    const text = updateDescription(state({ publishedAt: "2026-10-20T10:00:00Z" }), t, "en");
    expect(text).toContain("Published ");
    expect(text.startsWith("0.1.2 is newer than the 0.1.0 this build carries.")).toBe(true);
  });

  it("leaves the date out when the feed named no day, or named a day that is not one", () => {
    expect(updateDescription(state({ publishedAt: null }), t, "en")).not.toContain("Published");
    expect(updateDescription(state({ publishedAt: "not a date" }), t, "en")).not.toContain(
      "Published",
    );
  });
});
