// Appearance is the one setting that has to be on the document before first paint. These tests hold
// the two halves: what a mode resolves to, and that every lever the settings offer lands on the root
// element — plus the rule that a dev URL paints the window without ever reaching the disk.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ipc, type AppearanceState } from "../lib/ipc";
import { applyToDocument, effectiveMode, useAppearance } from "./appearance";

vi.mock("../lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/ipc")>();
  return {
    ...actual,
    ipc: {
      loadAppearance: vi.fn(),
      saveAppearance: vi.fn(),
    },
  };
});

const mock = vi.mocked(ipc);

const STORED: AppearanceState = {
  themeMode: "dark",
  resolvedMode: "dark",
  accent: "mint",
  glass: true,
  language: "en",
  uiFontSize: 14,
  codeFontSize: 12,
  fontStyle: "sans",
  contentWidth: "standard",
  showLineNumbers: true,
  wrapLongLines: false,
  appIcon: null,
};

let supportsBlur = true;

function root(): HTMLElement {
  return document.documentElement;
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("CSS", { supports: () => supportsBlur });
  Object.defineProperty(window.navigator, "userAgent", {
    value: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)",
    configurable: true,
  });
  window.history.replaceState({}, "", "/");
  useAppearance.setState(useAppearance.getInitialState());
  for (const attribute of [
    "data-theme-mode",
    "data-accent",
    "data-surface",
    "data-font-style",
    "data-content-width",
    "data-code-lines",
    "data-code-wrap",
    "data-platform",
  ]) {
    root().removeAttribute(attribute);
  }
  root().style.removeProperty("--font-ui");
  root().style.removeProperty("--font-code");
  supportsBlur = true;
});

afterEach(() => {
  vi.unstubAllGlobals();
  delete (window.navigator as unknown as { userAgent?: string }).userAgent;
});

describe("resolving the mode", () => {
  it("paints a concrete mode, taking the host's resolution while the choice is system", () => {
    expect(effectiveMode({ ...STORED, themeMode: "system", resolvedMode: "light" })).toBe("light");
    expect(effectiveMode({ ...STORED, themeMode: "system", resolvedMode: "dark" })).toBe("dark");
    expect(effectiveMode({ ...STORED, themeMode: "light", resolvedMode: "dark" })).toBe("light");
    expect(effectiveMode({ ...STORED, themeMode: "dark", resolvedMode: "light" })).toBe("dark");
  });
});

describe("painting the document", () => {
  it("puts every setting on the root element as an attribute or a custom property", () => {
    applyToDocument({
      ...STORED,
      themeMode: "system",
      resolvedMode: "light",
      accent: "rose",
      fontStyle: "serif",
      contentWidth: "wide",
      uiFontSize: 15,
      codeFontSize: 13,
      showLineNumbers: false,
      wrapLongLines: true,
    });

    expect(root().dataset.themeMode).toBe("light");
    expect(root().dataset.accent).toBe("rose");
    expect(root().dataset.fontStyle).toBe("serif");
    expect(root().dataset.contentWidth).toBe("wide");
    expect(root().dataset.codeLines).toBe("off");
    expect(root().dataset.codeWrap).toBe("on");
    expect(root().dataset.platform).toBe("darwin");
    expect(root().style.getPropertyValue("--font-ui")).toBe("15px");
    expect(root().style.getPropertyValue("--font-code")).toBe("13px");
  });

  it("degrades glass to a solid surface when the webview cannot blur, or the user said no", () => {
    applyToDocument({ ...STORED, glass: true });
    expect(root().dataset.surface).toBe("glass");

    supportsBlur = false;
    applyToDocument({ ...STORED, glass: true });
    expect(root().dataset.surface).toBe("solid");

    supportsBlur = true;
    applyToDocument({ ...STORED, glass: false });
    expect(root().dataset.surface).toBe("solid");
  });
});

describe("loading and saving", () => {
  it("loads what the host stored and paints it", async () => {
    mock.loadAppearance.mockResolvedValueOnce({
      ...STORED,
      accent: "bee",
      resolvedMode: "light",
      language: "ru",
    });

    await useAppearance.getState().load();

    const state = useAppearance.getState();
    expect(state.loaded).toBe(true);
    expect(state.accent).toBe("bee");
    expect(state.language).toBe("ru");
    expect(root().dataset.accent).toBe("bee");
    expect(root().dataset.themeMode).toBe("dark");
  });

  it("keeps the fallback rather than a blank window when there is no host to ask", async () => {
    mock.loadAppearance.mockRejectedValueOnce(new Error("plain vite dev"));

    await useAppearance.getState().load();

    const state = useAppearance.getState();
    expect(state.loaded).toBe(true);
    expect(state.accent).toBe("mint");
    expect(root().dataset.accent).toBe("mint");
    expect(root().dataset.themeMode).toBe("dark");
  });

  it.skipIf(!import.meta.env.DEV)(
    "lets a dev lever paint the window without persisting a choice the user never made",
    async () => {
      window.history.replaceState(
        {},
        "",
        "/?mode=light&accent=rose&font=serif&width=full&lines=off&wrap=on&glass=off",
      );
      mock.loadAppearance.mockResolvedValueOnce({ ...STORED });

      await useAppearance.getState().load();

      // The store is the disk's answer; only the paint was forced.
      expect(useAppearance.getState().themeMode).toBe("dark");
      expect(root().dataset.themeMode).toBe("light");
      expect(root().dataset.accent).toBe("rose");
      expect(root().dataset.fontStyle).toBe("serif");
      expect(root().dataset.contentWidth).toBe("full");
      expect(root().dataset.codeLines).toBe("off");
      expect(root().dataset.codeWrap).toBe("on");
      expect(root().dataset.surface).toBe("solid");
    },
  );

  it("shows a choice immediately, then adopts what the host actually stored", async () => {
    mock.saveAppearance.mockResolvedValueOnce({
      ...STORED,
      themeMode: "system",
      resolvedMode: "light",
      accent: "ink",
    });

    await useAppearance.getState().set({ accent: "rose" });

    expect(mock.saveAppearance).toHaveBeenCalledWith(expect.objectContaining({ accent: "rose" }));
    const state = useAppearance.getState();
    expect(state.themeMode).toBe("system");
    expect(state.accent).toBe("ink");
    expect(root().dataset.themeMode).toBe("light");
    expect(root().dataset.accent).toBe("ink");
  });

  it("keeps the choice on screen when it could not be written to disk", async () => {
    mock.saveAppearance.mockRejectedValueOnce(new Error("disk is full"));

    await useAppearance.getState().set({ accent: "bee" });

    expect(useAppearance.getState().accent).toBe("bee");
    expect(root().dataset.accent).toBe("bee");
  });
});
