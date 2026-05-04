# Proposal: Tachyon-UI Routing & Gateways Dashboard

## Context
The Tachyon-UI App Shell is now capable of navigating between logical views. The first core view to implement is the Layer 4 / Layer 7 Traffic Management dashboard, corresponding to the GitOps `TrafficConfiguration` schema.

## Problem
Without a reactive framework like React, managing complex nested forms (like adding a Route with multiple headers and path matching rules) and converting that DOM state into a strict JSON payload can become messy. We need a clean Vanilla JS approach to build the DOM, bind event listeners, and extract the JSON payload.

## Solution
Implement the Routing View component.
1. **Layout**: A two-column Tailwind CSS grid showing active "Gateways" on the left and "Routes" on the right.
2. **Interactive Form**: A slide-over panel to construct a new Route.
3. **Data Extraction**: A pure JS controller that traverses the form inputs and maps them directly to the `config-routing.wit` JSON schema before pushing to the MCP/API.