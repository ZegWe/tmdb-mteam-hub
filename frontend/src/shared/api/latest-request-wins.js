export class StaleRequestError extends Error {
  constructor(message = "请求结果已过期", { cause } = {}) {
    super(message, cause === undefined ? undefined : { cause });
    this.name = "StaleRequestError";
    this.code = "stale_request";
  }
}

export function createLatestRequestWins() {
  let sequence = 0;
  let activeController = null;

  function cancel(reason = new DOMException("Request cancelled", "AbortError")) {
    sequence += 1;
    activeController?.abort(reason);
    activeController = null;
  }

  function isCurrent(requestId) {
    return requestId === sequence;
  }

  async function run(task, { signal: callerSignal } = {}) {
    if (typeof task !== "function") throw new TypeError("task must be a function");

    const requestId = sequence + 1;
    sequence = requestId;
    activeController?.abort(new StaleRequestError());

    const controller = new AbortController();
    activeController = controller;
    const abortFromCaller = () => controller.abort(callerSignal?.reason);

    if (callerSignal?.aborted) {
      abortFromCaller();
    } else if (callerSignal) {
      callerSignal.addEventListener("abort", abortFromCaller, { once: true });
    }

    try {
      const value = await task({ signal: controller.signal, requestId });
      if (requestId !== sequence) throw new StaleRequestError();
      if (controller.signal.aborted) throw controller.signal.reason;
      return value;
    } catch (error) {
      if (requestId !== sequence) throw new StaleRequestError("请求结果已过期", { cause: error });
      throw error;
    } finally {
      callerSignal?.removeEventListener("abort", abortFromCaller);
      if (requestId === sequence && activeController === controller) activeController = null;
    }
  }

  return Object.freeze({ run, cancel, isCurrent });
}
