import { describe, expect, it } from "vitest";
import { doubanMetaRows } from "../features/media-detail/domain.js";

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

describe("Douban detail metadata", () => {
  it("renders rexxar title and language fields", () => {
    expect(
      plain(
        doubanMetaRows({
          original_title: "The Shawshank Redemption",
          aka: ["月黑高飞(港)", "刺激1995(台)"],
          languages: ["英语"],
          countries: ["美国"],
        }),
      ),
    ).toEqual([
      { label: "原名", value: "The Shawshank Redemption" },
      { label: "又名", value: "月黑高飞(港) · 刺激1995(台)" },
      { label: "国家/地区", value: "美国" },
      { label: "语言", value: "英语" },
    ]);
  });
});
