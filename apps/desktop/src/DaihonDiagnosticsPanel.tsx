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
    <section className="daihon-diagnostics" aria-label="読み込みの問題">
      <div className="daihon-diagnostics-head">
        <h3>読み込めなかった内容 {diagnostics.length}件</h3>
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
            <strong>{daihonPhaseLabel(diagnostic.phase)}</strong>
            <p>配布元の案内を確認するか、別の住人と世界を選んでください。</p>
          </li>
        ))}
      </ol>
    </section>
  );
}

function daihonPhaseLabel(phase: DaihonDiagnosticEntry["phase"]): string {
  switch (phase) {
    case "loadParse":
      return "台詞や動きの書き方に問題があります";
    case "loadValidate":
      return "内容を確認できませんでした";
    case "loadSpeaker":
      return "話す住人を確認できませんでした";
    case "runtimeValidate":
      return "場面を始められませんでした";
    case "runtimeExecute":
      return "場面の途中で問題が起きました";
  }
}
