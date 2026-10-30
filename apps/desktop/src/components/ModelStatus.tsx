import { useEffect, useState } from "react";

import { translate, type Locale } from "../i18n";
import { ipc } from "../lib/ipc";
import { useModel } from "../state/model";
import { Icon } from "./icons";

/**
 * The card's own shape, filled with the rows it is about to hold. The inspector stacks underneath,
 * so a single line of "Checking…" text made every group below jump upward the moment the round trip
 * answered — the column rearranged itself under the user on each launch, on a machine that takes a
 * second to reply.
 */
function StatusSkeleton({ locale }: { locale: Locale }) {
  return (
    <section
      aria-label={translate(locale, "model.free.title")}
      className="w-full shrink-0 border-b border-line bg-elevated p-[var(--space-4)] shadow-raised"
    >
      <div className="flex items-center gap-2">
        <Icon name="bolt" size={15} className="text-accent-ink" />
        <span className="skeleton-line w-32" />
        <span className="skeleton-line ml-auto h-5 w-14" />
      </div>
      <div className="mt-3 flex flex-col gap-1">
        {[0, 1].map((row) => (
          <div key={row} className="flex items-baseline justify-between gap-3">
            <span className="skeleton-line w-16" />
            <span className="skeleton-line w-28" />
          </div>
        ))}
      </div>
      <span className="skeleton-line mt-3 block w-24" />
      <span className="sr-only">{translate(locale, "model.status.checking")}</span>
    </section>
  );
}

/**
 * The strip that says which model answers and where it is answered. There are two routes: the free
 * tier served by OpenCode's service on this Mac — the built-in one — and a remote provider the user
 * configured. The local engine is no longer a route, so there is no packing, load or release row.
 */
export function ModelStatus({ locale }: { locale: Locale }) {
  const { status, error, refresh } = useModel();
  const remote = status?.remote ?? null;
  const free = status?.free ?? null;
  // The card cannot say whether a remote turn is possible without this bit, and it is the only thing
  // the Keychain will tell anyone. Read once per status answer; `null` while the answer is in flight.
  const [keySaved, setKeySaved] = useState<boolean | null>(null);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (!remote) {
      setKeySaved(null);
      return;
    }
    let live = true;
    void ipc
      .remoteKeyPresent()
      .then((present) => {
        if (live) setKeySaved(present);
      })
      .catch(() => {
        if (live) setKeySaved(null);
      });
    return () => {
      live = false;
    };
  }, [remote]);

  // No status yet, which is the ordinary first second of every launch: hold the card's shape rather
  // than returning a line the finished card grows out of. An error is the one state with nothing to
  // stand in for, so it still gets its own row below.
  if (!status && !error) return <StatusSkeleton locale={locale} />;
  if (error) {
    return (
      <p className="transcript-row" role="status">
        <span className="dot-running" aria-hidden />
        {error}
      </p>
    );
  }
  if (!status) return null;

  // A remote provider answers instead of the free tier, so the card shows that provider rather than
  // a free-model row that describes a run this one will never make.
  if (remote) {
    const providerHost =
      remote.baseUrl.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").replace(/\/.*$/, "") || "—";
    const ready = remote.allowed && remote.configured && keySaved === true;
    const badge = !remote.allowed
      ? translate(locale, "model.badge.needsConsent")
      : !remote.configured
        ? translate(locale, "model.badge.notSetUp")
        : keySaved === false
          ? translate(locale, "model.badge.noKey")
          : translate(locale, "model.badge.remote");
    const note = !remote.allowed
      ? translate(locale, "model.remote.noConsent")
      : !remote.configured
        ? translate(locale, "model.remote.notConfigured")
        : keySaved === false
          ? translate(locale, "model.remote.noKey")
          : keySaved === null
            ? translate(locale, "model.remote.keyChecking")
            : translate(locale, "model.remote.ready");
    return (
      <section
        aria-label={translate(locale, "model.remote.title")}
        className="w-full shrink-0 border-b border-line bg-elevated p-[var(--space-4)] shadow-raised"
      >
        <div className="flex items-center gap-2">
          <Icon name="globe" size={15} className="text-accent-ink" />
          <h2 className="min-w-0 flex-1 truncate text-ui font-semibold text-fg">
            {remote.model || translate(locale, "model.remote.untitled")}
          </h2>
          <span
            className={`shrink-0 rounded-[var(--radius-pill)] px-2 py-0.5 text-fine ${
              ready ? "bg-accent-soft text-fg-strong" : "bg-sunken text-muted"
            }`}
          >
            {badge}
          </span>
        </div>
        <dl className="mt-3 flex flex-col gap-1 text-fine text-muted">
          <div className="flex items-baseline justify-between gap-3">
            <dt className="shrink-0">{translate(locale, "model.remote.provider")}</dt>
            <dd className="truncate font-mono text-fg">{providerHost}</dd>
          </div>
          <div className="flex items-baseline justify-between gap-3">
            <dt className="shrink-0">{translate(locale, "model.remote.window")}</dt>
            <dd className="truncate font-mono tabular-nums text-fg">{remote.contextTokens}</dd>
          </div>
          <div className="flex items-baseline justify-between gap-3">
            <dt className="shrink-0">{translate(locale, "model.remote.consent")}</dt>
            <dd className="truncate text-fg">
              {remote.allowed
                ? translate(locale, "model.remote.consentOn")
                : translate(locale, "model.remote.consentOff")}
            </dd>
          </div>
          <div className="flex items-baseline justify-between gap-3">
            <dt className="shrink-0">{translate(locale, "model.remote.key")}</dt>
            <dd className="truncate text-fg">
              {keySaved === null
                ? translate(locale, "model.remote.keyCheckingShort")
                : keySaved
                  ? translate(locale, "model.remote.keySaved")
                  : translate(locale, "model.remote.keyAbsent")}
            </dd>
          </div>
        </dl>
        <p className="mt-3 text-fine text-muted">{note}</p>
      </section>
    );
  }

  // The free tier is the built-in route, so the card is about the one thing that decides whether a
  // prompt can be answered: is the service on this Mac running?
  const app = status.opencode;
  const ready = app.available;
  const badge = ready
    ? translate(locale, "model.badge.free")
    : translate(locale, "model.badge.noService");
  const note = ready
    ? translate(locale, "model.free.appReady")
    : translate(locale, "model.free.noService");
  return (
    <section
      aria-label={translate(locale, "model.free.title")}
      className="w-full shrink-0 border-b border-line bg-elevated p-[var(--space-4)] shadow-raised"
    >
      <div className="flex items-center gap-2">
        <Icon name="bolt" size={15} className="text-accent-ink" />
        <h2 className="min-w-0 flex-1 truncate text-ui font-semibold text-fg">
          {free?.label ?? translate(locale, "model.free.title")}
        </h2>
        <span
          className={`shrink-0 rounded-[var(--radius-pill)] px-2 py-0.5 text-fine ${
            ready ? "bg-accent-soft text-fg-strong" : "bg-sunken text-muted"
          }`}
        >
          {badge}
        </span>
      </div>
      <dl className="mt-3 flex flex-col gap-1 text-fine text-muted">
        <div className="flex items-baseline justify-between gap-3">
          <dt className="shrink-0">{translate(locale, "model.remote.provider")}</dt>
          <dd className="truncate text-fg">{free?.provider ?? "—"}</dd>
        </div>
        <div className="flex items-baseline justify-between gap-3">
          <dt className="shrink-0">{translate(locale, "model.free.route")}</dt>
          <dd className="truncate text-fg">
            {`${translate(locale, "model.free.appRoute")}${app.version ? ` · ${app.version}` : ""}`}
          </dd>
        </div>
      </dl>
      <p className="mt-3 text-fine text-muted">{note}</p>
    </section>
  );
}
