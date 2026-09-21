import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { DictionarySuggestion, WindowPopperApi } from "../../shared";
import { useTranslationSuggestions } from "./useTranslationSuggestions";

describe("translation input suggestions", () => {
  it("ignores old responses and cancels input on change, generation change and unmount", async () => {
    let finishOld!: (items: DictionarySuggestion[]) => void;
    const old = new Promise<DictionarySuggestion[]>((resolve) => {
      finishOld = resolve;
    });
    const suggest = vi
      .fn()
      .mockImplementation((_id, _request, _version, text) =>
        text === "ca" ? old : Promise.resolve(text ? [{ word: "cat", explanation: "猫" }] : []),
      );
    window._popper_ = { translationInputSuggestions: suggest } as unknown as WindowPopperApi;
    const { result, rerender, unmount } = renderHook(
      ({ input, request }) => useTranslationSuggestions("one", request, input, true),
      { initialProps: { input: "ca", request: "first" } },
    );
    rerender({ input: "cat", request: "first" });
    await waitFor(() => expect(result.current.items[0]?.word).toBe("cat"));
    await act(async () => finishOld([{ word: "car", explanation: "旧候选" }]));
    expect(result.current.items[0]?.word).toBe("cat");
    expect(suggest.mock.calls.some((args) => args[3] === "")).toBe(true);
    rerender({ input: "", request: "second" });
    await waitFor(() => expect(result.current.items).toEqual([]));
    unmount();
    expect(suggest.mock.lastCall).toEqual(["one", "second", expect.any(Number), ""]);
    const versions = suggest.mock.calls.map((args) => args[2] as number);
    expect(versions.every((v, i) => i === 0 || v > versions[i - 1]!)).toBe(true);
  });

  it("delegates recognition to Rust and cancels suggestions when disabled", async () => {
    const suggest = vi.fn().mockResolvedValue([]);
    window._popper_ = { translationInputSuggestions: suggest } as unknown as WindowPopperApi;
    const { result, rerender } = renderHook(
      ({ enabled }) => useTranslationSuggestions("one", "request", "你好 world", enabled),
      { initialProps: { enabled: true } },
    );
    await waitFor(() =>
      expect(suggest).toHaveBeenLastCalledWith("one", "request", expect.any(Number), "你好 world"),
    );
    rerender({ enabled: false });
    await waitFor(() =>
      expect(suggest).toHaveBeenLastCalledWith("one", "request", expect.any(Number), ""),
    );
    expect(result.current.error).toBe("");
  });
});
