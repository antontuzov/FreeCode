import { Icon, type IconName } from "./icons";
import { translate, type Locale } from "../i18n";

type NavRow = { key: string; icon: IconName };

const PRIMARY: NavRow[] = [
  { key: "newTask", icon: "newTask" },
  { key: "search", icon: "search" },
];

const PLACES: NavRow[] = [
  { key: "knowledge", icon: "knowledge" },
  { key: "sites", icon: "sites" },
  { key: "automations", icon: "automations" },
  { key: "extensions", icon: "extensions" },
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
}: {
  icon?: IconName;
  label: string;
  active?: boolean;
  indent?: boolean;
}) {
  return (
    <button
      type="button"
      data-active={active || undefined}
      className={`nav-item ${indent ? "pl-[34px] text-[calc(var(--font-ui)-1px)]" : ""}`}
    >
      {icon && <Icon name={icon} />}
      <span className="truncate">{label}</span>
    </button>
  );
}

export function Sidebar({
  locale,
  fixture,
  onOpenSettings,
}: {
  locale: Locale;
  fixture: boolean;
  onOpenSettings: () => void;
}) {
  const workspaces = fixture ? FIXTURE_WORKSPACES : [];

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
        <button type="button" aria-label={translate(locale, "sidebar.toggle")} className="icon-btn">
          <Icon name="sidebar" />
        </button>
        <button type="button" aria-label={translate(locale, "nav.back")} className="icon-btn">
          <Icon name="back" />
        </button>
        <button type="button" aria-label={translate(locale, "nav.forward")} className="icon-btn">
          <Icon name="forward" />
        </button>
      </div>

      <div className="flex flex-col gap-1 p-[var(--space-3)]">
        <span className="pill w-fit">
          <Icon name="coding" />
          {translate(locale, "sidebar.coding")}
        </span>
        {PRIMARY.map((row) => (
          <Row key={row.key} icon={row.icon} label={translate(locale, `sidebar.${row.key}`)} />
        ))}
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
        {PLACES.map((row) => (
          <Row key={row.key} icon={row.icon} label={translate(locale, `sidebar.${row.key}`)} />
        ))}
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
