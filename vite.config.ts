import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri 期望前端在固定端口提供，并要求相对资源路径
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [svelte()],

  // Tauri 在桌面端不支持热重载协议，统一走普通的 HMR
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: {
      // 忽略 Rust 目录，避免触发前端无关重载
      ignored: ["**/src-tauri/**"],
    },
  },
  // Tauri 打包成桌面应用时，资源必须用相对路径
  base: "./",
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
}));
