import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { viteSingleFile } from 'vite-plugin-singlefile';

// Output a single, fully-inlined HTML bundle. Rust embeds the result via
// `include_str!`, so we cannot ship sibling JS/CSS files — everything lives
// in `dist/index.html`.
export default defineConfig({
  plugins: [svelte(), viteSingleFile()],
  build: {
    target: 'es2022',
    cssCodeSplit: false,
    assetsInlineLimit: 100_000_000,
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
      },
    },
    chunkSizeWarningLimit: 4096,
    minify: true,
    sourcemap: false,
  },
  server: {
    port: 5173,
    proxy: {
      '/api':  { target: 'http://127.0.0.1:7428', changeOrigin: false },
      '/mcp':  { target: 'http://127.0.0.1:7428', changeOrigin: false },
    },
  },
});
