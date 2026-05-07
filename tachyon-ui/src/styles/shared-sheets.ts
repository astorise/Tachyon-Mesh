import stylesheetText from "../style.css?inline";

export const tachyonSharedStylesheet = new CSSStyleSheet();
tachyonSharedStylesheet.replaceSync(stylesheetText);
