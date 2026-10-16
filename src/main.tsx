// Fonts ship with the app so it looks the same on every desktop: Inter for
// the interface and Charis SIL, a Charter serif, for reading.
import "@fontsource-variable/inter/opsz.css";
import "@fontsource-variable/inter/opsz-italic.css";
import "@fontsource/charis-sil/400.css";
import "@fontsource/charis-sil/400-italic.css";
import "@fontsource/charis-sil/700.css";
import "@fontsource/charis-sil/700-italic.css";
// Global styles first, so module styles (imported by the components) can
// override the shared primitives without extra specificity.
import "./styles/tokens.css";
import "./styles/base.css";
import "./components/components.css";
import "./app/app.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { createBackend } from "./api";
import { App } from "./app/App";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App backend={createBackend()} />
  </StrictMode>,
);
