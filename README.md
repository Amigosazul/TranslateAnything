# translate-filter · 截图框选翻译滤镜

快捷键框选屏幕任意区域，区域内文字自动 OCR + 翻译，译文以可拖动、可缩放的"滤镜"浮窗覆盖显示；浮窗挪到哪里，就翻译哪里。交互参考 PixPin，只做翻译，不做图像处理。

- 技术栈：Tauri 2 + Rust + Vite（Windows 优先）
- OCR：Windows 原生 `Windows.Media.Ocr`（离线）
- 翻译：默认免 key 的微软 Edge 翻译（失败自动回退 Bing 网页版），可选 Google 免费接口、微软/DeepL/Google 官方 API、DeepLX、OpenAI 兼容大模型

## 文档

| 文档 | 内容 |
|---|---|
| [CLAUDE.md](CLAUDE.md) | 项目状态、已定决策（AI 会话上下文） |
| [docs/01-需求说明.md](docs/01-需求说明.md) | 交互流程、功能清单、验收标准 |
| [docs/02-技术方案.md](docs/02-技术方案.md) | 架构、模块、关键依赖、坑位 |
| [docs/03-开发环境.md](docs/03-开发环境.md) | 工具链安装与验证 |

`recovered-chats/` 是立项时两次 AI 对话的存档，仅作历史参考。

## 当前进度

🚧 M1 开发中 — 快捷键 → 框选 → 截屏 → OCR → 翻译 → 浮窗显示的链路已打通。

## 配置

不写配置也能用（默认 `edge` 引擎免 key）。需要切换引擎、配置回退顺序或代理时，复制 [config.example.json](config.example.json) 到 `~/.translate-filter/config.json` 修改，字段说明见 [docs/02-技术方案.md](docs/02-技术方案.md) 的“翻译 Provider 抽象”一节。

## 许可证

[MIT](LICENSE)
