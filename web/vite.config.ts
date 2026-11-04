import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// Everything that is not a page is the API, proxied to the Rust server when the Vite dev server is
// used. The normal way to run is the built pages served by the server itself at /app.
const api = ['/auth', '/me', '/deals', '/ladder', '/breaks', '/feeds', '/positions', '/alm', '/market', '/activity', '/attention', '/events', '/healthz'];

export default defineConfig({
  base: '/app/',
  publicDir: '../assets',
  plugins: [react()],
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false, target: 'es2022' },
  server: {
    port: 5173,
    proxy: Object.fromEntries(api.map((p) => [p, { target: 'http://127.0.0.1:8787', changeOrigin: false }])),
  },
  test: { environment: 'node', include: ['src/**/*.test.ts'] },
});
