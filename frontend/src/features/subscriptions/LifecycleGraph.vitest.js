import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import LifecycleGraph from "./LifecycleGraph.vue";

function record(overrides = {}) {
  return {
    subject_id: "subject-1",
    lifecycle_state: "queued",
    attention_tags: [],
    ...overrides,
  };
}

function labels(wrapper, state) {
  return wrapper
    .findAll(`.subscription-state-node-${state} .subscription-state-label`)
    .map((node) => node.text());
}

describe("LifecycleGraph", () => {
  it.each([
    {
      lifecycle: "queued",
      aria: "订阅状态：待处理",
      done: [],
      current: ["入队"],
      todo: ["元数据", "搜索", "下载", "硬链接中", "完成"],
    },
    {
      lifecycle: "downloading",
      aria: "订阅状态：下载中",
      done: ["入队", "元数据", "搜索"],
      current: ["下载"],
      todo: ["硬链接中", "完成"],
    },
    {
      lifecycle: "completed",
      aria: "订阅状态：已完成",
      done: ["入队", "元数据", "搜索", "下载", "硬链接中"],
      current: ["完成"],
      todo: [],
    },
  ])("renders $lifecycle node states", ({ lifecycle, aria, done, current, todo }) => {
    const wrapper = mount(LifecycleGraph, {
      props: { record: record({ lifecycle_state: lifecycle }) },
    });

    expect(wrapper.attributes("aria-label")).toBe(aria);
    expect(wrapper.findAll(".subscription-state-node")).toHaveLength(6);
    expect(labels(wrapper, "done")).toEqual(done);
    expect(labels(wrapper, "current")).toEqual(current);
    expect(labels(wrapper, "todo")).toEqual(todo);
  });

  it.each([
    {
      label: "waiting release",
      value: record({ lifecycle_state: "searching", attention_tags: ["waiting_release"] }),
      attention: "waiting_release",
      text: "等待发布",
      aria: "订阅状态：等待发布",
    },
    {
      label: "retry blocked",
      value: record({ lifecycle_state: "downloading", attention_tags: ["retry_blocked"] }),
      attention: "retry_blocked",
      text: "阻塞",
      aria: "订阅状态：重试阻塞",
    },
    {
      label: "skipped",
      value: record({ lifecycle_state: "queued", attention_tags: ["skipped"] }),
      attention: "skipped",
      text: "跳过",
      aria: "订阅状态：已跳过",
    },
    {
      label: "failed",
      value: record({ lifecycle_state: "linking", attention_tags: ["failed"] }),
      attention: "failed",
      text: "失败",
      aria: "订阅状态：失败",
    },
  ])("renders $label attention on the current node", ({ value, attention, text, aria }) => {
    const wrapper = mount(LifecycleGraph, { props: { record: value } });
    const attentionNode = wrapper.get(`.subscription-state-node-${attention}`);

    expect(wrapper.attributes("aria-label")).toBe(aria);
    expect(attentionNode.classes()).toContain("subscription-state-node-current");
    expect(attentionNode.get(".subscription-state-attention").text()).toBe(text);
    expect(wrapper.findAll(".subscription-state-attention")).toHaveLength(1);
  });
});
