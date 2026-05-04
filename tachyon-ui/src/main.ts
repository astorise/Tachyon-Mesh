import "./animations";
import { Router } from "./router";

document.addEventListener("DOMContentLoaded", () => {
  const router = new Router();
  const sidebar = document.getElementById("sidebar");
  const toggleSidebar = document.getElementById("toggle-sidebar");

  toggleSidebar?.addEventListener("click", () => {
    sidebar?.classList.toggle("-ml-64");
  });

  router.handleRoute();
});
