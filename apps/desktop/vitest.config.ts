import { defineConfig } from "vitest/config";

// The stores are browser code — `nav` reads the URL at import, `appearance` paints `documentElement`
// — so every test runs under a DOM rather than under a hand-made stand-in for one.
export default defineConfig({
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
  },
});
