// The shell's iconography, drawn inline rather than pulled from an icon package: the reference
// shots use a consistent 1.7px line at 24px, and a dependency would either not match it or drag in
// hundreds of glyphs the app never renders.
//
// Every path inherits `currentColor`, so an icon is themed by the token its row uses — an icon
// that hard-codes a fill would survive a theme switch and break the dark/light contract.
import type { SVGProps } from "react";

const PATHS = {
  /* The header's column control, drawn as the reference crop draws it: a window, the narrow cell at
     its left being the sidebar itself, and a chevron pointing *at* that cell because pressing the
     control folds the column into it. It used to be a hamburger, which names a menu rather than a
     panel. `sidebarExpand` is the same window with the chevron turned round, for the rail — where the
     arrow points the way the column is about to go. */
  sidebar:
    "M3 5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM9 3v18M16.5 9l-3 3 3 3",
  sidebarExpand:
    "M3 5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM9 3v18M13.5 9l3 3-3 3",
  panelRight:
    "M15 4h4a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1h-4M3 5a1 1 0 0 1 1-1h16a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z",
  back: "M15 5l-7 7 7 7",
  forward: "M9 5l7 7-7 7",
  // A terminal inside a square, not a check: the pill names the mode that edits files, and the
  // reference draws its prompt rather than its outcome.
  coding: "M4 4h16v16H4zM7 9l3 3-3 3M13 15h4",
  // A dial with the hand leaving the ring, not a plus: the gap and the stroke are one gesture, which
  // is how the reference marks the row that starts a run.
  newTask: "M11 12l8-8M13.1 4.3A8 8 0 1 0 17.9 8",
  search: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35",
  workspace: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  knowledge: "M4 5a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2zM8 3v17",
  sites: "M3 5h18v14H3zM3 10h18M8 5v5",
  automations:
    "M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M18.4 5.6l-2.1 2.1M7.7 16.3l-2.1 2.1M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z",
  // A jigsaw piece with a knob on its top edge and another on its right, off the reference crop. Each
  // knob is one arc of radius 2 over a chord of 2.6: because the chord is shorter than the diameter,
  // the major arc closes more than half the circle and the two ends pinch in, which is the neck — so
  // the ball-on-stem look costs one command rather than the six points a stem would be drawn with.
  extensions:
    "M7 9.3h1.7a2 2 0 1 1 2.6 0H13a1.5 1.5 0 0 1 1.5 1.5v1.7a2 2 0 1 1 0 2.6v1.7a1.5 1.5 0 0 1-1.5 1.5H7a1.5 1.5 0 0 1-1.5-1.5V10.8A1.5 1.5 0 0 1 7 9.3z",
  // Eight lobes on a polar profile (radius 6.6→8.6, flattened tops), sampled every 7.5° rather than
  // hand-plotted: the reference's cog has rounded teeth in narrow notches, which no short arc chain
  // draws without a dozen magic numbers. Edit the profile by changing the generator's constants, not
  // by nudging individual points — the outline is one curve, and a lone vertex breaks it.
  settings:
    "M12 3.4L13.1 3.5L13.7 5.6L14.5 5.9L15.3 6.3L17.2 5.2L18.1 5.9L18.8 6.8L17.7 8.7L18.1 9.5L18.4 10.3L20.5 10.9L20.6 12L20.5 13.1L18.4 13.7L18.1 14.5L17.7 15.3L18.8 17.2L18.1 18.1L17.2 18.8L15.3 17.7L14.5 18.1L13.7 18.4L13.1 20.5L12 20.6L10.9 20.5L10.3 18.4L9.5 18.1L8.7 17.7L6.8 18.8L5.9 18.1L5.2 17.2L6.3 15.3L5.9 14.5L5.6 13.7L3.5 13.1L3.4 12L3.5 10.9L5.6 10.3L5.9 9.5L6.3 8.7L5.2 6.8L5.9 5.9L6.8 5.2L8.7 6.3L9.5 5.9L10.3 5.6L10.9 3.5zM12 14.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z",
  // Three quarters of a circle, open at the top-right: the reference's step pill carries a loader
  // rather than a number-in-a-box, and this is the one glyph that reads as "the loop is turning"
  // whether or not the caller spins it. `Composer` adds the spin while the round runs.
  step: "M12 4a8 8 0 1 0 8 8",
  // A prompt with no box around it, unlike `coding`'s: this glyph is only ever drawn inside an
  // inspector chip, which is already a square. Two squares nested at 14px read as one smudge.
  skills: "M5 8l4 4-4 4M12 16h7",
  artifact: "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6",
  webReading:
    "M2 12a10 10 0 1 0 20 0 10 10 0 1 0-20 0M2 12h20M12 2a15 15 0 0 1 0 20 15 15 0 0 1 0-20",
  // Two sheets, the lower one only a line under the first: this group lists what the answer was
  // assembled from. It used to carry `download`'s arrow verbatim, which is a thing you do to a file
  // rather than a thing a file can be.
  source: "M12 3 3 8l9 5 9-5zM3 13l9 5 9-5",
  terminal: "M4 17l6-6-6-6M12 19h8",
  thought: "M12 3a7 7 0 0 0-4 12.7V18a1 1 0 0 0 1 1h6a1 1 0 0 0 1-1v-2.3A7 7 0 0 0 12 3zM9 21h6",
  readFile: "M6 2h9l5 5v13a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2zM9 13h7M9 17h5",
  editFile: "M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z",
  searchContent: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35M8 11h6",
  mic: "M12 15a3 3 0 0 0 3-3V6a3 3 0 0 0-6 0v6a3 3 0 0 0 3 3zM19 10a7 7 0 0 1-14 0M12 17v4",
  stop: "M6 6h12v12H6z",
  send: "M12 19V5M5 12l7-7 7 7",
  attach: "M21 12.5 12.5 21a5 5 0 0 1-7-7l8.5-8.5a3.5 3.5 0 0 1 5 5L10.5 19",
  plus: "M12 5v14M5 12h14",
  help: "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM9.1 9a3 3 0 0 1 5.8 1c0 2-3 2.5-3 4.5M12 17.5h.01",
  chevronDown: "M6 9l6 6 6-6",
  chevronRight: "M9 6l6 6-6 6",
  monitor: "M3 4h18v12H3zM8 20h8M12 16v4",
  clock: "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 6v6l4 2",
  bolt: "M13 2 4 14h7l-1 8 9-12h-7z",
  diamond: "M6 3h12l3 6-9 12L3 9zM3 9h18M9 3 6 9l6 12 6-12-3-6",
  sliders:
    "M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0M16 6a2 2 0 1 0-4 0 2 2 0 0 0 4 0zM10 12a2 2 0 1 0-4 0 2 2 0 0 0 4 0zM18 18a2 2 0 1 0-4 0 2 2 0 0 0 4 0z",
  wand: "M15 4V2M15 10V8M12.5 6h-2M18.5 6h-2M5 20l9-9M13.5 7.5 16.5 10.5",
  shield: "M12 3 4 6v6c0 4.5 3.2 7.8 8 9 4.8-1.2 8-4.5 8-9V6z",
  cube: "M12 2 3 7v10l9 5 9-5V7zM3 7l9 5 9-5M12 12v10",
  // Three stacked sheets with the top one closed: the disk the page counts the app's own directories
  // on. Kept off `cube`, which is the model this app loads, because the two are one row apart in the
  // nav and a glyph that repeats another's meaning is how a 13px rail turns into a puzzle.
  storage:
    "M5 6c0-1.7 3.1-3 7-3s7 1.3 7 3-3.1 3-7 3-7-1.3-7-3zM5 6v12c0 1.7 3.1 3 7 3s7-1.3 7-3V6M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3",
  sun: "M12 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10zM12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4",
  moon: "M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z",
  close: "M18 6 6 18M6 6l12 12",
  // A can with its lid off, the two inner lines being the slats the reference draws. The sides taper
  // inward so the glyph does not read as `sites`' window at 13px: the trapezoid is what says bin.
  trash:
    "M4 7h16M9.5 7V5a1 1 0 0 1 1-1h3a1 1 0 0 1 1 1v2M6.5 7l.8 12.1a2 2 0 0 0 2 1.9h5.4a2 2 0 0 0 2-1.9L17.5 7M10 11v6M14 11v6",
  voice: "M12 3v18M8 8v8M4 11v2M16 6v12M20 10v4",
  user: "M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2M12 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8z",
  download: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3",
  // Two sheets, the back one drawn as an open outline so it never crosses the front one's edge.
  copy: "M9 8h9a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2zM5 16a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2",
  context: "M4 4h16v16H4zM4 10h16M10 4v16",
  review: "M9 11l3 3L22 4M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11",
  // The tick the reference puts at the head of every finished step row: a circle with the check
  // inside it, drawn on the same 24 grid as `review`'s box so the two read as one family. They are
  // not the same mark — the panel's square says "the run's changes", this says "this one is done".
  check: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM8.3 12.2l2.6 2.6 5-5.4",
  // The reference's outline glyph: one entry on a trunk, and the two it branches to. It is the tree a
  // diff folds to when the hunks are put away, so the boxes stand for the file names rather than for
  // files — `readFile`'s folded corner already owns that.
  outline: "M4 3h5v4H4zM6.5 7v9M6.5 10H11M11 8h5v4h-5zM6.5 16H11M11 14h5v4h-5z",
  warning:
    "M12 9v4M12 17h.01M10.3 3.6 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.6a2 2 0 0 0-3.4 0z",
  external: "M17 3h4v4M21 3l-9 9M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6",
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({
  name,
  size = 16,
  ...rest
}: { name: IconName; size?: number } & Omit<SVGProps<SVGSVGElement>, "name">) {
  return (
    <svg
      aria-hidden
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
      {...rest}
    >
      <path d={PATHS[name]} />
    </svg>
  );
}
