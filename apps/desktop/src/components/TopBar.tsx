import { translate, type Locale } from "../i18n";

export function TopBar({ locale }: { locale: Locale }) {
  return (
    <header className="flex h-11 shrink-0 items-center gap-2 border-b border-line px-[var(--space-4)] pl-[var(--traffic-light-inset)]">
      <span className="text-sm font-medium text-fg">SmoLLM Studio</span>
      <span className="ml-auto flex items-center gap-1">
        <button
          type="button"
          aria-label={translate(locale, "topbar.toggleInspector")}
          className="rounded-[var(--radius-sm)] px-2 py-1 text-muted hover:bg-hover hover:text-fg"
        >
          &#x25A4;
        </button>
        <button
          type="button"
          aria-label={translate(locale, "topbar.toggleBrowser")}
          className="rounded-[var(--radius-sm)] px-2 py-1 text-muted hover:bg-hover hover:text-fg"
        >
          &#x25CB;
        </button>
      </span>
    </header>
  );
}
