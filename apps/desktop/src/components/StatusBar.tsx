import { translate, type Locale } from "../i18n";
import { useChat } from "../state/chat";
import { useModel } from "../state/model";
import { useTranscript } from "../state/transcript";
import { Icon } from "./icons";

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

  // Context usage is only meaningful once something has measured it. A real run reports none yet —
  // the fork only tells us after the reply — so the meter reads as unknown instead of inventing a
  // percentage, which would be the one number in this window the user might act on.
  const used = contextUsed;

  return (
    <footer className="flex h-8 shrink-0 items-center gap-3 border-t border-line bg-bg px-[var(--space-4)] text-[calc(var(--font-ui)-2px)] text-muted">
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
      <span className="w-8 text-right font-mono">{used === null ? "—" : `${used}%`}</span>
    </footer>
  );
}
