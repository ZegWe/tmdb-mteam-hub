import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import DownloadTaskList from "./DownloadTaskList.vue";

function record(overrides = {}) {
  return {
    subject_id: "subject-1",
    lifecycle_state: "downloading",
    ...overrides,
  };
}

describe("DownloadTaskList", () => {
  it("renders the not-pushed empty state without progress or episodes", () => {
    const wrapper = mount(DownloadTaskList, {
      props: { record: record({ lifecycle_state: "queued" }) },
    });

    expect(wrapper.get(".empty-hint").text()).toBe("暂无下载任务");
    expect(wrapper.find("[aria-label^='下载进度']").exists()).toBe(false);
    expect(wrapper.findAll("h4").map((heading) => heading.text())).toEqual(["下载"]);
  });

  it("renders nested download artifacts and derives episode rows from artifact files", () => {
    const wrapper = mount(DownloadTaskList, {
      props: {
        record: record({
          downloads: [
            {
              id: "download-1",
              torrent_title: "Nested.Release.2160p",
              qb_server_id: "home",
              qb_server_name: "家庭 qB",
              qb_category: "电影",
              qb_save_dir_name: "downloads",
              qb_hash: "hash-1",
              qb_state: "downloading",
              state: "downloading",
              progress: 0.625,
              total_size: 4096,
              checked_at: 200,
              files: [
                {
                  name: "S01E03.mkv",
                  size: 4096,
                  progress: 0.75,
                  season_number: 1,
                  episode_number: 3,
                  episode_label: "S01E03",
                },
              ],
            },
          ],
        }),
      },
    });

    expect(wrapper.get("[aria-label='下载进度 63%']").exists()).toBe(true);
    expect(wrapper.text()).toContain("种子Nested.Release.2160p");
    expect(wrapper.text()).toContain("qB家庭 qB");
    expect(wrapper.text()).toContain("qB 状态downloading");
    expect(wrapper.text()).toContain("大小4.00 KB");
    expect(wrapper.get(".subscription-episode-title").text()).toBe("S01E03");
    expect(wrapper.get(".subscription-episode-state").text()).toBe("下载中");
    expect(wrapper.get(".subscription-episode-files").text()).toBe("0/1");
  });
});
