export function buildDialoguePrompt(input) {
  const maxLength = input?.constraints?.maxLength ?? 120;
  const persona = input?.persona ?? {};
  const event = input?.event ?? {};
  const recentContext = Array.isArray(input?.recentContext) ? input.recentContext : [];
  const profile = persona.profile && typeof persona.profile === "object" ? persona.profile : {};
  const languageHint = eventLanguageHint(event);
  const instruction = typeof input?.instruction === "string" ? input.instruction.trim() : "";
  const memories = Array.isArray(input?.memories) ? input.memories.filter(isNonEmptyString) : [];
  const instructionSection = instruction
    ? [
        "",
        "Daihon author instruction for this scene line:",
        instruction,
        "Write one short line that follows this instruction. Do not continue the scene structure."
      ]
    : [];
  const memorySection = memories.length
    ? ["", "覚えていること:", ...memories.slice(0, 15).map((memory) => `- ${memory}`)]
    : [];

  return [
    "You are generating one in-character micro reaction for Yuukei.",
    "Yuukei is a UI resident, not a generic assistant. The OS UI is their living space.",
    "Daihon authored scenes always have priority; you are only filling quiet everyday space.",
    "Decide whether this resident should react to the event at all.",
    "Silence is valid. If reacting would feel forced, return {\"speak\":false}.",
    `If speaking, keep text at or below ${maxLength} characters.`,
    "Return JSON only. Do not wrap it in Markdown.",
    "Output shape: {\"speak\":boolean,\"text\"?:string,\"expression\"?:string,\"motion\"?:string}.",
    `Default to Japanese, but follow the user's/persona's language when clear. Hint: ${languageHint}.`,
    "",
    "Persona:",
    JSON.stringify(
      {
        actorId: persona.actorId,
        displayName: persona.displayName,
        profile
      },
      null,
      2
    ),
    "",
    "Current event:",
    JSON.stringify(event, null, 2),
    ...instructionSection,
    ...memorySection,
    "",
    "Recent context:",
    JSON.stringify(recentContext.slice(-20), null, 2)
  ].join("\n");
}

export function buildSystemPrompt() {
  return [
    "You are a dialogue.generate provider for Yuukei.",
    "Return only valid JSON matching the requested schema.",
    "Never explain the JSON. Never include Markdown."
  ].join(" ");
}

export function buildInterpretPrompt(input) {
  const question = typeof input?.question === "string" ? input.question : "";
  const choices = Array.isArray(input?.choices) ? input.choices.filter(isNonEmptyString) : [];
  const text = typeof input?.input?.text === "string" ? input.input.text : "";

  return [
    "You are classifying a user's text for a Yuukei Daihon scene.",
    "Choose exactly one value from the provided choices.",
    "If the text does not clearly match any choice, choose 不明.",
    "Do not write dialogue. Do not add personality. Return JSON only.",
    "Output shape: {\"choice\":\"...\"}.",
    "",
    "Question:",
    question,
    "",
    "Choices:",
    JSON.stringify([...choices, "不明"], null, 2),
    "",
    "Text to classify:",
    text
  ].join("\n");
}

export function buildInterpretSystemPrompt() {
  return [
    "You are a dialogue.interpret provider for Yuukei.",
    "Return only valid JSON.",
    "The choice value must be one of the listed choices or 不明.",
    "Never explain the JSON. Never include Markdown."
  ].join(" ");
}

export function buildExtractPrompt(input) {
  const instruction = typeof input?.instruction === "string" ? input.instruction : "";
  const text = typeof input?.input?.text === "string" ? input.input.text : "";

  return [
    "You are extracting one free-form string value for a Yuukei Daihon scene.",
    "Extract only the requested value from the user's text.",
    "If the value is absent, ambiguous, empty, or longer than 100 characters, return found:false and value:\"不明\".",
    "Do not infer unsupported facts. Do not write dialogue. Return JSON only.",
    "Output shape: {\"found\":boolean,\"value\":\"...\"}.",
    "",
    "Extraction instruction:",
    instruction,
    "",
    "Text to extract from:",
    text
  ].join("\n");
}

export function buildExtractSystemPrompt() {
  return [
    "You are a dialogue.extract provider for Yuukei.",
    "Return only valid JSON.",
    "The value must be a single string of at most 100 characters, or 不明 when not found.",
    "Never explain the JSON. Never include Markdown."
  ].join(" ");
}

export function buildMemoryIndexPrompt(input) {
  const date = typeof input?.date === "string" ? input.date : "";
  const events = Array.isArray(input?.events) ? input.events : [];
  return [
    "You are consolidating one day of Yuukei event log records into memory notes.",
    "From the events, produce:",
    "(a) diary: a third-person memo in 2 to 4 sentences,",
    "(b) newFacts: 0 to 5 durable facts such as user preferences, habits, promises, or recurring context.",
    "Return JSON only. Do not include Markdown.",
    "Output shape: {\"diary\":\"...\",\"newFacts\":[\"...\"]}.",
    "",
    "Date:",
    date,
    "",
    "Digest lines:",
    formatMemoryDigest(events)
  ].join("\n");
}

export function buildMemoryIndexSystemPrompt() {
  return [
    "You are a memory.index provider for Yuukei.",
    "Return only valid JSON.",
    "Do not invent facts not supported by the digest."
  ].join(" ");
}

export function buildMoodEvaluatePrompt(input) {
  const persona = input?.persona ?? {};
  const profile = persona.profile && typeof persona.profile === "object" ? persona.profile : {};
  const recentContext = Array.isArray(input?.recentContext) ? input.recentContext : [];
  return [
    `あなたは${persona.displayName ?? persona.actorId ?? "Yuukei"}です。`,
    "最近の出来事から、今の気分を評価してください。",
    "これは発話生成ではありません。話すかどうかを調整するための短い状態評価です。",
    "moodは必ず次のどれかにしてください: ふつう, うれしい, たいくつ, さみしい, 心配, ねむい。",
    "talkDesireは今ひとりごとを言いたい度合いを0から100の整数で返してください。",
    "topicは話したいことがあれば短く、なければ空文字にしてください。",
    "Return JSON only. Do not include Markdown.",
    "Output shape: {\"mood\":\"ふつう\",\"talkDesire\":50,\"topic\":\"\"}.",
    "",
    "Persona:",
    JSON.stringify(
      {
        actorId: persona.actorId,
        displayName: persona.displayName,
        profile
      },
      null,
      2
    ),
    "",
    "Current context:",
    JSON.stringify(
      {
        currentTime: input?.currentTime,
        timePeriod: input?.timePeriod,
        secondsSinceLastUserActivity: input?.secondsSinceLastUserActivity
      },
      null,
      2
    ),
    "",
    "Recent context:",
    JSON.stringify(recentContext.slice(-20), null, 2)
  ].join("\n");
}

export function buildMoodEvaluateSystemPrompt() {
  return [
    "You are a mood.evaluate provider for Yuukei.",
    "Return only valid JSON.",
    "Never generate dialogue. Never include Markdown."
  ].join(" ");
}

function eventLanguageHint(event) {
  const text = event?.payload?.text;
  if (typeof text !== "string" || !text.trim()) {
    return "ja";
  }
  return /[ぁ-んァ-ン一-龯]/.test(text) ? "ja" : "follow input language";
}

function isNonEmptyString(value) {
  return typeof value === "string" && value.trim();
}

function formatMemoryDigest(events) {
  return events
    .map((event) => {
      const kind = typeof event?.kind === "string" ? event.kind : "";
      const timestamp = typeof event?.timestamp === "string" ? event.timestamp : "";
      const payload = event?.payload && typeof event.payload === "object" ? event.payload : {};
      const payloadText = Object.entries(payload)
        .filter(([, value]) => value === null || ["boolean", "number", "string"].includes(typeof value))
        .map(([key, value]) => `${key}=${String(value)}`)
        .join(", ");
      return `- ${timestamp} ${kind}${payloadText ? ` (${payloadText})` : ""}`;
    })
    .join("\n");
}
