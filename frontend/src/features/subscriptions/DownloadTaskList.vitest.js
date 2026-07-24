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
  it("renders the empty state without progress or task panels", () => {
    const wrapper = mount(DownloadTaskList, {
      props: { record: record({ lifecycle_state: "queued" }) },
    });

    expect(wrapper.get(".empty-hint").text()).toBe("暂无下载任务");
    expect(wrapper.find("[aria-label^='下载进度']").exists()).toBe(false);
    expect(wrapper.findAll("h4").map((heading) => heading.text())).toEqual(["下载任务"]);
  });

  it("renders download tasks as collapsible panels with episode labels, download rows, links, and files", () => {
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

    // Task panel summary
    const summary = wrapper.get(".subscription-task-summary");
    expect(summary.get(".subscription-task-episode").text()).toBe("S01E03");
    expect(summary.get(".subscription-task-title").text()).toBe("Nested.Release.2160p");
    expect(summary.get(".subscription-task-state").text()).toBe("下载中");

    // Download rows inside the panel
    const body = wrapper.get(".subscription-task-body");
    expect(body.text()).toContain("种子Nested.Release.2160p");
    expect(body.text()).toContain("qB家庭 qB");
    expect(body.text()).toContain("qB 状态downloading");
    expect(body.text()).toContain("大小4.00 KB");

    // Files
    expect(body.get(".subscription-file-name").text()).toBe("S01E03.mkv");
    expect(body.get(".subscription-file-status").text()).toBe("75%");
  });

  it("matches links to downloads by download_artifact_id", () => {
    const wrapper = mount(DownloadTaskList, {
      props: {
        record: record({
          downloads: [
            {
              id: "download-1",
              torrent_title: "Show.S01.2160p",
              state: "downloading",
              files: [
                { name: "S01E01.mkv", season_number: 1, episode_number: 1 },
                { name: "S01E02.mkv", season_number: 1, episode_number: 2 },
              ],
            },
          ],
          links: [
            {
              id: "link-1",
              download_artifact_id: "download-1",
              state: "partial",
              source_path: "/downloads/Show",
              target_dir: "/library/剧集",
              checked_at: 200,
              files: [
                {
                  source_path: "/downloads/Show/S01E01.mkv",
                  target_path: "/library/剧集/S01E01.mkv",
                  outcome: "linked",
                },
              ],
            },
          ],
        }),
      },
    });

    // Episode label should be a range
    expect(wrapper.get(".subscription-task-episode").text()).toBe("S01E01-E02");

    // Link info should be inside the task panel body
    const body = wrapper.get(".subscription-task-body");
    expect(body.text()).toContain("硬链接 /library/剧集");
    expect(body.text()).toContain("链接状态部分完成");
    expect(body.text()).toContain("目标目录/library/剧集");
    expect(body.text()).toContain("下载任务download-1");

    // Files from both download and link
    const fileNames = body
      .findAll(".subscription-file-name")
      .map((node) => node.text());
    expect(fileNames).toContain("S01E01.mkv");
    expect(fileNames).toContain("S01E02.mkv");
    expect(fileNames).toContain("/library/剧集/S01E01.mkv");
  });

  it("renders orphan links without a matching download as independent panels", () => {
    const wrapper = mount(DownloadTaskList, {
      props: {
        record: record({
          downloads: [],
          links: [
            {
              id: "link-orphan",
              state: "completed",
              target_dir: "/library/独立",
              files: [
                { target_path: "/library/独立/file.mkv", outcome: "linked" },
              ],
            },
          ],
        }),
      },
    });

    const sections = wrapper.findAll("h4").map((h) => h.text());
    expect(sections).toContain("独立硬链接");

    const orphanSummary = wrapper.get(".subscription-task-summary");
    expect(orphanSummary.get(".subscription-task-episode").text()).toBe("—");
    expect(orphanSummary.get(".subscription-task-title").text()).toBe("/library/独立");
  });

  it("renders continuous episode range label", () => {
    const wrapper = mount(DownloadTaskList, {
      props: {
        record: record({
          downloads: [
            {
              id: "download-1",
              torrent_title: "Season.Pack.2160p",
              state: "completed",
              files: [
                { season_number: 2, episode_number: 1 },
                { season_number: 2, episode_number: 2 },
                { season_number: 2, episode_number: 3 },
                { season_number: 2, episode_number: 4 },
                { season_number: 2, episode_number: 5 },
              ],
            },
          ],
        }),
      },
    });

    expect(wrapper.get(".subscription-task-episode").text()).toBe("S02E01-E05");
  });
});
