import { createApp } from "vue";
import App from "./App.vue";
import { tooltipDirective } from "./directives/tooltip";
import { applyTheme, loadTheme } from "./composables/useTheme";
import "./style.css";

const app = createApp(App);
app.directive("tooltip", tooltipDirective);
applyTheme(loadTheme());
app.mount("#app");
