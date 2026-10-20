// The composer's model pill: which of the catalog's models answers the next prompt.
//
// A picker rather than a jump to Settings, because the choice belongs to the moment that notices the
// answer is not what was asked for. It is also the other half of the attach button: only a model whose
// card publishes a projector can read a picture, so a prompt carrying an image is a weights decision as
// much as a wording one, and the row that says "reads pictures" is what makes that visible before the
// send rather than in a refusal after it.
//
// Every number on a row is the catalog's own — the same figures the Knowledge Center quotes — and the
// only thing added here is which of those files are already on this machine, which is the host's to
// know. Nothing is estimated: the packs a model offers are shown as the range they span.
import { useEffect, useState } from "react";

import { Icon } from "./icons";
import { translate, type Locale } from "../i18n";
import { ipc, type CatalogModel, type KnowledgeCenter } from "../lib/ipc";
import { useModel } from "../state/model";
import { useSettings } from "../state/settings";

/** `?picker=open` stands in for the click this panel needs and the native window cannot be given. */
const SEEDED = new URLSearchParams(window.location.search).get("picker") === "open";

/** The model's own packs, as the span they offer rather than a pick from them: which one this machine
 * gets is the host's resolution, and it is printed on the status strip below. */
function spanOf(model: CatalogModel): string {
  const sizes = model.packs.map((pack) => pack.sizeGb);
  const low = Math.min(...sizes);
  const high = Math.max(...sizes);
  return low === high ? `${low.toFixed(2)} GB` : `${low.toFixed(2)}–${high.toFixed(2)} GB`;
}

export function ModelPicker({
  locale,
  onOpenSettings,
}: {
  locale: Locale;
  /** The Settings page's model section, where the rest of the launch knobs live. */
  onOpenSettings: () => void;
}) {
  const status = useModel((s) => s.status);
  // A remote provider has no pack, no download and no catalog row here: its model id is free text in
  // Settings, so this panel can only say which provider answers and offer the page that changes it.
  const remote = status?.remote ?? null;
  const [open, setOpen] = useState(SEEDED);
  const [center, setCenter] = useState<KnowledgeCenter | null>(null);
  const [error, setError] = useState<string | null>(null);
  const t = (key: string, params?: Record<string, string | number>) =>
    translate(locale, `modelPicker.${key}`, params);

  // Read the first time the panel opens rather than at every launch: the pill names its own model
  // without the catalog, and the center's answer walks the weights directory to be got. A selected
  // provider never reads it at all — walking this machine for packs cannot answer a remote prompt.
  useEffect(() => {
    if (!open || remote || center || error) return;
    void ipc
      .knowledgeCenter()
      .then(setCenter)
      .catch((e: unknown) => setError(String(e)));
  }, [open, remote, center, error]);

  const choose = async (id: string) => {
    setOpen(false);
    if (!status || id === status.catalogId) return;
    const settings = useSettings.getState();
    if (!settings.loaded) await settings.load();
    await useSettings.getState().update((doc) => {
      doc.localModel.catalogId = id;
      // A forced packing is a claim about the model it was chosen for, so it does not travel with the
      // choice: the new model gets the same detection the first one did.
      doc.localModel.packing = "auto";
    });
    // No engine control is pressed from here. The host compares the weights the live server loaded
    // against what the settings now name, and starts another one when the next prompt arrives.
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
        <Icon name={remote ? "globe" : "cube"} size={13} />
        {/* The pill carries the model's name rather than the word "Model": which weights answer is the
            fact a user reads off it, and the label only matters once the panel is open. A remote
            provider may have no model id yet, and then the pill names the source instead. */}
        <span className="max-w-[16ch] truncate">
          {remote ? remote.model || t("remote") : (status?.displayName ?? t("title"))}
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
                <div className="flex items-start gap-[var(--space-3)] px-[var(--space-3)] py-[var(--space-2)]">
                  <span className="row-copy min-w-0">
                    <span className="row-title truncate">{remote.model || t("remote")}</span>
                    <span className="row-desc truncate">{providerHost}</span>
                  </span>
                  <span className="pill shrink-0">{t("remote")}</span>
                </div>
                <p className="px-[var(--space-3)] pb-[var(--space-2)] text-fine text-muted">
                  {t("remoteNote")}
                </p>
              </>
            )}
            {!remote && !center && (
              <p
                role={error ? "alert" : undefined}
                className="px-[var(--space-3)] py-[var(--space-2)] text-fine text-muted"
              >
                {error ?? t("loading")}
              </p>
            )}
            {!remote &&
              center?.models.map((model) => {
                const held = model.packs.filter((pack) => pack.onDisk).length;
                const current = model.id === status?.catalogId;
                return (
                  <button
                    key={model.id}
                    type="button"
                    data-current={current || undefined}
                    className="picker-row"
                    onClick={() => void choose(model.id)}
                  >
                    <span className="row-copy min-w-0">
                      <span className="row-title truncate">{model.name}</span>
                      <span className="row-desc truncate">
                        {`${t("packs", { held, total: model.packs.length })} · ${spanOf(model)}`}
                      </span>
                    </span>
                    {/* The two chips a choice between models actually turns on: can it see the picture
                      in the composer, and is it already the one answering. */}
                    {model.vision.length > 0 && (
                      <span className="pill shrink-0">{t("vision")}</span>
                    )}
                    {current && <span className="pill shrink-0">{t("current")}</span>}
                  </button>
                );
              })}
            <div className="border-t border-line px-[var(--space-3)] py-[var(--space-2)]">
              {!remote && <p className="text-fine text-faint">{t("note")}</p>}
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
