// The footer's identity row and the menu it opens: the reference puts the six appearance decisions a
// person changes on purpose — language, style, light or dark, typeface, text size, reading width — one
// click from wherever they are, rather than only behind the Settings page.
//
// Every row here writes the field the Appearance page writes, through the store the window is painted
// from, so the two surfaces cannot disagree about what the window is wearing, and the choice lands the
// moment it is made rather than on the next Settings reload.
//
// What the window does not act on is not offered. The reference's menu also carries "UI scale", and this
// build has no zoom control to point at, so that row is absent rather than a control that pretends —
// the same rule the Appearance page applies to the five rows it marks as not in this build.
import { useState } from "react";

import { MascotMark } from "./BrandMark";
import { Icon } from "./icons";
import { translate, type Locale } from "../i18n";
import {
  INTERFACE_STYLES,
  type Accent,
  type ContentWidth,
  type FontStyle,
  type Language,
  type ThemeMode,
} from "../lib/ipc";
import { shortcutLabel } from "../lib/shortcuts";
import { useAppearance } from "../state/appearance";

/** `?menu=<group>` stands in for the click this row needs and the native window cannot be given; bare
 * `?menu=1` opens the first group. */
const SEEDED = new URLSearchParams(window.location.search).get("menu");

/** The three steps the reference names. The Appearance page keeps its exact-pixel field for anyone who
 * wants a size between them, so a tick here appears only when the stored size *is* one of these three
 * and the row above prints the pixel figure when it is not. */
const SIZES = [
  { px: 13, key: "small" },
  { px: 14, key: "medium" },
  { px: 16, key: "large" },
] as const;

type Option = {
  value: string;
  label: string;
  /** The style's own action colour, for the Theme group. */ dot?: string;
};

type Group = {
  /** The `?menu=` handle. */
  id: string;
  label: string;
  /** The stored value as a string, which is what an option is compared against to know it is live. */
  current: string;
  /** The name printed at the parent row's right end. */
  value: string;
  options: Option[];
  choose: (v: string) => void;
};

export function IdentityMenu({
  locale,
  onOpenSettings,
  onOpenKnowledge,
}: {
  locale: Locale;
  onOpenSettings: () => void;
  onOpenKnowledge: () => void;
}) {
  const a = useAppearance();
  const t = (key: string) => translate(locale, key);
  const opt = (v: string) => t(`settings.opt.${v}`);
  const [open, setOpen] = useState(SEEDED !== null);
  const [group, setGroup] = useState<string>(SEEDED && SEEDED !== "1" ? SEEDED : "language");

  const sizeOptions = SIZES.map((s) => ({ value: String(s.px), label: opt(s.key) }));
  const groups: Group[] = [
    {
      id: "language",
      label: t("settings.row.language"),
      current: a.language,
      value: opt(a.language),
      options: (["en", "ru"] as Language[]).map((v) => ({ value: v, label: opt(v) })),
      choose: (v) => void a.set({ language: v as Language }),
    },
    {
      id: "style",
      label: t("settings.section.interfaceStyle"),
      current: a.accent,
      value: INTERFACE_STYLES.find((s) => s.id === a.accent)?.label ?? a.accent,
      options: INTERFACE_STYLES.map((s) => ({
        value: s.id,
        label: s.label,
        dot: `var(--style-${s.id})`,
      })),
      choose: (v) => void a.set({ accent: v as Accent }),
    },
    {
      id: "mode",
      label: t("settings.row.themeMode"),
      current: a.themeMode,
      value: opt(a.themeMode),
      options: (["light", "dark", "system"] as ThemeMode[]).map((v) => ({
        value: v,
        label: opt(v),
      })),
      choose: (v) => void a.set({ themeMode: v as ThemeMode }),
    },
    {
      id: "font",
      label: t("settings.row.fontStyle"),
      current: a.fontStyle,
      value: opt(a.fontStyle),
      options: (["sans", "serif", "mono"] as FontStyle[]).map((v) => ({
        value: v,
        label: opt(v),
      })),
      choose: (v) => void a.set({ fontStyle: v as FontStyle }),
    },
    {
      id: "size",
      label: t("settings.row.uiFontSize"),
      current: String(a.uiFontSize),
      value:
        sizeOptions.find((o) => o.value === String(a.uiFontSize))?.label ?? `${a.uiFontSize}px`,
      options: sizeOptions,
      choose: (v) => void a.set({ uiFontSize: Number(v) }),
    },
    {
      id: "width",
      label: t("settings.row.contentWidth"),
      current: a.contentWidth,
      value: opt(a.contentWidth),
      options: (["standard", "wide", "full"] as ContentWidth[]).map((v) => ({
        value: v,
        label: opt(v),
      })),
      choose: (v) => void a.set({ contentWidth: v as ContentWidth }),
    },
  ];
  /** Which group the submenu shows. A `?menu=` handle this list does not hold — or one that has since
   * been renamed — opens no submenu rather than guessing at one, because a tick next to the wrong
   * option is worse than a column that is simply not there. */
  const live = groups.find((g) => g.id === group);

  const pick = (g: Group, o: Option) => {
    g.choose(o.value);
    setOpen(false);
  };

  /** The menu is a column of groups beside a column of the live group's options, so the row you are on
   * stays visible while you choose inside it — which is what the reference draws, and what a submenu
   * that replaced the list would not be. */
  const panel = (
    <div
      role="menu"
      aria-label={t("sidebar.menu")}
      className="menu-panel"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          setOpen(false);
        } else if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
          // The two arrows walk the group list, which is how a menu you cannot hover is used.
          const step = e.key === "ArrowRight" ? 1 : groups.length - 1;
          const i = groups.findIndex((g) => g.id === live?.id);
          const next = groups[((i < 0 ? 0 : i) + step) % groups.length];
          if (next) setGroup(next.id);
          e.preventDefault();
        }
      }}
    >
      <div className="menu-main">
        <p className="section-label">{t("sidebar.localUser")}</p>
        {groups.map((g) => (
          <button
            key={g.id}
            type="button"
            role="menuitem"
            aria-haspopup="menu"
            aria-expanded={live?.id === g.id}
            data-open={live?.id === g.id || undefined}
            className="menu-row"
            onMouseEnter={() => setGroup(g.id)}
            onFocus={() => setGroup(g.id)}
            onClick={() => setGroup(g.id)}
          >
            <span className="menu-label truncate">{g.label}</span>
            <span className="menu-value truncate">{g.value}</span>
            <Icon name="chevronRight" size={12} className="shrink-0 text-faint" />
          </button>
        ))}
        <div className="menu-sep" />
        {/* The two rows that leave the menu: both are pages this build has, which is why the
            reference's "Changelog" and "Get the mobile app" have no counterparts here. */}
        <button
          type="button"
          role="menuitem"
          className="menu-row"
          onClick={() => {
            setOpen(false);
            onOpenKnowledge();
          }}
        >
          <Icon name="knowledge" size={14} className="shrink-0 text-muted" />
          <span className="menu-label truncate">{t("sidebar.knowledge")}</span>
        </button>
        <button
          type="button"
          role="menuitem"
          className="menu-row"
          onClick={() => {
            setOpen(false);
            onOpenSettings();
          }}
        >
          <Icon name="sliders" size={14} className="shrink-0 text-muted" />
          <span className="menu-label truncate">{t("sidebar.settings")}</span>
          <span className="menu-value">{shortcutLabel("settings")}</span>
        </button>
      </div>

      {live && (
        <div className="menu-sub" role="group" aria-label={live.label}>
          {live.options.map((o) => {
            const current = o.value === live.current;
            return (
              <button
                key={o.value}
                type="button"
                role="menuitemradio"
                aria-checked={current}
                className="menu-row"
                onClick={() => pick(live, o)}
              >
                {/* The tick slot is always there, so every label starts at the same x whether or not the
                  option is the live one. */}
                <Icon
                  name="tick"
                  size={12}
                  className={current ? "shrink-0 text-fg-strong" : "shrink-0 opacity-0"}
                />
                {o.dot !== undefined && (
                  <span aria-hidden className="menu-dot shrink-0" style={{ background: o.dot }} />
                )}
                <span className="menu-label truncate">{o.label}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );

  return (
    <span className="relative flex min-w-0 flex-1">
      <button
        type="button"
        className="nav-item min-w-0 flex-1"
        aria-label={t("sidebar.menu")}
        aria-expanded={open}
        aria-haspopup="menu"
        title={t("sidebar.menu")}
        onClick={() => setOpen((was) => !was)}
      >
        <span aria-hidden className="grid size-7 shrink-0 place-items-center">
          <MascotMark size={26} />
        </span>
        <span className="truncate">{t("sidebar.localUser")}</span>
        <Icon name="chevronDown" size={12} className="ml-auto shrink-0 text-faint" />
      </button>
      {open && (
        <>
          {/* Transparent, as the model picker's is: the menu repaints the window it is sitting in, so
              covering that window with a mask would hide the very change being judged. */}
          <span className="picker-scrim" onClick={() => setOpen(false)} />
          {panel}
        </>
      )}
    </span>
  );
}
