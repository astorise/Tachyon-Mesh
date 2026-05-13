# Technical Specification: Loading States & UX Foundation

## 1. CSS Skeletons (`tachyon-ui/src/style.css`)
Leverage Tailwind v4 to create utility classes for skeleton loading. Add this to the base styles:

```css
@layer utilities {
  .skeleton-pulse {
    @apply animate-pulse bg-gray-200 dark:bg-gray-800 rounded;
  }
  .skeleton-text {
    @apply skeleton-pulse h-4 w-3/4 mb-2;
  }
  .skeleton-block {
    @apply skeleton-pulse h-32 w-full rounded-lg;
  }
}
```

## 2. The `WithLoadingState` Abstraction (`tachyon-ui/src/components/base/TachyonConfigDashboard.ts`)
Since we are using Vanilla Web Components, we can implement this as a protected method in the base class that all 11 panels extend.

```typescript
// Inside TachyonConfigDashboard.ts
protected async withLoadingState<T>(
  task: () => Promise<T>, 
  containerSelector: string = '.panel-content'
): Promise<T | void> {
  const container = this.querySelector(containerSelector);
  if (container) {
    container.innerHTML = `
      <div class="p-4 w-full">
        <div class="skeleton-text w-1/3"></div>
        <div class="skeleton-block"></div>
        <div class="skeleton-text w-1/2 mt-4"></div>
      </div>
    `;
  }

  try {
    const result = await task();
    return result;
  } catch (error) {
    this.handlePanelError(error, task);
  }
}

protected handlePanelError(error: any, retryTask?: Function) {
  // Clear skeleton, show error icon
  // Trigger actionable toast if retryTask is provided
}
```

## 3. Actionable Toasts (`tachyon-ui/src/components/layout/TachyonToastManager.ts`)
Update the `showToast` method signature to accept an action payload.

```typescript
export interface ToastOptions {
  type: 'info' | 'success' | 'error' | 'warning';
  message: string;
  action?: {
    label: string;
    onClick: () => void;
  };
}

// In TachyonToastManager rendering logic:
if (options.action) {
  const btn = document.createElement('button');
  btn.className = 'ml-4 px-3 py-1 text-xs bg-white/20 hover:bg-white/30 rounded';
  btn.textContent = options.action.label;
  btn.addEventListener('click', () => {
    options.action!.onClick();
    this.dismissToast(toastId);
  });
  toastElement.appendChild(btn);
}
```