import { computed, reactive, readonly, ref } from "vue";
import { pushMteamTorrent } from "../../shared/api/endpoints/qb.js";
import { qbPushPayload } from "./domain.js";

const defaultTransport = Object.freeze({
  push: (payload, options) => pushMteamTorrent(payload, options),
});

export function createQbPushDialogStore({ settingsStore, transport = defaultTransport } = {}) {
  if (!settingsStore) throw new TypeError("settingsStore is required");

  const open = ref(false);
  const loading = ref(false);
  const form = reactive({
    torrentId: "",
    title: "",
    serverIndex: "",
    category: "",
    savepath: "",
  });
  const servers = settingsStore.runtimeQbServers;
  const label = computed(() =>
    form.title
      ? `${form.title}（${form.torrentId}）`
      : form.torrentId
        ? `种子 ID · ${form.torrentId}`
        : "",
  );
  const selectedServer = computed(() => servers.value[Number(form.serverIndex)] || null);
  const selectedServerLabel = computed(() => {
    const server = selectedServer.value;
    return server
      ? server.name || server.base_url || `服务器 ${Number(form.serverIndex) + 1}`
      : "未配置 qB（请打开 API 设置）";
  });
  let lifecycleRevision = 0;
  let openRequestSequence = 0;
  let submitController = null;
  let disposed = false;

  function isCurrentLifecycle(revision) {
    return !disposed && revision === lifecycleRevision;
  }

  async function openForTorrent(torrent) {
    if (disposed || loading.value) return null;
    const revision = lifecycleRevision;
    const requestId = ++openRequestSequence;
    try {
      await settingsStore.ensureRuntimeLoaded();
    } catch (error) {
      if (!isCurrentLifecycle(revision) || requestId !== openRequestSequence) return null;
      throw error;
    }
    if (!isCurrentLifecycle(revision) || requestId !== openRequestSequence) return null;
    form.torrentId = String(torrent?.id || "");
    form.title = String(torrent?.name || torrent?.title || "").trim();
    form.serverIndex = servers.value.length ? "0" : "";
    form.category = "";
    form.savepath = "";
    open.value = true;
  }

  function close() {
    open.value = false;
  }

  async function submit() {
    if (disposed || loading.value) return null;
    if (!servers.value.length) throw new Error("请先在 API 设置中配置 qB 服务器");
    const server = selectedServer.value;
    const serverId = String(server?.id || "").trim();
    if (!serverId) throw new Error("所选 qB 服务器无效");
    const revision = lifecycleRevision;
    const controller = new AbortController();
    submitController = controller;
    loading.value = true;
    try {
      const response = await transport.push(
        qbPushPayload({
          serverId,
          torrentId: form.torrentId,
          category: form.category,
          savepath: form.savepath,
        }),
        { signal: controller.signal },
      );
      if (!isCurrentLifecycle(revision)) return null;
      close();
      return { response, server };
    } catch (error) {
      if (!isCurrentLifecycle(revision) || controller.signal.aborted) return null;
      throw error;
    } finally {
      if (submitController === controller) submitController = null;
      if (isCurrentLifecycle(revision)) loading.value = false;
    }
  }

  function dispose() {
    disposed = true;
    lifecycleRevision += 1;
    openRequestSequence += 1;
    submitController?.abort();
    submitController = null;
    close();
    loading.value = false;
  }

  return Object.freeze({
    open: readonly(open),
    loading: readonly(loading),
    form,
    servers,
    label,
    selectedServerLabel,
    openForTorrent,
    close,
    submit,
    dispose,
  });
}
