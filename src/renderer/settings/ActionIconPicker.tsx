import type { JSX } from "react";

import { ActionIcon } from "../components/ActionIcon";
import { LUCIDE_ICON_NAMES, isLucideIconName } from "../components/lucideIconRegistry";

const POPULAR_ICONS = [
  "languages",
  "file-question",
  "scan-text",
  "search",
  "clipboard-copy",
  "wand-sparkles",
  "message-circle-question",
  "sparkles",
  "book-open",
  "brain",
  "code-xml",
  "message-square-text",
  "pen-line",
  "spell-check",
  "text-search",
  "whole-word",
] as const;

interface ActionIconPickerProps {
  value: string;
  onChange: (value: string) => void;
}

export function ActionIconPicker({ value, onChange }: ActionIconPickerProps): JSX.Element {
  const valid = isLucideIconName(value);

  return (
    <div className="icon-picker">
      <div className={`icon-picker__preview ${valid ? "" : "icon-picker__preview--invalid"}`}>
        <ActionIcon name={value} size={19} />
      </div>
      <div className="icon-picker__content">
        <input
          className="control"
          value={value}
          list="popper-lucide-icons"
          maxLength={64}
          spellCheck={false}
          aria-invalid={!valid}
          placeholder="例如 languages"
          onChange={(event) => onChange(event.target.value.trim().toLowerCase())}
        />
        <datalist id="popper-lucide-icons">
          {LUCIDE_ICON_NAMES.map((name) => (
            <option value={name} key={name} />
          ))}
        </datalist>
        <div className="icon-picker__popular" aria-label="常用图标">
          {POPULAR_ICONS.map((name) => (
            <button
              className={value === name ? "is-selected" : ""}
              type="button"
              title={name}
              aria-label={`选择图标 ${name}`}
              aria-pressed={value === name}
              key={name}
              onClick={() => onChange(name)}
            >
              <ActionIcon name={name} size={16} />
            </button>
          ))}
        </div>
        {!valid && (
          <span className="field__hint field__hint--error">请选择有效的 Lucide 图标名称。</span>
        )}
      </div>
    </div>
  );
}
