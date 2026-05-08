# Title: UI Internationalization (i18n) & Interactive Onboarding

## Problem Statement
The Tachyon-Mesh UI currently hardcodes all text in English and drops new users directly into complex Edge/Mesh configuration panels. This steep learning curve and lack of localization hinder adoption, particularly for non-English speaking operators and those unfamiliar with the mesh topology concepts.

## Objective
1. Implement a lightweight, Vanilla JS-friendly i18n system to support multiple languages, starting with English and French.
2. Create an interactive onboarding mechanism (Guided Tour/Tooltips) using native Web Components and GSAP to help new users navigate the Control Plane, Asset Registry, and IAM staging features.