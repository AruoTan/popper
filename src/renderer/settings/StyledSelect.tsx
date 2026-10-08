import { Check, ChevronDown } from "lucide-react";
import { createPortal } from "react-dom";
import {
  useEffect, useId, useLayoutEffect, useRef, useState,
  type CSSProperties, type JSX, type KeyboardEvent,
} from "react";

export interface SelectOption {
  value: string;
  label: string;
  group?: string;
}

interface StyledSelectProps {
  options: readonly SelectOption[];
  value: string;
  onChange: (value: string) => void;
  className?: string;
  "aria-label": string;
  disabled?: boolean;
  editable?: boolean;
  invalid?: boolean;
  placeholder?: string;
  maxLength?: number;
}

export function StyledSelect({
  options, value, onChange, className, "aria-label": ariaLabel,
  disabled = false, editable = false, invalid, placeholder, maxLength,
}: StyledSelectProps): JSX.Element {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [searching, setSearching] = useState(false);
  const [position, setPosition] = useState<CSSProperties>({});
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement | HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const typeahead = useRef({ text: "", time: 0 });
  const filtered = editable && searching
    ? options.filter((option) => option.label.toLowerCase().includes(value.toLowerCase()))
    : options;
  // Typing searches the complete icon catalog without mounting every entry.
  const selectedIndex = options.findIndex((option) => option.value === value);
  const pageStart = editable && !searching && selectedIndex >= 100 ? selectedIndex - 50 : 0;
  const visibleOptions = editable ? filtered.slice(pageStart, pageStart + 100) : filtered;
  const activeOption = visibleOptions[activeIndex];
  const selectedLabel = options.find((option) => option.value === value)?.label ?? value;

  function close(): void {
    setOpen(false);
    setActiveIndex(-1);
    setSearching(false);
  }

  function show(): void {
    if (disabled) return;
    setOpen(true);
    setSearching(false);
    setActiveIndex(selectedIndex >= 0 ? selectedIndex - pageStart : 0);
  }

  function select(option: SelectOption): void {
    onChange(option.value);
    close();
    triggerRef.current?.focus();
  }

  useEffect(() => {
    if (disabled) close();
  }, [disabled]);

  useLayoutEffect(() => {
    if (!open) return;
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const viewport = window.visualViewport;
    const width = viewport?.width ?? window.innerWidth;
    const height = viewport?.height ?? window.innerHeight;
    const offsetX = viewport?.offsetLeft ?? 0;
    const offsetY = viewport?.offsetTop ?? 0;
    const below = height + offsetY - rect.bottom - 12;
    const above = rect.top - offsetY - 12;
    const upwards = below < 240 && above > below;
    const menuWidth = Math.min(Math.max(rect.width, 180), width - 16);
    setPosition({
      left: Math.max(offsetX + 8, Math.min(rect.left, offsetX + width - menuWidth - 8)),
      width: menuWidth,
      maxHeight: Math.max(0, Math.min(280, upwards ? above : below)),
      ...(upwards ? { bottom: window.innerHeight - rect.top + 5 } : { top: rect.bottom + 5 }),
    });
  }, [open, visibleOptions.length]);

  useEffect(() => {
    if (!open) return;
    function outside(event: PointerEvent): void {
      const target = event.target as Node;
      if (!containerRef.current?.contains(target) && !listRef.current?.contains(target)) close();
    }
    function scroll(event: Event): void {
      if (event.target instanceof Node && listRef.current?.contains(event.target)) return;
      close();
    }
    document.addEventListener("pointerdown", outside);
    window.addEventListener("scroll", scroll, true);
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    window.visualViewport?.addEventListener("resize", close);
    window.visualViewport?.addEventListener("scroll", close);
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("scroll", scroll, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
      window.visualViewport?.removeEventListener("resize", close);
      window.visualViewport?.removeEventListener("scroll", close);
    };
  }, [open]);

  useEffect(() => {
    if (open && activeIndex >= 0) {
      document.getElementById(`${id}-option-${activeIndex}`)?.scrollIntoView?.({ block: "nearest" });
    }
  }, [open, activeIndex, id]);

  function handleKeyDown(event: KeyboardEvent): void {
    if (disabled) return;
    if (event.key === "Escape" && open) {
      event.preventDefault();
      event.stopPropagation(); // Close the menu before its containing dialog.
      close();
      return;
    }
    if (event.key === "Tab") { close(); return; }
    if (["ArrowDown", "ArrowUp", "Enter"].includes(event.key) || (!editable && event.key === " ")) {
      event.preventDefault();
      if (!open) { show(); return; }
      if (event.key === "Enter" || event.key === " ") {
        if (activeOption) select(activeOption);
        else close();
      } else if (visibleOptions.length) {
        const direction = event.key === "ArrowDown" ? 1 : -1;
        setActiveIndex((current) => (current + direction + visibleOptions.length) % visibleOptions.length);
      }
      return;
    }
    if (open && !editable && (event.key === "Home" || event.key === "End")) {
      event.preventDefault();
      setActiveIndex(event.key === "Home" ? 0 : visibleOptions.length - 1);
      return;
    }
    if (!editable && event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      const now = Date.now();
      const previous = now - typeahead.current.time < 700 ? typeahead.current.text : "";
      const text = previous + event.key.toLowerCase();
      typeahead.current = { text, time: now };
      if (!open) show();
      const index = options.findIndex((option) => option.label.toLowerCase().startsWith(text));
      if (index >= 0) setActiveIndex(index);
    }
  }

  const accessibility = {
    role: "combobox",
    "aria-label": ariaLabel,
    "aria-expanded": open,
    "aria-controls": open ? `${id}-listbox` : undefined,
    "aria-activedescendant": open && activeOption ? `${id}-option-${activeIndex}` : undefined,
    "aria-haspopup": "listbox" as const,
    disabled,
  };

  return (
    <div
      className={["styled-select", className].filter(Boolean).join(" ")}
      ref={containerRef}
      onKeyDown={handleKeyDown}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget) && !listRef.current?.contains(event.relatedTarget)) close();
      }}
    >
      {editable ? (
        <>
          <input
            {...accessibility}
            ref={(node) => { triggerRef.current = node; }}
            className={`control styled-select__input${open ? " is-open" : ""}`}
            aria-autocomplete="list"
            aria-invalid={invalid}
            autoComplete="off"
            spellCheck={false}
            placeholder={placeholder}
            maxLength={maxLength}
            value={value}
            onClick={() => { if (!open) show(); }}
            onChange={(event) => {
              onChange(event.target.value.trim().toLowerCase());
              setSearching(true);
              setOpen(true);
              setActiveIndex(0);
            }}
          />
          <button
            type="button"
            className="styled-select__toggle"
            aria-label={`${open ? "收起" : "展开"}${ariaLabel}候选`}
            tabIndex={-1}
            disabled={disabled}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => { triggerRef.current?.focus(); if (open) close(); else show(); }}
          >
            <ChevronDown size={12} className="dropdown-chevron" aria-hidden="true" />
          </button>
        </>
      ) : (
        <button
          {...accessibility}
          ref={(node) => { triggerRef.current = node; }}
          type="button"
          className={`styled-select__trigger control${open ? " is-open" : ""}`}
          onClick={() => { if (open) close(); else show(); }}
        >
          <span className="styled-select__value">{selectedLabel}</span>
          <ChevronDown size={12} className="dropdown-chevron" aria-hidden="true" />
        </button>
      )}
      {open && createPortal(
        <ul
          id={`${id}-listbox`}
          className="styled-select__dropdown"
          role="listbox"
          aria-label={ariaLabel}
          ref={listRef}
          style={position}
        >
          {visibleOptions.map((option, index) => (
            <li role="presentation" key={option.value}>
              {option.group && option.group !== visibleOptions[index - 1]?.group && (
                <div className="styled-select__group" role="presentation">{option.group}</div>
              )}
              <div
                id={`${id}-option-${index}`}
                role="option"
                aria-selected={option.value === value}
                className={[
                  "styled-select__option", option.value === value ? "is-selected" : "",
                  index === activeIndex ? "is-focused" : "",
                ].filter(Boolean).join(" ")}
                onMouseEnter={() => setActiveIndex(index)}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => select(option)}
              >
                <span className="styled-select__option-label">{option.label}</span>
                {option.value === value && <Check size={14} className="styled-select__check" aria-hidden="true" />}
              </div>
            </li>
          ))}
          {visibleOptions.length === 0 && <li className="styled-select__hint" role="presentation">没有匹配的选项</li>}
          {filtered.length > visibleOptions.length && <li className="styled-select__hint" role="presentation">输入名称以筛选更多图标</li>}
        </ul>, document.body,
      )}
    </div>
  );
}
