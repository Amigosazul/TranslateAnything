// 设置输入框、选区和翻译框共用的圆角范围（CSS 像素）。
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
export const FRAME_RADIUS_MAX = 128;
export const FRAME_RADIUS_EVENT = "frame-corner-radius";
export const FRAME_RADIUS_PREVIEW_EVENT = "frame-corner-radius-preview";

export function clampFrameRadius(radius: number): number {
  return Number.isFinite(radius) ? Math.min(FRAME_RADIUS_MAX, Math.max(0, Math.round(radius))) : 0;
}

/** 订阅设置变化后读取保存值，让选区和翻译框使用同一套圆角。 */
export async function initFrameCornerRadius(element: HTMLElement) {
  let saved: number | undefined;
  let preview: number | null = null;
  const apply = () => element.style.setProperty("--frame-radius", `${clampFrameRadius(preview ?? saved ?? 0)}px`);
  await Promise.all([
    listen<number>(FRAME_RADIUS_EVENT, ({ payload }) => {
      saved = payload;
      apply();
    }),
    listen<number | null>(FRAME_RADIUS_PREVIEW_EVENT, ({ payload }) => {
      preview = payload;
      apply();
    }),
  ]);
  const radius = await invoke<number>("frame_get_corner_radius");
  saved ??= radius;
  apply();
}
