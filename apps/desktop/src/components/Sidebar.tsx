import { Icon, type IconName } from "./icons";
import { translate, type Locale } from "../i18n";
import type { SessionSummary } from "../lib/ipc";
import { shortcutLabel, type ShortcutId } from "../lib/shortcuts";
import { useSessions } from "../state/sessions";

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
  { key: "extensions", icon: "extensions", placeholder: true },
];

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
}) {
  return (
    <button
      type="button"
      data-active={active || undefined}
      // Not `disabled`: a disabled button swallows the hover that carries the sentence explaining
      // why it does nothing, which is the one thing the row still has to say.
      aria-disabled={hint ? true : undefined}
      title={hint ?? shortcut ?? tip}
      className={`nav-item ${indent ? "indent" : ""}`}
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
};

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

  return (
    <nav
      aria-label={translate(locale, "sidebar.workspaces")}
      className="flex h-full w-[var(--sidebar-width)] shrink-0 flex-col border-r border-line bg-panel"
    >
      {/* Traffic lights occupy the top-left of the window, so the row starts below them. */}
      <div
        className="flex items-center gap-3 px-[var(--space-3)] pt-[var(--space-3)] pb-1"
        style={{ paddingLeft: "max(var(--space-3), var(--traffic-light-inset))" }}
      >
        {/* The three arrows above are the reference chrome: one window, one conversation, so there
            is nothing to collapse and nowhere to go back to. They are drawn and named as such. */}
        <button
          type="button"
          aria-label={translate(locale, "sidebar.toggle")}
          aria-disabled
          title={notInBuild}
          className="icon-btn"
        >
          <Icon name="sidebar" />
        </button>
        <button
          type="button"
          aria-label={translate(locale, "nav.back")}
          aria-disabled
          title={notInBuild}
          className="icon-btn"
        >
          <Icon name="back" />
        </button>
        <button
          type="button"
          aria-label={translate(locale, "nav.forward")}
          aria-disabled
          title={notInBuild}
          className="icon-btn"
        >
          <Icon name="forward" />
        </button>
      </div>

      <div className="flex flex-col gap-1 p-[var(--space-3)]">
        <span className="pill w-fit">
          <Icon name="coding" />
          {translate(locale, "sidebar.coding")}
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
                return (
                  <Row
                    key={id ?? c.title}
                    label={c.title}
                    indent
                    active={id !== null && id === activeId}
                    tip={
                      c.meta
                        ? `${translate(locale, "sidebar.openConversation")} · ${c.meta}`
                        : undefined
                    }
                    onClick={id ? () => void open(id) : undefined}
                  />
                );
              })}
            </div>
          ))
        )}

        <p className="section-label">{translate(locale, "sidebar.places")}</p>
        {PLACES.map((r) => row(r))}
      </div>

      <div className="flex items-center gap-2 border-t border-line p-[var(--space-3)]">
        <span
          aria-hidden
          className="grid size-7 shrink-0 place-items-center rounded-full bg-accent-soft text-fg-strong"
        >
          <Icon name="user" size={14} />
        </span>
        <span className="min-w-0 flex-1 truncate text-quiet text-fg">
          {translate(locale, "sidebar.localUser")}
        </span>
        <button
          type="button"
          aria-label={translate(locale, "sidebar.settings")}
          title={shortcutLabel("settings")}
          className="icon-btn"
          onClick={onOpenSettings}
        >
          <Icon name="settings" />
        </button>
      </div>
    </nav>
  );
}
