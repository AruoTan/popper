import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";

import { StyledSelect, type SelectOption } from "./StyledSelect";

const options: SelectOption[] = [
  { value: "zh-CN", label: "简体中文" },
  { value: "en-US", label: "English" },
  { value: "ja-JP", label: "日本語" },
];

function Picker({ editable = false }: { editable?: boolean }) {
  const [value, setValue] = useState("zh-CN");
  return <StyledSelect aria-label="语言" options={options} value={value} onChange={setValue} editable={editable} />;
}

describe("StyledSelect", () => {
  it("opens above clipping containers, marks the selection and commits a clicked option", () => {
    const { container } = render(<Picker />);
    const trigger = screen.getByRole("combobox", { name: "语言" });
    fireEvent.click(trigger);
    const list = screen.getByRole("listbox", { name: "语言" });
    expect(container.contains(list)).toBe(false);
    expect(trigger).toHaveAttribute("aria-controls", list.id);
    expect(screen.getByRole("option", { name: "简体中文" })).toHaveAttribute("aria-selected", "true");
    fireEvent.click(screen.getByRole("option", { name: "English" }));
    expect(trigger).toHaveTextContent("English");
    expect(trigger).toHaveFocus();
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    fireEvent.click(trigger);
    expect(screen.getByRole("option", { name: "English" })).toHaveAttribute("aria-selected", "true");
  });

  it("supports arrows, Home/End, typeahead and confirmation without changing value on navigation", () => {
    render(<Picker />);
    const trigger = screen.getByRole("combobox");
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "End" });
    expect(trigger).toHaveTextContent("简体中文");
    expect(document.getElementById(trigger.getAttribute("aria-activedescendant")!)).toHaveTextContent("日本語");
    fireEvent.keyDown(trigger, { key: "Home" });
    fireEvent.keyDown(trigger, { key: "ArrowUp" });
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(trigger).toHaveTextContent("日本語");
    fireEvent.keyDown(trigger, { key: "e" });
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(trigger).toHaveTextContent("English");
  });

  it("dismisses on outside pointer, focus loss, Tab, resize and anchor scroll, but allows menu scrolling", () => {
    render(<><Picker /><button>外部</button></>);
    const trigger = screen.getByRole("combobox");
    const outside = screen.getByRole("button", { name: "外部" });
    const dismissals = [
      () => fireEvent.pointerDown(outside),
      () => fireEvent.blur(trigger, { relatedTarget: outside }),
      () => fireEvent.keyDown(trigger, { key: "Tab" }),
      () => fireEvent.resize(window),
      () => fireEvent.scroll(document),
      () => fireEvent.blur(window),
    ];
    for (const dismiss of dismissals) {
      fireEvent.click(trigger);
      fireEvent.scroll(screen.getByRole("listbox"));
      expect(screen.getByRole("listbox")).toBeInTheDocument();
      dismiss();
      expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    }
  });

  it("does not open while disabled and removes an open portal when disabled or unmounted", () => {
    const props = { "aria-label": "语言", options, value: "zh-CN", onChange: vi.fn() };
    const view = render(<StyledSelect {...props} disabled />);
    fireEvent.click(screen.getByRole("combobox"));
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    view.rerender(<StyledSelect {...props} />);
    fireEvent.click(screen.getByRole("combobox"));
    view.rerender(<StyledSelect {...props} disabled />);
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    view.rerender(<StyledSelect {...props} />);
    fireEvent.click(screen.getByRole("combobox"));
    view.unmount();
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("opens upwards near the bottom and keeps wide menus inside the viewport", () => {
    render(<Picker />);
    const trigger = screen.getByRole("combobox");
    vi.spyOn(trigger, "getBoundingClientRect").mockReturnValue({
      top: window.innerHeight - 80, bottom: window.innerHeight - 44,
      left: window.innerWidth - 60, right: window.innerWidth + 100,
      width: 160, height: 36, x: 0, y: 0, toJSON: () => ({}),
    });
    fireEvent.click(trigger);
    const menu = screen.getByRole("listbox");
    expect(menu.style.bottom).toBe("85px");
    expect(menu.style.top).toBe("");
    expect(parseFloat(menu.style.left) + parseFloat(menu.style.width)).toBeLessThanOrEqual(window.innerWidth - 8);
  });

  it("filters editable choices, permits arbitrary input and handles an empty result", () => {
    render(<Picker editable />);
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "Eng" } });
    expect(screen.getAllByRole("option")).toHaveLength(1);
    fireEvent.keyDown(input, { key: "Enter" });
    expect(input).toHaveValue("en-US");
    fireEvent.change(input, { target: { value: "unknown" } });
    expect(screen.getByText("没有匹配的选项")).toBeInTheDocument();
    expect(input).not.toHaveAttribute("aria-activedescendant");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(input).toHaveValue("unknown");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("searches choices beyond the initial icon catalog page", () => {
    const catalog = Array.from({ length: 150 }, (_, index) => ({ value: `icon-${index}`, label: `icon-${index}` }));
    function Icons() {
      const [value, setValue] = useState("");
      return <StyledSelect aria-label="图标" options={catalog} value={value} onChange={setValue} editable />;
    }
    render(<Icons />);
    const input = screen.getByRole("combobox");
    fireEvent.click(input);
    expect(screen.getAllByRole("option")).toHaveLength(100);
    fireEvent.change(input, { target: { value: "icon-149" } });
    fireEvent.click(screen.getByRole("option", { name: "icon-149" }));
    expect(input).toHaveValue("icon-149");
    fireEvent.click(input);
    expect(screen.getByRole("option", { name: "icon-149" })).toHaveAttribute("aria-selected", "true");
  });
});
