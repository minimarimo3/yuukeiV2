# Yuukei Intelligence

Official process Extension that provides `dialogue.generate`.

It reads one `CapabilityInvocation` JSON object from stdin and writes one `CapabilityResult` JSON object to stdout. It does not store conversation history or read Yuukei event log files directly.

## Providers

- `gemini`: Google Generative Language API `v1beta/models/{model}:generateContent`. Set `GEMINI_API_KEY`; the default model is `gemini-2.5-flash`.
- `openai-compatible`: `POST {baseUrl}/chat/completions`. The default `baseUrl` is `http://127.0.0.1:1234/v1` for LM Studio. For ChatGPT-compatible servers, set `baseUrl`, `apiKey`, and `model`.

Provider config is read from `manifest.json` and can be overridden with environment variables:

- `YUUKEI_INTELLIGENCE_PROVIDER`
- `YUUKEI_INTELLIGENCE_TIMEOUT_MS`
- `GEMINI_API_KEY`
- `GEMINI_MODEL`
- `OPENAI_COMPATIBLE_BASE_URL`
- `OPENAI_COMPATIBLE_API_KEY`
- `OPENAI_COMPATIBLE_MODEL`

Failures return `{ "speak": false }` as a normal capability result.
