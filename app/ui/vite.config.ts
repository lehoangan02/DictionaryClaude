import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Relative base so the same build works from the dev server and inside the
// Tauri webview (custom protocol).
export default defineConfig({
  base: './',
  plugins: [svelte()],
  server: { proxy: { '/api': 'http://127.0.0.1:8787' } },
  build: { target: 'es2022', sourcemap: false },
});
