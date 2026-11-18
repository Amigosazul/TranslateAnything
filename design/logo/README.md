# 随译 Logo

图形是"文中藏 A"：一个"文"字，交叉点以下的两笔加一道横正好是"A"；一道扫描线横穿交叉点，线下灰色是还没翻译的原文"A"，线上蓝色是已经改写好的"文"——对应随译"识别并原地改写"的核心动作。

本目录的 SVG 都由 `build.mjs` 生成：

| 文件 | 用途 |
|---|---|
| `icon.svg` | 应用图标（48px 及以上） |
| `icon-small.svg` | 小尺寸加粗版（16～40px，字形放大、笔画加粗），打进 `icon.ico` 的小尺寸 |
| `icon-bright.svg` | 亮色底备用版 |
| `mark.svg` | 透明底标志，用于与文字组合 |
| `tray.svg` | 托盘图标（透明底，深浅任务栏都可见） |

改动后重新生成 SVG 并替换 `src-tauri/icons` 下的全套图标（含拼装 `icon.ico` 和托盘 `tray.png`）：

```bash
node design/logo/build.mjs --apply
```

替换图标后 Cargo 不一定会重新编译，需要先 `touch src-tauri/build.rs` 再构建。
