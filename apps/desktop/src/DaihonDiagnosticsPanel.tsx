import type { DaihonDiagnosticEntry } from "./yuukeiClient";

export type DaihonDiagnosticsPanelProps = {
  diagnostics: DaihonDiagnosticEntry[];
  expanded: boolean;
  onToggle: () => void;
};

export function DaihonDiagnosticsPanel({
  diagnostics,
  expanded,
  onToggle,
}: DaihonDiagnosticsPanelProps) {
  if (diagnostics.length === 0) {
    return null;
  }

  const collapsed = diagnostics.length >= 5 && !expanded;
  const visibleDiagnostics = collapsed ? diagnostics.slice(0, 4) : diagnostics;

  return (
    <section className="daihon-diagnostics" aria-label="Daihon errors">
      <div className="daihon-diagnostics-head">
        <h3>Daihon エラー {diagnostics.length}件</h3>
        {diagnostics.length >= 5 ? (
          <button type="button" onClick={onToggle}>
            {expanded ? "折りたたむ" : "すべて表示"}
          </button>
        ) : null}
      </div>
      <ol className="daihon-diagnostic-list">
        {visibleDiagnostics.map((diagnostic, index) => (
          <li
            className={`daihon-diagnostic-row is-${diagnostic.severity}`}
            key={[
              diagnostic.occurredAt,
              diagnostic.code,
              diagnostic.scriptPath,
              diagnostic.line,
              diagnostic.column,
              index,
            ].join(":")}
          >
            <div className="daihon-diagnostic-meta">
              <span>{daihonPhaseLabel(diagnostic.phase)}</span>
              <span>{daihonLocationLabel(diagnostic)}</span>
            </div>
            <strong>{diagnostic.message}</strong>
            <small>{diagnostic.code}</small>
            {diagnostic.help ? <p>{diagnostic.help}</p> : null}
            {diagnostic.sourceEventType ? (
              <small>
                {diagnostic.sourceEventType}
                {diagnostic.sourceEventId
                  ? ` / ${diagnostic.sourceEventId}`
                  : ""}
              </small>
            ) : null}
          </li>
        ))}
      </ol>
    </section>
  );
}

function daihonPhaseLabel(phase: DaihonDiagnosticEntry["phase"]): string {
  switch (phase) {
    case "loadParse":
      return "ロード/構文";
    case "loadValidate":
      return "ロード/検証";
    case "loadSpeaker":
      return "ロード/話者";
    case "runtimeValidate":
      return "実行/検証";
    case "runtimeExecute":
      return "実行";
  }
}

function daihonLocationLabel(diagnostic: DaihonDiagnosticEntry): string {
  const path = diagnostic.scriptPath ?? diagnostic.packRoot ?? "unknown";
  if (diagnostic.line && diagnostic.column) {
    return `${path}:${diagnostic.line}:${diagnostic.column}`;
  }
  if (diagnostic.line) {
    return `${path}:${diagnostic.line}`;
  }
  return path;
}
