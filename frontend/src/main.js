import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import AuthGate from "./app/AuthGate.vue";
import { createAppRoutes } from "./app/routes.js";
import "./styles/foundation.css";
import "./styles/base.css";
import "./styles.css";

const router = createRouter({
  history: createWebHashHistory(),
  routes: createAppRoutes(),
});

createApp(AuthGate).use(router).mount("#app");
