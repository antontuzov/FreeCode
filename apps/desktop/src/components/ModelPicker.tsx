// The composer's model pill: which free model answers the next prompt.
//
// The app has one built-in route — the free tier served by OpenCode on this Mac — so this panel is
// the list that service publishes, plus the remote provider when the user has selected one. A picker
// rather than a jump to Settings, because the choice belongs to the moment that notices the answer is
// not what was asked for, and the pill names the model rather than the word "Model".
import { useEffect, useState } from "react";

import { Icon } from "./icons";
import { ModelMark } from "./ModelMark";
import { translate, type Locale } from "../i18n";
import { ipc, type FreeModelInfo } from "../lib/ipc";
import { useModel } from "../state/model";
import { useSettings } from "../state/settings";

/** `?picker=open` stands in for the click this panel needs and the native window cannot be given. */
const SEEDED = new URLSearchParams(window.location.search).get("picker") === "open";

export function ModelPicker({
  locale,
  onOpenSettings,
}: {
  locale: Locale;
  /** The Settings page's model section, where the remote provider is configured. */
  onOpenSettings: () => void;
}) {
  const status = useModel((s) => s.status);
  // A remote provider has no registry row of its own: its model id is free text in Settings, so this
  // panel can only say which provider answers and offer the page that changes it.
  const remote = status?.remote ?? null;
  const free = status?.free ?? null;
  const [open, setOpen] = useState(SEEDED);
  const [freeList, setFreeList] = useState<FreeModelInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const t = (key: string, params?: Record<string, string | number>) =>
    translate(locale, `modelPicker.${key}`, params);

  // The registry is a small fixed list the host holds in memory, so it is read with the panel and
  // not cached: the free group is the one part of this panel a remote selection still shows.
  useEffect(() => {
    if (!open || freeList) return;
    void ipc
      .freeModelsList()
      .then(setFreeList)
      .catch((e: unknown) => setError(String(e)));
  }, [open, freeList]);

  const chooseFree = async (id: string) => {
    const settings = useSettings.getState();
    if (!settings.loaded) await settings.load();
    const entry = freeList?.find((m) => m.id === id);
    // A row answers when the service on this Mac is answering, which is the only route the registry
    // has. A row without it is listed and not pickable: the host would refuse the turn anyway, and a
    // row that looks pickable but can only fail is the bug a silent no-op would hide.
    if (!entry || !entry.local) return;
    setOpen(false);
    if (id === status?.free?.id) return;
    await useSettings.getState().update((doc) => {
      doc.freeModel = id;
      // The free pick answers before the provider row would, but the Settings page shows one source
      // at a time: flipping it here keeps the page from claiming Remote while a free model answers.
      doc.remoteProvider.source = "local";
    });
    await useModel.getState().refresh();
  };

  const providerHost = remote
    ? remote.baseUrl.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").replace(/\/.*$/, "") || "—"
    : "";

  return (
    <span className="relative">
      <button
        type="button"
        onClick={() => setOpen((was) => !was)}
        aria-expanded={open}
        className="flex items-center gap-1.5 rounded-[var(--radius-pill)] px-2 py-1 text-fine text-muted hover:bg-hover hover:text-fg"
      >
        <Icon name={remote ? "globe" : "bolt"} size={13} />
        {/* The pill carries the model's name rather than the word "Model": which model answers is the
            fact a user reads off it, and the label only matters once the panel is open. A remote
            provider may have no model id yet, and then the pill names the source instead. */}
        <span className="max-w-[16ch] truncate">
          {remote ? remote.model || t("remote") : (free?.label ?? t("title"))}
        </span>
        <Icon name="chevronDown" size={12} />
      </button>

      {open && (
        <>
          {/* Transparent: the panel is about the prompt still sitting in the box above it, and a
              dimmed ground would hide the words the choice has to fit. */}
          <span className="picker-scrim" onClick={() => setOpen(false)} />
          <div
            role="dialog"
            aria-modal="true"
            aria-label={t("title")}
            className="picker-panel"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                setOpen(false);
              }
            }}
          >
            <p className="section-label px-[var(--space-3)] pt-[var(--space-2)]">{t("title")}</p>
            {remote && (
              <>
                <div className="flex items-center gap-[var(--space-2)] px-[var(--space-3)] py-[var(--space-2)]">
                  {/* No maker mark here: a remote provider's model id is free text the app cannot
                      verify, so the row keeps the panel's own sign for "answered elsewhere". */}
                  <span aria-hidden className="model-mark">
                    <Icon name="globe" size={14} />
                  </span>
                  <span className="row-copy min-w-0">
                    <span className="model-name truncate">{remote.model || t("remote")}</span>
                    <span className="row-desc truncate">{providerHost}</span>
                  </span>
                  <span className="pill shrink-0">{t("remote")}</span>
                </div>
                <p className="px-[var(--space-3)] pb-[var(--space-2)] text-fine text-muted">
                  {t("remoteNote")}
                </p>
              </>
            )}
            {!freeList && (
              <p
                role={error ? "alert" : undefined}
                className="px-[var(--space-3)] py-[var(--space-2)] text-fine text-muted"
              >
                {error ?? t("loading")}
              </p>
            )}
            {/* The free tier: the list the service publishes, answered by the FreeCode app on this
                Mac. A row is pickable exactly when that service is answering; one with nothing behind
                it is listed and marked, and picking it is refused in the same breath — the host would
                refuse the turn anyway. */}
            {freeList !== null && freeList.length > 0 && (
              <>
                <p className="section-label px-[var(--space-3)] pt-[var(--space-2)]">
                  {t("freeGroup")}
                </p>
                {freeList.map((model) => {
                  const current = model.id === free?.id;
                  return (
                    <button
                      key={model.id}
                      type="button"
                      data-current={current || undefined}
                      aria-disabled={!model.local || undefined}
                      className="picker-row"
                      onClick={() => void chooseFree(model.id)}
                    >
                      <ModelMark provider={model.provider} />
                      <span className="row-copy min-w-0">
                        <span className="model-name truncate">{model.label}</span>
                        <span className="row-desc truncate">
                          {model.local
                            ? `${model.provider} · ${t("freeViaApp")}`
                            : `${model.provider} · ${t("freeOnlyInside")}`}
                        </span>
                      </span>
                      <span className="pill shrink-0">{t("freeBadge")}</span>
                      {/* The checkmark the reference list uses: a tick, not a word, so "Free" stays the
                          only chip on the row. */}
                      {current && <Icon name="tick" size={14} className="shrink-0 text-accent" />}
                    </button>
                  );
                })}
              </>
            )}
            <div className="border-t border-line px-[var(--space-3)] py-[var(--space-2)]">
              {!remote && (
                <p className="text-fine text-faint">
                  {t(status?.opencode.available ? "freeNoteApp" : "freeNote")}
                </p>
              )}
              <button
                type="button"
                className="nav-item mt-[var(--space-1)]"
                onClick={() => {
                  setOpen(false);
                  onOpenSettings();
                }}
              >
                <Icon name="sliders" />
                <span>{t("settings")}</span>
              </button>
            </div>
          </div>
        </>
      )}
    </span>
  );
}
