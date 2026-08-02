import { formatMemoryTimestamp } from "./appShared";
import type { MemoryEntryKind, ResidentMemoryState } from "./yuukeiClient";

export type MemorySettingsPanelProps = {
  memoryState: ResidentMemoryState | null;
  memoryError: string | null;
  loading: boolean;
  editingFactId: string | null;
  editingFactText: string;
  onBeginFactEdit: (id: string, text: string) => void;
  onCancelFactEdit: () => void;
  onFactDraftChange: (text: string) => void;
  onSaveFact: (id: string) => Promise<void>;
  onForgetEntry: (kind: MemoryEntryKind, id: string) => Promise<void>;
  onForgetAll: () => Promise<void>;
  onLoadMore: () => Promise<void>;
  onRefresh: () => Promise<void>;
};

export function MemorySettingsPanel({
  memoryState,
  memoryError,
  loading,
  editingFactId,
  editingFactText,
  onBeginFactEdit,
  onCancelFactEdit,
  onFactDraftChange,
  onSaveFact,
  onForgetEntry,
  onForgetAll,
  onLoadMore,
  onRefresh,
}: MemorySettingsPanelProps) {
  const facts = memoryState?.facts ?? [];
  const episodes = memoryState?.episodes ?? [];
  const episodeTotal = memoryState?.episodeTotal ?? 0;
  const hasMemory = facts.length > 0 || episodeTotal > 0;
  const hasMoreEpisodes = episodes.length < episodeTotal;

  return (
    <>
      <div className="settings-copy memory-copy">
        <h2>住人の記憶</h2>
        <p className="settings-title">住人が覚えていること</p>
        <p className="settings-note">
          あなたの好みなどの長く覚えていることは編集できます。思い出は内容を変えず、忘れさせることだけができます。
        </p>
        {memoryError ? <p className="settings-error">{memoryError}</p> : null}
        {!memoryError && !loading && !hasMemory ? (
          <p className="settings-note">まだ記憶がありません。</p>
        ) : null}

        <section className="memory-section" aria-label="長く覚えていること">
          <div className="memory-section-head">
            <h3>長く覚えていること</h3>
            <span>{facts.length}</span>
          </div>
          <div className="memory-list">
            {facts.map((fact) => {
              const editing = editingFactId === fact.id;
              return (
                <article className="memory-row" key={fact.id}>
                  {editing ? (
                    <textarea
                      aria-label="覚えていることを編集"
                      value={editingFactText}
                      maxLength={500}
                      onChange={(event) =>
                        onFactDraftChange(event.currentTarget.value)
                      }
                    />
                  ) : (
                    <div className="memory-text">
                      <p>{fact.text}</p>
                      <small>{formatMemoryTimestamp(fact.updatedAt)}</small>
                    </div>
                  )}
                  <div className="memory-actions">
                    {editing ? (
                      <>
                        <button
                          type="button"
                          className="compact-button"
                          disabled={loading}
                          onClick={() => void onSaveFact(fact.id)}
                        >
                          保存
                        </button>
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={loading}
                          onClick={onCancelFactEdit}
                        >
                          キャンセル
                        </button>
                      </>
                    ) : (
                      <>
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={loading}
                          onClick={() => onBeginFactEdit(fact.id, fact.text)}
                        >
                          編集
                        </button>
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={loading}
                          onClick={() => void onForgetEntry("fact", fact.id)}
                        >
                          削除
                        </button>
                      </>
                    )}
                  </div>
                </article>
              );
            })}
          </div>
        </section>

        <section className="memory-section" aria-label="最近の思い出">
          <div className="memory-section-head">
            <h3>最近の思い出</h3>
            <span>
              {episodes.length}/{episodeTotal}
            </span>
          </div>
          <div className="memory-list">
            {episodes.map((episode) => (
              <article className="memory-row" key={episode.id}>
                <div className="memory-text">
                  <p>{episode.text}</p>
                  <small>{formatMemoryTimestamp(episode.timestamp)}</small>
                </div>
                <div className="memory-actions">
                  <button
                    type="button"
                    className="secondary-button compact-button"
                    disabled={loading}
                    onClick={() => void onForgetEntry("episode", episode.id)}
                  >
                    削除
                  </button>
                </div>
              </article>
            ))}
          </div>
          {hasMoreEpisodes ? (
            <button
              type="button"
              className="secondary-button memory-more-button"
              disabled={loading}
              onClick={() => void onLoadMore()}
            >
              もっと見る
            </button>
          ) : null}
        </section>
      </div>
      <div className="settings-actions memory-panel-actions">
        <button
          type="button"
          onClick={() => void onRefresh()}
          disabled={loading}
        >
          更新
        </button>
        <button
          type="button"
          className="secondary-button"
          onClick={() => void onForgetAll()}
          disabled={loading || !hasMemory}
        >
          すべて忘れる
        </button>
      </div>
    </>
  );
}
