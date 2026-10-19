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

/**
 * Memory on the ladder the machine itself quotes it on.
 *
 * A 24 GB Mac reports 25769803776 bytes, which `formatBytes` renders as 25.8 GB — arithmetically
 * right and useless, because the number the user can check it against is on the About box, and that
 * box counts memory in 1024s while every disk tool on the same machine counts in 1000s. So the two
 * formatters are not one rule applied twice: each matches the one place the figure can be verified,
 * and the row says which is which.
 */
export function formatMemory(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // Whole sizes are what memory ships as, and "24.0 GB" reads as a measurement the app is unsure of.
  const text = Number.isInteger(value) ? String(value) : value.toFixed(1);
  return `${text} ${UNITS[unit]}`;
}
