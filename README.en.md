# Snap Desk

[简体中文](README.md) · English

A free, open-source screenshot and screen recording tool for Windows, with scrolling capture, image annotation, image pinning, OCR, translation, and clipboard management.

[Download](https://github.com/bitdan/snap-desk/releases) · [Quick Start](#quick-start) · [Technical Guide (Chinese)](TECHNICAL.md) · [Roadmap (Chinese)](Snap_Desk_TODO.md)

## Download

Download the Windows x64 installer or standalone EXE from [Snap Desk releases](https://github.com/bitdan/snap-desk/releases). Install offline OCR models, the local translation engine, and language packs in the app as needed.

## Features

`[x]` / ✅ means supported; `[ ]` means not yet supported. See the [roadmap (Chinese)](Snap_Desk_TODO.md) for future plans.

- **Offline**: capture, recognition, or translation runs on your computer. Installing resources and checking for updates may require internet access.
- **Online API**: text or selected images are sent to your chosen provider. Quotas, pricing, and usage rights depend on that provider.
- **Local API**: requests go to a local endpoint. Whether it accesses cloud services depends on its configuration; a local endpoint does not guarantee offline processing.

### Screenshot & Annotation

- [x] **Offline** · Region and full-screen capture, with snapping to windows.
- [x] **Offline** · Multiple monitors, mixed display scaling, capture delay, and cursor options.
- [x] **Offline** · Color picker, rectangles, ellipses, lines, arrows, arrow labels, and polylines.
- [x] **Offline** · Pen, highlighter, text, numbered labels, spotlight, and magnifier annotations.
- [x] **Offline** · Mosaic, blur, pixel erasing, and annotation deletion.
- [x] **Offline** · Shared annotation tools for captures and pinned images, further editing, undo/redo, and toolbar reordering.
- [x] **Offline** · Copy and save images, export PNG/JPEG/WebP, and customize filenames.

### Scrolling Screenshot

- [x] **Offline** · Manual horizontal or vertical scrolling, reversing direction along the same axis, and automatic stitching.
- [x] **Offline** · Live preview, failure recovery, and PNG export with a white or transparent background.
- [ ] Extra-large scrolling captures.
- [ ] General-purpose full-page browser capture.

### Image Pinning

- [x] **Offline** · Pin screenshots, images, or clipboard content to the desktop.
- [x] **Offline** · Multiple windows, grouping, locking, zoom, opacity, and always-on-top controls.
- [x] **Offline** · Adjust, annotate, copy, and save images.
- [x] **Depends on the selected service** · Access OCR, QR code recognition, and translation from pinned images.

### Screen Recording

- [x] **Offline / Windows** · Record a region or display and export MP4/GIF.
- [x] **Offline / Windows** · Pause/resume, keyboard and mouse visualization, and live annotations.
- [x] **Offline / Windows** · Record system audio and microphone input in MP4 recordings.
- [ ] Webcam overlay, H.265, HDR, and advanced video editing.

### OCR

| Status | Engine / Service | Type | Notes |
| --- | --- | --- | --- |
| ✅ | PaddleOCR · PP-OCRv6 | Offline | Install recognition resources on first use |
| ✅ | Windows OCR | Offline | Uses installed Windows recognition language packs |
| ✅ | OCR.space | Online API | Requires an API key; large screenshots are compressed to meet the free tier's 1 MB limit |
| ✅ | Paddle API | Online API | Requires an AI Studio token; model options include PP-OCRv6, PaddleOCR-VL-1.6, and PP-StructureV3 |
| ✅ | Groq Vision | Online API | Select a model that supports image recognition |
| ✅ | Custom vision service | Online API / Local API | Connect a compatible service with image input support |

- [x] Edit, copy, or translate recognized text.
- [x] Recognize QR codes locally; confirm before opening detected links.
- [x] Use the selected OCR engine without automatically switching to an online service when local recognition fails.

When online OCR is selected, recognition uploads the selected image directly. See the [OCR technical guide (Chinese)](TECHNICAL.md#ocr-接入) for endpoints, models, quotas, and commercial-use terms.

### Translation

| Status | Engine / Service | Type | Notes |
| --- | --- | --- | --- |
| ✅ | CTranslate2 | Offline | Chinese–English translation after installing the engine and language pack |
| ✅ | DeepL API Free / Pro | Online API | Use your own API key |
| ✅ | Groq | Online API | Select a text model available to your account |
| ✅ | OpenAI | Online API | Configure a model and API key |
| ✅ | Ollama and other local services | Local API | Connect a compatible local text model service |
| ✅ | Custom text service | Online API / Local API | Configure the endpoint, model, and required credentials |

- [x] A separate translation window for typed text, clipboard text, selected-text shortcuts, and screenshot translation.
- [x] Press `Alt+T` to open the translation window; selected text is copied and translated automatically on Windows. The shortcut is customizable.
- [x] Press `Enter` to translate and `Shift+Enter` for a new line; switch languages, copy results, merge line breaks, and keep the window on top.
- [x] Shared translation access from the screenshot toolbar, pinned images, and translation window. Online text translation sends only the text to translate.

See the [translation technical guide (Chinese)](TECHNICAL.md#翻译接入) for dedicated API adapters and compatible services.

### API Service Management

- [x] Separate OCR, translation, and service settings pages, with cloud APIs first, followed by local APIs and offline engines.
- [x] Multiple account/model connections. Saved connections replace provider presets while preserving separate accounts.
- [x] Display the active service, model, and current-use indicator; distinguish the configuration being viewed from the one in use.
- [x] Store secrets in Windows Credential Manager, isolated by connection and endpoint; exclude secrets from configuration exports.
- [x] Link to official credential pages and connect with API keys/tokens.
- [ ] Sign in through a browser to authorize Snap Desk directly.

### Clipboard Manager

- [x] **Offline** · Text, image, and file-path history with categories, sorting, search, favorites, and previews.
- [x] **Offline** · Copy, pin, and export to PDF, long PNG images, or individual files. File history stores paths only.

### Settings & Updates

- [x] **Offline** · Custom shortcuts, conflict checks, global shortcut enable/disable, and an interactive interface guide.
- [x] **Offline** · Chinese/English interface, light/dark themes, configuration import/export, and diagnostic logs.
- [x] **Online** · Stable/beta updates for both installer and standalone EXE distributions.

## Quick Start

1. Select an engine in OCR settings. Install resources for offline engines, or enter your own API key/token and save an online service.
2. Select a local or online translation service and set the default target language. Configure additional accounts or models in service management.
3. Check the current OCR/translation summary and the current-use indicator in the menu to confirm which configuration is active.
4. Use the screenshot toolbar, pinned-image menu, or translation window. View and customize shortcuts in shortcut settings.

Paddle offers an official asynchronous endpoint (recommended) and a synchronous endpoint with a custom address. Use the official asynchronous option unless your console provides a synchronous endpoint. Online services may incur charges; quotas and usage rights depend on your provider's account terms.

For development, builds, and additional service integrations, see the [technical guide (Chinese)](TECHNICAL.md).

## FAQ

### Which platforms are supported?

Snap Desk is available for Windows x64 as an installer or standalone EXE. Screenshot capture, screen recording, OCR, translation, and clipboard history are available in one desktop app.

### Can OCR and translation work offline?

Yes. PaddleOCR and Windows OCR recognize text locally. CTranslate2 supports offline Chinese–English translation after its resources are installed. You can also choose online services such as OCR.space, Paddle API, DeepL, and Groq using your own credentials.

### Is Snap Desk free?

Snap Desk is free and open source under GPL-3.0-only. Third-party OCR and translation APIs may charge for usage; free quotas depend on the provider.

## Acknowledgements

- [Tauri](https://github.com/tauri-apps/tauri): an easy-to-use GUI framework.
- [Pot](https://github.com/pot-app/pot-desktop): a reference for the translation and OCR modules. Thanks for sharing the project as open source.

## License

[GPL-3.0-only](LICENSE)
