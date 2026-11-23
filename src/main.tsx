import React from "react";
import ReactDOM from "react-dom/client";
import { isTauri } from "@tauri-apps/api/core";
import App from "./App";
import { AppearanceProvider } from "./components/AppearanceProvider";
import { TooltipProvider } from "./components/ui/overlays";
import { ToastProvider } from "./components/ui/toast";
import { bootstrapAppearance } from "./lib/appearance";
import "./styles.css";

bootstrapAppearance();
void import("./lib/performanceDiagnostics").then(async ({installPerformanceDiagnostics})=>{
  const cleanup = await installPerformanceDiagnostics();
  import.meta.hot?.dispose(() => cleanup?.());
}).catch(()=>undefined);
document.documentElement.dataset.windowPlatform = isTauri() && /Mac/i.test(navigator.platform) ? "macos" : "default";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AppearanceProvider>
      <TooltipProvider>
        <ToastProvider>
          <App />
        </ToastProvider>
      </TooltipProvider>
    </AppearanceProvider>
  </React.StrictMode>
);
