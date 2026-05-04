# Design: Tachyon-UI App Shell

## 1. The HTML Skeleton (`index.html`)
The main entry point MUST use the following structural pattern with Tailwind utility classes for a dark "Edge-Native" aesthetic.

    <div id="app" class="flex h-screen w-full bg-slate-950 text-slate-50 overflow-hidden font-sans">
        <aside id="sidebar" class="w-64 bg-slate-900 flex-shrink-0 border-r border-slate-800 flex flex-col transition-all duration-300">
            <div class="h-16 flex items-center px-6 border-b border-slate-800">
                <span class="text-lg font-bold text-cyan-400">Tachyon Mesh</span>
            </div>
            <nav id="sidebar-nav" class="flex-grow py-4 overflow-y-auto">
                <a href="#/dashboard" class="nav-link flex items-center gap-3 px-4 py-2 mx-2 rounded-md text-slate-400 hover:text-cyan-400 hover:bg-slate-800/50 transition-colors">Dashboard</a>
                <a href="#/routing" class="nav-link flex items-center gap-3 px-4 py-2 mx-2 rounded-md text-slate-400 hover:text-cyan-400 hover:bg-slate-800/50 transition-colors">Routing & Gateways</a>
                <a href="#/security" class="nav-link flex items-center gap-3 px-4 py-2 mx-2 rounded-md text-slate-400 hover:text-cyan-400 hover:bg-slate-800/50 transition-colors">Security & IAM</a>
            </nav>
        </aside>

        <div class="flex flex-col flex-grow w-full">
            <header id="topbar" class="h-16 bg-slate-950 border-b border-slate-800 flex items-center justify-between px-6">
                <button id="toggle-sidebar" class="text-slate-400 hover:text-slate-100 focus:outline-none">
                    [Menu]
                </button>
                <div class="flex items-center gap-4">
                    <span id="network-status" class="text-sm text-emerald-400">Connected</span>
                    <span id="user-role" class="text-sm bg-slate-800 px-2 py-1 rounded">Admin</span>
                </div>
            </header>

            <main id="route-view" class="flex-grow p-6 overflow-y-auto">
                </main>
        </div>
    </div>

## 2. The Vanilla JS Router (`src/router.ts`)
Avoid external dependencies. Use a simple class observing window hashes.

    export class Router {
        private routes: Record<string, () => string>;
        
        constructor() {
            this.routes = {
                '/dashboard': () => `<div><h1 class="text-2xl text-slate-100">Dashboard</h1><p class="text-slate-400">Overview of your Edge Mesh</p></div>`,
                '/routing': () => `<div><h1 class="text-2xl text-slate-100">Routing</h1><p class="text-slate-400">L4/L7 Configuration</p></div>`
            };
            window.addEventListener('hashchange', () => this.handleRoute());
        }

        public handleRoute() {
            const path = window.location.hash.replace('#', '') || '/dashboard';
            const renderer = this.routes[path] || this.routes['/dashboard'];
            const viewContainer = document.getElementById('route-view');
            
            if (viewContainer) {
                // Trigger GSAP transition out, swap content, transition in
                window.dispatchEvent(new CustomEvent('route-change', { detail: { renderer, container: viewContainer } }));
            }
        }
    }

## 3. GSAP Animations (`src/animations.ts`)
The UI must feel fluid and premium.

1.  **Sidebar Stagger**: On initial load, `.nav-link` elements animate in using `gsap.from(".nav-link", { opacity: 0, x: -20, stagger: 0.1, ease: "power2.out" })`.
2.  **View Transitions**: Listen to the `route-change` event. Use GSAP to fade out the `#route-view` content, inject the new HTML string, and fade it back in (`gsap.fromTo(container.children, { opacity: 0, y: 10 }, { opacity: 1, y: 0, duration: 0.3 })`).