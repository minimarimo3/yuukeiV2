import type { ExtensionSettingField } from "@yuukei/protocol";
import { useEffect, useState } from "react";
import type {
  ExtensionCapabilityUsage,
  ExtensionSettingsChangeResult,
  InstalledExtension,
  YuukeiClient,
} from "./yuukeiClient";

export type ExtensionSettingsFormProps = {
  extension: InstalledExtension;
  client: YuukeiClient;
  disabled: boolean;
  onResult: (result: ExtensionSettingsChangeResult) => void;
};

export type ExtensionUsageSectionProps = {
  usage?: ExtensionCapabilityUsage;
};

export function ExtensionUsageSection({ usage }: ExtensionUsageSectionProps) {
  const rows =
    usage?.capabilities.flatMap((capability) =>
      capability.models.map((model) => ({
        capability: capability.capability,
        ...model,
      })),
    ) ?? [];
  if (rows.length === 0) {
    return null;
  }

  return (
    <section
      className="extension-usage"
      aria-label={`${usage?.extensionId ?? "extension"} token usage`}
    >
      <h3>トークン使用量</h3>
      <div className="extension-usage-table">
        <div className="extension-usage-row extension-usage-head">
          <span>capability / model</span>
          <span>全期間</span>
          <span>直近7日</span>
        </div>
        {rows.map((row) => (
          <div
            className="extension-usage-row"
            key={`${row.capability}:${row.provider}:${row.model}`}
          >
            <span>
              <strong>{row.capability}</strong>
              <small>
                {row.provider} / {row.model}
              </small>
            </span>
            <TokenUsageTotalsView totals={row.allTime} />
            <TokenUsageTotalsView totals={row.last7Days} />
          </div>
        ))}
      </div>
    </section>
  );
}

export type TokenUsageTotalsViewProps = {
  totals: {
    requests: number;
    inputTokens: number;
    outputTokens: number;
  };
};

export function TokenUsageTotalsView({ totals }: TokenUsageTotalsViewProps) {
  return (
    <span className="extension-usage-totals">
      <span>リクエスト {formatNumber(totals.requests)}</span>
      <span>入力 {formatNumber(totals.inputTokens)}</span>
      <span>出力 {formatNumber(totals.outputTokens)}</span>
    </span>
  );
}

export function ExtensionSettingsForm({
  extension,
  client,
  disabled,
  onResult,
}: ExtensionSettingsFormProps) {
  const [draft, setDraft] = useState<Record<string, unknown>>(() =>
    initialSettingDraft(extension),
  );
  const [secretDraft, setSecretDraft] = useState<Record<string, string>>({});
  const [dirtyKeys, setDirtyKeys] = useState<Set<string>>(() => new Set());
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: 編集中ドラフトを保護するため、リセット条件をid/schema/valuesの変化に限定する意図的設計
  useEffect(() => {
    setDraft(initialSettingDraft(extension));
    setSecretDraft({});
    setDirtyKeys(new Set());
    setError(null);
  }, [
    extension.extensionId,
    extension.settingsSchema,
    extension.settingValues,
  ]);

  const fields = extension.settingsSchema?.fields ?? [];
  const visibleFields = fields.filter((field) => fieldIsVisible(field, draft));

  async function saveSettings() {
    setSaving(true);
    setError(null);
    try {
      const nonSecretValues: Record<string, unknown> = {};
      for (const field of fields) {
        if (field.type === "secret") continue;
        const hasSavedValue = Object.hasOwn(extension.settingValues, field.key);
        if (!hasSavedValue && !dirtyKeys.has(field.key)) continue;
        if (
          hasSavedValue &&
          dirtyKeys.has(field.key) &&
          valuesEqual(draft[field.key], fieldDefaultValue(field))
        ) {
          nonSecretValues[field.key] = null;
        } else {
          nonSecretValues[field.key] = draft[field.key] ?? null;
        }
      }
      let result = await client.setExtensionSettingValues(
        extension.extensionId,
        nonSecretValues,
      );
      for (const field of fields) {
        if (field.type !== "secret") continue;
        const value = secretDraft[field.key];
        if (value && value.length > 0) {
          result = await client.setExtensionSecret(
            extension.extensionId,
            field.key,
            value,
          );
        }
      }
      setSecretDraft({});
      setDirtyKeys(new Set());
      onResult(result);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }

  async function clearSecret(key: string) {
    setSaving(true);
    setError(null);
    try {
      const result = await client.setExtensionSecret(
        extension.extensionId,
        key,
        null,
      );
      setSecretDraft((current) => ({ ...current, [key]: "" }));
      onResult(result);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section
      className="extension-settings-form"
      aria-label={`${extension.displayName} settings`}
    >
      {visibleFields.map((field) => (
        <ExtensionSettingControl
          key={field.key}
          field={field}
          value={draft[field.key]}
          secretValue={secretDraft[field.key] ?? ""}
          secretSet={extension.secretsSet.includes(field.key)}
          disabled={disabled || saving}
          onValueChange={(value) => {
            setDraft((current) => ({ ...current, [field.key]: value }));
            setDirtyKeys((current) => new Set(current).add(field.key));
          }}
          onSecretChange={(value) =>
            setSecretDraft((current) => ({ ...current, [field.key]: value }))
          }
          onSecretClear={() => clearSecret(field.key)}
        />
      ))}
      {error ? <p className="settings-error">{error}</p> : null}
      <div className="extension-settings-actions">
        <button
          type="button"
          className="secondary-button compact-button"
          disabled={disabled || saving}
          onClick={saveSettings}
        >
          保存
        </button>
      </div>
    </section>
  );
}

export type ExtensionSettingControlProps = {
  field: ExtensionSettingField;
  value: unknown;
  secretValue: string;
  secretSet: boolean;
  disabled: boolean;
  onValueChange: (value: unknown) => void;
  onSecretChange: (value: string) => void;
  onSecretClear: () => void;
};

export function ExtensionSettingControl({
  field,
  value,
  secretValue,
  secretSet,
  disabled,
  onValueChange,
  onSecretChange,
  onSecretClear,
}: ExtensionSettingControlProps) {
  const id = `extension-setting-${field.key.replace(/[^A-Za-z0-9_-]/g, "-")}`;
  return (
    <label className="extension-setting-field" htmlFor={id}>
      <span>
        <strong>{field.label}</strong>
        {"description" in field && field.description ? (
          <small>{field.description}</small>
        ) : null}
      </span>
      {field.type === "string" ? (
        <input
          id={id}
          type="text"
          value={typeof value === "string" ? value : ""}
          disabled={disabled}
          onChange={(event) => onValueChange(event.currentTarget.value)}
        />
      ) : null}
      {field.type === "number" ? (
        <input
          id={id}
          type="number"
          value={typeof value === "number" ? String(value) : ""}
          min={field.min}
          max={field.max}
          disabled={disabled}
          onChange={(event) => {
            const next = event.currentTarget.value;
            onValueChange(next === "" ? null : Number(next));
          }}
        />
      ) : null}
      {field.type === "boolean" ? (
        <input
          id={id}
          type="checkbox"
          checked={Boolean(value)}
          disabled={disabled}
          onChange={(event) => onValueChange(event.currentTarget.checked)}
        />
      ) : null}
      {field.type === "select" ? (
        <select
          id={id}
          value={typeof value === "string" ? value : ""}
          disabled={disabled}
          onChange={(event) => onValueChange(event.currentTarget.value)}
        >
          {field.options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      ) : null}
      {field.type === "secret" ? (
        <span className="extension-secret-control">
          <input
            id={id}
            type="password"
            value={secretValue}
            placeholder={secretSet ? "設定済み" : ""}
            disabled={disabled}
            onChange={(event) => onSecretChange(event.currentTarget.value)}
          />
          {secretSet ? (
            <button
              type="button"
              className="secondary-button compact-button"
              disabled={disabled}
              onClick={onSecretClear}
            >
              クリア
            </button>
          ) : null}
        </span>
      ) : null}
    </label>
  );
}

function initialSettingDraft(
  extension: InstalledExtension,
): Record<string, unknown> {
  const draft: Record<string, unknown> = {};
  for (const field of extension.settingsSchema?.fields ?? []) {
    if (field.type === "secret") continue;
    draft[field.key] =
      extension.settingValues[field.key] ?? fieldDefaultValue(field) ?? null;
  }
  return draft;
}

function fieldDefaultValue(field: ExtensionSettingField): unknown {
  if ("default" in field) {
    return field.default;
  }
  return undefined;
}

function valuesEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat("ja-JP").format(value);
}

function fieldIsVisible(
  field: ExtensionSettingField,
  values: Record<string, unknown>,
): boolean {
  if (!("visibleWhen" in field) || !field.visibleWhen) {
    return true;
  }
  return values[field.visibleWhen.key] === field.visibleWhen.equals;
}
