import { useEffect, useState } from "react";
import type { SelectionDetectionDebug, SelectionDetectionHealth } from "../../shared";
import { useSelectionDetectionDebug } from "../lib/useSelectionDetectionDebug";

function healthLabel(health: SelectionDetectionHealth): string {
  const applicability = health.applicable ? "适用" : "不适用";
  return `${health.status} · ${health.reason} · ${applicability}`;
}

function routeLabel(report: SelectionDetectionDebug): string {
  if (report.method === "clipboard") return "已通过受控 Ctrl+C 取词";
  if (report.fallbackAttempted) return "已尝试受控 Ctrl+C";
  return "未进入受控 Ctrl+C";
}

export function SelectionDetectionDebugPanel({ sessionId }: { sessionId: string }) {
  const { enabled } = useSelectionDetectionDebug();
  const [report, setReport] = useState<SelectionDetectionDebug | null>(null);
  const [loadError, setLoadError] = useState("");
  const [copyState, setCopyState] = useState("");

  useEffect(() => {
    let current = true;
    setReport(null);
    setLoadError("");
    setCopyState("");
    const fetchReport = window._popper_?.getSelectionDetectionDebug;
    if (!enabled || !fetchReport) return;
    void fetchReport(sessionId).then(
      (value) => { if (current) setReport(value); },
      (error: unknown) => { if (current) setLoadError(String(error)); },
    );
    return () => { current = false; };
  }, [sessionId, enabled]);

  if (!enabled || (!report && !loadError)) return null;

  const copy = async () => {
    if (!report) return;
    try {
      await window._popper_.copyText(JSON.stringify(report, null, 2));
      setCopyState("已复制");
    } catch {
      setCopyState("复制失败");
    }
  };

  return (
    <details className="result-selection-debug">
      <summary>
        <strong>DEV · 选区偏移诊断</strong>
        <span>{report ? `${report.health.status} · ${routeLabel(report)}` : "读取失败"}</span>
      </summary>
      <div className="result-selection-debug__body">
        {loadError && <p role="alert">诊断读取失败：{loadError}</p>}
        {report && (
          <>
            <dl>
              <dt>初次检测</dt><dd>{report.initialHealth ? healthLabel(report.initialHealth) : "未记录"}</dd>
              <dt>最终检测</dt><dd>{healthLabel(report.health)}</dd>
              {report.health.clickValidation && (
                <>
                  <dt>点击校验</dt>
                  <dd>按下：{report.health.clickValidation.down.inSelection ? "对应" : "不对应"} · 松开：{report.health.clickValidation.up.inSelection ? "对应" : "不对应"}</dd>
                </>
              )}
              <dt>实际取词</dt><dd>{routeLabel(report)} · {report.provider ?? "未知 provider"} · {report.method ?? report.outcome}</dd>
              <dt>Ctrl+C 注入</dt><dd>{report.copyInjected ? "已注入" : "未注入"}</dd>
              <dt>左键手势</dt><dd>{report.gesture ? `${report.gesture.kind} (${report.gesture.down.x}, ${report.gesture.down.y}) → (${report.gesture.up.x}, ${report.gesture.up.y})` : "无可用鼠标上下文"}</dd>
              <dt>DPI / 耗时</dt><dd>{report.dpiReady ? "Per-Monitor V2" : "DPI 上下文不可用"} · {report.durationMs.toFixed(1)} ms</dd>
            </dl>
            <p className="result-selection-debug__note">报告含选区和附近文本。可在设置左下角关闭 DEV Debug；每次启动应用默认关闭。</p>
            {report.logPath && <p className="result-selection-debug__path">日志：{report.logPath}</p>}
            <button type="button" onClick={() => void copy()}>复制诊断报告</button>
            <span role="status">{copyState}</span>
            {report.stepsTruncated && <p>步骤已达到记录上限，保留最初 128 条和最后 64 条。</p>}
            <pre>{JSON.stringify(report, null, 2)}</pre>
          </>
        )}
      </div>
    </details>
  );
}
