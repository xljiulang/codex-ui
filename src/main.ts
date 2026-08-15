import { createApp } from "vue";
import App from "./App.vue";
import { tooltipDirective } from "./directives/tooltip";
import { applyTheme, loadTheme } from "./composables/useTheme";
import "./styles/index.css";

const app = createApp(App);
app.directive("tooltip", tooltipDirective);
applyTheme(loadTheme());
app.mount("#app");
