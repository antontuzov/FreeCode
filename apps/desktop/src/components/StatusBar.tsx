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
 * The reference's meter is not a bar but a ruler of slanted ticks: the used fraction drawn in the
 * text's own colour, the rest in the hairline's. A bar carries the accent and says "how full", which
 * is a verdict; the ticks are neutral and say "how far along the window this run has got", which is a
 * measurement. The count is the reference's own, read off its footer.
 */
const TICKS = 26;

/** Same rule as `percentLabel` keeps for the figure: a window with anything in it must not read empty,
 * so the first tick is spent as soon as the server reports a token. */
function ticksUsed(used: number | null): number {
  if (!used) return 0;
  return Math.min(TICKS, Math.max(1, Math.round((TICKS * used) / 100)));
}

/**
 * The strip the reference puts under the composer. Its badge is the same guard verdict the status
 * card uses, so the bottom of the window cannot claim "Local" while the top refuses to run.
 */
export function StatusBar({ locale }: { locale: Locale }) {
  const status = useModel((s) => s.status);
  const engine = useChat((s) => s.engine);
  const contextUsed = useTranscript((s) => s.contextUsed);

  // A selected provider makes the local engine's packing and backend beside the point, and a badge
  // reading "Remote" would hide the two refusals that can stop a prompt before it leaves: consent and
  // an unconfigured address. Those are named here, and the key is the card's to report.
  const remote = status?.remote ?? null;

  const badge = remote
    ? !remote.allowed
      ? translate(locale, "model.badge.needsConsent")
      : !remote.configured
        ? translate(locale, "model.badge.notSetUp")
        : translate(locale, "model.badge.remote")
    : status?.blocked
      ? translate(locale, "model.badge.notRunning")
      : status?.local
        ? translate(locale, "model.badge.local")
        : translate(locale, "model.badge.remote");

  // While a server is up, this pair describes the run rather than the machine. The status' backend is
  // what this host *could* decode on, so it goes on saying "Metal" after layer offload is switched
  // off and every layer is being decoded on the CPU.
  const runLine = remote
    ? `${remote.model || translate(locale, "model.remote.untitled")} · ${
        remote.baseUrl.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").replace(/\/.*$/, "") || "—"
      }`
    : engine
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
        className="flex shrink-0 items-center gap-[1.5px] text-fg"
        style={{ transform: "skewX(-18deg)" }}
      >
        {Array.from({ length: TICKS }, (_, i) => (
          <span
            key={i}
            aria-hidden
            className={`h-[9px] w-[1.5px] rounded-[1px] ${i < ticksUsed(used) ? "bg-current" : "bg-line-strong"}`}
          />
        ))}
      </span>
      <span className="w-8 text-right font-mono tabular-nums">
        {used === null ? "—" : percentLabel(used)}
      </span>
    </footer>
  );
}
