import { invoke } from "@tauri-apps/api/core";
import gsap from 'gsap';

document.addEventListener('DOMContentLoaded', () => {
  // 1. Initial GSAP Entrance Animations
  const tl = gsap.timeline();

  tl.from('.sidebar', { x: -50, opacity: 0, duration: 0.6, ease: 'power3.out' })
    .from('.header', { y: -20, opacity: 0, duration: 0.4, ease: 'power2.out' }, '-=0.4')
    .from('.stagger-card', { 
      y: 30, 
      opacity: 0, 
      duration: 0.6, 
      stagger: 0.1, 
      ease: 'back.out(1.2)' 
    }, '-=0.2');

  // Continual subtle pulse for the status dot
  gsap.to('.pulse-dot', {
    opacity: 0.4,
    scale: 0.8,
    duration: 1.5,
    repeat: -1,
    yoyo: true,
    ease: 'sine.inOut'
  });

  // 2. Tauri Bridge Logic
  const refreshBtn = document.getElementById('refresh-btn');
  const activeFaaS = document.getElementById('active-faas');
  
  refreshBtn?.addEventListener('click', async () => {
    // Animate button click
    gsap.fromTo(refreshBtn, { scale: 0.95 }, { scale: 1, duration: 0.2, ease: 'bounce.out' });

    try {
      // NOTE FOR CODEX: Ensure a tauri command named 'get_engine_status' exists in src-tauri/src/main.rs
      // It should return a string or number representing active sandboxes.
      const response = await invoke('get_engine_status');
      
      if (activeFaaS) {
        activeFaaS.innerText = String(response);
        // Flash effect on data update
        gsap.fromTo(activeFaaS, { color: '#22d3ee' }, { color: '#ffffff', duration: 1 });
      }
    } catch (error) {
      console.error('Tauri invoke error:', error);
      if (activeFaaS) activeFaaS.innerText = "Err";
    }
  });
});
