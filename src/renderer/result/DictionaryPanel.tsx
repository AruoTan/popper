import { useEffect, useRef, useState, type JSX } from "react";
import type { DictionarySnapshot, DictionarySuggestion, StudyBook } from "../../shared";
import { getErrorMessage } from "../lib/errors";

export function useDictionarySession(sessionId: string): DictionarySnapshot | null {
  const [snapshot, setSnapshot] = useState<DictionarySnapshot | null>(null);
  useEffect(() => {
    let disposed = false;
    setSnapshot(null);
    const accept = (next: DictionarySnapshot | null): void => {
      if (!disposed && next?.sessionId === sessionId) {
        setSnapshot((old) =>
          !old || old.sessionId !== sessionId || next.revision > old.revision ? next : old,
        );
      }
    };
    const stop = window._popper_.onDictionaryChanged?.(accept);
    void window._popper_
      .getDictionaryState?.(sessionId)
      .then(accept)
      .catch(() => {});
    return () => {
      disposed = true;
      stop?.();
    };
  }, [sessionId]);
  return snapshot;
}

export function DictionaryPanel({
  snapshot,
  onAi,
  onQuery,
  suggestions = snapshot.suggestions,
  querying = false,
}: {
  snapshot: DictionarySnapshot;
  onAi: () => Promise<boolean>;
  onQuery: (word: string) => Promise<void>;
  suggestions?: DictionarySuggestion[];
  querying?: boolean;
}): JSX.Element {
  const [message, setMessage] = useState("");
  const [books, setBooks] = useState<StudyBook[]>([]);
  const [showBooks, setShowBooks] = useState(false);
  const [category, setCategory] = useState("");
  const [booksLoading, setBooksLoading] = useState(false);
  const [adding, setAdding] = useState(false);
  const [added, setAdded] = useState<string[]>([]);
  const [audioLoading, setAudioLoading] = useState(false);
  const generation = useRef(snapshot.queryGeneration);
  const audio = useRef<HTMLAudioElement | null>(null);
  const addingRef = useRef(false);
  generation.current = snapshot.queryGeneration;

  useEffect(() => {
    setShowBooks(false);
    setCategory("");
    setAdded([]);
    setMessage("");
    audio.current?.pause();
    setAudioLoading(false);
  }, [snapshot.query, snapshot.queryGeneration]);
  useEffect(
    () => () => {
      generation.current = -1;
      audio.current?.pause();
    },
    [],
  );

  const query = (word: string): void => {
    void onQuery(word);
  };
  const loadBooks = async (): Promise<void> => {
    const expected = generation.current;
    setShowBooks(true);
    setBooksLoading(true);
    setBooks([]);
    setCategory("");
    setMessage("");
    try {
      if (!window._popper_.getEudicBooks) throw new Error("生词本服务不可用");
      const items = await window._popper_.getEudicBooks(snapshot.sessionId);
      if (generation.current === expected) {
        setBooks(items);
        setCategory("");
      }
    } catch (error) {
      if (generation.current === expected) setMessage(getErrorMessage(error, "获取生词本失败"));
    } finally {
      if (generation.current === expected) setBooksLoading(false);
    }
  };
  const add = async (): Promise<void> => {
    if (!category || addingRef.current || added.includes(category)) return;
    const expected = generation.current;
    const selected = category;
    addingRef.current = true;
    setAdding(true);
    setMessage("");
    try {
      if (!window._popper_.addEudicWord) throw new Error("生词本服务不可用");
      await window._popper_.addEudicWord(snapshot.sessionId, expected, selected);
      if (generation.current === expected) {
        setAdded((old) => [...old, selected]);
        setMessage("已加入所选生词本（重复词不会重复添加）");
      }
    } catch (error) {
      if (generation.current === expected) setMessage(getErrorMessage(error, "添加失败"));
    } finally {
      addingRef.current = false;
      setAdding(false);
    }
  };
  const speak = async (accent: 1 | 2): Promise<void> => {
    if (audioLoading) return;
    const expected = generation.current;
    audio.current?.pause();
    setAudioLoading(true);
    setMessage("");
    try {
      const url = await window._popper_.dictionaryAudio?.(snapshot.sessionId, accent);
      if (generation.current !== expected || !url) return;
      const player = new Audio(url);
      audio.current = player;
      await player.play();
    } catch (error) {
      if (generation.current === expected) setMessage(getErrorMessage(error, "播放发音失败"));
    } finally {
      if (generation.current === expected) setAudioLoading(false);
    }
  };
  const entry = snapshot.entry;
  const actions = (
    <div className="dictionary-actions">
      <button type="button" disabled={snapshot.status === "loading"} onClick={() => void onAi()}>
        改用 AI 翻译
      </button>
      {entry && (
        <button type="button" onClick={() => void loadBooks()}>
          加入欧路生词本
        </button>
      )}
    </div>
  );
  return (
    <section className="dictionary-panel" aria-label="有道词典">
      <div className="dictionary-source">
        有道词典 · {snapshot.mode === "ai" ? "AI 翻译 / 追问" : "英汉释义"}
      </div>
      {snapshot.status === "loading" && <p role="status">正在查询词典…</p>}
      {snapshot.status === "missing" && (
        <p>没有找到词典释义{snapshot.mode === "ai" ? "，已转为 AI 翻译。" : "。"}</p>
      )}
      {snapshot.status === "cancelled" && <p>查询已取消，可重新查词。</p>}
      {snapshot.error && <p role="alert">{snapshot.error}</p>}
      {snapshot.suggestionError && (
        <p className="dictionary-hint">{snapshot.suggestionError}，仍可直接查词。</p>
      )}
      {snapshot.status === "error" && (
        <button type="button" onClick={() => void query(snapshot.query)}>
          重试查词
        </button>
      )}
      {entry && (
        <article className="dictionary-entry">
          <h2>{entry.word}</h2>
          <div className="dictionary-entry-toolbar">
            {actions}
            <div className="dictionary-phones">
              {entry.ukPhone && (
                <button
                  type="button"
                  disabled={audioLoading}
                  onClick={() => void speak(1)}
                  aria-label="播放英式发音"
                >
                  英 /{entry.ukPhone}/ ♫
                </button>
              )}
              {entry.usPhone && (
                <button
                  type="button"
                  disabled={audioLoading}
                  onClick={() => void speak(2)}
                  aria-label="播放美式发音"
                >
                  美 /{entry.usPhone}/ ♫
                </button>
              )}
            </div>
          </div>
          <ul className="dictionary-definitions">
            {entry.definitions.map((definition, i) => (
              <li key={i}>{definition}</li>
            ))}
          </ul>
          {entry.forms.length > 0 && (
            <div className="dictionary-forms">
              {entry.forms.map((form, i) => (
                <span key={i}>
                  {form.name}：<b>{form.value}</b>
                </span>
              ))}
            </div>
          )}
          {entry.examples.length > 0 && (
            <>
              <h3>双语例句</h3>
              <ol className="dictionary-examples">
                {entry.examples.map((example, i) => (
                  <li key={i}>
                    <p>{example.text}</p>
                    <p>{example.translation}</p>
                  </li>
                ))}
              </ol>
            </>
          )}
        </article>
      )}
      {!entry && actions}
      {showBooks && entry && (
        <div className="dictionary-books" role="group" aria-label="选择欧路生词本">
          <p>
            添加词条：<strong>{entry.word}</strong>
          </p>
          {booksLoading ? (
            <p role="status">正在获取生词本…</p>
          ) : (
            <>
              <select
                aria-label="目标生词本"
                value={category}
                onChange={(event) => setCategory(event.target.value)}
                disabled={adding}
              >
                <option value="">请选择生词本</option>
                {books.map((book) => (
                  <option value={book.id} key={book.id}>
                    {book.name}
                    {added.includes(book.id) ? "（已添加）" : ""}
                  </option>
                ))}
              </select>
              {!books.length && <p>暂无可用生词本，请配置授权或在欧路创建后刷新。</p>}
              <button
                type="button"
                disabled={!category || adding || added.includes(category)}
                onClick={() => void add()}
              >
                {adding ? "正在添加…" : added.includes(category) ? "已添加" : "确认添加"}
              </button>
              <button type="button" disabled={adding} onClick={() => void loadBooks()}>
                刷新
              </button>
            </>
          )}
          <button type="button" onClick={() => setShowBooks(false)}>
            收起
          </button>
        </div>
      )}
      {message && <p role="status">{message}</p>}
      {suggestions.length > 0 && (
        <ul className="dictionary-suggestions" aria-label="联想词条">
          {suggestions.map((item) => (
            <li key={item.word}>
              <button
                type="button"
                aria-label={`${item.word} ${item.explanation}`}
                disabled={querying}
                onClick={() => void query(item.word)}
              >
                <strong>{item.word}</strong>
                <span>{item.explanation}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
