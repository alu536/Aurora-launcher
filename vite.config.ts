import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Port 1420 must match "devUrl" in src-tauri/tauri.conf.json.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    target: "es2020",
    outDir: "dist",
  },
});
