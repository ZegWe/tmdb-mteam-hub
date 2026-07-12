import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import LinkResult from "./LinkResult.vue";

function record(overrides = {}) {
  return {
    subject_id: "subject-1",
    ...overrides,
  };
}

function fileNames(wrapper) {
  return wrapper.findAll(".subscription-file-name").map((node) => node.text());
}

describe("LinkResult", () => {
  it("renders no link or file sections when the record has neither result", () => {
    const wrapper = mount(LinkResult, { props: { record: record() } });

    expect(wrapper.findAll("section")).toHaveLength(0);
    expect(wrapper.text()).toBe("");
  });

  it("preserves file note priority and status/progress formatting", () => {
    const wrapper = mount(LinkResult, {
      props: {
        record: record({
          downloads: [
            {
              id: "download-1",
              state: "failed",
              files: [
                {
                  name: "errored.mkv",
                  error: "链接失败",
                  source_path: "/downloads/errored.mkv",
                  size: 2048,
                },
                {
                  name: "sourced.mkv",
                  source_path: "/downloads/sourced.mkv",
                  size: 2048,
                  progress: 0.25,
                },
                { name: "sized.mkv", size: 1024, progress: 0.5 },
              ],
            },
          ],
          links: [
            {
              id: "link-1",
              state: "completed",
              files: [{ target_path: "/library/status-only.mkv", outcome: "linked" }],
            },
          ],
        }),
      },
    });

    const rows = wrapper.findAll(".subscription-file-row");
    expect(rows[0].get(".subscription-file-note").text()).toBe("链接失败");
    expect(rows[0].get(".subscription-file-status").text()).toBe("failed");
    expect(rows[1].get(".subscription-file-note").text()).toBe("/downloads/sourced.mkv");
    expect(rows[1].get(".subscription-file-status").text()).toBe("25%");
    expect(rows[2].get(".subscription-file-note").text()).toBe("1.00 KB");
    expect(rows[2].get(".subscription-file-status").text()).toBe("50%");
    expect(rows[3].get(".subscription-file-name").text()).toBe("/library/status-only.mkv");
    expect(rows[3].find(".subscription-file-note").exists()).toBe(false);
    expect(rows[3].get(".subscription-file-status").text()).toBe("linked");
  });

  it("merges download and link artifact files in order and caps the rendered list at 120", () => {
    const downloadFiles = Array.from({ length: 80 }, (_, index) => ({
      name: `download-${index + 1}.mkv`,
    }));
    const linkFiles = Array.from({ length: 80 }, (_, index) => ({
      target_path: `/library/link-${index + 1}.mkv`,
    }));
    const wrapper = mount(LinkResult, {
      props: {
        record: record({
          downloads: [{ id: "download-1", state: "completed", files: downloadFiles }],
          links: [{ id: "link-1", state: "completed", files: linkFiles }],
        }),
      },
    });

    const names = fileNames(wrapper);
    expect(names).toHaveLength(120);
    expect(names.slice(0, 2)).toEqual(["download-1.mkv", "download-2.mkv"]);
    expect(names[79]).toBe("download-80.mkv");
    expect(names[119]).toBe("/library/link-40.mkv");
    expect(names).not.toContain("/library/link-41.mkv");
  });

  it("renders nested link artifacts and merges nested download/link files", () => {
    const wrapper = mount(LinkResult, {
      props: {
        record: record({
          downloads: [
            {
              id: "download-1",
              files: [{ name: "downloaded.mkv", size: 2048, progress: 0.5 }],
            },
          ],
          links: [
            {
              id: "link-1",
              download_artifact_id: "download-1",
              state: "partial",
              source_path: "/downloads/release",
              target_dir: "/library/电影",
              checked_at: 200,
              files: [
                {
                  source_path: "/downloads/release/downloaded.mkv",
                  target_path: "/library/电影/downloaded.mkv",
                  size: 2048,
                  outcome: "linked",
                  error: null,
                },
              ],
            },
          ],
        }),
      },
    });

    expect(wrapper.text()).toContain("链接状态部分完成");
    expect(wrapper.text()).toContain("目标目录/library/电影");
    expect(wrapper.text()).toContain("下载任务download-1");
    expect(fileNames(wrapper)).toEqual(["downloaded.mkv", "/library/电影/downloaded.mkv"]);
    expect(wrapper.findAll(".subscription-file-status").map((node) => node.text())).toEqual([
      "50%",
      "linked",
    ]);
  });
});
