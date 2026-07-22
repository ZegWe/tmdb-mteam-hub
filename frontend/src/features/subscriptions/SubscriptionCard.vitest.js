import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import SubscriptionCard from "./SubscriptionCard.vue";

function record(overrides = {}) {
  return {
    subject_id: "movie-1",
    title: "普通电影",
    lifecycle_state: "queued",
    attention_tags: [],
    active: true,
    media_kind: "movie",
    schedulable: true,
    blocked_reason: null,
    poster_url: "https://example.test/poster.jpg",
    release_year: 2026,
    ...overrides,
  };
}

describe("SubscriptionCard", () => {
  it.each([
    {
      label: "schedulable movie",
      value: record(),
      expectedStatus: "待处理",
    },
    {
      label: "inactive history",
      value: record({
        subject_id: "inactive-1",
        title: "历史订阅",
        active: false,
        schedulable: false,
        blocked_reason: "subscription_inactive",
        lifecycle_state: "completed",
      }),
      expectedStatus: "已完成",
    },
    {
      label: "unsupported TV",
      value: record({
        subject_id: "tv-1",
        title: "未开放剧集",
        media_kind: "tv",
        schedulable: false,
        blocked_reason: "tv_not_supported",
        lifecycle_state: "downloading",
      }),
      expectedStatus: "下载中",
    },
  ])("renders $label with title, year, image, and status badge only", ({ value, expectedStatus }) => {
    const wrapper = mount(SubscriptionCard, { props: { record: value } });

    expect(wrapper.get(".title").text()).toBe(value.title);
    expect(wrapper.get(".subtle").text()).toBe(String(value.release_year || value.subject_id));
    expect(wrapper.get("img").attributes("src")).toBe("https://example.test/poster.jpg");
    expect(wrapper.get(".subscription-status").text()).toBe(expectedStatus);

    // No capability badges on cards
    const cardText = wrapper.text();
    expect(cardText).not.toContain("可调度");
    expect(cardText).not.toContain("不可调度");
    expect(cardText).not.toContain("已停用");
    expect(cardText).not.toContain("TV 未开放");
  });

  it("emits one semantic open intent for click and keyboard activation", async () => {
    const value = record();
    const wrapper = mount(SubscriptionCard, { props: { record: value } });

    expect(wrapper.attributes("role")).toBe("link");
    expect(wrapper.attributes("tabindex")).toBe("0");
    expect(wrapper.attributes("aria-label")).toBe("打开订阅 普通电影");

    await wrapper.trigger("click");
    await wrapper.trigger("keydown", { key: "Enter" });

    expect(wrapper.emitted("open")).toEqual([[value], [value]]);
  });
});
