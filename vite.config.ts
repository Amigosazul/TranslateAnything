import { defineConfig } from "vite";
// @ts-expect-error 未安装 @types/node 时的类型报错
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  // 以下选项只针对 `tauri dev` / `tauri build`
  // 1. 不清屏，避免挡住 Rust 报错
  clearScreen: false,
  // 2. Tauri 需要固定端口，被占用直接失败
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. 不监听 src-tauri
      ignored: ["**/src-tauri/**"],
    },
  },
  // 多页面：框选遮罩页、翻译框页、设置页
  build: {
    rollupOptions: {
      input: {
        overlay: "overlay.html",
        frame: "frame.html",
        settings: "settings.html",
      },
    },
  },
}));
