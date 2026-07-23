import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import DoubanInterestPanel from "./DoubanInterestPanel.vue";
import MediaMetadataPanel from "./MediaMetadataPanel.vue";
import MediaPrimaryPanel from "./MediaPrimaryPanel.vue";
import MteamTorrentPanel from "./MteamTorrentPanel.vue";
import TvSeasonPanel from "./TvSeasonPanel.vue";

describe("media-detail panels", () => {
  it("renders primary identity and metadata through focused read-only inputs", () => {
    const primary = mount(MediaPrimaryPanel, {
      props: {
        primary: {
          title: "拆分后的主详情",
          mediaType: "movie",
          date: "2026-07-12",
          poster: "/poster.jpg",
          externalLinks: [{ href: "https://example.test/title", label: "IMDb · tt42" }],
        },
      },
    });
    const metadata = mount(MediaMetadataPanel, {
      props: {
        primary: {
          data: { tagline: "独立元数据面板" },
          metaRows: [{ label: "片长", value: "123 分钟" }],
        },
      },
    });

    expect(primary.get("h3").text()).toBe("拆分后的主详情");
    expect(primary.get("img").attributes("src")).toBe("/poster.jpg");
    expect(primary.get(".detail-external-ids a").attributes("href")).toBe(
      "https://example.test/title",
    );
    expect(metadata.get(".tagline-block").text()).toBe("独立元数据面板");
    expect(metadata.get(".detail-meta-row").text()).toContain("123 分钟");
  });

  it("emits Douban editing intent without mutating its model", async () => {
    const interest = {
      status: "已加载",
      error: "",
      mark: { interest: "collect", rating: "3", category: "", tags: "旧标签" },
      ratingLabel: "3 星",
      categoryLabel: "",
      categories: [],
      tagHistory: ["冷门"],
      saveDisabled: false,
    };
    const before = JSON.stringify(interest);
    const wrapper = mount(DoubanInterestPanel, { props: { interest } });

    await wrapper.findAll(".douban-mark-mode button")[0].trigger("click");
    await wrapper.get("select").setValue("5");
    await wrapper.get(".douban-tag-input input").setValue("新标签");
    await wrapper.get(".douban-tag-chip").trigger("click");
    await wrapper
      .findAll("button")
      .find((button) => button.text() === "保存")
      .trigger("click");

    expect(wrapper.emitted("set-interest")[0]).toEqual(["wish"]);
    expect(wrapper.emitted("update-rating")[0]).toEqual(["5"]);
    expect(wrapper.emitted("update-tags")[0]).toEqual(["新标签"]);
    expect(wrapper.emitted("tag-suggestion")[0]).toEqual(["冷门"]);
    expect(wrapper.emitted("save-interest")).toHaveLength(1);
    expect(JSON.stringify(interest)).toBe(before);
  });

  it("owns season expansion and per-season render states", async () => {
    const wrapper = mount(TvSeasonPanel, {
      props: {
        seasons: [{ season_number: 1, name: "第一季", episode_count: 1 }],
        episodes: {
          1: [
            {
              episode_number: 1,
              name: "第一集",
              overview: "分集面板内容",
              still_url: "/still.jpg",
            },
          ],
        },
        loading: {},
        errors: {},
      },
    });
    const details = wrapper.get("details");
    details.element.open = true;
    await details.trigger("toggle");

    expect(wrapper.emitted("load-season")[0]).toEqual([1]);
    expect(wrapper.get(".tv-ep-title").text()).toBe("第一集");
    expect(wrapper.get(".tv-ep-still").attributes("src")).toBe("/still.jpg");
    expect(wrapper.text()).toContain("分集面板内容");
  });

  it("owns M-Team source selection and qB push intent", async () => {
    const torrent = {
      id: "torrent-42",
      name: "Panel.Release.2160p",
      size: 4096,
      status: { seeders: 10, leechers: 1 },
    };
    const wrapper = mount(MteamTorrentPanel, {
      props: {
        mteam: {
          sources: [
            { source: "imdb", label: "IMDb" },
            { source: "keyword", label: "原标题" },
          ],
          activeSource: "imdb",
          rows: [torrent],
          loading: false,
          error: "",
        },
      },
    });

    await wrapper.findAll("[role='tab']")[1].trigger("click");
    const trigger = wrapper.get(".torrent-push-trigger");
    await trigger.trigger("click");

    expect(wrapper.emitted("select-source")[0]).toEqual(["keyword"]);
    expect(wrapper.emitted("push-torrent")[0][0]).toEqual(torrent);
    expect(wrapper.emitted("push-torrent")[0][1]).toBe(trigger.element);
    expect(wrapper.get(".torrent-card-link").attributes("href")).toContain("torrent-42");
  });

  it("allows pushes only for TV rows recognized in the selected season", async () => {
    const wrapper = mount(MteamTorrentPanel, {
      props: {
        mteam: {
          mediaType: "tv",
          seasons: [
            { season_number: 1, episode_count: 8 },
            { season_number: 2, episode_count: 10 },
          ],
          seasonNumber: 2,
          sources: [{ source: "tv_season", label: "第 2 季" }],
          activeSource: "tv_season",
          rows: [
            {
              id: "safe",
              name: "Series.S02E03-E06",
              tv_match: { label: "S02E03-E06 部分合集", compatible: true },
            },
            {
              id: "unsafe",
              name: "Series.1080p",
              tv_match: { label: "未识别集数", compatible: false },
            },
          ],
          loading: false,
          error: "",
        },
      },
    });

    expect(wrapper.findAll(".torrent-push-trigger")).toHaveLength(1);
    expect(wrapper.findAll(".torrent-tv-match")[1].classes()).toContain("is-incompatible");
    await wrapper.get("select").setValue("1");
    expect(wrapper.emitted("select-season")[0]).toEqual([1]);
  });
});
