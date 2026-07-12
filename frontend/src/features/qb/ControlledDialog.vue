<template>
  <dialog
    ref="dialogElement"
    :aria-labelledby="labelledby || undefined"
    @cancel="handleCancel"
    @close="handleNativeClose"
    @click.self="requestClose('backdrop')"
  >
    <slot :request-close="requestClose" />
  </dialog>
</template>

<script setup>
import { nextTick, onBeforeUnmount, ref, watch } from "vue";

const props = defineProps({
  open: { type: Boolean, default: false },
  labelledby: { type: String, default: "" },
  initialFocus: { type: String, default: "" },
  returnFocus: { type: Object, default: null },
});
const emit = defineEmits(["request-close"]);
const dialogElement = ref(null);
let returnFocusTarget = null;
let disposing = false;

function focusableElement(value) {
  return typeof HTMLElement !== "undefined" && value instanceof HTMLElement ? value : null;
}

function enabledFocusableElement(value) {
  const element = focusableElement(value);
  if (!element || element.matches(":disabled, [aria-disabled='true']") || element.tabIndex < 0) {
    return null;
  }
  return element;
}

function captureReturnFocus() {
  returnFocusTarget = focusableElement(props.returnFocus);
}

function focusInitialControl() {
  const dialog = dialogElement.value;
  if (!dialog?.open || !props.initialFocus) return;
  const target = [...dialog.querySelectorAll(props.initialFocus)]
    .map(enabledFocusableElement)
    .find(Boolean);
  target?.focus();
}

function restoreFocus() {
  if (disposing) {
    returnFocusTarget = null;
    return;
  }
  const target = returnFocusTarget;
  returnFocusTarget = null;
  if (!target?.isConnected) return;
  void nextTick(() => target.focus());
}

function showNativeDialog() {
  const dialog = dialogElement.value;
  if (!dialog?.isConnected || dialog.open) return;
  captureReturnFocus();
  if (typeof dialog.showModal === "function") dialog.showModal();
  else dialog.setAttribute("open", "");
  void nextTick(focusInitialControl);
}

function closeNativeDialog() {
  const dialog = dialogElement.value;
  if (dialog?.open) {
    if (typeof dialog.close === "function") dialog.close();
    else dialog.removeAttribute("open");
  }
  restoreFocus();
}

function syncNativeState(open) {
  if (open) showNativeDialog();
  else closeNativeDialog();
}

function requestClose(reason = "dismiss") {
  if (!props.open || disposing) return;
  emit("request-close", reason);
}

function handleCancel(event) {
  event.preventDefault();
  requestClose("cancel");
}

function handleNativeClose() {
  restoreFocus();
  requestClose("native-close");
}

watch(() => props.open, syncNativeState, { immediate: true, flush: "post" });

onBeforeUnmount(() => {
  disposing = true;
  const dialog = dialogElement.value;
  if (dialog?.open) {
    if (typeof dialog.close === "function") dialog.close();
    else dialog.removeAttribute("open");
  }
  returnFocusTarget = null;
});
</script>

<style src="./styles.css"></style>
