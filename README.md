# translate-filter · 截图框选翻译滤镜

快捷键框选屏幕任意区域，框内文字被实时识别翻译，译文按原文位置覆盖显示（沉浸式），看起来就像软件本身支持了这种语言；内容滚动、变化时译文跟着刷新，翻译框不影响下方软件的操作。交互参考 PixPin，只做翻译，不做图像处理。

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

🚧 M2 开发中 — 沉浸式实时翻译：框选后译文按原位置覆盖显示，内容滚动/变化时实时跟随刷新。

## 配置

不写配置也能用（默认 `edge` 引擎免 key）。需要切换引擎、配置回退顺序或代理时，复制 [config.example.json](config.example.json) 到 `~/.translate-filter/config.json` 修改，字段说明见 [docs/02-技术方案.md](docs/02-技术方案.md) 的“翻译 Provider 抽象”一节。

## 许可证

[MIT](LICENSE)
