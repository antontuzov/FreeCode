import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";

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
  // One row, and the one that has a page behind it: this app installs no add-ons, but it does have a
  // set of capabilities the agent is handed, and the page the row opens is that list — with what this
  // build runs of it. The catalog and the automations surfaces still exist behind their own deep links,
  // but they are not rows here: a footer of five destinations was the reference's shape, not this
  // build's.
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
  /** Whether the row is held at the head of the list, which the row marks with a tack. */
  pinned: boolean;
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
    const conversation: Conversation = {
      id: item.id,
      title: item.title,
      meta: item.model,
      pinned: item.pinned ?? false,
    };
    if (found) found.conversations.push(conversation);
    else byName.set(name, { name, conversations: [conversation] });
  }
  return [...byName.values()];
}

/** Only ever rendered under `?fixture=1`, to compare the layout against docs/reference-ui. */
const FIXTURE_WORKSPACES: Workspace[] = [
  {
    name: "freecode",
    conversations: [
      { id: null, title: "Phase 0 host", pinned: false },
      { id: null, title: "Catalog pin", pinned: false },
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
  gutter,
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
  /**
   * A conversation row's tack slot: `true` holds the row at the head of the list and draws the tack,
   * `false` reserves the same air empty. The slot is on every conversation row rather than only the
   * pinned ones, because a list whose labels start at two different x is read as two lists.
   */
  gutter?: boolean;
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
      {gutter !== undefined && (
        <span aria-hidden className="grid w-4 shrink-0 place-items-center">
          {gutter && <Icon name="pin" size={11} className="text-faint" />}
        </span>
      )}
      {icon && <Icon name={icon} />}
      <span className="truncate">{label}</span>
    </button>
  );
}

/** One press in a conversation row's menu. */
type Press = {
  id: string;
  icon: IconName;
  label: string;
  /** The note at the press's right end, where an outcome is worth printing rather than assuming: the
   * copy press leaves "Copied" there, the pinned one says where the row now sits. */
  value?: string;
  /** The arrow that promises a column this press does not open — a group picker the build has no. */
  chevron?: boolean;
  /** The one press that costs something, drawn in the colour the window already uses for that. */
  danger?: boolean;
  /** Why this press does not run, said on hover. A press that refuses runs nothing on click. */
  refuse?: string;
  run?: () => void;
};

/** The gap the panel keeps from the row it belongs to, and the same air at the viewport's edge. */
const MENU_GAP = 4;

/**
 * `?rowmenu=<conversation id>` opens that row's menu the first time the list shows it.
 *
 * Dev-only, and for the same reason the footer's `?menu=` and the picker's `?picker` exist: this panel
 * is behind a press on a control the hover brings out, and the window takes no injected clicks, so
 * without a seed its pixels could never be compared against the reference. Nothing in the shipped UI
 * sets it.
 */
const SEEDED_ROW = new URLSearchParams(window.location.search).get("rowmenu");

/**
 * A conversation row's menu, floating where the row's `⋯` was pressed.
 *
 * It is placed from the row's own rect rather than anchored inside the column because the list it sits
 * on scrolls, and a panel drawn inside a scroll container is clipped by it — the rows at the bottom
 * would get a menu cut in half. So it is fixed to the viewport, aligned to the row's right edge like
 * the reference's, and turned to open upward when the space below the row cannot hold it.
 */
function TaskMenu({
  anchor,
  presses,
  onClose,
}: {
  anchor: { top: number; bottom: number; right: number };
  presses: Press[];
  onClose: () => void;
}) {
  const panel = useRef<HTMLDivElement>(null);
  // Measured after the first paint rather than estimated from the row count: the panel's height is the
  // sum of a font, a padding and a separator, and a constant for it would be wrong the moment the type
  // scale moved. One extra layout on the press that opens the menu is the whole of the cost.
  const [top, setTop] = useState(anchor.bottom + MENU_GAP);
  useLayoutEffect(() => {
    const height = panel.current?.offsetHeight ?? 0;
    const below = anchor.bottom + MENU_GAP;
    setTop(
      below + height > window.innerHeight
        ? Math.max(MENU_GAP, anchor.top - MENU_GAP - height)
        : below,
    );
  }, [anchor]);

  return (
    <>
      {/* Transparent, as the footer menu's scrim is: the panel repaints nothing behind it, and a mask
          over the window would hide the row the menu is about. */}
      <span className="picker-scrim" onClick={onClose} />
      <div
        ref={panel}
        role="menu"
        className="menu-panel menu-panel--row"
        style={{ top, right: anchor.right }}
        onKeyDown={(e) => {
          if (e.key !== "Escape") return;
          e.preventDefault();
          e.stopPropagation();
          onClose();
        }}
      >
        <div className="menu-main">
          {presses.map((p) => (
            <Fragment key={p.id}>
              {/* The line belongs to the press that ends a conversation, not to the row above it: the
                  reference separates the keeping from the losing, and only that. */}
              {p.danger && <div className="menu-sep" />}
              <button
                type="button"
                role="menuitem"
                aria-disabled={p.refuse ? true : undefined}
                title={p.refuse ?? p.label}
                className={`menu-row ${p.danger ? "menu-row--danger" : ""} ${
                  p.refuse && !p.danger ? "menu-row--inert" : ""
                }`}
                onClick={() => {
                  if (p.refuse) return;
                  p.run?.();
                }}
              >
                <Icon name={p.icon} size={14} className="shrink-0" />
                <span className="menu-label truncate">{p.label}</span>
                {p.value && <span className="menu-value">{p.value}</span>}
                {p.chevron && (
                  <Icon name="chevronRight" size={12} className="shrink-0 text-faint" />
                )}
              </button>
            </Fragment>
          ))}
        </div>
      </div>
    </>
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
    <div className="flex items-center gap-0.5">
      <button
        type="button"
        aria-label={translate(locale, "nav.back")}
        aria-disabled={canBack ? undefined : true}
        title={canBack ? translate(locale, "nav.back") : translate(locale, "nav.noHistoryBehind")}
        className="icon-btn"
        onClick={back}
      >
        <Icon name="back" size={14} />
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
        <Icon name="forward" size={14} />
      </button>
      {/* The crop carries the magnifier a step apart from the arrows, and the palette it opens is
          the same one the nav row and ⌘K reach. */}
      <button
        type="button"
        aria-label={translate(locale, "sidebar.search")}
        title={`${translate(locale, "sidebar.search")} · ${shortcutLabel("palette")}`}
        className="icon-btn ml-[var(--space-1)]"
        onClick={onOpenPalette}
      >
        <Icon name="search" size={14} />
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
  const rename = useSessions((s) => s.rename);
  const togglePinned = useSessions((s) => s.togglePinned);
  const exportTo = useSessions((s) => s.exportTo);
  const running = useTranscript((s) => s.running);
  /* Which row's delete has been pressed once. Deleting a conversation has no undo in this window —
   * the file goes to the Trash, but the row and everything the transcript was showing leave the
   * screen — so the press asks for itself twice. One row at a time can be armed, because an arm that
   * follows the pointer to its neighbour would have the second press delete a conversation nobody
   * promised to delete. It lives in the menu now rather than on the row, which is where the sentence
   * "press again" has room to be read. */
  const [armed, setArmed] = useState<string | null>(null);
  // The row whose `⋯` has been pressed, and the rect that press was made in. The rect is stored rather
  // than read on every render so the panel stays where it was opened even as the list reflows under it.
  const [menu, setMenu] = useState<{
    id: string;
    top: number;
    bottom: number;
    right: number;
  } | null>(null);
  // The copy press keeps its menu open to say whether it worked, which is the only reason this exists.
  const [copied, setCopied] = useState<boolean | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const collapsed = useNav((s) => s.collapsed);
  const toggleSidebar = useNav((s) => s.toggleSidebar);
  const notInBuild = translate(locale, "common.notInBuild");
  const t = (key: string) => translate(locale, key);
  const workspaces = fixture
    ? FIXTURE_WORKSPACES
    : group(items, translate(locale, "sidebar.localUser"));

  const closeMenu = () => {
    setMenu(null);
    setArmed(null);
    setCopied(null);
  };

  // The seed runs once, and on the render after the list arrives: the conversations come back from the
  // host, so the first pass has no row to find. Running once is also what lets the seeded panel be
  // closed — a seed that re-fired whenever the menu went null would reopen it under the pointer.
  const seeded = useRef(false);
  useEffect(() => {
    if (!SEEDED_ROW || seeded.current) return;
    const found = [...document.querySelectorAll<HTMLElement>("[data-task-menu]")].find(
      (el) => el.dataset.taskMenu === SEEDED_ROW,
    );
    if (!found) return;
    seeded.current = true;
    const rect = found.getBoundingClientRect();
    setMenu({
      id: SEEDED_ROW,
      top: rect.top,
      bottom: rect.bottom,
      right: window.innerWidth - rect.right + 3,
    });
  }, [items]);

  const startRename = (id: string, from: string) => {
    closeMenu();
    setRenaming(id);
    setDraft(from);
  };

  /** Keep what was typed. An empty field is not a name — the host refuses one for the same reason the
   * row would otherwise disappear into a blank label — so a cleared field cancels rather than files a
   * conversation with nothing on it. */
  const commitRename = (id: string) => {
    if (renaming !== id) return;
    setRenaming(null);
    const title = draft.trim();
    if (title) void rename(id, title);
  };

  const copyId = (id: string) => {
    // No plugin: this is the web standard, and the press that needs it is a click, which is the focus
    // the clipboard asks for. A rejection is printed in the row that was pressed rather than swallowed,
    // because a copy that silently did nothing is indistinguishable from one that worked.
    navigator.clipboard
      ?.writeText(id)
      .then(() => setCopied(true))
      .catch(() => setCopied(false));
  };

  const pressDelete = (id: string) => {
    if (armed !== id) {
      setArmed(id);
      return;
    }
    closeMenu();
    void remove(id);
  };

  /** The presses the reference's menu draws, and what this build runs behind each of them.
   *
   * Four of the nine are the reference's own rows with nothing behind them here, and they stay on
   * screen saying so: the window is one conversation deep (the host holds a single history), the
   * groups are derived from the folder a run worked in rather than sorted by hand, and nothing arrives
   * for a row to be unread. Their presses are the layout's contract; their refusal is the honest part.
   */
  const presses = (c: Conversation): Press[] => {
    const busy = c.id !== null && c.id === activeId && running;
    return [
      { id: "window", icon: "sites", label: t("task.openWindow"), refuse: notInBuild },
      {
        id: "copy",
        icon: "copy",
        label: t("task.copyId"),
        value: copied === null ? undefined : t(copied ? "task.copied" : "task.copyFailed"),
        run: () => copyId(c.id ?? ""),
      },
      {
        id: "rename",
        icon: "editFile",
        label: t("task.rename"),
        run: () => startRename(c.id ?? "", c.title),
      },
      {
        id: "export",
        icon: "download",
        label: t("task.export"),
        run: () => {
          const id = c.id;
          closeMenu();
          // The host asks where to put the document, and a cancelled chooser answers with nothing.
          if (id) void exportTo(id);
        },
      },
      {
        id: "pin",
        icon: "pin",
        label: t(c.pinned ? "task.unpin" : "task.pin"),
        value: c.pinned ? t("task.pinned") : undefined,
        run: () => {
          const id = c.id;
          closeMenu();
          if (id) void togglePinned(id);
        },
      },
      { id: "group", icon: "workspace", label: t("task.group"), chevron: true, refuse: notInBuild },
      { id: "unread", icon: "mail", label: t("task.unread"), refuse: notInBuild },
      { id: "side", icon: "fork", label: t("task.sideTask"), refuse: notInBuild },
      {
        id: "delete",
        icon: "trash",
        label: t(
          busy ? "sidebar.delete.busy" : armed === c.id ? "sidebar.deleteAgain" : "sidebar.delete",
        ),
        danger: true,
        refuse: busy ? t("sidebar.delete.busy") : undefined,
        run: () => pressDelete(c.id ?? ""),
      },
    ];
  };

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
        className="app-sidebar flex h-full w-[var(--sidebar-rail-width)] shrink-0 flex-col items-center gap-1 border-r border-line bg-panel pt-[var(--traffic-light-clear)] pb-[var(--space-3)]"
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
      className="app-sidebar flex h-full w-[var(--sidebar-width)] shrink-0 flex-col border-r border-line bg-panel"
    >
      {/* The traffic lights own the window's top-left, so the row starts clear of them sideways and
          sits on their centre line: this band is the titlebar's, and the reference draws the toggle
          and the two history arrows on that line rather than stacked under it. */}
      <div
        className="flex h-[calc(var(--traffic-light-center)*2)] items-center gap-0.5 px-[var(--space-3)]"
        style={{ paddingLeft: "max(var(--space-3), var(--traffic-light-inset))" }}
      >
        {/* The reference's header row: a column that folds, the two steps along the surfaces this
            window has opened, and the search over everything. Each of the arrows says so when the
            stack has nothing on that side of it rather than firing into a no-op. The glyphs are
            drawn at 14px in the 28px hit area and the row is set on a 2px gap, because the
            reference's own controls sit on a 30px pitch — a 28px square and a 2px step — and the
            looser gap that used to hold them apart read as four separate controls rather than one
            row. */}
        <button
          type="button"
          aria-label={toggleLabel}
          title={toggleLabel}
          className="icon-btn"
          onClick={toggleSidebar}
        >
          <Icon name="sidebar" size={14} />
        </button>
        <NavSteps locale={locale} onOpenPalette={actions.openPalette} />
      </div>

      {/* The band above ends at the lights' own line, so the first row of the column gets the extra
          step: a hover pill that starts where the titlebar band leaves off reads as jammed under the
          window's chrome rather than as its first item. */}
      <div className="flex flex-col gap-1 p-[var(--space-3)] pt-[var(--space-4)]">
        <span className="pill w-fit">
          <Icon name="coding" />
          {translate(locale, "sidebar.agent")}
        </span>
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
                const isOpen = id !== null && menu?.id === id;
                return (
                  <div key={id ?? c.title} className="group relative">
                    {renaming === id ? (
                      // The name being changed stays on the row it belongs to rather than moving into a
                      // dialog: the field is the row, and the row is what the rest of the list is
                      // comparing it against.
                      <div className="nav-item indent">
                        <span aria-hidden className="w-4 shrink-0" />
                        <input
                          autoFocus
                          className="field-input rename-field"
                          aria-label={t("task.rename")}
                          value={draft}
                          onChange={(e) => setDraft(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              commitRename(id ?? "");
                            } else if (e.key === "Escape") {
                              e.preventDefault();
                              // Escape abandons the text; blur below keeps it, which is the difference
                              // between the two ways out of a field.
                              setRenaming(null);
                            }
                          }}
                          onBlur={() => commitRename(id ?? "")}
                        />
                      </div>
                    ) : (
                      <Row
                        label={c.title}
                        indent
                        gutter={c.pinned}
                        className={id ? "trailing" : undefined}
                        active={id !== null && id === activeId}
                        tip={
                          c.meta
                            ? `${translate(locale, "sidebar.openConversation")} · ${c.meta}`
                            : undefined
                        }
                        onClick={id ? () => void open(id) : undefined}
                      />
                    )}
                    {id && renaming !== id && (
                      // Only on hover, because a control standing on every row of the list makes the
                      // sidebar look like a toolbar rather than a history. The one whose menu is open
                      // stays lit: a press that vanishes with the pointer off it looks like a press
                      // that did not land.
                      <button
                        type="button"
                        aria-label={t("task.menu")}
                        aria-expanded={isOpen}
                        aria-haspopup="menu"
                        title={t("task.menu")}
                        data-task-menu={id}
                        className={`absolute right-[3px] top-1/2 grid size-5 -translate-y-1/2 place-items-center rounded-[var(--radius-sm)] transition-opacity hover:bg-[var(--bg-hover)] focus-visible:opacity-100 ${
                          isOpen
                            ? "text-fg opacity-100"
                            : "text-muted opacity-0 group-hover:opacity-100"
                        }`}
                        onClick={(event) => {
                          // The row underneath opens the conversation; this press is not that.
                          event.stopPropagation();
                          if (isOpen) {
                            closeMenu();
                            return;
                          }
                          const rect = event.currentTarget.getBoundingClientRect();
                          setMenu({
                            id,
                            top: rect.top,
                            bottom: rect.bottom,
                            right: window.innerWidth - rect.right + 3,
                          });
                          setCopied(null);
                          setArmed(null);
                        }}
                      >
                        <Icon name="more" size={14} />
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

      {menu &&
        (() => {
          // The row is looked up rather than carried in the menu state, so a rename or a pin that
          // re-lists the conversations cannot leave the open panel describing the row as it was before
          // the press. A menu whose row has gone — deleted from the Trash page while the panel sat
          // open — draws nothing rather than a panel of presses with nothing to press on.
          const conversation = workspaces
            .flatMap((ws) => ws.conversations)
            .find((c) => c.id === menu.id);
          return conversation ? (
            <TaskMenu anchor={menu} presses={presses(conversation)} onClose={closeMenu} />
          ) : null;
        })()}
    </nav>
  );
}
