import { afterEach, describe, expect, it, vi } from "vitest";

import { resolveLanguage } from "./context";

afterEach(() => vi.restoreAllMocks());

describe("system language resolution", () => {
  it.each([
    { language: "en-US", languages: ["en-US", "zh-CN"], expected: "en" },
    { language: "zh-CN", languages: ["zh-CN", "en-US"], expected: "zh-CN" },
  ] as const)("uses the preferred $language locale", ({ language, languages, expected }) => {
    vi.spyOn(window.navigator, "language", "get").mockReturnValue(language);
    vi.spyOn(window.navigator, "languages", "get").mockReturnValue(languages);

    expect(resolveLanguage("system")).toBe(expected);
  });
});
