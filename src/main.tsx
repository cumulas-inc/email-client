import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { createBackend } from "./api";
import { App } from "./app/App";
import "./styles/tokens.css";
import "./styles/base.css";
import "./components/components.css";
import "./app/app.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App backend={createBackend()} />
  </StrictMode>,
);
