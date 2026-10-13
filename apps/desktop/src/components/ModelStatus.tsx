import { useEffect } from "react";

import { translate, type Locale } from "../i18n";
import { useModel } from "../state/model";
import { Icon } from "./icons";

/**
 * The strip that makes the "Local" badge honest: it is fed by the host's real catalog + engine
 * resolution, and it names the load-guard refusal instead of hiding it.
 */
export function ModelStatus({ locale }: { locale: Locale }) {
  const { status, error, loading, refresh } = useModel();

  useEffect(() => {
    void refresh();
  }, [refresh]);

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
      </dl>
      {status.blocked ? (
        <div className="mt-3 rounded-[var(--radius-md)] border border-danger/40 p-3">
          <p className="text-[calc(var(--font-ui)-1px)] font-medium text-danger">
            {status.blocked.title}
          </p>
          <p className="mt-1 text-[calc(var(--font-ui)-2px)] text-muted">{status.blocked.detail}</p>
          {/* The action is printed, not yet clickable: opening it is the Model Manager's job in
              Phase 2, where the app fetches the fork engine itself instead of redirecting. */}
          <p className="mt-2 text-[calc(var(--font-ui)-2px)] text-fg">
            {status.blocked.ctaLabel}:{" "}
            <span className="font-mono text-muted">{status.blocked.ctaUrl}</span>
          </p>
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
