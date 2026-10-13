import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import SubscriptionDetailView from "./SubscriptionDetailView.vue";

describe("TV torrent selection", () => {
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
