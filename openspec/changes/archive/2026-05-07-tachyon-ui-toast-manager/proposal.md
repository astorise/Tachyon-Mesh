# Proposal: Zero-Panic Global Toast Notification System

## 1. Context
While our `TachyonConfigDashboard` base class handles local form feedback (`showFeedback`), systemic events (e.g., connection lost to Rust core, background sync successes) need a global way to notify the user without breaking the current view.

## 2. Solution
Create a floating `<tachyon-toast-manager>` Web Component. It sits outside the App Shell routing view and listens for custom events (`app:notify`).
It strictly enforces the "Zero-Panic" policy: Rust errors are displayed as elegant, dismissible red toasts rather than crashing the UI or polluting the console silently.