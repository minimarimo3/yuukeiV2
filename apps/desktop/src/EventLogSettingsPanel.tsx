import {
  eventKindLabel,
  eventLogSummary,
  eventPrivacyLabel,
  formatEventLogTimestamp,
} from "./appShared";
import type {
  EventLogPage,
  EventLogPrivacyCategoryFilter,
} from "./yuukeiClient";

export type EventLogSettingsPanelProps = {
  page: EventLogPage | null;
  error: string | null;
  loading: boolean;
  kindPrefix: string;
  privacyFilter: EventLogPrivacyCategoryFilter;
  deleteBefore: string;
  deletePrefix: string;
  onKindPrefixChange: (value: string) => void;
  onPrivacyFilterChange: (value: EventLogPrivacyCategoryFilter) => void;
  onDeleteBeforeChange: (value: string) => void;
  onDeletePrefixChange: (value: string) => void;
  onApplyFilters: () => void;
  onLoadMore: () => void;
  onRefresh: () => void;
  onDeleteBefore: () => void;
  onDeletePrefix: () => void;
  onDeleteAll: () => void;
};

export function EventLogSettingsPanel({
  page,
  error,
  loading,
  kindPrefix,
  privacyFilter,
  deleteBefore,
  deletePrefix,
  onKindPrefixChange,
  onPrivacyFilterChange,
  onDeleteBeforeChange,
  onDeletePrefixChange,
  onApplyFilters,
  onLoadMore,
  onRefresh,
  onDeleteBefore,
  onDeletePrefix,
  onDeleteAll,
}: EventLogSettingsPanelProps) {
  const records = page?.records ?? [];
  return (
    <>
      <div className="memory-copy event-log-copy">
        <section className="memory-section">
          <div className="memory-section-head">
            <div>
              <h3>生活の記録</h3>
              <p className="settings-note">
                住人との会話や、住人が気づいたことを確認できます。難しいデータ形式は表示しません。
              </p>
            </div>
            <span>
              {page
                ? `${page.total}件中 ${records.length}件を表示`
                : "読み込み中"}
            </span>
          </div>
          {error ? <p className="settings-error">{error}</p> : null}
          <div className="event-log-filters">
            <label>
              <span>表示するできごと</span>
              <select
                value={kindPrefix}
                onChange={(event) =>
                  onKindPrefixChange(event.currentTarget.value)
                }
              >
                <option value="">すべて</option>
                <option value="conversation.">あなたからの会話</option>
                <option value="dialogue.">住人からの会話</option>
                <option value="desktop.">パソコン上で気づいたこと</option>
                <option value="presence.">住人の日々のようす</option>
                <option value="memory.">記憶の変化</option>
                <option value="scene.">物語の進行</option>
              </select>
            </label>
            <label>
              <span>記録のきっかけ</span>
              <select
                value={privacyFilter}
                onChange={(event) =>
                  onPrivacyFilterChange(
                    event.currentTarget.value as EventLogPrivacyCategoryFilter,
                  )
                }
              >
                <option value="all">すべて</option>
                <option value="desktopObservation">パソコン上の変化</option>
                <option value="none">住人とのやりとり</option>
              </select>
            </label>
            <button
              type="button"
              className="secondary-button compact-button"
              disabled={loading}
              onClick={onApplyFilters}
            >
              適用
            </button>
          </div>
          <div className="memory-list event-log-list">
            {records.map((record) => (
              <article className="memory-row event-log-row" key={record.id}>
                <div className="memory-text">
                  <p>{eventLogSummary(record)}</p>
                  <dl className="event-log-meta">
                    <div>
                      <dt>できごと</dt>
                      <dd>{eventKindLabel(record.type)}</dd>
                    </div>
                    <div>
                      <dt>日時</dt>
                      <dd>{formatEventLogTimestamp(record.timestamp)}</dd>
                    </div>
                    <div>
                      <dt>記録元</dt>
                      <dd>{eventPrivacyLabel(record.privacy?.category)}</dd>
                    </div>
                  </dl>
                </div>
              </article>
            ))}
            {records.length === 0 ? (
              <p className="settings-note">表示できる記録はありません。</p>
            ) : null}
          </div>
          {page?.nextCursor ? (
            <button
              type="button"
              className="secondary-button memory-more-button"
              disabled={loading}
              onClick={onLoadMore}
            >
              もっと見る
            </button>
          ) : null}
        </section>
        <section className="memory-section event-log-delete danger-zone-section">
          <div className="memory-section-head">
            <h3>削除</h3>
          </div>
          <label>
            <span>この日時より前</span>
            <input
              type="datetime-local"
              value={deleteBefore}
              onChange={(event) =>
                onDeleteBeforeChange(event.currentTarget.value)
              }
            />
            <button type="button" disabled={loading} onClick={onDeleteBefore}>
              期間指定で削除
            </button>
          </label>
          <label>
            <span>できごとを選んで削除</span>
            <select
              value={deletePrefix}
              onChange={(event) =>
                onDeletePrefixChange(event.currentTarget.value)
              }
            >
              <option value="">選択してください</option>
              <option value="conversation.">あなたからの会話</option>
              <option value="dialogue.">住人からの会話</option>
              <option value="desktop.">パソコン上で気づいたこと</option>
              <option value="presence.">住人の日々のようす</option>
              <option value="memory.">記憶の変化</option>
              <option value="scene.">物語の進行</option>
            </select>
            <button type="button" disabled={loading} onClick={onDeletePrefix}>
              選んだ記録を削除
            </button>
          </label>
        </section>
      </div>
      <div className="settings-actions memory-panel-actions">
        <button type="button" onClick={onRefresh} disabled={loading}>
          更新
        </button>
        <button
          type="button"
          className="danger-button"
          onClick={onDeleteAll}
          disabled={loading}
        >
          すべて削除
        </button>
      </div>
    </>
  );
}
