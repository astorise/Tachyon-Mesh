import gsap from "gsap";

window.addEventListener("DOMContentLoaded", () => {
  gsap.from(".nav-link", {
    opacity: 0,
    x: -20,
    stagger: 0.1,
    duration: 0.35,
    ease: "power2.out",
  });
});

window.addEventListener("route-change", (event) => {
  const { renderer, container } = (event as CustomEvent<{
    renderer: () => string;
    container: HTMLElement;
  }>).detail;

  gsap.to(container.children, {
    opacity: 0,
    y: -8,
    duration: 0.16,
    ease: "power1.in",
    onComplete: () => {
      container.innerHTML = renderer();
      gsap.fromTo(
        container.children,
        { opacity: 0, y: 10 },
        { opacity: 1, y: 0, duration: 0.3, ease: "power2.out" },
      );
    },
  });
});
