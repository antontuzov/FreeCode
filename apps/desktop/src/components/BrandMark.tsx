import { hostMascot } from "../lib/ipc";

/**
 * The app's two faces, and the opposite trades they make.
 *
 * `BrandMark` is drawn rather than pasted: an inline SVG that takes the live accent, so it reads the
 * same in light, dark, Forest, Mint, Bee and Parchment. A brand image that cannot be re-coloured is
 * a second palette, and the whole design system is that there is only one. It is also what ships:
 * the bundle icon is a raster `scripts/render-app-icon.py` cuts from the same silhouette, because a
 * face chosen at every launch cannot be baked into a file that is written once.
 *
 * A bonsai in a pot: the model this app runs is pruned down to ternary weights and kept on the
 * machine it was downloaded to.
 *
 * `MascotMark` spends the other trade on purpose. Twelve hand-drawn duck heads, pasted exactly as
 * they are, because a mascot re-tinted to the accent stops being a mascot — it is art on its own
 * ground, ringed and cut to a circle, and it is the face the app shows when it runs. The drawn mark
 * stays what carries the name.
 */
export function BrandMark({ size = 44 }: { size?: number }) {
  return (
    <svg
      aria-hidden
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
    >
      {/* Two canopy pads and the branch that carries the smaller one. */}
      <ellipse cx="10.4" cy="6.9" rx="5.5" ry="3.5" fill="currentColor" stroke="none" />
      <ellipse cx="16.6" cy="10.5" rx="3.2" ry="2.1" fill="currentColor" stroke="none" />
      <path d="M10.1 11.1 14.4 9.9" strokeWidth="0.75" strokeLinecap="round" />
      {/* The trunk: out of the pot, one bend, into the crown. */}
      <path
        d="M11 14.9 9.5 12 10.3 9.4"
        strokeWidth="1.15"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      {/* The pot: a rim over a tapered body. */}
      <path d="M6.9 13.9h8.2v1.1H6.9z" fill="currentColor" stroke="none" />
      <path d="M7.7 15.3h6.6l-.9 4.1H8.6z" fill="currentColor" stroke="none" />
    </svg>
  );
}

/** Cut from the sheet by `scripts/render-mascot-logos.py`. The set is the directory rather than a
 * list of imports, so a thirteenth duck is a file dropped beside the others — though the host keeps
 * its own twelve in `brand.rs`, because a Dock tile is bytes handed to `NSImage` and wants a line
 * there too. */
const mascotTiles = import.meta.glob<string>("../assets/mascots/*.webp", {
  eager: true,
  query: "?url",
  import: "default",
});

const mascots = Object.keys(mascotTiles)
  .sort()
  .map((path) => mascotTiles[path]);

/** One duck per run, and it is the host's duck: the same picture goes on the Dock the moment the app
 * opens (`brand.rs`), so the window that asks would otherwise be wearing a different face than the
 * tile beside it.
 *
 * `?mascot=<1..12>` names the tile instead, for the same reason the other deep links exist: a pick
 * nobody can re-ask for is a set of twelve that only one of ever got looked at. */
function pickTile(count: number): number {
  const forced = Number(new URLSearchParams(window.location.search).get("mascot"));
  if (Number.isInteger(forced) && forced >= 1 && forced <= count) return forced;
  if (hostMascot !== null && hostMascot <= count) return hostMascot;
  // A page opened outside the app is handed no number by anybody, and a mascot that waits on a host
  // to appear is a mascot that never shows.
  return 1 + Math.floor(Math.random() * count);
}

const mascot = mascots[pickTile(mascots.length) - 1];

export function MascotMark({ size = 96 }: { size?: number }) {
  return (
    <img
      // The app's name is printed under it, so the picture is that word a second time for a screen
      // reader, and `alt=""` is the answer rather than a new translation key.
      alt=""
      aria-hidden
      src={mascot}
      width={size}
      height={size}
      className="rounded-full border border-line object-cover"
    />
  );
}
