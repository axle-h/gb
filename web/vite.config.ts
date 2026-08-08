import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// `npm run build` → `web/dist`, which `rust-embed` bakes into the binary (`src/web/assets.rs`).
// `npm run dev` serves the same app on :5173 with hot reload and proxies the two SSE endpoints to a
// `gb serve` on :8080 — which is why the app only ever uses relative URLs.
export default defineConfig({
  plugins: [react()],
  // ⚠️ `public/.gitkeep` is not decoration. `rust-embed` fails to *compile* if `web/dist` does not
  // exist, so a committed `web/dist/.gitkeep` is what lets a fresh checkout build before anyone has
  // run `npm run build` — and `vite build` empties `dist` first, which would delete it. Copying it
  // back from `public/` on every build keeps the committed file in place instead of leaving a
  // deletion in `git status` after each build.
  build: {
    // One JS file and one CSS file. The app is ~10 KB of source; chunking it would only cost
    // requests, and `assets.rs` has less to serve.
    assetsInlineLimit: 0,
    rollupOptions: { output: { manualChunks: undefined } },
  },
  server: {
    // Both endpoints are SSE, which the dev proxy streams as-is — the app only ever uses relative
    // URLs, so nothing in it knows whether it is being served by Vite or by `gb`.
    proxy: { '/api': 'http://localhost:8080' },
  },
});
