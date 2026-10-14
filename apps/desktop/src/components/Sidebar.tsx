import { Icon, type IconName } from "./icons";
import { translate, type Locale } from "../i18n";

type NavRow = {
  key: string;
  icon: IconName;
  onActivate?: (actions: SidebarActions) => void;
  /** A row the reference layout draws and this build has nothing behind. It stays on screen — the
   * layout is the contract — but a click on it says so rather than looking like a lost one. */
  placeholder?: true;
};

const PRIMARY: NavRow[] = [
  { key: "newTask", icon: "newTask", onActivate: (a) => a.newTask },
  { key: "search", icon: "search", placeholder: true },
];

const PLACES: NavRow[] = [
  { key: "knowledge", icon: "knowledge", placeholder: true },
  { key: "sites", icon: "sites", placeholder: true },
  { key: "automations", icon: "automations", placeholder: true },
  { key: "extensions", icon: "extensions", placeholder: true },
];

type Workspace = { name: string; tasks: string[] };

/** Only ever rendered under `?fixture=1`; a real workspace list arrives with the session store. */
const FIXTURE_WORKSPACES: Workspace[] = [
  { name: "smollm-studio", tasks: ["Phase 0 host", "Catalog pin"] },
];

function Row({
  icon,
  label,
  active,
  indent,
  onClick,
  hint,
}: {
  icon?: IconName;
  label: string;
  active?: boolean;
  indent?: boolean;
  onClick?: () => void;
  hint?: string;
}) {
  return (
    <button
      type="button"
      data-active={active || undefined}
      // Not `disabled`: a disabled button swallows the hover that carries the sentence explaining
      // why it does nothing, which is the one thing the row still has to say.
      aria-disabled={hint ? true : undefined}
      title={hint}
      className={`nav-item ${indent ? "pl-[34px] text-[calc(var(--font-ui)-1px)]" : ""}`}
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
  const workspaces = fixture ? FIXTURE_WORKSPACES : [];
  const notInBuild = translate(locale, "common.notInBuild");

  const row = (r: NavRow) => (
    <Row
      key={r.key}
      icon={r.icon}
      label={translate(locale, `sidebar.${r.key}`)}
      hint={r.placeholder ? notInBuild : undefined}
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
          <p className="px-3 py-2 text-[calc(var(--font-ui)-1px)] text-faint">
            {translate(locale, "sidebar.emptyWorkspaces")}
          </p>
        ) : (
          workspaces.map((ws) => (
            <div key={ws.name}>
              <Row icon="workspace" label={ws.name} />
              {ws.tasks.map((task) => (
                <Row key={task} label={task} indent />
              ))}
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
        <span className="min-w-0 flex-1 truncate text-[calc(var(--font-ui)-1px)] text-fg">
          {translate(locale, "sidebar.localUser")}
        </span>
        <button
          type="button"
          aria-label={translate(locale, "sidebar.settings")}
          className="icon-btn"
          onClick={onOpenSettings}
        >
          <Icon name="settings" />
        </button>
      </div>
    </nav>
  );
}
