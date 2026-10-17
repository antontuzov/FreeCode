// The palette the sidebar's Search row opens.
//
// It searches the conversations the host filed (#56) rather than the shell's own controls, because
// that is the one store this app has real content in: a palette over "New task" and "Settings" would
// be a two-item list dressed up as a feature. The words it looks for are the words that were said —
// the host reads the turns, which never cross the boundary until a conversation is opened, so the
// search has to happen where the files are.
import { useEffect, useRef, useState, type ReactNode } from "react";

import { Icon } from "./icons";
import { translate, type Locale } from "../i18n";
import { ipc, type SessionHit } from "../lib/ipc";
import { shortcutLabel } from "../lib/shortcuts";
import { useSessions } from "../state/sessions";

/** Before anything is typed the panel is a recent list, and a list longer than the panel is not
 * something the user can read down to. */
const RECENT_MAX = 8;

/**
 * The query marked inside the line it was found in.
 *
 * A result with no marked word is a title the query does not appear in, and the user cannot tell
 * whether the row is theirs. The search is the host's case-insensitive one; this only paints what it
 * already answered, and says nothing when the excerpt's cut left the match outside it.
 */
function mark(text: string, query: string): ReactNode {
  const needle = query.trim().toLowerCase();
  const at = needle ? text.toLowerCase().indexOf(needle) : -1;
  if (at < 0) return text;
  return (
    <>
      {text.slice(0, at)}
      <mark>{text.slice(at, at + needle.length)}</mark>
      {text.slice(at + needle.length)}
    </>
  );
}

export function Palette({
  locale,
  initial,
  onClose,
}: {
  locale: Locale;
  /** The query to start with: `?search=<query>` seeds it, which is the only way this surface gets
   * captured natively — the window takes no injected keystrokes. */
  initial: string;
  onClose: () => void;
}) {
  const recent = useSessions((s) => s.items);
  const open = useSessions((s) => s.open);
  const error = useSessions((s) => s.error);
  const [query, setQuery] = useState(initial);
  const [hits, setHits] = useState<SessionHit[]>([]);
  const [selected, setSelected] = useState(0);

  // Only the newest answer may paint. Two searches a keystroke apart come back in whatever order the
  // host finishes them, and the earlier, wider list landing last would replace the narrower one the
  // box now asks for with results for a query that is gone.
  const request = useRef(0);
  useEffect(() => {
    const n = (request.current += 1);
    if (!query.trim()) {
      setHits([]);
      return;
    }
    void ipc.searchSessions(query).then((found) => {
      if (request.current === n) setHits(found);
    });
  }, [query]);

  const searching = Boolean(query.trim());
  const results = searching
    ? hits.map((hit) => ({
        id: hit.id,
        title: hit.title,
        // The line the host matched, so the row says why it is here, over whose it was.
        detail: hit.excerpt,
        who: translate(locale, hit.role === "user" ? "palette.asked" : "palette.answered"),
      }))
    : recent.slice(0, RECENT_MAX).map((item) => ({
        id: item.id,
        title: item.title,
        detail: `${item.workspace} · ${item.model}`,
        who: null as string | null,
      }));

  // The keyboard's position belongs to the list it was moved through; a new list has a new first row.
  useEffect(() => setSelected(0), [hits, recent, query]);

  const choose = (id: string) => {
    void open(id).then(() => {
      // A refusal keeps the panel open with the reason in it: closing would turn "a reply is still
      // arriving" into a click that did nothing at all.
      if (!useSessions.getState().error) onClose();
    });
  };

  const step = (delta: number) => {
    if (!results.length) return;
    setSelected((i) => (i + delta + results.length) % results.length);
  };

  return (
    <>
      {/* The shell stays under it, and pressing that shell is how a user says they did not mean to
          search. The panel itself never loses the click: it is a sibling above, not a child. */}
      <div className="palette-scrim" onClick={onClose} />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={translate(locale, "sidebar.search")}
        className="palette-panel"
      >
        <div className="flex items-center gap-[var(--space-3)] px-[var(--space-3)] py-[var(--space-2)]">
          <Icon name="search" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              // Stopped here rather than left to the window, because `useShortcuts` skips a key the
              // surface in front has already answered — and this is the surface that has it.
              if (e.key === "ArrowDown") {
                e.preventDefault();
                step(1);
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                step(-1);
              } else if (e.key === "Enter") {
                e.preventDefault();
                const hit = results[selected];
                if (hit) choose(hit.id);
              } else if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              }
            }}
            placeholder={translate(locale, "palette.placeholder")}
            aria-label={translate(locale, "palette.placeholder")}
            className="min-w-0 flex-1 bg-transparent text-ui text-fg outline-none placeholder:text-faint"
          />
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-[var(--space-2)] pb-[var(--space-2)]">
          <p className="section-label">
            {translate(locale, searching ? "palette.results" : "palette.recent")}
          </p>
          {results.map((result, i) => (
            <button
              key={result.id}
              type="button"
              data-selected={i === selected || undefined}
              // The keyboard's row is followed into view; a list the panel scrolls inside could
              // otherwise carry the selection somewhere off the visible part of it.
              ref={i === selected ? (el) => el?.scrollIntoView({ block: "nearest" }) : undefined}
              className="palette-row"
              onClick={() => choose(result.id)}
            >
              <span className="row-copy min-w-0">
                <span className="row-title truncate">{result.title}</span>
                <span className="row-desc line-clamp-2">
                  {/* The size steps down, the ink does not: the keyboard's row paints the style's own
                      tint, and a fill costs the word standing on it contrast. One rung down here is a
                      word that goes unread on the one line the user is about to open. */}
                  {result.who && <span className="text-micro">{`${result.who} · `}</span>}
                  {mark(result.detail, query)}
                </span>
              </span>
              {/* The arrow the Enter key would act on, drawn only where the keyboard is. */}
              {i === selected && <Icon name="chevronRight" size={14} />}
            </button>
          ))}
          {!results.length && (
            <p className="px-[var(--space-3)] py-[var(--space-2)] text-quiet text-faint">
              {searching
                ? translate(locale, "palette.none", { query: query.trim() })
                : translate(locale, "palette.noneYet")}
            </p>
          )}
          {error && (
            <p role="alert" className="px-[var(--space-3)] py-1 text-fine text-danger">
              {error}
            </p>
          )}
        </div>

        <div className="flex items-center gap-[var(--space-2)] border-t border-line px-[var(--space-3)] py-[var(--space-2)] text-fine text-muted">
          <span className="kbd">↵</span>
          <span>{translate(locale, "palette.open")}</span>
          <span className="kbd">{shortcutLabel("escape")}</span>
          <span>{translate(locale, "palette.close")}</span>
        </div>
      </div>
    </>
  );
}
