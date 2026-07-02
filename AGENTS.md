# AGENTS.md

This directory is the canonical design root for rebuilding Yuukei from scratch.
It is not documentation for the existing MVP implementation.

## Read First

Before changing this design or implementing from it, read these files in order:

1. `README.md`
2. `01-concept.md`
3. `02-architecture.md`
4. `03-protocols.md`
5. `04-event-log-and-memory.md`
6. `05-world-pack-and-daihon.md`
7. `06-build-guidance-for-codex.md`

If you only have time for the minimum, read `README.md`, `02-architecture.md`, `03-protocols.md`, and `06-build-guidance-for-codex.md`.

## Product Intent

- Yuukei is a platform for UI residents, not a generic assistant, chatbot, or desktop mascot.
- The OS UI is the resident's living space: terrain, rooms, tools, weather, entrances, and the outside world.
- The user should feel that ordinary device use becomes shared life with the resident.
- Daihon creates deliberate character moments. AI fills everyday gaps; it does not replace authored scenes.
- LLM, memory, TTS, STT, embedding, vision, protocol hooks, event subscriptions, and specialized hardware support are replaceable Extensions, including official bundled default Extensions.

## Architecture Rules

- Keep resident continuity in `Resident Home`.
- Keep device-specific sensors, OS permissions, local files, microphones, cameras, notifications, and local Extension hosting in `Device Host`.
- Keep body, rendering, speech bubbles, animation, mobile widgets, and visual effects in `Surface Client`.
- Keep LLM, TTS, STT, memory retrieval, memory indexing, embedding, and similar abilities behind manifest-declared Extension capabilities routed by the internal `CapabilityRouter`.
- Keep worldview, cast, scripts, allowed signals, UI-space interpretation, and required capabilities in `World Pack`.
- Keep arbitrary user code, message interception, event subscriptions, event emission, and signal alias donation behind `Extension`; Extensions operate on public protocol messages, not Core internals.
- Keep Daihon behind a host/service boundary. Do not make `Resident Home` depend on Daihon internal parser/runtime types.
- Treat `canonical event log` as the source of truth for records. Treat memory databases, summaries, facts, episodes, vector indexes, and extension-specific binary stores as derived data.

## Implementation Bias

- Rust is a good default for `Resident Home` and protocol/event-log foundations.
- Tauri is a good default for the first desktop `Device Host` and desktop `Surface Client`.
- Do not put Tauri `AppHandle`, WebView, OS window handles, or platform APIs inside `Resident Home`.
- Design local-first, but keep the same protocol usable when `Resident Home` runs on LAN or cloud infrastructure.
- Start with protocol types, event log, headless `Resident Home`, minimal `Surface Client`, then `Device Host`, Daihon integration, and finally official default Extensions.
- Prefer clear JSON-serializable message contracts before rich UI, model quality, or renderer polish.

## Extension and Capability Rules

- Extensions do not call each other directly. Route composition through `Resident Home` and the internal `CapabilityRouter`.
- Extensions do not mutate `Resident Home`, `Surface Client`, or event log storage directly. They return protocol-level hook results or proposed RuntimeEvents for `Resident Home` to validate and record.
- A TTS Extension must not care whether text came from Daihon or an LLM. It receives normalized `speech.synthesis` input through the capability route.
- A Memory Extension must not become the owner of the resident's life history. It reads permitted event-log ranges and builds rebuildable indexes.
- World Packs declare required or optional capabilities; they do not name a specific Extension as a hard dependency unless the design explicitly needs that.
- Extension configuration UI may live in a `Device Host`, but capability registration, permissions, and selection belong to `Resident Home`.

## Documentation Rules

- Keep this directory as the new-design source of truth.
- Do not describe the existing MVP as the target structure. Mention it only when warning against being anchored by it.
- When adding a concept, place it in the owning document:
  - product experience: `01-concept.md`
  - components and ownership: `02-architecture.md`
  - messages and RPC: `03-protocols.md`
  - event log and memory: `04-event-log-and-memory.md`
  - World Pack and Daihon: `05-world-pack-and-daihon.md`
  - build order and Codex guidance: `06-build-guidance-for-codex.md`
- If a new idea does not fit any file, question whether it belongs in Yuukei Core.

## Avoid

- Starting from a chat UI.
- Making LLM quality the center of the product.
- Putting memory schema, summaries, facts, episodes, or vector indexes directly into Core as the only supported model.
- Giving `Surface Client` personality, long-term state, or authority to choose capabilities.
- Leaking `Device Host` OS APIs into `Resident Home`.
- Connecting extensions directly to each other.
- Exposing Core internal function names or mutable internal state as an extension API.
- Letting World Packs call OS APIs, AI APIs, or extension-specific APIs directly.
- Adding compatibility layers for the unpublished MVP when a cleaner new design is better.
