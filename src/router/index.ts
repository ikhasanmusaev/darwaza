import { createRouter, createWebHistory, RouteRecordRaw } from "vue-router";

const routes: RouteRecordRaw[] = [
  { path: "/", redirect: "/hosts" },
  {
    path: "/hosts",
    name: "hosts",
    component: () => import("../pages/HostsListPage.vue"),
  },
  {
    path: "/hosts/new",
    name: "host-new",
    component: () => import("../pages/HostFormPage.vue"),
  },
  {
    path: "/hosts/:id/edit",
    name: "host-edit",
    component: () => import("../pages/HostFormPage.vue"),
    props: true,
  },
  {
    path: "/snippets",
    name: "snippets",
    component: () => import("../pages/SnippetsListPage.vue"),
  },
  {
    path: "/forwards",
    name: "forwards",
    component: () => import("../pages/PortForwardsPage.vue"),
  },
  {
    path: "/recordings",
    name: "recordings",
    component: () => import("../pages/RecordingsPage.vue"),
  },
  {
    path: "/settings",
    name: "settings",
    component: () => import("../pages/SettingsPage.vue"),
  },
  {
    path: "/identities",
    name: "identities",
    component: () => import("../pages/IdentitiesPage.vue"),
  },
  {
    path: "/identities/new",
    name: "identity-new",
    component: () => import("../pages/IdentityFormPage.vue"),
  },
  {
    path: "/identities/:id/edit",
    name: "identity-edit",
    component: () => import("../pages/IdentityFormPage.vue"),
    props: true,
  },
  { path: "/settings/identities", redirect: "/identities" },
  { path: "/settings/identities/new", redirect: "/identities/new" },
  { path: "/settings/identities/:id/edit", redirect: (to) => `/identities/${to.params.id}/edit` },
  {
    path: "/settings/known-hosts",
    name: "settings-known-hosts",
    component: () => import("../pages/KnownHostsPage.vue"),
  },
];

export const router = createRouter({
  history: createWebHistory(),
  routes,
});
