// The control vocabulary of the Settings surface, taken control-for-control from
// docs/reference-ui/01-03: a labelled row, a bordered field trigger, an accent switch, shortcut
// chips and the four interface-style preview cards.
import type { ReactNode } from "react";

import { Icon, type IconName } from "./icons";
import type { ShortcutDoc } from "../lib/ipc";

export function Section({
  label,
  description,
  children,
}: {
  label: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="mb-[var(--space-6)]">
      <h2 className="text-[calc(var(--font-ui)-1px)] text-muted">{label}</h2>
      {description && (
        <p className="mt-1 text-[calc(var(--font-ui)-1px)] text-muted">{description}</p>
      )}
      <div className="card mt-[var(--space-3)]">{children}</div>
    </section>
  );
}

export function Row({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <div className="card-row">
      <div className="row-copy">
        <span className="row-title">{title}</span>
        {description && <span className="row-desc">{description}</span>}
      </div>
      {children}
    </div>
  );
}

export function Toggle({
  on,
  label,
  onChange,
}: {
  on: boolean;
  label: string;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      data-on={on || undefined}
      className="toggle"
      onClick={() => onChange(!on)}
    />
  );
}

export function Select<T extends string>({
  value,
  options,
  label,
  icon,
  onChange,
}: {
  value: T;
  options: { value: T; label: string }[];
  label: string;
  icon?: IconName;
  onChange: (next: T) => void;
}) {
  return (
    <label className="field w-[176px]">
      {icon && <Icon name={icon} size={14} />}
      <select
        aria-label={label}
        className="field-input"
        value={value}
        onChange={(e) => onChange(e.target.value as T)}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <Icon name="chevronDown" size={14} />
    </label>
  );
}

/** The `14 px` box in shot 01: the unit sits inside the field, right-aligned with the number. */
export function NumberField({
  value,
  suffix,
  label,
  min,
  max,
  step = 1,
  onChange,
}: {
  value: number;
  suffix?: string;
  label: string;
  min: number;
  max: number;
  step?: number;
  onChange: (next: number) => void;
}) {
  return (
    <label className="field field-narrow">
      <input
        aria-label={label}
        className="field-input text-right"
        type="number"
        value={value}
        min={min}
        max={max}
        step={step}
        onChange={(e) => {
          const raw = Number(e.target.value);
          // An empty box is not a setting; hold the last value until a number arrives.
          if (Number.isFinite(raw)) onChange(Math.min(max, Math.max(min, raw)));
        }}
      />
      {suffix && <span className="text-faint">{suffix}</span>}
    </label>
  );
}

export function TextField({
  value,
  label,
  placeholder,
  mono,
  onChange,
}: {
  value: string;
  label: string;
  placeholder?: string;
  mono?: boolean;
  onChange: (next: string) => void;
}) {
  return (
    <label className="field w-[280px]">
      <input
        aria-label={label}
        className={`field-input ${mono ? "font-mono text-[calc(var(--font-code))]" : ""}`}
        type="text"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  );
}

/** `⌘ ⇧ L` as chips rather than one string, so each modifier reads as a key. */
export function ShortcutChips({ shortcut }: { shortcut: ShortcutDoc }) {
  const chips = [
    shortcut.cmd && "⌘",
    shortcut.ctrl && "⌃",
    shortcut.alt && "⌥",
    shortcut.shift && "⇧",
    shortcut.key.toUpperCase(),
  ].filter((c): c is string => Boolean(c));
  return (
    <span className="flex items-center gap-1">
      {chips.map((c) => (
        <kbd key={c} className="kbd">
          {c}
        </kbd>
      ))}
    </span>
  );
}

/**
 * One of the four interface-style cards. It paints the style it advertises through `--preview`, so
 * all four stay distinguishable whatever accent is live; selection is the outline plus the filled
 * dot, never colour alone.
 */
export function StyleCard({
  style,
  name,
  selected,
  onSelect,
}: {
  style: "forest" | "mint" | "bee" | "parchment";
  name: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      data-selected={selected || undefined}
      className={`style-card swatch-${style}`}
      onClick={onSelect}
    >
      <span className="style-preview" aria-hidden>
        <span className="style-window">
          <span className="style-bar" />
          <span className="style-dot" />
          <span className="style-panel">
            <span className="style-line w-[70%]" />
            <span className="style-line w-[45%]" />
          </span>
        </span>
      </span>
      <span className="style-name">{name}</span>
    </button>
  );
}
