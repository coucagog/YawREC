import { defineConfig } from "vite";

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  // Dev server : Tauri attend le port 5173 (cf. tauri.conf.json → devUrl)
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: false,
    hmr: {
      protocol: "ws",
      host: "localhost",
      port: 5173,
    },
    watch: {
      // Ne pas watcher le dossier Rust pour éviter les rebuilds bruyants
      ignored: ["**/src-tauri/**"],
    },
  },
  // Tauri attend le build dans ../dist (cf. tauri.conf.json → frontendDist)
  build: {
    target: "es2021",
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG,
    outDir: "dist",
  },
}));
