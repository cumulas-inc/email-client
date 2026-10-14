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
