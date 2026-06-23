import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import App from "./App.vue";
import "./styles.css";

const EmptyRoute = { template: "" };

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "main", component: EmptyRoute },
    { path: "/subscriptions", name: "subscriptions", component: EmptyRoute },
    { path: "/settings", name: "settings", component: EmptyRoute },
  ],
});

createApp(App).use(router).mount("#app");
