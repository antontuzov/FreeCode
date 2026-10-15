import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Tauri expects a fixed dev port and no auto-open (the window is native, not a browser tab).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2022",
    outDir: "dist",
    sourcemap: true,
    // The answer renderer carries KaTeX and the common highlighter grammars, which pushes the entry
    // chunk past Vite's web default. This bundle is read from the app bundle's own disk at window
    // start, not fetched over a network, and the same process is about to map a 0.46 GB weights file
    // — so the default's threshold is measuring the wrong thing here.
    chunkSizeWarningLimit: 900,
  },
});
