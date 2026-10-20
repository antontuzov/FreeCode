// The control vocabulary of the Settings surface, taken control-for-control from
// docs/reference-ui/01-03: a labelled row, a bordered field trigger, an accent switch, shortcut
// chips and the interface-style preview cards.
import type { ReactNode } from "react";

import { Icon, type IconName } from "./icons";
import { type Accent, type ShortcutDoc } from "../lib/ipc";
import { shortcutChips } from "../lib/shortcuts";

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
      <h2 className="text-quiet text-muted">{label}</h2>
      {description && <p className="mt-1 text-quiet text-muted">{description}</p>}
      <div className="card mt-[var(--space-3)]">{children}</div>
    </section>
  );
}

export function Row({
  title,
  description,
  inert,
  children,
}: {
  title: string;
  description?: string;
  /** Set when this row writes a choice nothing in the window reads yet. The control stays live — the
   * document it saves is the same one a later build will render — but the row does not get to imply
   * an effect it does not have. */
  inert?: string;
  /** Absent on a row whose only answer is its description — a directory has no trailing figure to
   * draw, and emptying the slot for it is what keeps the path from breaking into an orphaned letter. */
  children?: ReactNode;
}) {
  return (
    <div className="card-row">
      <div className="row-copy">
        <span className="row-title">
          {title}
          {inert && <span className="row-inert">{inert}</span>}
        </span>
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
        className={`field-input ${mono ? "field-mono" : ""}`}
        type="text"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  );
}

/**
 * A secret being pasted. The browser never offers it back and the DOM keeps it as a password until
 * the caller clears it, which is what a key typed once should look like; the field's own value is
 * never written into the settings document — it goes to the Keychain and the box is emptied.
 */
export function SecretField({
  value,
  label,
  placeholder,
  onChange,
}: {
  value: string;
  label: string;
  placeholder?: string;
  onChange: (next: string) => void;
}) {
  return (
    <label className="field w-[280px]">
      <input
        aria-label={label}
        autoComplete="off"
        className="field-input field-mono"
        placeholder={placeholder}
        spellCheck={false}
        type="password"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  );
}

/** `⌘ ⇧ L` as chips rather than one string, so each modifier reads as a key. */
export function ShortcutChips({
  shortcut,
  hint,
}: {
  shortcut: ShortcutDoc;
  /** Set when no key handler in this build answers the combination. The chips stay — the binding is
   * a stored preference the user may have set deliberately — but they stop reading as a promise, and
   * they do it by dropping one step on the ink ladder rather than by fading: an opacity here would
   * put the smallest text in the app back under the contrast floor. */
  hint?: string;
}) {
  const chips = shortcutChips(shortcut);
  return (
    <span className="flex items-center gap-1" title={hint} aria-disabled={hint ? true : undefined}>
      {chips.map((c) => (
        <kbd key={c} className={hint ? "kbd kbd-inert" : "kbd"}>
          {c}
        </kbd>
      ))}
    </span>
  );
}

/**
 * One of the interface-style cards. It paints the style it advertises through `--preview`, so every
 * family stays distinguishable whatever accent is live; selection is the outline plus the filled dot,
 * never colour alone.
 */
export function StyleCard({
  style,
  name,
  selected,
  onSelect,
}: {
  style: Accent;
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

/**
 * One mascot tile the app can wear. The picture is the whole control — a tile has no name, so the
 * accessible label gives its number, which is also how the settings document and `?mascot=` say it.
 */
export function IconCard({
  source,
  label,
  selected,
  onSelect,
}: {
  source: string;
  label: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      aria-label={label}
      data-selected={selected || undefined}
      className="icon-card"
      onClick={onSelect}
    >
      {/* The label is on the button, so the picture is decoration here rather than the word again. */}
      <img alt="" aria-hidden src={source} width={44} height={44} className="rounded-full" />
    </button>
  );
}
