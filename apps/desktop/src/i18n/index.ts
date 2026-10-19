// A hand-rolled catalog rather than a framework: the surface is small, and every transcript label
// already has one authoritative key in Rust (`smollm_core::transcript::StepKind::i18n_key`).
import en from "./en.json";
import zh from "./zh.json";

export type Locale = "en" | "zh";

const catalogs: Record<Locale, Record<string, string>> = {
  en: en as Record<string, string>,
  zh: zh as Record<string, string>,
};

export const TRANSCRIPT_KEYS = Object.keys(catalogs.en).filter((k) =>
  k.startsWith("transcript.step."),
);

export function interpolate(template: string, params: Record<string, string | number>): string {
  return template.replace(/{{(\w+)}}/g, (_all, name: string) => String(params[name] ?? ""));
}

export function translate(
  locale: Locale,
  key: string,
  params: Record<string, string | number> = {},
): string {
  const table = catalogs[locale] ?? catalogs.en;
  // A missing key falls back to English rather than rendering the key itself in the transcript.
  const template = table[key] ?? catalogs.en[key] ?? key;
  return interpolate(template, params);
}

/**
 * Whether the catalog holds this key at all.
 *
 * [`translate`] makes a missing key visible by printing it, which is right for a row the user is
 * reading and wrong for a string being *tested*: settings search matches a control's description when
 * it has one, and comparing against the fallback would let a query for `settings.row` find every row
 * that has no description. English is the authority because `check-i18n` fails the build when the two
 * catalogs hold different keys.
 */
export function has(key: string): boolean {
  return key in catalogs.en;
}
