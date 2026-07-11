import { eventLogSummary, formatEventLogTimestamp } from "./appShared";
import type {
  EventLogPage,
  EventLogPrivacyCategoryFilter
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
  onDeleteAll
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
                保存されているイベントの種類と内容を確認できます。
              </p>
            </div>
            <span>{page ? `${records.length} / ${page.total}` : "loading"}</span>
          </div>
          {error ? <p className="settings-error">{error}</p> : null}
          <div className="event-log-filters">
            <label>
              <span>種類</span>
              <input
                type="text"
                value={kindPrefix}
                placeholder="desktop."
                onChange={(event) => onKindPrefixChange(event.currentTarget.value)}
              />
            </label>
            <label>
              <span>privacy</span>
              <select
                value={privacyFilter}
                onChange={(event) =>
                  onPrivacyFilterChange(
                    event.currentTarget.value as EventLogPrivacyCategoryFilter
                  )
                }
              >
                <option value="all">すべて</option>
                <option value="desktopObservation">desktop-observation</option>
                <option value="none">なし</option>
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
                  <small>
                    {formatEventLogTimestamp(record.timestamp)} / {record.kind} /{" "}
                    {record.privacy?.category ?? "privacyなし"}
                  </small>
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
        <section className="memory-section event-log-delete">
          <div className="memory-section-head">
            <h3>削除</h3>
          </div>
          <label>
            <span>この日時より前</span>
            <input
              type="datetime-local"
              value={deleteBefore}
              onChange={(event) => onDeleteBeforeChange(event.currentTarget.value)}
            />
            <button type="button" disabled={loading} onClick={onDeleteBefore}>
              期間指定で削除
            </button>
          </label>
          <label>
            <span>種類の前方一致</span>
            <input
              type="text"
              value={deletePrefix}
              placeholder="desktop."
              onChange={(event) => onDeletePrefixChange(event.currentTarget.value)}
            />
            <button type="button" disabled={loading} onClick={onDeletePrefix}>
              種類指定で削除
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
          className="secondary-button"
          onClick={onDeleteAll}
          disabled={loading}
        >
          全削除
        </button>
      </div>
    </>
  );
}
