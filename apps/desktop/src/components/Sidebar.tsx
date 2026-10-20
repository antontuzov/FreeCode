import { useState } from "react";

import { MascotMark } from "./BrandMark";
import { IdentityMenu } from "./IdentityMenu";
import { Icon, type IconName } from "./icons";
import { translate, type Locale } from "../i18n";
import type { SessionSummary } from "../lib/ipc";
import { shortcutLabel, type ShortcutId } from "../lib/shortcuts";
import { useNav } from "../state/nav";
import { useSessions } from "../state/sessions";
import { useTranscript } from "../state/transcript";

type NavRow = {
  key: string;
  icon: IconName;
  /** Called with the actions when the row is clicked. It has to *run* one: an `a.newTask` without the
   * parentheses hands back the function and returns, and a row that does that renders, takes the
   * click and does nothing — a `=> void` signature cannot catch it, because a function returning
   * something is still assignable to one that returns nothing. */
  onActivate?: (actions: SidebarActions) => void;
  /** The key that fires this row from anywhere in the window, named on hover. Only ever a shortcut
   * `lib/shortcuts.ts` answers, so the hint cannot get ahead of the handler. */
  shortcut?: ShortcutId;
  /** A row the reference layout draws and this build has nothing behind. It stays on screen — the
   * layout is the contract — but a click on it says so rather than looking like a lost one. */
  placeholder?: true;
  /** A hover line that says what the row holds rather than what it cannot do, named by key so a
   * module-level row still reads in the locale the window answers in. */
  tipKey?: string;
};

const PRIMARY: NavRow[] = [
  { key: "newTask", icon: "newTask", shortcut: "newTask", onActivate: (a) => a.newTask() },
  // The row that opens the palette. `placeholder` went with it: the search has a store to search
  // now, so the honest sentence about this build not having one stopped being honest.
  { key: "search", icon: "search", shortcut: "palette", onActivate: (a) => a.openPalette() },
];

const PLACES: NavRow[] = [
  // The same reason the search row's placeholder went: this row has a page behind it now, so it
  // opens it instead of saying this build has nothing to open.
  {
    key: "knowledge",
    icon: "knowledge",
    tipKey: "sidebar.knowledgeAbout",
    onActivate: (a) => a.openKnowledge(),
  },
  { key: "sites", icon: "sites", placeholder: true },
  { key: "automations", icon: "automations", placeholder: true },
  // The row's local meaning: this app installs no add-ons, but it does have a set of capabilities the
  // agent is handed, and the page the row opens is that list — with what this build runs of it.
  {
    key: "extensions",
    icon: "extensions",
    tipKey: "sidebar.extensionsAbout",
    onActivate: (a) => a.openExtensions(),
  },
];

/** The rows the collapsed rail keeps. A placeholder has no click to give back, and a bare glyph that
 * does nothing is the failure this build already fixed once when the rows had labels to apologise
 * under — the rail only carries what this build answers. */
const RAIL_ROWS = [...PRIMARY, ...PLACES].filter((r) => r.onActivate && !r.placeholder);

type Conversation = {
  /** null for a row that only exists to show the layout: the fixture's tasks have no conversation to
   * reopen, and a row that looks clickable and is not is the failure this build already fixed once. */
  id: string | null;
  title: string;
  /** The model that answered, shown on hover rather than in the row: the column is 260 px wide and
   * the conversation's own words are what identifies it. */
  meta?: string;
};

type Workspace = { name: string; conversations: Conversation[] };

/**
 * The filed conversations, under the folder each happened in.
 *
 * The host lists them newest first, so the groups are taken in the order their first conversation
 * appears and each group keeps that order. One workspace today — the root the host resolved — but
 * the grouping is what the reference draws, and it is the shape a second folder would join.
 */
function group(items: SessionSummary[], unnamed: string): Workspace[] {
  const byName = new Map<string, Workspace>();
  for (const item of items) {
    const name = item.workspace || unnamed;
    const found = byName.get(name);
    const conversation: Conversation = { id: item.id, title: item.title, meta: item.model };
    if (found) found.conversations.push(conversation);
    else byName.set(name, { name, conversations: [conversation] });
  }
  return [...byName.values()];
}

/** Only ever rendered under `?fixture=1`, to compare the layout against docs/reference-ui. */
const FIXTURE_WORKSPACES: Workspace[] = [
  {
    name: "smollm-studio",
    conversations: [
      { id: null, title: "Phase 0 host" },
      { id: null, title: "Catalog pin" },
    ],
  },
];

function Row({
  icon,
  label,
  active,
  indent,
  onClick,
  hint,
  shortcut,
  tip,
  className,
}: {
  icon?: IconName;
  label: string;
  active?: boolean;
  indent?: boolean;
  onClick?: () => void;
  hint?: string;
  shortcut?: string;
  /** A hover line that says what the row holds rather than what it cannot do, so it needs a row that
   * is not `hint`-disabled to show it. */
  tip?: string;
  /** A `.nav-item` modifier — `trailing`, which clears room for a control at the row's right end. */
  className?: string;
}) {
  return (
    <button
      type="button"
      data-active={active || undefined}
      // Not `disabled`: a disabled button swallows the hover that carries the sentence explaining
      // why it does nothing, which is the one thing the row still has to say.
      aria-disabled={hint ? true : undefined}
      title={hint ?? shortcut ?? tip}
      className={`nav-item ${indent ? "indent" : ""} ${className ?? ""}`}
      onClick={onClick}
    >
      {icon && <Icon name={icon} />}
      <span className="truncate">{label}</span>
    </button>
  );
}

export type SidebarActions = {
  /** Forget the conversation and keep the server: the one row this build can honour. */
  newTask: () => void;
  /** Put the search over the shell. */
  openPalette: () => void;
  /** Open the Knowledge Center: the catalog and this machine's share of it. */
  openKnowledge: () => void;
  /** Open the Extensions page: the tools the agent is offered, and which of them this build runs. */
  openExtensions: () => void;
};

/** The steps the header row carries: two along the surface stack, and the search over everything.
 *
 * They live here rather than in the header they are drawn in because folding the sidebar takes the
 * header away with the column, and a control you lose by clicking the fold button is a control you
 * cannot reach. So the transcript header picks them up in that case, and the same three glyphs — the
 * same disabled look, the same hover line — stand at the head of the column instead. */
export function NavSteps({ locale, onOpenPalette }: { locale: Locale; onOpenPalette: () => void }) {
  const back = useNav((s) => s.back);
  const forward = useNav((s) => s.forward);
  const canBack = useNav((s) => s.cursor > 0);
  const canForward = useNav((s) => s.cursor < s.history.length - 1);

  return (
    <div className="flex items-center gap-1">
      <button
        type="button"
        aria-label={translate(locale, "nav.back")}
        aria-disabled={canBack ? undefined : true}
        title={canBack ? translate(locale, "nav.back") : translate(locale, "nav.noHistoryBehind")}
        className="icon-btn"
        onClick={back}
      >
        <Icon name="back" />
      </button>
      <button
        type="button"
        aria-label={translate(locale, "nav.forward")}
        aria-disabled={canForward ? undefined : true}
        title={
          canForward ? translate(locale, "nav.forward") : translate(locale, "nav.noHistoryAhead")
        }
        className="icon-btn"
        onClick={forward}
      >
        <Icon name="forward" />
      </button>
      {/* The crop carries the magnifier a step apart from the arrows, and the palette it opens is
          the same one the nav row and ⌘K reach. */}
      <button
        type="button"
        aria-label={translate(locale, "sidebar.search")}
        title={`${translate(locale, "sidebar.search")} · ${shortcutLabel("palette")}`}
        className="icon-btn ml-[var(--space-2)]"
        onClick={onOpenPalette}
      >
        <Icon name="search" />
      </button>
    </div>
  );
}

export function Sidebar({
  locale,
  fixture,
  onOpenSettings,
  actions,
}: {
  locale: Locale;
  fixture: boolean;
  onOpenSettings: () => void;
  actions: SidebarActions;
}) {
  const items = useSessions((s) => s.items);
  const activeId = useSessions((s) => s.activeId);
  const open = useSessions((s) => s.open);
  const remove = useSessions((s) => s.remove);
  const running = useTranscript((s) => s.running);
  // Which row's trash has been pressed once. Deleting a conversation has no undo and no second copy —
  // the file *is* the record — so the control asks for the press twice. One row at a time can be
  // armed, because an arm that follows the pointer to its neighbour would have the second press
  // delete a conversation nobody promised to delete.
  const [armed, setArmed] = useState<string | null>(null);
  const collapsed = useNav((s) => s.collapsed);
  const toggleSidebar = useNav((s) => s.toggleSidebar);
  const notInBuild = translate(locale, "common.notInBuild");
  const workspaces = fixture
    ? FIXTURE_WORKSPACES
    : group(items, translate(locale, "sidebar.localUser"));

  const row = (r: NavRow) => (
    <Row
      key={r.key}
      icon={r.icon}
      label={translate(locale, `sidebar.${r.key}`)}
      hint={r.placeholder ? notInBuild : undefined}
      shortcut={r.shortcut ? shortcutLabel(r.shortcut) : undefined}
      tip={r.tipKey ? translate(locale, r.tipKey) : undefined}
      onClick={r.onActivate ? () => r.onActivate?.(actions) : undefined}
    />
  );

  const toggleLabel = translate(locale, "sidebar.toggle");

  // Folded, the column is a rail of glyphs: the same controls this build answers, without the words
  // they were labelled with, and the toggle that opens it again where the pointer left it.
  if (collapsed) {
    return (
      <nav
        aria-label={translate(locale, "sidebar.workspaces")}
        className="flex h-full w-[var(--sidebar-rail-width)] shrink-0 flex-col items-center gap-1 border-r border-line bg-panel pt-[var(--traffic-light-clear)] pb-[var(--space-3)]"
      >
        <button
          type="button"
          aria-label={toggleLabel}
          title={toggleLabel}
          className="icon-btn"
          onClick={toggleSidebar}
        >
          {/* The chevron points the way the column is about to go, so the rail's control and the open
              column's are the same window with the arrow turned round. */}
          <Icon name="sidebarExpand" />
        </button>
        {RAIL_ROWS.map((r) => {
          const label = translate(locale, `sidebar.${r.key}`);
          // The label moves into the hover line, since the row that normally carries it is folded away
          // with the column.
          const shortcut = r.shortcut ? shortcutLabel(r.shortcut) : null;
          return (
            <button
              key={r.key}
              type="button"
              aria-label={label}
              title={shortcut ? `${label} · ${shortcut}` : label}
              className="icon-btn"
              onClick={() => r.onActivate?.(actions)}
            >
              <Icon name={r.icon} />
            </button>
          );
        })}
        <span className="flex-1" />
        {/* Folded, the row loses its words but not its face: the mascot is the one glyph in the rail
            that says which app this window belongs to. */}
        <button
          type="button"
          aria-label={translate(locale, "sidebar.localUser")}
          title={translate(locale, "sidebar.knowledgeAbout")}
          className="icon-btn"
          onClick={() => actions.openKnowledge()}
        >
          <MascotMark size={20} />
        </button>
        <button
          type="button"
          aria-label={translate(locale, "sidebar.settings")}
          title={shortcutLabel("settings")}
          className="icon-btn"
          onClick={onOpenSettings}
        >
          <Icon name="settings" />
        </button>
      </nav>
    );
  }

  return (
    <nav
      aria-label={translate(locale, "sidebar.workspaces")}
      className="flex h-full w-[var(--sidebar-width)] shrink-0 flex-col border-r border-line bg-panel"
    >
      {/* The traffic lights own the window's top-left, so the row starts clear of them sideways and
          sits on their centre line: this band is the titlebar's, and the reference draws the toggle
          and the two history arrows on that line rather than stacked under it. */}
      <div
        className="flex h-[calc(var(--traffic-light-center)*2)] items-center gap-1 px-[var(--space-3)]"
        style={{ paddingLeft: "max(var(--space-3), var(--traffic-light-inset))" }}
      >
        {/* The reference's header row: a column that folds, the two steps along the surfaces this
            window has opened, and the search over everything. Each of the arrows says so when the
            stack has nothing on that side of it rather than firing into a no-op. */}
        <button
          type="button"
          aria-label={toggleLabel}
          title={toggleLabel}
          className="icon-btn"
          onClick={toggleSidebar}
        >
          <Icon name="sidebar" />
        </button>
        <NavSteps locale={locale} onOpenPalette={actions.openPalette} />
      </div>

      {/* The band above ends at the lights' own line, so the first row of the column gets the extra
          step: a hover pill that starts where the titlebar band leaves off reads as jammed under the
          window's chrome rather than as its first item. */}
      <div className="flex flex-col gap-1 p-[var(--space-3)] pt-[var(--space-4)]">
        <div className="flex items-center gap-1">
          <span className="pill w-fit">
            <Icon name="coding" />
            {translate(locale, "sidebar.coding")}
          </span>
          {/* The reference hangs a puzzle badge off the mode pill rather than only in the Places
              list: the capabilities a run may use belong beside the mode that grants them. */}
          <button
            type="button"
            aria-label={translate(locale, "sidebar.extensions")}
            title={translate(locale, "sidebar.extensionsAbout")}
            className="icon-btn"
            onClick={() => actions.openExtensions()}
          >
            <Icon name="extensions" size={14} />
          </button>
        </div>
        {PRIMARY.map((r) => row(r))}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-[var(--space-3)] pb-[var(--space-3)]">
        <p className="section-label">{translate(locale, "sidebar.workspaces")}</p>
        {workspaces.length === 0 ? (
          <p className="px-3 py-2 text-quiet text-faint">
            {translate(locale, "sidebar.emptyConversations")}
          </p>
        ) : (
          workspaces.map((ws) => (
            <div key={ws.name}>
              <Row icon="workspace" label={ws.name} />
              {ws.conversations.map((c) => {
                // A local, because the handler below runs after the check and a property's narrowing
                // does not carry into a closure the way a local's does.
                const id = c.id;
                const isArmed = id !== null && armed === id;
                // The host refuses to delete the conversation a reply is streaming into, because the
                // run would file its answer back into the row that has just gone. So the control that
                // would be refused stays on the row that is running and says why, rather than looking
                // like a trash can that does not work.
                const busy = id !== null && id === activeId && running;
                const label = translate(
                  locale,
                  busy ? "sidebar.delete.busy" : isArmed ? "sidebar.deleteAgain" : "sidebar.delete",
                );
                return (
                  <div
                    key={id ?? c.title}
                    className="group relative"
                    // Sliding off the row disarms it. That is the whole of the timeout this control
                    // would otherwise need, and it has no timer to leak when the column folds.
                    onMouseLeave={() => setArmed((a) => (a === id ? null : a))}
                  >
                    <Row
                      label={c.title}
                      indent
                      className={id ? "trailing" : undefined}
                      active={id !== null && id === activeId}
                      tip={
                        c.meta
                          ? `${translate(locale, "sidebar.openConversation")} · ${c.meta}`
                          : undefined
                      }
                      onClick={id ? () => void open(id) : undefined}
                    />
                    {id && (
                      // Only on hover, because a trash glyph standing on every row of the list makes
                      // the sidebar look like a bin rather than a history. The one that has been
                      // pressed once stays lit — that state has to be visible to mean anything.
                      <button
                        type="button"
                        aria-label={label}
                        aria-disabled={busy ? true : undefined}
                        title={label}
                        className={`absolute right-[3px] top-1/2 grid size-5 -translate-y-1/2 place-items-center rounded-[var(--radius-sm)] transition-opacity hover:bg-[var(--bg-hover)] focus-visible:opacity-100 ${
                          busy
                            ? "text-faint opacity-100"
                            : isArmed
                              ? "text-danger opacity-100"
                              : "text-muted opacity-0 group-hover:opacity-100"
                        }`}
                        onClick={(event) => {
                          // The row underneath opens the conversation; this press is not that.
                          event.stopPropagation();
                          if (busy) return;
                          if (isArmed) {
                            setArmed(null);
                            void remove(id);
                          } else {
                            setArmed(id);
                          }
                        }}
                      >
                        <Icon name="trash" size={13} />
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          ))
        )}

        <p className="section-label">{translate(locale, "sidebar.places")}</p>
        {PLACES.map((r) => row(r))}
      </div>

      <div className="flex items-center gap-1 border-t border-line p-[var(--space-2)]">
        {/* The reference's footer is an identity row, and pressing it opens the menu rather than a
            page: the six appearance decisions live one click from wherever the person is, and the two
            rows that leave the menu — the catalog this machine holds, and the full Settings page — are
            both destinations this build has. The face on the row is the mascot the window and the Dock
            wear, which Appearance can pin. */}
        <IdentityMenu
          locale={locale}
          onOpenSettings={onOpenSettings}
          onOpenKnowledge={() => actions.openKnowledge()}
        />
        <button
          type="button"
          aria-label={translate(locale, "sidebar.settings")}
          title={shortcutLabel("settings")}
          className="icon-btn shrink-0"
          onClick={onOpenSettings}
        >
          <Icon name="settings" />
        </button>
      </div>
    </nav>
  );
}
