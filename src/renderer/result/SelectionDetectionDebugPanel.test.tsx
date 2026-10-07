import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SelectionDetectionDebug, SelectionDetectionHealth, WindowPopperApi } from "../../shared";
import { SelectionDetectionDebugPanel } from "./SelectionDetectionDebugPanel";

const health: SelectionDetectionHealth = {
  status: "NORMAL", applicable: true, reason: "EndpointsMatch", selectionDirection: "Forward",
  startEndpointValid: true, endEndpointValid: true, startGeometryValid: null,
  endGeometryValid: null, startDistance: null, endDistance: null, confidence: 0.7, stable: true,
};
const report: SelectionDetectionDebug = {
  version: "selection-offset-dev-v1", captureId: 1, selectionTimestampMs: 123,
  dpiReady: true, gesture: { kind: "Drag", down: { x: -100, y: 20 }, up: { x: -50, y: 20 } },
  fallbackAttempted: false, provider: "msaa-point", method: "accessibility", outcome: "selection",
  selectionText: "ution rew", initialHealth: { ...health, status: "UIA_UNAVAILABLE", applicable: false, reason: "BudgetExceeded" },
  health, durationMs: 183, steps: [{ stage: "detector.initial", elapsedMs: 180, details: { health } }],
};

function bridge(getReport?: WindowPopperApi["getSelectionDetectionDebug"]) {
  const copyText = vi.fn().mockResolvedValue(undefined);
  window._popper_ = { getSelectionDetectionDebug: getReport, copyText } as unknown as WindowPopperApi;
  return copyText;
}

afterEach(cleanup);

describe("temporary selection detection diagnostics", () => {
  it("starts collapsed, retains initial and final health, and copies the exact report", async () => {
    const get = vi.fn().mockResolvedValue(report);
    const copy = bridge(get);
    const { container } = render(<SelectionDetectionDebugPanel sessionId="session-1" />);
    await screen.findByText("DEV · 选区偏移诊断");
    expect(get).toHaveBeenCalledWith("session-1");
    expect(container.querySelector("details")).not.toHaveAttribute("open");
    expect(screen.getByText("UIA_UNAVAILABLE · BudgetExceeded · 不适用")).toBeInTheDocument();
    expect(screen.getByText("NORMAL · EndpointsMatch · 适用")).toBeInTheDocument();
    fireEvent.click(screen.getByText("复制诊断报告"));
    await screen.findByText("已复制");
    expect(copy).toHaveBeenCalledWith(JSON.stringify(report, null, 2));
    expect(screen.getByText(/未进入受控 Ctrl\+C · msaa-point/)).toBeInTheDocument();
  });

  it("shows multi-click containment evidence and copies it with the report", async () => {
    const clickReport: SelectionDetectionDebug = {
      ...report,
      gesture: { kind: "MultiClick", down: { x: 1345, y: 411 }, up: { x: 1345, y: 411 } },
      fallbackAttempted: true,
      health: {
        ...health,
        status: "SUSPICIOUS",
        reason: "ClickOutsideSelection",
        clickValidation: {
          down: { point: { x: 1345, y: 411 }, inSelection: false, geometryValid: null, distance: 100 },
          up: { point: { x: 1345, y: 411 }, inSelection: false, geometryValid: null, distance: 100 },
          selectionRectangles: [{ x: 100, y: 400, width: 100, height: 20 }],
        },
      },
    };
    const copy = bridge(vi.fn().mockResolvedValue(clickReport));
    render(<SelectionDetectionDebugPanel sessionId="double-click" />);
    await screen.findByText("按下：不对应 · 松开：不对应");
    fireEvent.click(screen.getByText("复制诊断报告"));
    await screen.findByText("已复制");
    expect(copy).toHaveBeenCalledWith(JSON.stringify(clickReport, null, 2));
  });

  it("distinguishes attempted fallback from successful clipboard capture", async () => {
    bridge(vi.fn().mockResolvedValue({ ...report, fallbackAttempted: true }));
    const { rerender } = render(<SelectionDetectionDebugPanel sessionId="attempted" />);
    await screen.findByText(/已尝试受控 Ctrl\+C · msaa-point/);
    bridge(vi.fn().mockResolvedValue({ ...report, fallbackAttempted: true, method: "clipboard", provider: "guarded-clipboard" }));
    rerender(<SelectionDetectionDebugPanel sessionId="copied" />);
    await screen.findByText(/已通过受控 Ctrl\+C 取词 · guarded-clipboard/);
  });

  it("does not show a panel when diagnostics are disabled or the API is absent", async () => {
    bridge();
    const { container, rerender } = render(<SelectionDetectionDebugPanel sessionId="absent" />);
    expect(container).toBeEmptyDOMElement();
    const get = vi.fn().mockResolvedValue(null);
    bridge(get);
    rerender(<SelectionDetectionDebugPanel sessionId="disabled" />);
    await waitFor(() => expect(get).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("ignores stale responses after changing the result session", async () => {
    let resolveOld!: (value: SelectionDetectionDebug) => void;
    const get = vi.fn().mockImplementationOnce(() => new Promise<SelectionDetectionDebug>((resolve) => { resolveOld = resolve; })).mockResolvedValueOnce(null);
    bridge(get);
    const { container, rerender } = render(<SelectionDetectionDebugPanel sessionId="old" />);
    rerender(<SelectionDetectionDebugPanel sessionId="new" />);
    await act(async () => { resolveOld(report); });
    expect(container).toBeEmptyDOMElement();
  });

  it("reports command and copy failures without breaking the result content", async () => {
    bridge(vi.fn().mockRejectedValue(new Error("session ended")));
    const { rerender } = render(<SelectionDetectionDebugPanel sessionId="failed" />);
    await screen.findByRole("alert");
    const copy = bridge(vi.fn().mockResolvedValue(report));
    copy.mockRejectedValueOnce(new Error("copy denied"));
    rerender(<SelectionDetectionDebugPanel sessionId="available" />);
    await screen.findByText("复制诊断报告");
    fireEvent.click(screen.getByText("复制诊断报告"));
    await screen.findByText("复制失败");
  });
});
