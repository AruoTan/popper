import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { WindowPopperApi } from "../../shared";
import { ShortcutRecorder } from "./ShortcutRecorder";

const setNative = vi.fn();
const change = vi.fn();
function Recorder(): React.JSX.Element {
  const [value, setValue] = useState("Control+Shift+Space");
  return <ShortcutRecorder value={value} onChange={(next) => { change(next); setValue(next); }} />;
}

beforeEach(() => {
  setNative.mockReset().mockResolvedValue(undefined);
  change.mockReset();
  Object.defineProperty(window, "_popper_", {
    configurable: true,
    value: { setQuickAskShortcutRecording: setNative } as unknown as WindowPopperApi,
  });
});

async function record(): Promise<HTMLElement> {
  const control = screen.getByRole("button", { name: "快捷提问快捷键" });
  fireEvent.click(control);
  await waitFor(() => expect(control).toHaveAttribute("aria-pressed", "true"));
  return control;
}

describe("ShortcutRecorder", () => {
  it("requires explicit recording, previews modifiers and commits a valid combination", async () => {
    const view = render(<Recorder />);
    const control = screen.getByRole("button", { name: "快捷提问快捷键" });
    expect(Array.from(control.querySelectorAll("kbd"), (key) => key.textContent)).toEqual(["Ctrl", "Shift", "Space"]);
    fireEvent.keyDown(control, { code: "KeyK", key: "k", ctrlKey: true });
    expect(change).not.toHaveBeenCalled();
    await record();
    expect(setNative).toHaveBeenCalledWith(true);
    fireEvent.keyDown(control, { key: "Control", code: "ControlLeft", ctrlKey: true });
    expect(control.querySelector("kbd")).toHaveTextContent("Ctrl");
    expect(change).not.toHaveBeenCalled();
    fireEvent.keyDown(control, { key: "k", code: "KeyK", ctrlKey: true, altKey: true });
    expect(change).toHaveBeenCalledWith("Control+Alt+KeyK");
    expect(control).toHaveAttribute("aria-pressed", "false");
    expect(control).toHaveTextContent(/Ctrl.*Alt.*K/);
    expect(screen.getByRole("status")).toHaveTextContent("已更新，保存后生效");
    await waitFor(() => expect(setNative).toHaveBeenLastCalledWith(false));
    view.unmount();
  });

  it("rejects unmodified keys and lets Escape cancel without changing the shortcut", async () => {
    render(<Recorder />);
    const control = await record();
    fireEvent.keyDown(control, { key: "k", code: "KeyK" });
    expect(screen.getByRole("status")).toHaveTextContent("请使用修饰键");
    expect(change).not.toHaveBeenCalled();
    fireEvent.keyDown(control, { key: "Escape", code: "Escape" });
    expect(control).toHaveTextContent(/Ctrl.*Shift.*Space/);
    expect(control).toHaveAttribute("aria-pressed", "false");
    await waitFor(() => expect(setNative).toHaveBeenLastCalledWith(false));
  });

  it("cancels on Tab, blur and unmount and releases native shortcut suppression", async () => {
    const view = render(<Recorder />);
    const control = await record();
    fireEvent.keyDown(control, { key: "Tab", code: "Tab" });
    expect(control).toHaveAttribute("aria-pressed", "false");
    await record();
    fireEvent.blur(control);
    expect(control).toHaveAttribute("aria-pressed", "false");
    await record();
    view.unmount();
    await waitFor(() => expect(setNative).toHaveBeenLastCalledWith(false));
    expect(change).not.toHaveBeenCalled();
  });

  it("releases a delayed begin after Escape during preparation", async () => {
    let resolve!: () => void;
    setNative.mockImplementationOnce(() => new Promise<void>((done) => { resolve = done; }));
    render(<Recorder />);
    const control = screen.getByRole("button", { name: "快捷提问快捷键" });
    fireEvent.click(control);
    await waitFor(() => expect(setNative).toHaveBeenCalledWith(true));
    expect(control).toHaveAttribute("aria-busy", "true");
    fireEvent.keyDown(control, { key: "Escape" });
    await act(async () => resolve());
    await waitFor(() => expect(setNative).toHaveBeenLastCalledWith(false));
    expect(control).toHaveAttribute("aria-pressed", "false");
    expect(control).toHaveAttribute("aria-busy", "false");
    expect(change).not.toHaveBeenCalled();
  });

  it("shows native failure and never enters recording", async () => {
    setNative.mockRejectedValueOnce(new Error("native unavailable"));
    render(<Recorder />);
    fireEvent.click(screen.getByRole("button", { name: "快捷提问快捷键" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("native unavailable"));
    expect(screen.getByRole("button", { name: "快捷提问快捷键" })).toHaveAttribute("aria-pressed", "false");
    expect(change).not.toHaveBeenCalled();
  });

  it("cancels recording when pointer down occurs outside the recorder", async () => {
    render(
      <div>
        <div data-testid="outside">外部区域</div>
        <Recorder />
      </div>,
    );
    const control = await record();
    expect(control).toHaveAttribute("aria-pressed", "true");
    fireEvent.pointerDown(screen.getByTestId("outside"));
    expect(control).toHaveAttribute("aria-pressed", "false");
    expect(change).not.toHaveBeenCalled();
    await waitFor(() => expect(setNative).toHaveBeenLastCalledWith(false));
  });
});

it('serializes a delayed native recording handoff between shortcut controls', async () => {
  let resolve!: () => void;
  setNative.mockImplementationOnce(() => new Promise<void>((done) => { resolve = done; }));
  render(<>
    <ShortcutRecorder id="capture" label="截图快捷键" value="PrintScreen" onChange={change} />
    <ShortcutRecorder id="paste" label="贴图快捷键" value="Control+Shift+KeyP" onChange={change} />
  </>);
  const capture = screen.getByRole('button', { name: '截图快捷键' });
  const paste = screen.getByRole('button', { name: '贴图快捷键' });
  fireEvent.click(capture);
  await waitFor(() => expect(setNative).toHaveBeenCalledOnce());
  fireEvent.pointerDown(paste); fireEvent.click(paste);
  expect(paste).toHaveAttribute('aria-busy', 'true');
  expect(setNative).toHaveBeenCalledOnce();
  await act(async () => resolve());
  await waitFor(() => expect(paste).toHaveAttribute('aria-pressed', 'true'));
  expect(capture).toHaveAttribute('aria-pressed', 'false');
  expect(setNative.mock.calls.map(([enabled]) => enabled)).toEqual([true, false, true]);
});
