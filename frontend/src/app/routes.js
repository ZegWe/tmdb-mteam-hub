export function createAppRoutes() {
  return [
    {
      path: "/",
      name: "main",
      meta: { navPage: "main" },
      component: () => import("../pages/SearchPage.vue"),
    },
    {
      path: "/detail/:mediaType/:id",
      name: "media-detail",
      meta: { navPage: "main" },
      component: () => import("../pages/MediaDetailPage.vue"),
    },
    {
      path: "/subscriptions",
      name: "subscriptions",
      meta: { navPage: "subscriptions" },
      component: () => import("../pages/SubscriptionsPage.vue"),
    },
    {
      path: "/subscriptions/:id",
      name: "subscription-detail",
      meta: { navPage: "subscriptions" },
      component: () => import("../pages/SubscriptionDetailPage.vue"),
    },
    {
      path: "/logs",
      name: "logs",
      meta: { navPage: "logs" },
      component: () => import("../pages/LogsPage.vue"),
    },
    {
      path: "/settings",
      name: "settings",
      meta: { navPage: "settings" },
      component: () => import("../pages/SettingsPage.vue"),
    },
    {
      path: "/:pathMatch(.*)*",
      name: "not-found",
      meta: { navPage: "" },
      component: () => import("../pages/NotFoundPage.vue"),
    },
  ];
}
