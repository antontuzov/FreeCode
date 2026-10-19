/**
 * A byte count as one figure a row can carry.
 *
 * The host reports bytes everywhere, because bytes are what the filesystem knows and what `du` will
 * answer; a page that printed 463290464 would be read once and never again. The units are decimal
 * for the same reason the catalog's `size_gb` is: the Knowledge Center already says 0.46 GB for the
 * pack this page calls 463 MB, and a second set of prefixes would give one file two sizes on two
 * pages of one app.
 */
const UNITS = ["B", "kB", "MB", "GB", "TB"] as const;

export function formatBytes(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  // A count a hair under the boundary rounds to four digits at this unit, and "1000 MB" reads as a bug
  // in the page rather than as the byte count it is.
  if (value >= 999.95 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ${UNITS[unit]}`;
}
