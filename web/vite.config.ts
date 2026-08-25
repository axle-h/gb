import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// `pnpm run build` → `web/dist`, which `rust-embed` bakes into the binary (`src/web/assets.rs`).
// `pnpm run dev` serves the same app on :5173 with hot reload and proxies everything the app asks
// of the server to a `gb serve` on :8080 — which is why the app only ever uses relative URLs.
export default defineConfig({
  plugins: [react()],
  // ⚠️ `public/.gitkeep` is not decoration. `rust-embed` fails to *compile* if `web/dist` does not
  // exist, so a committed `web/dist/.gitkeep` is what lets a fresh checkout build before anyone has
  // run `pnpm run build` — and `vite build` empties `dist` first, which would delete it. Copying it
  // back from `public/` on every build keeps the committed file in place instead of leaving a
  // deletion in `git status` after each build.
  build: {
    // One JS file and one CSS file. The app is ~10 KB of source; chunking it would only cost
    // requests, and `assets.rs` has less to serve.
    assetsInlineLimit: 0,
    rollupOptions: { output: { manualChunks: undefined } },
  },
  server: {
    // The app only ever uses relative URLs, so nothing in it knows whether it is being served by
    // Vite or by `gb`; the dev proxy streams `/api` as-is, SSE and the chunked video alike.
    // ⚠️ `/favicon.png` needs a line of its own because it does not live under `/api`. It is decoded
    // out of the cartridge by `src/web/sprites.rs`, so there is no file in `web/public/` for Vite to
    // fall back to and the dev server answered 404 — a missing tab icon that looks exactly like the
    // deployed one having broken.
    proxy: {
      '/api': 'http://localhost:8080',
      '/favicon.png': 'http://localhost:8080',
    },
  },
});
