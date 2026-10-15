import { useState } from "react";

import { translate, type Locale } from "../i18n";
import { Icon, type IconName } from "./icons";

type Group = { key: string; icon: IconName };

// The five groups the reference inspector shows, in order.
const GROUPS: Group[] = [
  { key: "subagents", icon: "subagents" },
  { key: "skills", icon: "skills" },
  { key: "artifact", icon: "artifact" },
  { key: "webReading", icon: "webReading" },
  { key: "source", icon: "source" },
];

type Item = { label: string; meta?: string; icon?: IconName };

/** `?fixture=1` only — see the note in state/transcript.ts about invented run output. */
const FIXTURE_ITEMS: Record<string, Item[]> = {
  subagents: [{ label: "Explore · llama.cpp", meta: "stopped" }, { label: "Audit engine claims" }],
  skills: [
    { label: "verify-smollm-studio-natively" },
    { label: "tauri-desktop-native-verification" },
  ],
  artifact: [{ label: "README.md", icon: "artifact" }],
  webReading: [{ label: "huggingface.co/PrismML-Eng", icon: "webReading" }],
  source: [{ label: "catalog/models.toml", icon: "artifact" }],
};

function Section({ group, items, locale }: { group: Group; items: Item[]; locale: Locale }) {
  const [open, setOpen] = useState(true);
  const label = translate(locale, `inspector.${group.key}`);

  return (
    <section className="border-b border-line/60">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 px-[var(--space-4)] py-[var(--space-3)] text-muted hover:bg-hover hover:text-fg"
      >
        <Icon
          name="chevronDown"
          size={13}
          className={`disclosure-icon ${open ? "" : "-rotate-90"}`}
        />
        <span className="text-quiet">{label}</span>
      </button>
      {open && (
        <ul className="disclosure-panel px-[var(--space-4)] pb-[var(--space-3)]">
          {items.length === 0 ? (
            <li className="py-1 text-fine text-faint">
              {translate(locale, "inspector.empty", { group: label })}
            </li>
          ) : (
            items.map((item) => (
              <li
                key={item.label}
                className="flex items-center gap-2 border-b border-dashed border-line/60 py-2 text-quiet text-fg last:border-0"
              >
                {item.icon ? <Icon name={item.icon} size={14} /> : null}
                <span className="min-w-0 flex-1 truncate">{item.label}</span>
                {item.meta && <span className="shrink-0 text-micro text-faint">{item.meta}</span>}
              </li>
            ))
          )}
        </ul>
      )}
    </section>
  );
}

export function Inspector({ locale, fixture }: { locale: Locale; fixture: boolean }) {
  const items = fixture ? FIXTURE_ITEMS : {};

  return (
    <aside
      aria-label={translate(locale, "inspector.title")}
      className="flex h-full w-[var(--inspector-width)] shrink-0 flex-col overflow-y-auto border-l border-line bg-bg"
    >
      {GROUPS.map((group) => (
        <Section key={group.key} group={group} items={items[group.key] ?? []} locale={locale} />
      ))}
    </aside>
  );
}
