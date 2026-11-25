import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The Rust binary embeds `dist/` and serves `index.html` for unknown non-API routes,
// so assets must resolve from the root regardless of the current client-side route.
const backend = process.env.LIBRISYNC_DEV_BACKEND ?? 'http://localhost:8383';

export default defineConfig({
  base: '/',
  plugins: [react()],
  server: {
    proxy: {
      // SSE (`/api/v1/events`) goes through the same proxy; http-proxy streams it
      // as long as nothing buffers the response.
      '/api': { target: backend, changeOrigin: false },
      '/ping': { target: backend, changeOrigin: false },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
  },
});
