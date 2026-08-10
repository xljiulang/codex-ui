import { createApp } from "vue";
import App from "./App.vue";
import { tooltipDirective } from "./directives/tooltip";
import "./style.css";

const app = createApp(App);
app.directive("tooltip", tooltipDirective);
app.mount("#app");
