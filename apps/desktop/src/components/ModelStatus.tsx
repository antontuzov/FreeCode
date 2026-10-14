import { useEffect } from "react";

import { translate, type Locale } from "../i18n";
import type { EngineInstallProgress, WeightsProgress } from "../lib/ipc";
import { useChat } from "../state/chat";
import { useModel } from "../state/model";
import { useTranscript } from "../state/transcript";
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
      <p className="mt-1 font-mono text-[calc(var(--font-ui)-2px)] text-muted">
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
      <p className="mt-2 flex items-center gap-1.5 text-[calc(var(--font-ui)-2px)] text-muted">
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
 * The strip that makes the "Local" badge honest: it is fed by the host's real catalog + engine
 * resolution, and it names the load-guard refusal instead of hiding it.
 */
export function ModelStatus({ locale }: { locale: Locale }) {
  const {
    status,
    error,
    loading,
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
  const running = useTranscript((s) => s.running);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // `?install-engine=1` runs the Install action and `?download-weights=1` the Download one on mount.
  // The native window takes no injected clicks, so these are how the two fetches and their progress
  // bars get verified in the real app.
  const params = new URLSearchParams(window.location.search);
  const autoInstall = params.get("install-engine") === "1";
  const autoDownload = params.get("download-weights") === "1";
  useEffect(() => {
    if (autoInstall) void installEngine();
  }, [autoInstall, installEngine]);
  useEffect(() => {
    if (autoDownload) void downloadWeights();
  }, [autoDownload, downloadWeights]);

  if (loading && !status) {
    return <p className="transcript-row">{translate(locale, "model.status.checking")}</p>;
  }
  if (error) {
    return (
      <p className="transcript-row" role="status">
        <span className="dot-running" aria-hidden />
        {error}
      </p>
    );
  }
  if (!status) return null;

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
      className="w-[var(--inspector-width)] shrink-0 border-b border-line bg-elevated p-[var(--space-4)]"
    >
      <div className="flex items-center gap-2">
        <Icon name="bolt" size={15} className="text-accent" />
        <h2 className="min-w-0 flex-1 truncate text-sm font-semibold text-fg">
          {status.displayName}
        </h2>
        <span
          className={`shrink-0 rounded-[var(--radius-pill)] px-2 py-0.5 text-[calc(var(--font-ui)-2px)] ${
            status.blocked ? "bg-sunken text-muted" : "bg-accent-soft text-fg-strong"
          }`}
        >
          {badge}
        </span>
      </div>
      <dl className="mt-3 flex flex-col gap-1 text-[calc(var(--font-ui)-2px)] text-muted">
        <div className="flex items-baseline justify-between gap-3">
          <dt className="shrink-0">{translate(locale, "model.packing")}</dt>
          <dd className="truncate font-mono text-fg">
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
      {engine && !status.blocked && (
        // This is the app's only way back: the composer's Stop ends an answer and keeps the server,
        // so without this control the weights stay resident until the window closes. The next
        // message starts a fresh server, and paying that load is what the click is for.
        <button
          type="button"
          onClick={() => void release()}
          disabled={running}
          className="mt-2 inline-flex items-center gap-1.5 rounded-[var(--radius-md)] border border-line px-2.5 py-1 text-[calc(var(--font-ui)-2px)] text-muted hover:bg-hover hover:text-fg disabled:opacity-60"
        >
          <Icon name="stop" size={13} />
          {translate(locale, "model.server.stop")}
        </button>
      )}
      {status.blocked ? (
        <div className="mt-3 rounded-[var(--radius-md)] border border-danger/40 p-3">
          <p className="text-[calc(var(--font-ui)-1px)] font-medium text-danger">
            {status.blocked.title}
          </p>
          <p className="mt-1 text-[calc(var(--font-ui)-2px)] text-muted">{status.blocked.detail}</p>
          {fetchAction ? (
            // The app fetches the pinned asset itself; the fallback URL text below stays for every
            // other refusal, where no button in the app could clear it.
            <>
              <button
                type="button"
                onClick={() => void fetchAction.run()}
                disabled={fetchAction.busy}
                className="mt-2 inline-flex items-center gap-1.5 rounded-[var(--radius-md)] bg-accent px-2.5 py-1 text-[calc(var(--font-ui)-2px)] font-medium text-accent-contrast disabled:opacity-60"
              >
                <Icon name="download" size={13} />
                {fetchAction.busy ? fetchAction.busyLabel : fetchAction.label}
              </button>
              {fetchAction.busy && fetchAction.bar}
              {fetchAction.error && (
                <p role="alert" className="mt-2 text-[calc(var(--font-ui)-2px)] text-danger">
                  {fetchAction.error}
                </p>
              )}
            </>
          ) : (
            <p className="mt-2 text-[calc(var(--font-ui)-2px)] text-fg">
              {status.blocked.ctaLabel}:{" "}
              <span className="font-mono text-muted">{status.blocked.ctaUrl}</span>
            </p>
          )}
        </div>
      ) : (
        <p className="mt-3 text-[calc(var(--font-ui)-2px)] text-muted">
          {status.engineInstalled && status.weightsPresent
            ? translate(locale, "model.status.ready")
            : status.engineInstalled
              ? translate(locale, "model.status.noWeights")
              : translate(locale, "model.status.noEngine")}
        </p>
      )}
      {status.warnings.length > 0 && (
        <ul className="mt-2 list-disc pl-5 text-[calc(var(--font-ui)-2px)] text-warning">
          {status.warnings.map((warning) => (
            <li key={warning}>{warning}</li>
          ))}
        </ul>
      )}
      {status.uncheckedWarnings.length > 0 && (
        // Folded, not dropped: these rows describe other machines, and hiding them would lose a
        // real known issue — but listing them beside a matched warning implies they apply here.
        <details className="mt-2 text-[calc(var(--font-ui)-2px)] text-muted">
          <summary className="cursor-pointer text-faint">
            {translate(locale, "model.warnings.unchecked", {
              count: status.uncheckedWarnings.length,
            })}
          </summary>
          <ul className="mt-1 list-disc pl-5">
            {status.uncheckedWarnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </details>
      )}
    </section>
  );
}
