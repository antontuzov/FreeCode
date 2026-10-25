// The setup flow is a window over whatever surface is underneath it rather than a page on the
// navigation stack: the reference opens it over Settings, and Back has to return to the page the
// user was reading instead of leaving setup. It lives in a store because three places press it —
// the first launch, Settings › Appearance, and the `?onboarding=` capture lever — and two of those
// are outside the component that draws it.
import { create } from "zustand";

const STEPS = 3;

/** `?onboarding=<1..3>` opens the flow at that step. The native window takes no injected clicks, so
 * a middle step is a state no capture could otherwise reach. */
function seededStep(): number {
  if (!import.meta.env.DEV) return 0;
  const asked = Number(new URLSearchParams(window.location.search).get("onboarding"));
  return Number.isInteger(asked) && asked >= 1 && asked <= STEPS ? asked - 1 : 0;
}

type OnboardingStore = {
  open: boolean;
  step: number;
  openSetup: (step?: number) => void;
  close: () => void;
  go: (step: number) => void;
};

export const ONBOARDING_STEPS = STEPS;

export const useOnboarding = create<OnboardingStore>((set) => ({
  open: false,
  step: seededStep(),
  openSetup: (step = seededStep()) =>
    set({ open: true, step: Math.min(STEPS - 1, Math.max(0, step)) }),
  close: () => set({ open: false }),
  go: (step) => set({ step: Math.min(STEPS - 1, Math.max(0, step)) }),
}));
