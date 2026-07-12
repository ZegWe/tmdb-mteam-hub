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
      badges: ["可调度"],
      toneClass: "badge-success",
    },
    {
      label: "inactive history",
      value: record({
        subject_id: "inactive-1",
        title: "历史订阅",
        active: false,
        schedulable: false,
        blocked_reason: "subscription_inactive",
      }),
      badges: ["已停用", "不可调度"],
      toneClass: "badge-ghost",
    },
    {
      label: "unsupported TV",
      value: record({
        subject_id: "tv-1",
        title: "未开放剧集",
        media_kind: "tv",
        schedulable: false,
        blocked_reason: "tv_not_supported",
      }),
      badges: ["TV 未开放", "不可调度"],
      toneClass: "badge-warning",
    },
    {
      label: "backend blocked movie",
      value: record({
        subject_id: "blocked-1",
        title: "后端阻止电影",
        schedulable: false,
        blocked_reason: "manual_hold",
      }),
      badges: ["自动处理受限：manual hold", "不可调度"],
      toneClass: "badge-error",
    },
  ])("renders $label badges from the subscription domain", ({ value, badges, toneClass }) => {
    const wrapper = mount(SubscriptionCard, { props: { record: value } });

    expect(wrapper.get(".title").text()).toBe(value.title);
    expect(wrapper.get(".subtle").text()).toBe("2026");
    expect(wrapper.get("img").attributes("src")).toBe("https://example.test/poster.jpg");
    for (const badge of badges) expect(wrapper.text()).toContain(badge);
    expect(wrapper.find(`.${toneClass}`).exists()).toBe(true);
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
