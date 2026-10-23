import { useEffect, useState } from "react";

import { translate, type Locale } from "../i18n";
import { ipc, type EngineInstallProgress, type WeightsProgress } from "../lib/ipc";
import { useChat } from "../state/chat";
import { useModel } from "../state/model";
import { ctaLabel, useTranscript } from "../state/transcript";
import { ExternalLink } from "./ExternalLink";
import { Icon } from "./icons";

/** The lock records sizes in bytes; a download label that needs three digits to read is no use. */
function megabytes(bytes: number): string {
  // One decimal on both sides of "x of y": switching precision at 10 MB made an 11.5 MB asset
  // report "11 MB of 12 MB" while its bar was already full.
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

/** The part of a fetch progress the two kinds of progress payload have in common. */
type Downloading = { phase: "downloading"; transferredBytes: number; totalBytes: number };

/** The bar both fetches draw: bytes moved against the size the catalog or the lock published. */
function ByteProgress({
  locale,
  label,
  progress,
}: {
  locale: Locale;
  label: string;
  progress: Downloading;
}) {
  const { transferredBytes, totalBytes } = progress;
  const percent = totalBytes > 0 ? Math.min(100, (transferredBytes / totalBytes) * 100) : 0;
  return (
    <div className="mt-2">
      <div
        aria-label={label}
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={Math.round(percent)}
        className="h-1 overflow-hidden rounded-full bg-sunken"
        role="progressbar"
      >
        <div className="h-full bg-accent" style={{ width: `${percent}%` }} />
      </div>
      <p className="mt-1 font-mono tabular-nums text-fine text-muted">
        {translate(locale, "model.fetch.progress", {
          transferred: megabytes(transferredBytes),
          total: megabytes(totalBytes),
        })}
      </p>
    </div>
  );
}

/** Byte progress while the host fetches the engine; `done` is the command's own return value. */
function InstallProgress({
  locale,
  progress,
}: {
  locale: Locale;
  progress: EngineInstallProgress | null;
}) {
  if (!progress) return null;
  if (progress.phase === "unpacking") {
    return (
      <p className="mt-2 flex items-center gap-1.5 text-fine text-muted">
        <span className="dot-running" aria-hidden />
        {translate(locale, "model.install.unpacking")}
      </p>
    );
  }
  if (progress.phase !== "downloading") return null;
  return (
    <ByteProgress locale={locale} label={translate(locale, "model.install")} progress={progress} />
  );
}

/** Byte progress while the host downloads the pack; `done` only lands once the command returns. */
function DownloadProgress({
  locale,
  progress,
}: {
  locale: Locale;
  progress: WeightsProgress | null;
}) {
  if (!progress || progress.phase !== "downloading") return null;
  return (
    <ByteProgress locale={locale} label={translate(locale, "model.download")} progress={progress} />
  );
}

/**
 * The card's own shape, filled with the rows it is about to hold. The inspector stacks underneath,
 * so a single line of "Checking…" text made every group below jump upward the moment the round trip
 * answered — the column rearranged itself under the user on each launch, on a machine that takes a
 * second to reply and longer on one that has a 27 GB pack to resolve.
 */
function StatusSkeleton({ locale }: { locale: Locale }) {
  return (
    <section
      aria-label={translate(locale, "model.packing")}
      className="w-full shrink-0 border-b border-line bg-elevated p-[var(--space-4)] shadow-raised"
    >
      <div className="flex items-center gap-2">
        <Icon name="bolt" size={15} className="text-accent-ink" />
        <span className="skeleton-line w-32" />
        <span className="skeleton-line ml-auto h-5 w-14" />
      </div>
      <div className="mt-3 flex flex-col gap-1">
        {[0, 1, 2].map((row) => (
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
 * The strip that makes the "Local" badge honest: it is fed by the host's real catalog + engine
 * resolution, and it names the load-guard refusal instead of hiding it.
 */
export function ModelStatus({ locale }: { locale: Locale }) {
  const {
    status,
    error,
    refresh,
    installing,
    progress,
    installError,
    installEngine,
    downloading,
    downloadProgress,
    downloadError,
    downloadWeights,
  } = useModel();
  const engine = useChat((s) => s.engine);
  const release = useChat((s) => s.release);
  const start = useChat((s) => s.start);
  const starting = useChat((s) => s.starting);
  const running = useTranscript((s) => s.running);
  const remote = status?.remote ?? null;
  const free = status?.free ?? null;
  // The card cannot say whether a remote turn is possible without this bit, and it is the only thing
  // the Keychain will tell anyone. Read once per status answer; `null` while the answer is in flight.
  const [keySaved, setKeySaved] = useState<boolean | null>(null);
  // The free gateway's key, under the same rule: presence only, never the value.
  const [freeKeySaved, setFreeKeySaved] = useState<boolean | null>(null);

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

  useEffect(() => {
    if (!free) {
      setFreeKeySaved(null);
      return;
    }
    let live = true;
    void ipc
      .freeKeyPresent()
      .then((present) => {
        if (live) setFreeKeySaved(present);
      })
      .catch(() => {
        if (live) setFreeKeySaved(null);
      });
    return () => {
      live = false;
    };
  }, [free]);

  // `?install-engine=1` runs the Install action and `?download-weights=1` the Download one on mount.
  // `?start-server=1` presses the card's own Start. The native window takes no injected clicks, so
  // these are how the two fetches and the engine load get verified in the real app.
  const params = new URLSearchParams(window.location.search);
  const autoInstall = params.get("install-engine") === "1";
  const autoDownload = params.get("download-weights") === "1";
  const autoStart = params.get("start-server") === "1";
  useEffect(() => {
    if (autoInstall) void installEngine();
  }, [autoInstall, installEngine]);
  useEffect(() => {
    if (autoDownload) void downloadWeights();
  }, [autoDownload, downloadWeights]);
  useEffect(() => {
    if (autoStart) void start();
  }, [autoStart, start]);

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

  // A free pick answers without the local engine, like the remote source: the card shows the pick
  // and its gateway instead of packing, engine and server rows that describe a run this one will
  // never make. The refusal order mirrors the host's: consent, then routability, then the key.
  if (free) {
    const ready = free.allowed && free.routable && freeKeySaved === true;
    const badge = !free.allowed
      ? translate(locale, "model.badge.needsConsent")
      : !free.routable
        ? translate(locale, "model.badge.notRoutable")
        : freeKeySaved === false
          ? translate(locale, "model.badge.noKey")
          : translate(locale, "model.badge.free");
    const note = !free.allowed
      ? translate(locale, "model.free.noConsent")
      : !free.routable
        ? translate(locale, "model.free.notRoutable")
        : freeKeySaved === false
          ? translate(locale, "model.free.noKey")
          : freeKeySaved === null
            ? translate(locale, "model.free.keyChecking")
            : translate(locale, "model.free.ready");
    return (
      <section
        aria-label={translate(locale, "model.free.title")}
        className="w-full shrink-0 border-b border-line bg-elevated p-[var(--space-4)] shadow-raised"
      >
        <div className="flex items-center gap-2">
          <Icon name="bolt" size={15} className="text-accent-ink" />
          <h2 className="min-w-0 flex-1 truncate text-ui font-semibold text-fg">{free.label}</h2>
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
            <dd className="truncate text-fg">{free.provider}</dd>
          </div>
          <div className="flex items-baseline justify-between gap-3">
            <dt className="shrink-0">{translate(locale, "model.remote.consent")}</dt>
            <dd className="truncate text-fg">
              {free.allowed
                ? translate(locale, "model.remote.consentOn")
                : translate(locale, "model.remote.consentOff")}
            </dd>
          </div>
          <div className="flex items-baseline justify-between gap-3">
            <dt className="shrink-0">{translate(locale, "model.remote.key")}</dt>
            <dd className="truncate text-fg">
              {freeKeySaved === null
                ? translate(locale, "model.remote.keyCheckingShort")
                : freeKeySaved
                  ? translate(locale, "model.remote.keySaved")
                  : translate(locale, "model.remote.keyAbsent")}
            </dd>
          </div>
        </dl>
        <p className="mt-3 text-fine text-muted">{note}</p>
      </section>
    );
  }

  // A remote source answers without the local engine, so the card shows the provider instead of
  // packing, engine and server rows that describe a run this one will never make. The local controls
  // are absent rather than disabled: starting a 6 GB server would not change where the prompt goes.
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

  // `local` is false both when a remote provider answers and when the guard refuses to run at all,
  // so the badge cannot be derived from that one bit: a refused load is not "Remote", it is
  // nothing, and claiming otherwise would hide the refusal behind a working-looking label.
  const badge = status.blocked
    ? translate(locale, "model.badge.notRunning")
    : status.local
      ? translate(locale, "model.badge.local")
      : translate(locale, "model.badge.remote");

  // Which refusal the app can clear itself, by fetching what the catalog and the lock pinned. The
  // host says which one it is — the UI never re-derives it from the file name — and both kinds render
  // the same shape, so they share one block rather than two near-copies of it. The labels are
  // translated here rather than at the button, so the i18n gate, which resolves only literal key
  // strings passed to translate, still sees every key this card can show.
  const fetchAction = !status.blocked
    ? null
    : status.blocked.installable
      ? {
          run: installEngine,
          busy: installing,
          error: installError,
          label: translate(locale, "model.install"),
          busyLabel: translate(locale, "model.install.busy"),
          bar: <InstallProgress locale={locale} progress={progress} />,
        }
      : status.blocked.downloadable
        ? {
            run: downloadWeights,
            busy: downloading,
            error: downloadError,
            label: translate(locale, "model.download"),
            busyLabel: translate(locale, "model.download.busy"),
            bar: <DownloadProgress locale={locale} progress={downloadProgress} />,
          }
        : null;

  return (
    <section
      aria-label={translate(locale, "model.packing")}
      className="w-full shrink-0 border-b border-line bg-elevated p-[var(--space-4)] shadow-raised"
    >
      <div className="flex items-center gap-2">
        <Icon name="bolt" size={15} className="text-accent-ink" />
        <h2 className="min-w-0 flex-1 truncate text-ui font-semibold text-fg">
          {status.displayName}
        </h2>
        <span
          className={`shrink-0 rounded-[var(--radius-pill)] px-2 py-0.5 text-fine ${
            status.blocked ? "bg-sunken text-muted" : "bg-accent-soft text-fg-strong"
          }`}
        >
          {badge}
        </span>
      </div>
      <dl className="mt-3 flex flex-col gap-1 text-fine text-muted">
        <div className="flex items-baseline justify-between gap-3">
          <dt className="shrink-0">{translate(locale, "model.packing")}</dt>
          <dd className="truncate font-mono tabular-nums text-fg">
            {status.packing} · {status.sizeGb.toFixed(2)} GB
          </dd>
        </div>
        <div className="flex items-baseline justify-between gap-3">
          <dt className="shrink-0">{translate(locale, "model.engine")}</dt>
          <dd className="truncate font-mono text-fg">{status.engineTag}</dd>
        </div>
        {engine && (
          // The card only says a server is up when the host confirms one is answering, so the
          // address here is the evidence behind the "Local" badge rather than a promise.
          <div className="flex items-baseline justify-between gap-3">
            <dt className="shrink-0">{translate(locale, "model.server")}</dt>
            <dd className="truncate font-mono text-fg">
              {engine.baseUrl.replace(/^\w+:\/\//, "")} · -c {engine.contextTokens}
            </dd>
          </div>
        )}
      </dl>
      {!engine && !status.blocked && (
        // The mirror of the release below: without it the only way back to a warm server is sending a
        // message and waiting through the load it triggered.
        <button
          type="button"
          onClick={() => void start()}
          disabled={starting || running}
          className="mt-2 inline-flex items-center gap-1.5 rounded-[var(--radius-md)] border border-line px-2.5 py-1 text-fine text-muted hover:bg-hover hover:text-fg disabled:opacity-60"
        >
          <Icon name="bolt" size={13} />
          {starting
            ? translate(locale, "model.server.starting")
            : translate(locale, "model.server.start")}
        </button>
      )}
      {engine && !status.blocked && (
        // This is the app's only way back: the composer's Stop ends an answer and keeps the server,
        // so without this control the weights stay resident until the window closes. The next
        // message starts a fresh server, and paying that load is what the click is for.
        <button
          type="button"
          onClick={() => void release()}
          disabled={running}
          className="mt-2 inline-flex items-center gap-1.5 rounded-[var(--radius-md)] border border-line px-2.5 py-1 text-fine text-muted hover:bg-hover hover:text-fg disabled:opacity-60"
        >
          <Icon name="stop" size={13} />
          {translate(locale, "model.server.stop")}
        </button>
      )}
      {status.blocked ? (
        <div className="mt-3 rounded-[var(--radius-md)] border border-danger/40 p-3">
          <p className="text-quiet font-medium text-danger">
            {translate(locale, `blocker.${status.blocked.code}.title`)}
          </p>
          <p className="mt-1 text-fine text-muted">
            {translate(locale, `blocker.${status.blocked.code}.detail`, status.blocked.params)}
          </p>
          {fetchAction ? (
            // The app fetches the pinned asset itself; the fallback URL text below stays for every
            // other refusal, where no button in the app could clear it.
            <>
              <button
                type="button"
                onClick={() => void fetchAction.run()}
                disabled={fetchAction.busy}
                className="mt-2 inline-flex items-center gap-1.5 rounded-[var(--radius-md)] bg-accent px-2.5 py-1 text-fine font-medium text-accent-contrast disabled:opacity-60"
              >
                <Icon name="download" size={13} />
                {fetchAction.busy ? fetchAction.busyLabel : fetchAction.label}
              </button>
              {fetchAction.busy && fetchAction.bar}
              {fetchAction.error && (
                <p role="alert" className="mt-2 text-fine text-danger">
                  {fetchAction.error}
                </p>
              )}
            </>
          ) : (
            // No button in this app can clear this one, so the link is the action — and it is a link,
            // not an address to read out to a browser.
            <div className="mt-2">
              <ExternalLink
                url={status.blocked.ctaUrl}
                label={ctaLabel(locale, status.blocked.ctaCode)}
                locale={locale}
              />
            </div>
          )}
          <details className="mt-2">
            <summary className="cursor-pointer text-micro text-faint">
              {translate(locale, "transcript.refusal.technical")}
            </summary>
            <p className="disclosure-panel mt-1 break-words font-mono text-micro text-muted">
              {status.blocked.technical}
            </p>
          </details>
        </div>
      ) : (
        <p className="mt-3 text-fine text-muted">
          {status.engineInstalled && status.weightsPresent
            ? translate(locale, "model.status.ready")
            : status.engineInstalled
              ? translate(locale, "model.status.noWeights")
              : translate(locale, "model.status.noEngine")}
        </p>
      )}
      {status.warnings.length > 0 && (
        <ul className="mt-2 list-disc pl-5 text-fine text-warning">
          {status.warnings.map((warning) => (
            <li key={warning}>{warning}</li>
          ))}
        </ul>
      )}
      {status.uncheckedWarnings.length > 0 && (
        // Folded, not dropped: these rows describe other machines, and hiding them would lose a
        // real known issue — but listing them beside a matched warning implies they apply here.
        <details className="mt-2 text-fine text-muted">
          <summary className="cursor-pointer text-faint">
            {translate(locale, "model.warnings.unchecked", {
              count: status.uncheckedWarnings.length,
            })}
          </summary>
          <ul className="disclosure-panel mt-1 list-disc pl-5">
            {status.uncheckedWarnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </details>
      )}
    </section>
  );
}
