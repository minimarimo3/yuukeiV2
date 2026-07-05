export function buildDialoguePrompt(input) {
  const maxLength = input?.constraints?.maxLength ?? 120;
  const persona = input?.persona ?? {};
  const event = input?.event ?? {};
  const recentContext = Array.isArray(input?.recentContext) ? input.recentContext : [];
  const profile = persona.profile && typeof persona.profile === "object" ? persona.profile : {};
  const languageHint = eventLanguageHint(event);

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

function eventLanguageHint(event) {
  const text = event?.payload?.text;
  if (typeof text !== "string" || !text.trim()) {
    return "ja";
  }
  return /[ぁ-んァ-ン一-龯]/.test(text) ? "ja" : "follow input language";
}
