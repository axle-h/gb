import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// `pnpm run build` writes `web/dist`, which `rust-embed` bakes into the binary. `pnpm run dev`
// proxies to `poke-agent-web` on :8080, so the app uses only relative URLs.
export default defineConfig({
  plugins: [react()],
  // `rust-embed` fails to compile without `web/dist`, and `vite build` empties it, so
  // `public/.gitkeep` copies the committed `dist/.gitkeep` back on every build.
  build: {
    // One JS file and one CSS file.
    assetsInlineLimit: 0,
    rollupOptions: { output: { manualChunks: undefined } },
  },
  server: {
    // `/favicon.png` is outside `/api` and is decoded from the cartridge, so it needs its own line.
    proxy: {
      '/api': 'http://localhost:8080',
      '/favicon.png': 'http://localhost:8080',
    },
  },
});
