import { useEffect, useState, type JSX } from "react";
import { BookOpen, ExternalLink, KeyRound, LoaderCircle } from "lucide-react";
import { getErrorMessage } from "../lib/errors";

export function DictionarySettings(): JSX.Element {
  const [configured, setConfigured] = useState(false);
  const [authorization, setAuthorization] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  useEffect(() => {
    let disposed = false;
    void window._popper_
      .eudicConfigured?.()
      .then((value) => {
        if (!disposed) setConfigured(value);
      })
      .catch((error: unknown) => {
        if (!disposed) setMessage(getErrorMessage(error, "无法读取授权状态"));
      });
    return () => {
      disposed = true;
    };
  }, []);
  const save = async (value: string): Promise<void> => {
    if (busy) return;
    setBusy(true);
    setMessage("");
    try {
      if (!window._popper_.setEudicAuthorization) throw new Error("授权配置服务不可用");
      await window._popper_.setEudicAuthorization(value);
      setConfigured(!!value.trim());
      setAuthorization("");
      setMessage(value.trim() ? "欧路授权已加密保存" : "欧路授权已清除");
    } catch (error) {
      setMessage(getErrorMessage(error, "保存失败"));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="dictionary-settings field field--wide">
      <div className="dictionary-settings__heading">
        <span className="dictionary-settings__icon" aria-hidden="true">
          <BookOpen size={19} />
        </span>
        <strong>欧路生词本</strong>
        <span className={`dictionary-settings__badge ${configured ? "is-configured" : ""}`}>
          {configured ? "已配置授权" : "未配置授权"}
        </span>
      </div>
      <p className="field__hint">
        从欧路 OpenAPI 授权页获取完整 Authorization（包含 NIS
        前缀，如页面提供）。仅在确认添加时上传当前词条。
      </p>
      <div>
        <button
          className="button button--ghost"
          type="button"
          onClick={() => {
            void window._popper_
              .openExternal("https://my.eudic.net/OpenAPI/Authorization")
              .catch((error: unknown) => setMessage(getErrorMessage(error, "无法打开授权页")));
          }}
        >
          <ExternalLink size={14} aria-hidden="true" />
          获取欧路授权
        </button>
      </div>
      <div className="key-control">
        <KeyRound size={16} aria-hidden="true" />
        <input
          aria-label="欧路 Authorization"
          type="password"
          autoComplete="off"
          value={authorization}
          placeholder="粘贴完整授权信息"
          onChange={(event) => setAuthorization(event.target.value)}
        />
      </div>
      <div className="dictionary-settings__actions">
        <button
          className="button"
          type="button"
          disabled={busy || !authorization.trim()}
          onClick={() => void save(authorization)}
        >
          {busy && <LoaderCircle size={14} className="settings-spin" aria-hidden="true" />}
          保存欧路授权
        </button>
        <button
          className="button button--danger"
          type="button"
          disabled={busy || !configured}
          onClick={() => void save("")}
        >
          清除授权
        </button>
      </div>
      {message && (
        <p className="field__hint" role="status">{message}</p>
      )}
    </div>
  );
}
