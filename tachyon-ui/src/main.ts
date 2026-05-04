import "./animations";
import { Router } from "./router";

document.addEventListener("DOMContentLoaded", () => {
  const router = new Router();
  (window as unknown as { tachyonRouter?: Router }).tachyonRouter = router;
  const sidebar = document.getElementById("sidebar");
  const toggleSidebar = document.getElementById("toggle-sidebar");

  toggleSidebar?.addEventListener("click", () => {
    sidebar?.classList.toggle("-ml-64");
  });

  router.handleRoute();
});
