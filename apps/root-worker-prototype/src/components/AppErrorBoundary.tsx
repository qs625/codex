import React from "react";

type AppErrorBoundaryProps = {
  children: React.ReactNode;
};

type AppErrorBoundaryState = {
  error: Error | null;
};

export function formatErrorBoundaryMessage(error: Error | null) {
  return error?.message?.trim() || "The renderer hit an unexpected error.";
}

export function rootMountErrorFallbackHtml(message: string) {
  const escapedMessage = message
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
  return `<main class="app-error-boundary" role="alert"><div class="app-error-boundary-panel"><p class="app-error-boundary-eyebrow">Renderer error</p><h1>Root Worker needs a refresh</h1><p>${escapedMessage}</p><button type="button" onclick="window.location.reload()">Reload</button></div></main>`;
}

export function installRootMountErrorFallback(
  documentRef: Pick<Document, "body">,
  error: Error,
) {
  documentRef.body.innerHTML = rootMountErrorFallbackHtml(
    formatErrorBoundaryMessage(error),
  );
}

export function AppErrorFallback({ error }: { error: Error | null }) {
  return (
    <main className="app-error-boundary" role="alert">
      <div className="app-error-boundary-panel">
        <p className="app-error-boundary-eyebrow">Renderer error</p>
        <h1>Root Worker needs a refresh</h1>
        <p>{formatErrorBoundaryMessage(error)}</p>
        <button type="button" onClick={() => window.location.reload()}>
          Reload
        </button>
      </div>
    </main>
  );
}

export class AppErrorBoundary extends React.Component<
  AppErrorBoundaryProps,
  AppErrorBoundaryState
> {
  state: AppErrorBoundaryState = {
    error: null,
  };

  static getDerivedStateFromError(error: Error): AppErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error("Root Worker renderer crashed", error, errorInfo);
  }

  render() {
    if (this.state.error) {
      return <AppErrorFallback error={this.state.error} />;
    }

    return this.props.children;
  }
}
