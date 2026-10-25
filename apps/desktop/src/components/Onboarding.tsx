// The first-run setup flow: the reference's three steps, bound to this app's truth.
//
// There is no gateway to detect and no account to connect, so the flow asks the two questions this
// build can answer — where a run works, and which of the three routes answers it — and hands the
// rest to the Settings page that already owns those controls. Finishing writes one small section
// of the settings document; pressing "Set up later" writes the same section with `later`, so the
// flow does not reopen on every launch because the user declined it once.
import { useEffect, useRef, useState } from "react";

import { MascotMark } from "./BrandMark";
import { Icon, type IconName } from "./icons";
import { translate, type Locale } from "../i18n";
import { ipc, type OnboardingPlace } from "../lib/ipc";
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

const ROUTES: { id: "free" | "local" | "remote"; icon: IconName; page: string; key: string }[] = [
  { id: "free", icon: "cube", page: "model", key: "model.free" },
  { id: "local", icon: "monitor", page: "model", key: "model.local" },
  { id: "remote", icon: "globe", page: "model", key: "model.remote" },
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

  const t: T = (key, params) => translate(locale, `onboarding.${key}`, params);

  // The stored answer is the one the card shows when the flow is re-run, so a second visit starts
  // where the first one stopped instead of at the default.
  useEffect(() => {
    if (!doc) return;
    if (doc.onboarding.place !== "undecided") setPlace(doc.onboarding.place);
    if (doc.project.root) setPath(doc.project.root);
  }, [doc]);

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
    await persist(place, true);
    close();
  };

  const skip = async () => {
    await persist("later", true);
    close();
  };

  const openRoute = async (page: string) => {
    await persist(place, true);
    close();
    onOpenPage(page);
  };

  const couldNotPick = translate(locale, "common.notInBuild");
  const route = doc?.freeModel
    ? "free"
    : doc?.remoteProvider.source === "remote"
      ? "remote"
      : doc
        ? "local"
        : null;

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
                  {ROUTES.map((option) => {
                    const current = route === option.id;
                    return (
                      <div key={option.id} className="card-row items-start" data-compact="true">
                        <span className={`mt-0.5 ${current ? "text-accent-ink" : "text-muted"}`}>
                          <Icon name={option.icon} size={18} />
                        </span>
                        <div className="row-copy">
                          <span className="row-title">
                            {t(`${option.key}.title`)}
                            {current && (
                              <span className="pill text-fine">{t("model.current")}</span>
                            )}
                          </span>
                          <span className="row-desc">{t(`${option.key}.desc`)}</span>
                        </div>
                        <button
                          type="button"
                          className="shrink-0 rounded-[var(--radius-md)] border border-line bg-elevated px-3 py-1.5 text-quiet hover:bg-hover"
                          onClick={() => void openRoute(option.page)}
                        >
                          {t(`${option.key}.cta`)}
                        </button>
                      </div>
                    );
                  })}
                </div>
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
