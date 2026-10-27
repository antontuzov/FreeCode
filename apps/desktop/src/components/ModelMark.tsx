// The maker a model row wears, in a tile beside its name.
//
// The picker lists models by name, and a name does not say who trained the weights — the one fact a
// choice between two free models actually turns on. So a row carries its provider's sign where the app
// has one, the app's own mark where the model is its own, and a plain letter where it has neither:
// GLM, Beam, Flint and LongCat publish no mark this build can copy, and the rule for a maker the app
// cannot name is the rule it already holds for a site it cannot reach — a generic shape, never an
// invented look-alike.
//
// The tile is decorative and says `aria-hidden` to prove it: the row's own text already names the
// provider, so a screen reader that read the letter too would only hear the same word twice.
import { BrandMark } from "./BrandMark";
import { Icon, type IconName } from "./icons";

/** The published signs this build carries, keyed on the maker's name in lower case. `MiMo` is
 * Xiaomi's model, so the row naming it gets Xiaomi's mark rather than a letter. */
const MARKS: [string, IconName][] = [
  ["deepseek", "deepseek"],
  ["nvidia", "nvidia"],
  ["xiaomi", "xiaomi"],
  ["mimo", "xiaomi"],
  ["minimax", "minimax"],
];

/** The two makers whose models the app calls its own: the promotion model it ships as the default, and
 * the Bonsai family its engine loads on this machine. They wear the drawn bonsai rather than a
 * letter, because on those rows the mark and the maker are the same party. */
const OWN = ["freecode", "prism-ml"];

export function ModelMark({ provider }: { provider: string }) {
  const key = provider.trim().toLowerCase();
  const mark = MARKS.find(([name]) => key.startsWith(name))?.[1];
  return (
    <span aria-hidden className="model-mark">
      {mark ? (
        <Icon name={mark} size={14} />
      ) : OWN.includes(key) ? (
        <BrandMark size={17} />
      ) : (
        <span className="model-mark-initial">{key.slice(0, 1).toUpperCase()}</span>
      )}
    </span>
  );
}
