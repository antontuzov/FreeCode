import { translate, type Locale } from "../i18n";

const SECTIONS = [
  "workspaces",
  "recent",
  "knowledge",
  "sites",
  "automations",
  "extensions",
] as const;

/** Phase 0 skeleton: the nav rail exists and is themed; Phase 1 matches the screenshot exactly. */
export function Sidebar({ locale }: { locale: Locale }) {
  return (
    <nav
      aria-label={translate(locale, "sidebar.workspaces")}
      className="flex h-full w-[var(--sidebar-width)] shrink-0 flex-col gap-1 border-r border-line bg-elevated/40 p-2"
    >
      {SECTIONS.map((section) => (
        <button
          key={section}
          type="button"
          className="rounded-[var(--radius-md)] px-3 py-2 text-left text-muted transition-colors hover:bg-hover hover:text-fg"
        >
          {translate(locale, `sidebar.${section}`)}
        </button>
      ))}
      <div className="mt-auto">
        <button
          type="button"
          className="rounded-[var(--radius-md)] px-3 py-2 text-left text-muted hover:bg-hover hover:text-fg"
        >
          {translate(locale, "sidebar.settings")}
        </button>
      </div>
    </nav>
  );
}
