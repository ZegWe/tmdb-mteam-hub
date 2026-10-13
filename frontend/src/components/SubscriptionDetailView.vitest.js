import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import SubscriptionDetailView from "./SubscriptionDetailView.vue";

describe("TV torrent selection", () => {
  it("keeps saved candidates collapsed and provides original links and useful statistics", async () => {
    const wrapper = mount(SubscriptionDetailView, {
      props: {
        selectedSubscription: {
          subject_id: "123",
          title: "Movie",
          media_kind: "movie",
          active: true,
          schedulable: true,
          lifecycle_state: "searching",
          execution_state: "idle",
          candidates: [
            {
              torrent_id: "456",
              title: "Movie 2160p",
              size: "1073741824",
              seeders: 0,
              leechers: 12,
              uploaded_at: "2026-10-13 10:00:00",
            },
          ],
        },
      },
    });
    expect(wrapper.emitted("search-candidates")).toBeUndefined();
    const history = wrapper.get(".subscription-saved-candidates");
    expect(history.element.open).toBe(false);
    expect(history.text()).toContain("已保存的候选（非本次搜索）");
    const link = history.get("a");
    expect(link.attributes()).toMatchObject({
      href: "https://kp.m-team.cc/detail/456",
      target: "_blank",
      rel: "noopener noreferrer",
    });
    expect(history.get(".subscription-candidate-stats").text()).toBe(
      "大小 1.00 GB · 做种 0 · 下载中 12 · 发布 2026-10-13 10:00:00",
    );
    await wrapper.setProps({ searched: true });
    expect(history.element.open).toBe(true);
    wrapper.unmount();
  });

  it("distinguishes continuing missing episodes from clearing old downloads", async () => {
    const wrapper = mount(SubscriptionDetailView, {
      props: {
        selectedSubscription: {
          subject_id: "tv-1",
          title: "Show",
          media_kind: "tv",
          active: true,
          schedulable: true,
          lifecycle_state: "searching",
          execution_state: "idle",
          candidates: [{ torrent_id: "next", title: "Show S01E02", source: "keyword" }],
          downloads: [{ torrent_id: "old", state: "downloaded" }],
        },
      },
    });
    const button = (text) => wrapper.findAll("button").find((item) => item.text() === text);
    await button("选中并下载").trigger("click");
    await button("清理旧任务并换种").trigger("click");
    expect(wrapper.emitted("confirm-candidate")).toEqual([
      ["next", false],
      ["next", true],
    ]);
    wrapper.unmount();
  });
});
