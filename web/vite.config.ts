import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // the subtitle renderer finds its worker and wasm next to itself
  optimizeDeps: { exclude: ["jassub"] },
  server: {
    port: 5173,
    proxy: {
      "/api": { target: "http://localhost:8484", changeOrigin: true },
    },
  },
  build: {
    target: "es2022",
    sourcemap: false,
  },
});
