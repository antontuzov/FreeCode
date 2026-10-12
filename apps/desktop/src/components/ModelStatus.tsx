import { useEffect } from "react";

import { translate, type Locale } from "../i18n";
import { useModel } from "../state/model";

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
      className="surface m-[var(--space-4)] rounded-[var(--radius-lg)] border p-[var(--space-4)] shadow-[var(--shadow)]"
    >
      <div className="flex items-center gap-2">
        <h2 className="text-sm font-semibold text-fg">{status.displayName}</h2>
        <span
          className={`rounded-full px-2 py-0.5 text-xs ${
            status.blocked ? "bg-sunken text-muted" : "bg-accent-soft text-fg"
          }`}
        >
          {badge}
        </span>
      </div>
      <dl className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1 text-xs text-muted">
        <div className="flex justify-between">
          <dt>{translate(locale, "model.packing")}</dt>
          <dd className="font-mono text-fg">
            {status.packing} · {status.sizeGb.toFixed(2)} GB
          </dd>
        </div>
        <div className="flex justify-between">
          <dt>{translate(locale, "model.engine")}</dt>
          <dd className="font-mono text-fg">{status.engineTag}</dd>
        </div>
      </dl>
      {status.blocked ? (
        <div className="mt-3 rounded-[var(--radius-md)] border border-danger/40 p-3">
          <p className="text-sm font-medium text-danger">{status.blocked.title}</p>
          <p className="mt-1 text-xs text-muted">{status.blocked.detail}</p>
          {/* The action is printed, not yet clickable: opening it is the Model Manager's job in
              Phase 2, where the app fetches the fork engine itself instead of redirecting. */}
          <p className="mt-2 text-xs text-fg">
            {status.blocked.ctaLabel}:{" "}
            <span className="font-mono text-muted">{status.blocked.ctaUrl}</span>
          </p>
        </div>
      ) : (
        <p className="mt-3 text-xs text-muted">
          {status.engineInstalled && status.weightsPresent
            ? translate(locale, "model.status.ready")
            : status.engineInstalled
              ? translate(locale, "model.status.noWeights")
              : translate(locale, "model.status.noEngine")}
        </p>
      )}
      {status.warnings.length > 0 && (
        <ul className="mt-2 list-disc pl-5 text-xs text-warning">
          {status.warnings.map((warning) => (
            <li key={warning}>{warning}</li>
          ))}
        </ul>
      )}
    </section>
  );
}
