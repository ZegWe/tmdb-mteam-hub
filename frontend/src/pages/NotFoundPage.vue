<template>
  <section id="page-not-found" class="app-page is-active">
    <header class="top">
      <h1>页面不存在</h1>
      <p class="sub">当前地址没有对应的功能页面。</p>
    </header>
    <button type="button" class="btn btn-primary" @click="router.replace('/')">返回主功能</button>
  </section>
</template>

<script setup>
import { useRouter } from "vue-router";

const router = useRouter();
</script>
