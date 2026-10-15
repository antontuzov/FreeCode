import { translate, type Locale } from "../i18n";
import { useChat } from "../state/chat";
import { useModel } from "../state/model";
import { useTranscript } from "../state/transcript";
import { Icon } from "./icons";

/**
 * The server counts tokens, so the figure arrives as a fraction of the window and 31 tokens of a
 * 32k one is 0.09%. Rounded to whole percent that reads "0%" — an empty meter for a window that is
 * not empty — so the low end keeps a decimal instead.
 */
function percentLabel(used: number): string {
  return used > 0 && used < 10 ? `${used.toFixed(1)}%` : `${Math.round(used)}%`;
}

/**
 * The strip the reference puts under the composer. Its badge is the same guard verdict the status
 * card uses, so the bottom of the window cannot claim "Local" while the top refuses to run.
 */
export function StatusBar({ locale }: { locale: Locale }) {
  const status = useModel((s) => s.status);
  const engine = useChat((s) => s.engine);
  const contextUsed = useTranscript((s) => s.contextUsed);

  const badge = status?.blocked
    ? translate(locale, "model.badge.notRunning")
    : status?.local
      ? translate(locale, "model.badge.local")
      : translate(locale, "model.badge.remote");

  // While a server is up, this pair describes the run rather than the machine. The status' backend is
  // what this host *could* decode on, so it goes on saying "Metal" after layer offload is switched
  // off and every layer is being decoded on the CPU.
  const runLine = engine
    ? `${engine.packing} · ${engine.backend}`
    : status
      ? `${status.packing} · ${status.backend}`
      : null;

  // The meter shows what the last finished run measured and nothing before that. A real run reports
  // its own token count, so the figure is the server's, not an estimate made from the text.
  const used = contextUsed;

  return (
    <footer className="flex h-8 shrink-0 items-center gap-3 border-t border-line bg-bg px-[var(--space-4)] text-fine text-muted">
      <span className="flex items-center gap-1.5">
        <Icon name="monitor" size={13} />
        {badge}
      </span>
      {runLine && <span className="truncate font-mono text-faint">{runLine}</span>}
      <span className="flex-1" />
      <span
        role="progressbar"
        aria-valuenow={used ?? 0}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={translate(locale, "statusbar.context")}
        className="h-1.5 w-24 overflow-hidden rounded-[var(--radius-pill)] bg-sunken"
      >
        <span className="block h-full bg-accent" style={{ width: `${used ?? 0}%` }} aria-hidden />
      </span>
      <span className="w-8 text-right font-mono tabular-nums">
        {used === null ? "—" : percentLabel(used)}
      </span>
    </footer>
  );
}
