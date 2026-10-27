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
  bare,
  children,
}: {
  label: string;
  description?: string;
  /** A section whose content is its own surface — a row of circles needs no card around it, and the
   * card's hairline would only draw a box the picker does not have. */
  bare?: boolean;
  children: ReactNode;
}) {
  return (
    <section className="settings-section mb-[var(--space-6)]">
      <h2 className="settings-section-title text-quiet uppercase tracking-[0.08em] text-muted">
        {label}
      </h2>
      {description && (
        <p className="settings-section-description mt-1 text-quiet text-muted">{description}</p>
      )}
      <div className={bare ? "mt-[var(--space-3)]" : "card mt-[var(--space-3)]"}>{children}</div>
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
  disabled,
  onChange,
}: {
  on: boolean;
  label: string;
  /** A row with no field to save yet draws the switch in the state the reference shows and says
   * "not in this build" beside its title; a live switch that writes nowhere would be a lie. */
  disabled?: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      aria-disabled={disabled || undefined}
      data-on={on || undefined}
      disabled={disabled}
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
  disabled,
  onChange,
}: {
  value: T;
  options: { value: T; label: string }[];
  label: string;
  icon?: IconName;
  disabled?: boolean;
  onChange: (next: T) => void;
}) {
  return (
    <label className="field w-[176px]" data-disabled={disabled || undefined}>
      {icon && <Icon name={icon} size={14} />}
      <select
        aria-label={label}
        className="field-input"
        disabled={disabled}
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
 * One theme tile: the three colours the theme is made of, then its name. The dots read the palette
 * the same way the reference's tiles do — action colour, wash, neutral ground — and selection is the
 * tinted card plus the accent outline rather than the dots alone.
 */
export function ThemeTile({
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
      className={`theme-tile swatch-${style}`}
      onClick={onSelect}
    >
      <span className="theme-dots" aria-hidden>
        <span className="theme-dot" />
        <span className="theme-dot" />
        <span className="theme-dot theme-dot-neutral" />
      </span>
      <span className="theme-name">{name}</span>
    </button>
  );
}

/** One accent swatch. The chosen one is marked by its tick alone: the picker is a straight row of
 * plain circles, so a ring around one of them would outline a shape the row does not otherwise have. */
export function AccentSwatch({
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
      aria-label={name}
      aria-pressed={selected}
      data-selected={selected || undefined}
      title={name}
      className={`accent-swatch swatch-${style}`}
      onClick={onSelect}
    >
      {selected && <Icon name="tick" size={16} />}
    </button>
  );
}

/** Two or three exclusive answers in one field. A group of buttons rather than a radio list, because
 * every option is short enough to sit side by side and the choice is immediate. */
export function Segmented<T extends string>({
  value,
  options,
  label,
  disabled,
  onChange,
}: {
  value: T;
  options: { value: T; label: string }[];
  label: string;
  disabled?: boolean;
  onChange: (next: T) => void;
}) {
  return (
    <div
      role="group"
      aria-label={label}
      aria-disabled={disabled || undefined}
      data-disabled={disabled || undefined}
      className="segmented"
    >
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          aria-pressed={value === o.value}
          data-active={value === o.value || undefined}
          disabled={disabled}
          className="segment"
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** One step on the text-size scale. The name is the tier; the figure is the size the window will
 * actually paint, which is the number the setting stores. */
export function SizeCard({
  name,
  figure,
  selected,
  onSelect,
}: {
  name: string;
  figure: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      data-selected={selected || undefined}
      className="preset-card"
      onClick={onSelect}
    >
      <span className="preset-name">{name}</span>
      <span className="preset-figure">{figure}</span>
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
