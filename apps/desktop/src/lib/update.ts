import type { UpdateState } from "./ipc";

/** The translator the page hands in: a key and its parameters in, a sentence out. */
export type Translate = (key: string, params?: Record<string, string | number>) => string;

/**
 * What the published build means for the build that is running.
 *
 * Two different sentences, not one sentence with a slot: a published build that carries this build's
 * own version is not "nothing happened" — it is the answer that this build is current. The date is
 * appended when the feed gave one, because how old a release is says something about the feed that
 * a reader of the row would want.
 */
export function updateDescription(update: UpdateState, t: Translate, locale: string): string {
  const sentence = update.available
    ? t("update.newer", { latest: update.latest, current: update.current })
    : t("update.same", { latest: update.latest });
  if (!update.publishedAt) return sentence;
  const when = new Date(update.publishedAt);
  if (Number.isNaN(when.getTime())) return sentence;
  return `${sentence} ${t("update.publishedOn", { date: when.toLocaleDateString(locale) })}`;
}
