import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { EngineClient } from "./engine/client";
import "./styles.css";

const engine = new EngineClient();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App engine={engine} />
  </StrictMode>,
);
