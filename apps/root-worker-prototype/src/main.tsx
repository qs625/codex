import React from "react";
import ReactDOM from "react-dom/client";
import "@xterm/xterm/css/xterm.css";
import App from "./App";
import {
  AppErrorBoundary,
  installRootMountErrorFallback,
} from "./components/AppErrorBoundary";
import "./styles.css";

const rootElement = document.getElementById("root");

if (!rootElement) {
  const error = new Error("The renderer root element is missing.");
  console.error("Root Worker renderer failed to mount", error);
  installRootMountErrorFallback(document, error);
} else {
  ReactDOM.createRoot(rootElement).render(
    <React.StrictMode>
      <AppErrorBoundary>
        <App />
      </AppErrorBoundary>
    </React.StrictMode>,
  );
}
