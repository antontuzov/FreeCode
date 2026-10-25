// The first-run setup flow: the reference's three steps, bound to this app's truth.
//
// There is no gateway to detect and no account to connect, so the flow asks the two questions this
// build can answer — where a run works, and which free model answers it — and hands the rest to the
// Settings page that already owns those controls. The third step lists the free tier itself rather
// than a menu of routes: the local engine is not what this app is for, and a user who finishes setup
// should have a working free model, not a choice they still have to make. Finishing writes one small
// section of the settings document; pressing "Set up later" writes the same section with `later`, so
// the flow does not reopen on every launch because the user declined it once.
import { useEffect, useRef, useState } from "react";

import { MascotMark } from "./BrandMark";
import { Icon, type IconName } from "./icons";
import { translate, type Locale } from "../i18n";
import { ipc, type FreeModelInfo, type OnboardingPlace } from "../lib/ipc";
import { useModel } from "../state/model";
import { ONBOARDING_STEPS, useOnboarding } from "../state/onboarding";
import { useSettings } from "../state/settings";

type Place = Exclude<OnboardingPlace, "undecided">;
type T = (key: string, params?: Record<string, string | number>) => string;

const FEATURES: { icon: IconName; key: string }[] = [
  { icon: "wand", key: "welcome.feature1" },
  { icon: "workspace", key: "welcome.feature2" },
  { icon: "shield", key: "welcome.feature3" },
];

const PLACES: { id: Place; icon: IconName; key: string }[] = [
  { id: "this_mac", icon: "monitor", key: "place.thisMac" },
  { id: "project_folder", icon: "workspace", key: "place.folder" },
  { id: "later", icon: "clock", key: "place.later" },
];

export function Onboarding({
  locale,
  onOpenPage,
}: {
  locale: Locale;
  /** The flow hands the user to the Settings page that owns the control it just named. */
  onOpenPage: (page: string) => void;
}) {
  const doc = useSettings((s) => s.doc);
  const update = useSettings((s) => s.update);
  const step = useOnboarding((s) => s.step);
  const go = useOnboarding((s) => s.go);
  const close = useOnboarding((s) => s.close);
  const dialog = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<Place>("this_mac");
  const [path, setPath] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);
  const [pickFailed, setPickFailed] = useState(false);
  const [freeList, setFreeList] = useState<FreeModelInfo[] | null>(null);
  const [freeFailed, setFreeFailed] = useState(false);
  const [picked, setPicked] = useState<string | null>(null);

  const t: T = (key, params) => translate(locale, `onboarding.${key}`, params);

  // The stored answer is the one the card shows when the flow is re-run, so a second visit starts
  // where the first one stopped instead of at the default.
  useEffect(() => {
    if (!doc) return;
    if (doc.onboarding.place !== "undecided") setPlace(doc.onboarding.place);
    if (doc.project.root) setPath(doc.project.root);
  }, [doc]);

  // The free tier is the host's registry, so it is read when the third step opens: a small fixed
  // list, and the first visit is the one that has to offer the one live choice. The saved pick wins
  // on a second visit; otherwise the first row with a live route is preselected, and a row with no
  // route is shown but not pickable — the host would refuse the turn anyway.
  useEffect(() => {
    if (step !== 2 || freeList) return;
    void ipc
      .freeModelsList()
      .then((rows) => {
        setFreeList(rows);
        const saved = doc?.freeModel ?? null;
        const live = rows.find((r) => r.routable || r.local);
        setPicked(rows.some((r) => r.id === saved) ? saved : ((live ?? rows[0])?.id ?? null));
      })
      .catch(() => setFreeFailed(true));
  }, [step, freeList, doc]);

  useEffect(() => {
    // `preventScroll` because WebKit otherwise scrolls the step body to reveal the dialog it was
    // just asked to focus, and a tall step opens with its first card already under the header.
    dialog.current?.focus({ preventScroll: true });
    // The window's own keys — Esc stops a run — must not fire from under a setup dialog. The capture
    // phase is what lets this run before them; the dialog has no other way out than its buttons.
    const hold = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
      }
    };
    window.addEventListener("keydown", hold, true);
    return () => window.removeEventListener("keydown", hold, true);
  }, []);

  const persist = async (answer: Place, completed: boolean) => {
    await update((d) => {
      d.onboarding.place = answer;
      if (completed) d.onboarding.completed = true;
    });
  };

  const chooseFolder = async () => {
    setPicking(true);
    setPickFailed(false);
    try {
      const picked = await ipc.chooseProjectFolder(t("place.folder.chooseTitle"));
      if (!picked) return;
      setPlace("project_folder");
      setPath(picked);
      await update((d) => {
        d.project.root = picked;
        d.onboarding.place = "project_folder";
      });
    } catch {
      setPickFailed(true);
    } finally {
      setPicking(false);
    }
  };

  const next = async () => {
    if (step === 0) {
      go(1);
      return;
    }
    if (step === 1) {
      await persist(place, false);
      go(2);
      return;
    }
    await update((d) => {
      d.onboarding.place = place;
      d.onboarding.completed = true;
      if (picked) {
        d.freeModel = picked;
        // The free pick answers before the provider row would; the Settings page shows one source at
        // a time, so setup sets that row to local rather than leaving it claiming Remote.
        d.remoteProvider.source = "local";
      }
    });
    close();
    await useModel.getState().refresh();
  };

  const skip = async () => {
    await persist("later", true);
    close();
  };

  const openModelSettings = async () => {
    await update((d) => {
      d.onboarding.place = place;
      d.onboarding.completed = true;
      if (picked) {
        d.freeModel = picked;
        d.remoteProvider.source = "local";
      }
    });
    close();
    await useModel.getState().refresh();
    onOpenPage("model");
  };

  const couldNotPick = translate(locale, "common.notInBuild");

  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-[var(--scrim)] p-[var(--space-6)]">
      <div
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="onboarding-title"
        tabIndex={-1}
        className="surface flex max-h-[min(820px,92vh)] w-[min(760px,94vw)] flex-col overflow-hidden rounded-[var(--radius-lg)] border border-line bg-elevated text-fg shadow-raised"
      >
        {/* The mark and the step's question are the dialog, not its contents: they stay put while a
            step whose rows outrun the window scrolls under them. */}
        <div className="shrink-0 px-[var(--space-6)] pt-[var(--space-6)]">
          <div className="flex justify-center">
            <span className="rounded-full ring-1 ring-line">
              <MascotMark size={92} />
            </span>
          </div>

          <div className="mx-auto mt-[var(--space-6)] w-full max-w-[560px] text-center">
            <h2 id="onboarding-title" className="text-[26px] font-semibold text-fg-strong">
              {t(step === 0 ? "welcome.title" : step === 1 ? "place.title" : "model.title")}
            </h2>
            <p className="mt-2 text-quiet text-muted">
              {t(
                step === 0 ? "welcome.subtitle" : step === 1 ? "place.subtitle" : "model.subtitle",
              )}
            </p>
            {step === 0 && <p className="mt-[var(--space-4)]">{t("welcome.lead")}</p>}
            {step === 2 && <p className="mt-[var(--space-4)] text-muted">{t("model.lead")}</p>}
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-[var(--space-6)] pb-[var(--space-2)]">
          <div className="mx-auto mt-[var(--space-4)] w-full max-w-[560px]">
            {step === 0 && (
              <>
                <div className="card text-left">
                  {FEATURES.map((feature) => (
                    <div key={feature.key} className="card-row items-start" data-compact="true">
                      <span className="mt-0.5 text-accent-ink">
                        <Icon name={feature.icon} size={18} />
                      </span>
                      <div className="row-copy">
                        <span className="row-title">{t(`${feature.key}.title`)}</span>
                        <span className="row-desc">{t(`${feature.key}.desc`)}</span>
                      </div>
                    </div>
                  ))}
                </div>
                <p className="mt-[var(--space-4)] flex items-start gap-2 text-fine text-faint">
                  <Icon name="help" size={14} className="mt-0.5 shrink-0" />
                  <span>{t("welcome.note")}</span>
                </p>
              </>
            )}

            {step === 1 && (
              <>
                <div
                  role="radiogroup"
                  aria-label={t("place.title")}
                  className="flex flex-col gap-[var(--space-3)]"
                >
                  {PLACES.map((option) => {
                    const selected = place === option.id;
                    return (
                      <div
                        key={option.id}
                        data-selected={selected || undefined}
                        className={`rounded-[var(--radius-md)] border p-4 transition-colors ${
                          selected ? "border-accent bg-accent-soft" : "border-line hover:bg-hover"
                        }`}
                      >
                        <button
                          type="button"
                          role="radio"
                          aria-checked={selected}
                          className="flex w-full items-start gap-3 text-left"
                          onClick={() => setPlace(option.id)}
                        >
                          <span className={`mt-0.5 ${selected ? "text-accent-ink" : "text-muted"}`}>
                            <Icon name={option.icon} size={18} />
                          </span>
                          <span className="min-w-0 flex-1">
                            <span className="flex flex-wrap items-center gap-2">
                              <span className="text-ui font-medium">
                                {t(`${option.key}.title`)}
                              </span>
                              {option.id === "this_mac" && (
                                <span className="pill text-fine">{t("place.recommended")}</span>
                              )}
                            </span>
                            <span className="mt-1 block text-quiet text-muted">
                              {t(`${option.key}.desc`)}
                            </span>
                            {option.id === "project_folder" && selected && (
                              <span className="mt-1 block truncate font-mono text-fine text-faint">
                                {path ?? t("place.folder.none")}
                              </span>
                            )}
                          </span>
                          <span
                            aria-hidden
                            className={`mt-1 grid size-[18px] shrink-0 place-items-center rounded-full border ${
                              selected ? "border-accent" : "border-line-strong"
                            }`}
                          >
                            {selected && <span className="size-[9px] rounded-full bg-accent" />}
                          </span>
                        </button>
                        {option.id === "project_folder" && selected && (
                          <div className="mt-[var(--space-3)] flex flex-wrap items-center gap-3 border-t border-line pt-[var(--space-3)]">
                            <button
                              type="button"
                              className="rounded-[var(--radius-md)] border border-line bg-elevated px-3 py-1.5 text-quiet hover:bg-hover"
                              disabled={picking}
                              onClick={() => void chooseFolder()}
                            >
                              {t(picking ? "place.folder.choosing" : "place.folder.choose")}
                            </button>
                            {pickFailed && (
                              <span className="text-fine text-danger">{couldNotPick}</span>
                            )}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
                <button
                  type="button"
                  className="mx-auto mt-[var(--space-4)] block text-quiet text-muted underline decoration-line underline-offset-4 hover:text-fg"
                  onClick={() => void skip()}
                >
                  {t("place.skip")}
                </button>
              </>
            )}

            {step === 2 && (
              <>
                <div className="card text-left">
                  {freeList === null &&
                    (freeFailed ? (
                      <p role="alert" className="card-row text-quiet text-danger">
                        {t("model.failed")}
                      </p>
                    ) : (
                      <p className="card-row text-quiet text-muted">{t("model.loading")}</p>
                    ))}
                  {freeList?.map((option) => {
                    const selected = picked === option.id;
                    const live = option.routable || option.local;
                    return (
                      <button
                        key={option.id}
                        type="button"
                        role="radio"
                        aria-checked={selected}
                        aria-disabled={!live || undefined}
                        data-selected={selected || undefined}
                        className="card-row items-start text-left disabled:opacity-50"
                        disabled={!live}
                        onClick={() => setPicked(option.id)}
                      >
                        <span className={`mt-0.5 ${selected ? "text-accent-ink" : "text-muted"}`}>
                          <Icon name={option.local ? "bolt" : "globe"} size={18} />
                        </span>
                        <span className="row-copy">
                          <span className="row-title">{option.label}</span>
                          <span className="row-desc">
                            {option.local
                              ? `${option.provider} · ${t("model.routeApp")}`
                              : option.routable
                                ? `${option.provider} · ${t("model.routeGateway")}`
                                : `${option.provider} · ${t("model.routeNone")}`}
                          </span>
                        </span>
                        <span
                          aria-hidden
                          className={`mt-1 grid size-[18px] shrink-0 place-items-center rounded-full border ${
                            selected ? "border-accent" : "border-line-strong"
                          }`}
                        >
                          {selected && <span className="size-[9px] rounded-full bg-accent" />}
                        </span>
                      </button>
                    );
                  })}
                </div>
                {freeList !== null && !freeList.some((r) => r.routable || r.local) && (
                  <p className="mt-[var(--space-3)] text-quiet text-muted">{t("model.noneLive")}</p>
                )}
                <button
                  type="button"
                  className="mx-auto mt-[var(--space-4)] block text-quiet text-muted underline decoration-line underline-offset-4 hover:text-fg"
                  onClick={() => void openModelSettings()}
                >
                  {t("model.settings")}
                </button>
              </>
            )}
          </div>
        </div>

        <footer className="grid shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-[var(--space-3)] border-t border-line px-[var(--space-6)] py-[var(--space-4)]">
          <span className="text-fine text-faint">
            {t("stepOf", { current: step + 1, total: ONBOARDING_STEPS })}
          </span>
          <div className="flex items-center gap-1.5" aria-hidden>
            {Array.from({ length: ONBOARDING_STEPS }, (_, i) => (
              <span
                key={i}
                className={`size-[7px] rounded-full ${i === step ? "bg-accent" : "bg-line-strong"}`}
              />
            ))}
          </div>
          <div className="flex items-center justify-end gap-[var(--space-3)]">
            <button
              type="button"
              className="rounded-[var(--radius-md)] px-3 py-2 text-ui text-fg hover:bg-hover disabled:opacity-40"
              disabled={step === 0}
              onClick={() => go(step - 1)}
            >
              {t("back")}
            </button>
            <button
              type="button"
              className="rounded-[var(--radius-md)] bg-accent px-4 py-2 text-ui font-medium text-accent-contrast transition-opacity hover:opacity-90 disabled:opacity-50"
              disabled={!doc}
              onClick={() => void next()}
            >
              {t(step === ONBOARDING_STEPS - 1 ? "finish" : "next")}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
