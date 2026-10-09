import { describe, expect, it } from "vitest";
import { dictionaryEntrySchema } from "../dictionary";

describe("dictionary vocabulary metadata", () => {
  it("keeps vocabulary tags and accepts older entries without metadata", () => {
    const entry = { word: "account", ukPhone: null, usPhone: null, definitions: ["n. 账户"], forms: [], examples: [] };
    expect(dictionaryEntrySchema.parse(entry).tags).toEqual([]);
    const tags = ["高中", "CET4", "CET6", "考研", "IELTS", "TOEFL", "商务英语"];
    expect(dictionaryEntrySchema.parse({ ...entry, tags }).tags).toEqual(tags);
  });
});
