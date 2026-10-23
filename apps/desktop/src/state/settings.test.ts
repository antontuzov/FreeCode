// The settings surface edits the one document the host owns. Its contract is that saving a page
// cannot reset a field the page does not render: the loaded value is cloned, one field is assigned,
// and the whole object goes back — with the window's appearance re-read either way.
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ipc, type SettingsDoc } from "../lib/ipc";
import { useAppearance } from "./appearance";
import { useSettings } from "./settings";

vi.mock("../lib/ipc", () => ({
  ACCENTS: [],
  ipc: {
    loadSettings: vi.fn(),
    saveSettings: vi.fn(),
    loadAppearance: vi.fn(),
    saveAppearance: vi.fn(),
  },
}));

const mock = vi.mocked(ipc);

function document_(): SettingsDoc {
  return {
    schemaVersion: 1,
    appearance: {
      themeMode: "dark",
      interfaceStyle: "mint",
      glassEffects: true,
      terminalTheme: "follow_theme",
      useBuiltInBrowserForTerminalLinks: true,
      language: "en",
      fontStyle: "sans",
      contentWidth: "standard",
      fileIcons: "qoder",
      appIcon: null,
      uiFontSizePx: 14,
      lightCodeTheme: "github-light",
      darkCodeTheme: "github-dark",
      showLineNumbers: true,
      wrapLongLines: false,
      codeFontSizePx: 12,
    },
    sampling: {
      thinking: {
        temperature: 0.6,
        topP: 0.95,
        topK: 20,
        minP: 0,
        presencePenalty: 0,
        repetitionPenalty: 1,
      },
      instruct: {
        temperature: 0.2,
        topP: 0.9,
        topK: 20,
        minP: 0,
        presencePenalty: 0,
        repetitionPenalty: 1,
      },
      effort: "auto",
      budget: { maxTokens: 16_384, contextTokens: 32_768 },
      systemPrompt: "",
      nonThinkingMode: false,
    },
    voice: {
      inputDevice: "automatic",
      voiceInput: false,
      recognitionPolish: false,
      voiceprintRecognition: false,
      sendAutomatically: false,
      commonWordCorrections: [],
      liveVoiceShortcut: { cmd: false, shift: false, alt: false, ctrl: false, key: "" },
    },
    approvals: {
      mode: "ask_always",
      toolOverrides: {},
      shellEnabled: false,
      subagentsEnabled: false,
      shellAllow: [],
      shellDeny: [],
      shellTimeoutMs: 30_000,
    },
    privacy: { telemetry: false, allowRemoteProviders: false },
    remoteProvider: {
      source: "local",
      baseUrl: "",
      model: "",
      contextTokens: 32_768,
    },
    freeModel: null,
    context: { compressAtRatio: 0.8, keepRecentTurns: 4 },
    project: { root: "" },
    localModel: {
      catalogId: "ternary-bonsai-1-7b",
      packing: "auto",
      modelDir: null,
      hfTokenEnv: null,
      offloadAllLayers: true,
      flashAttention: true,
      visionLazyLoad: true,
      dedicatedServerSlots: false,
    },
  };
}

function at<T>(list: T[], index: number): T {
  const item = list[index];
  if (item === undefined) throw new Error(`no entry at ${index}`);
  return item;
}

const appearanceLoad = vi.fn(async () => {});

beforeEach(() => {
  vi.clearAllMocks();
  useSettings.setState(useSettings.getInitialState());
  useAppearance.setState({ ...useAppearance.getInitialState(), load: appearanceLoad });
});

describe("loading", () => {
  it("holds the host's document and clears any earlier failure", async () => {
    useSettings.setState({ error: "old failure" });
    mock.loadSettings.mockResolvedValueOnce(document_());

    await useSettings.getState().load();

    expect(useSettings.getState().doc).toEqual(document_());
    expect(useSettings.getState().loaded).toBe(true);
    expect(useSettings.getState().error).toBeNull();
  });

  it("says it is loaded when the file cannot be read, rather than holding the surface hostage", async () => {
    mock.loadSettings.mockRejectedValueOnce(new Error("settings are corrupt"));

    await useSettings.getState().load();

    expect(useSettings.getState().doc).toBeNull();
    expect(useSettings.getState().loaded).toBe(true);
    expect(useSettings.getState().error).toContain("settings are corrupt");
  });
});

describe("saving a page", () => {
  it("does nothing when there is no document to edit", async () => {
    await useSettings.getState().update((doc) => {
      doc.appearance.uiFontSizePx = 99;
    });
    expect(mock.saveSettings).not.toHaveBeenCalled();
    expect(appearanceLoad).not.toHaveBeenCalled();
  });

  it("clones the loaded document, changes the field, and sends the whole object back", async () => {
    const loaded = document_();
    mock.loadSettings.mockResolvedValueOnce(loaded);
    await useSettings.getState().load();
    const stored = { ...document_(), schemaVersion: 2 };
    mock.saveSettings.mockResolvedValueOnce(stored);

    await useSettings.getState().update((doc) => {
      doc.appearance.uiFontSizePx = 16;
      doc.appearance.interfaceStyle = "rose";
    });

    // The object on screen before the save is not the one that was mutated.
    expect(loaded.appearance.uiFontSizePx).toBe(14);
    const sent = at(mock.saveSettings.mock.calls, 0)[0];
    expect(sent.appearance.uiFontSizePx).toBe(16);
    expect(sent.appearance.interfaceStyle).toBe("rose");
    // A field no control on this page renders still went back untouched.
    expect(sent.approvals.shellTimeoutMs).toBe(30_000);
    expect(sent.context.keepRecentTurns).toBe(4);
    expect(useSettings.getState().doc).toBe(stored);
    expect(appearanceLoad).toHaveBeenCalledTimes(1);
  });

  it("keeps the attempted value on screen and reports that it did not stick", async () => {
    mock.loadSettings.mockResolvedValueOnce(document_());
    await useSettings.getState().load();
    mock.saveSettings.mockRejectedValueOnce(new Error("settings file is read-only"));

    await useSettings.getState().update((doc) => {
      doc.appearance.interfaceStyle = "bee";
    });

    expect(useSettings.getState().doc?.appearance.interfaceStyle).toBe("bee");
    expect(useSettings.getState().error).toContain("read-only");
    // The window chrome still re-reads the file: the save failing does not mean the paint should lie.
    expect(appearanceLoad).toHaveBeenCalledTimes(1);
  });
});
