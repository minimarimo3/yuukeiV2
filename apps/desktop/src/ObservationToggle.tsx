export type ObservationToggleProps = {
  label: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
};

export function ObservationToggle({
  label,
  description,
  checked,
  disabled,
  onChange,
}: ObservationToggleProps) {
  return (
    <label className="extension-toggle observation-toggle">
      <input
        type="checkbox"
        aria-label={label}
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
      <span>
        <strong>{label}</strong>
        <small>{description}</small>
      </span>
    </label>
  );
}
