import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import { router } from "./router";
import { i18n } from "./i18n";
import { useSessionsStore } from "./stores/sessions";
import { useSettingsStore } from "./stores/settings";
import { applyAppTheme } from "./lib/theme";
import "./style.css";

const app = createApp(App);
const pinia = createPinia();
app.use(pinia);
app.use(router);
app.use(i18n);

useSessionsStore(pinia).init();

const settingsStore = useSettingsStore(pinia);
settingsStore.ensureLoaded().then(() => {
  applyAppTheme(settingsStore.get("app.theme") ?? "dark");
});

app.mount("#app");
