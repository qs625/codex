import { useEffect, useMemo, useState, type ChangeEvent } from "react";
import { createPortal } from "react-dom";
import QRCode from "qrcode";

import {
  pendingLoginFromResponse,
  resolveOpenAiAuthState,
  type PendingOpenAiLogin,
} from "../lib/accountSettings";
import {
  buildAndroidConnectionPayload,
  validateAndroidConnectionEndpoint,
} from "../lib/androidConnectionPayload";
import {
  buildConfigSaveParams,
  buildGlobalSettingsSections,
  buildSettingsConfigState,
  createModelHubOptionEntry,
  createProviderRegistryEntry,
  getUnsetDraftValue,
  isModelOptionsDirty,
  isProviderRegistryDirty,
  isSettingsDirty,
  resetModelOptionDrafts,
  resetFieldDrafts,
  resetProviderRegistryDrafts,
  updateFieldDraft,
  validateSettingsDrafts,
  type ConfigInventoryRow,
  type ConfigFieldState,
  type ModelOptionEntry,
  type ProviderRegistryEntry,
  type ResourceOverviewSection,
  type ResourceOverviewRow,
  type SettingsFieldSection,
} from "../lib/configSettings";
import { toErrorMessage } from "../lib/shared";
import type {
  AccountLoginCompletedNotification,
  ConfigReadResponse,
  ConfigWriteResponse,
  GetAccountResponse,
  LoginAccountResponse,
} from "../types";

type SettingsPanelStatus = "loading" | "ready" | "saving";
type AccountPanelStatus = "loading" | "ready" | "saving";
type AndroidConnectionInfo = Awaited<
  ReturnType<Window["codexDesktop"]["getAndroidConnectionInfo"]>
>;
type AndroidConnectionDraft = {
  endpoint: string;
  token: string;
  autoEndpoint: string | null;
  autoToken: string | null;
};

export function SettingsPanel({
  onClose,
  workspacePath,
}: {
  onClose: () => void;
  workspacePath: string;
}) {
  const [status, setStatus] = useState<SettingsPanelStatus>("loading");
  const [fields, setFields] = useState<ConfigFieldState[]>([]);
  const [providerRegistry, setProviderRegistry] = useState<
    ProviderRegistryEntry[]
  >([]);
  const [modelOptions, setModelOptions] = useState<ModelOptionEntry[]>([]);
  const [configInventory, setConfigInventory] = useState<ConfigInventoryRow[]>(
    [],
  );
  const [resourceOverview, setResourceOverview] = useState<
    ResourceOverviewSection[]
  >([]);
  const [userConfigPath, setUserConfigPath] = useState<string | null>(null);
  const [userVersion, setUserVersion] = useState<string | null>(null);
  const [accountResponse, setAccountResponse] =
    useState<GetAccountResponse | null>(null);
  const [accountStatus, setAccountStatus] =
    useState<AccountPanelStatus>("loading");
  const [apiKeyDraft, setApiKeyDraft] = useState("");
  const [pendingLogin, setPendingLogin] = useState<PendingOpenAiLogin | null>(
    null,
  );
  const [authError, setAuthError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [androidDraft, setAndroidDraft] = useState<AndroidConnectionDraft>({
    endpoint: "",
    token: "",
    autoEndpoint: null,
    autoToken: null,
  });
  const [androidConnectionInfo, setAndroidConnectionInfo] =
    useState<AndroidConnectionInfo | null>(null);
  const dirty = useMemo(
    () =>
      isSettingsDirty(fields) ||
      isProviderRegistryDirty(providerRegistry) ||
      isModelOptionsDirty(modelOptions),
    [fields, providerRegistry, modelOptions],
  );
  const unsetValue = getUnsetDraftValue();
  const globalSections = useMemo(
    () => buildGlobalSettingsSections(fields),
    [fields],
  );
  const openAiAuthState = useMemo(
    () => resolveOpenAiAuthState(accountResponse),
    [accountResponse],
  );

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  useEffect(() => {
    return window.codexDesktop.subscribe((payload) => {
      if (payload.type === "status" && payload.status?.mobileConnection) {
        applyAndroidConnectionInfo(payload.status.mobileConnection);
        return;
      }
      if (payload.type !== "notification" || !payload.notification) {
        return;
      }
      const { method, params } = payload.notification as {
        method: string;
        params?: unknown;
      };
      if (method === "account/updated") {
        void loadAccount();
      }
      if (method === "account/login/completed") {
        handleLoginCompleted(params as AccountLoginCompletedNotification);
      }
    });
  }, [pendingLogin?.loginId]);

  useEffect(() => {
    let cancelled = false;
    async function loadConfig() {
      setStatus("loading");
      setError(null);
      setNotice(null);
      try {
        const response = (await window.codexDesktop.readConfig({
          includeLayers: true,
          cwd: workspacePath || null,
        })) as ConfigReadResponse;
        if (cancelled) {
          return;
        }
        const state = buildSettingsConfigState(response);
        setFields(state.fields);
        setProviderRegistry(state.providerRegistry);
        setModelOptions(state.modelOptions);
        setConfigInventory(state.configInventory);
        setResourceOverview(state.resourceOverview);
        setUserConfigPath(state.userConfigPath);
        setUserVersion(state.userVersion);
        setStatus("ready");
      } catch (loadError) {
        if (cancelled) {
          return;
        }
        setError(toErrorMessage(loadError));
        setStatus("ready");
      }
    }
    void loadConfig();
    return () => {
      cancelled = true;
    };
  }, [workspacePath]);

  useEffect(() => {
    void loadAccount();
  }, []);

  useEffect(() => {
    let cancelled = false;
    async function loadAndroidConnectionInfo() {
      try {
        const response = await window.codexDesktop.getAndroidConnectionInfo();
        if (cancelled) {
          return;
        }
        applyAndroidConnectionInfo(response);
      } catch (loadError) {
        if (!cancelled) {
          setAndroidConnectionInfo({
            enabled: false,
            reason: toErrorMessage(loadError),
          });
        }
      }
    }
    void loadAndroidConnectionInfo();
    return () => {
      cancelled = true;
    };
  }, []);

  function applyAndroidConnectionInfo(response: AndroidConnectionInfo) {
    setAndroidConnectionInfo(response);
    setAndroidDraft((current) =>
      applyAndroidConnectionInfoDraft(current, response),
    );
  }

  async function loadAccount() {
    setAccountStatus("loading");
    try {
      const response = (await window.codexDesktop.readAccount({
        refreshToken: false,
      })) as GetAccountResponse;
      setAccountResponse(response);
      if (response.account) {
        setPendingLogin(null);
        setAuthError(null);
      }
    } catch (accountError) {
      setAuthError(toErrorMessage(accountError));
    } finally {
      setAccountStatus("ready");
    }
  }

  async function saveSettings() {
    const validationErrors = validateSettingsDrafts(providerRegistry, modelOptions);
    if (validationErrors.length > 0) {
      setError(validationErrors[0] ?? "Settings contain invalid values.");
      return;
    }
    const params = buildConfigSaveParams(
      fields,
      userVersion,
      providerRegistry,
      modelOptions,
    );
    if (!params) {
      setNotice("No changes to save.");
      return;
    }
    setStatus("saving");
    setError(null);
    setNotice(null);
    try {
      const response = (await window.codexDesktop.batchWriteConfig(
        params,
      )) as ConfigWriteResponse;
      const nextRead = (await window.codexDesktop.readConfig({
        includeLayers: true,
        cwd: workspacePath || null,
      })) as ConfigReadResponse;
      const state = buildSettingsConfigState(nextRead);
      setFields(state.fields);
      setProviderRegistry(state.providerRegistry);
      setModelOptions(state.modelOptions);
      setConfigInventory(state.configInventory);
      setResourceOverview(state.resourceOverview);
      setUserConfigPath(state.userConfigPath ?? response.filePath ?? null);
      setUserVersion(state.userVersion ?? response.version ?? null);
      setNotice(
        buildSaveNotice(response.status, Boolean(params.reloadUserConfig)),
      );
    } catch (saveError) {
      setError(toErrorMessage(saveError));
    } finally {
      setStatus("ready");
    }
  }

  function updateDraft(keyPath: string, value: string) {
    setFields((current) => updateFieldDraft(current, keyPath, value));
    setNotice(null);
  }

  function updateProviderEntry(
    id: string,
    patch: Partial<ProviderRegistryEntry>,
  ) {
    setProviderRegistry((current) =>
      current.map((entry) => (entry.id === id ? { ...entry, ...patch } : entry)),
    );
    setNotice(null);
  }

  function updateModelOption(id: string, patch: Partial<ModelOptionEntry>) {
    setModelOptions((current) =>
      current.map((entry) => (entry.id === id ? { ...entry, ...patch } : entry)),
    );
    setNotice(null);
  }

  function handleLoginCompleted(notification: AccountLoginCompletedNotification) {
    if (notification.loginId && !pendingLogin?.loginId) {
      void loadAccount();
      return;
    }
    if (
      notification.loginId &&
      pendingLogin?.loginId &&
      notification.loginId !== pendingLogin.loginId
    ) {
      return;
    }
    setPendingLogin(null);
    if (notification.success) {
      setAuthError(null);
      setNotice("OpenAI authentication completed.");
      void loadAccount();
      return;
    }
    setAuthError(notification.error ?? "OpenAI authentication failed.");
    void loadAccount();
  }

  async function startApiKeyLogin() {
    const apiKey = apiKeyDraft.trim();
    if (!apiKey) {
      setAuthError("Enter an API key before connecting.");
      return;
    }
    setAccountStatus("saving");
    setAuthError(null);
    try {
      await window.codexDesktop.startAccountLogin({
        type: "apiKey",
        apiKey,
      });
      setApiKeyDraft("");
      setNotice("API key connected.");
      await loadAccount();
    } catch (loginError) {
      setAuthError(toErrorMessage(loginError));
    } finally {
      setAccountStatus("ready");
    }
  }

  async function startChatgptLogin() {
    await startManagedLogin({ type: "chatgpt" });
  }

  async function startDeviceLogin() {
    await startManagedLogin({ type: "chatgptDeviceCode" });
  }

  async function startManagedLogin(
    params: { type: "chatgpt" } | { type: "chatgptDeviceCode" },
  ) {
    setAccountStatus("saving");
    setAuthError(null);
    try {
      const response = (await window.codexDesktop.startAccountLogin(
        params,
      )) as LoginAccountResponse;
      const nextPending = pendingLoginFromResponse(response);
      setPendingLogin(nextPending);
      if (nextPending?.authUrl) {
        await window.codexDesktop.openLink(nextPending.authUrl);
        setNotice("Opened ChatGPT sign-in in your browser.");
      }
    } catch (loginError) {
      setAuthError(toErrorMessage(loginError));
    } finally {
      setAccountStatus("ready");
    }
  }

  async function cancelLogin() {
    if (!pendingLogin) {
      return;
    }
    setAccountStatus("saving");
    setAuthError(null);
    try {
      await window.codexDesktop.cancelAccountLogin({
        loginId: pendingLogin.loginId,
      });
      setPendingLogin(null);
      setNotice("OpenAI sign-in canceled.");
      await loadAccount();
    } catch (cancelError) {
      setAuthError(toErrorMessage(cancelError));
    } finally {
      setAccountStatus("ready");
    }
  }

  const panel = (
    <div
      className="settings-layer"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <section
        className="settings-panel"
        aria-label="Settings"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="settings-header">
          <div className="settings-title-block">
            <h2>Settings</h2>
            <span title={userConfigPath ?? undefined}>
              {userConfigPath ?? "User config.toml"}
            </span>
          </div>
          <button type="button" className="settings-close" onClick={onClose}>
            Close
          </button>
        </header>

        <div className="settings-body">
          <nav className="settings-nav" aria-label="Settings sections">
            <a href="#settings-android-companion">Android Companion</a>
            <a href="#settings-providers">Providers</a>
            <a href="#settings-models">Models</a>
            <a href="#settings-editable">Editable Config</a>
            <a href="#settings-all-config">All Config</a>
            <a href="#settings-resources">Resources</a>
          </nav>

          <div className="settings-content">
            {status === "loading" ? (
              <div className="settings-state">Loading config...</div>
            ) : null}

            {error ? (
              <div className="settings-error" role="alert">
                <span>{error}</span>
                <button type="button" onClick={() => setError(null)}>
                  Dismiss
                </button>
              </div>
            ) : null}

            {notice ? <div className="settings-notice">{notice}</div> : null}

            <AndroidConnectionSection
              connectionInfo={androidConnectionInfo}
              endpoint={androidDraft.endpoint}
              onEndpointChange={(value) =>
                setAndroidDraft((current) => ({ ...current, endpoint: value }))
              }
              onTokenChange={(value) =>
                setAndroidDraft((current) => ({ ...current, token: value }))
              }
              token={androidDraft.token}
            />

            <div className="settings-section" id="settings-providers">
              <div className="settings-section-heading">
                <h3>Providers</h3>
              </div>
              <div className="settings-provider-list">
                <OpenAiProviderCard
                  accountStatus={accountStatus}
                  authError={authError}
                  authState={openAiAuthState}
                  apiKeyDraft={apiKeyDraft}
                  onApiKeyChange={setApiKeyDraft}
                  onCancelLogin={() => void cancelLogin()}
                  onStartApiKeyLogin={() => void startApiKeyLogin()}
                  onStartChatgptLogin={() => void startChatgptLogin()}
                  onStartDeviceLogin={() => void startDeviceLogin()}
                  pendingLogin={pendingLogin}
                />
              </div>
            </div>

            <div className="settings-section" id="settings-models">
              <div className="settings-section-heading settings-section-heading-row">
                <h3>Configured Models</h3>
                <button
                  type="button"
                  onClick={() =>
                    setModelOptions((current) => [
                      ...current,
                      createModelHubOptionEntry(current),
                    ])
                  }
                >
                  Add ModelHub Model
                </button>
              </div>
              <div className="settings-provider-list">
                {modelOptions.filter((entry) => !entry.isDeleted).length ===
                0 ? (
                  <div className="settings-state compact">
                    No configured model catalog entries.
                  </div>
                ) : null}
                {modelOptions
                  .filter((entry) => !entry.isDeleted)
                  .map((entry) => (
                    <ModelOptionCard
                      entry={entry}
                      key={entry.id}
                      onChange={updateModelOption}
                    />
                  ))}
              </div>
            </div>

            <div className="settings-section">
              <div className="settings-section-heading settings-section-heading-row">
                <h3>Custom Providers</h3>
                <button
                  type="button"
                  onClick={() =>
                    setProviderRegistry((current) => [
                      ...current,
                      createProviderRegistryEntry(current),
                    ])
                  }
                >
                  Add Provider
                </button>
              </div>
              <div className="settings-provider-list">
                {providerRegistry.filter((entry) => !entry.isDeleted).length ===
                0 ? (
                  <div className="settings-state compact">
                    No custom provider registry entries.
                  </div>
                ) : null}
                {providerRegistry
                  .filter((entry) => !entry.isDeleted)
                  .map((entry) => (
                    <ProviderRegistryCard
                      entry={entry}
                      key={entry.id}
                      onChange={updateProviderEntry}
                    />
                  ))}
              </div>
            </div>

            <div className="settings-section" id="settings-editable">
              <div className="settings-section-heading">
                <h3>Editable Config</h3>
              </div>
              {globalSections.length > 0 ? (
                globalSections.map((section) => (
                  <SettingsSectionFields
                    key={section.id}
                    onChange={updateDraft}
                    section={section}
                    unsetValue={unsetValue}
                  />
                ))
              ) : status !== "loading" ? (
                <div className="settings-state">
                  No editable config values found.
                </div>
              ) : null}
            </div>

            <ReadonlyConfigSection rows={configInventory} />
            <ResourceOverview sections={resourceOverview} />
          </div>
        </div>

        <footer className="settings-actions">
          <div className="settings-status">
            {dirty ? "Unsaved changes" : "Up to date"}
          </div>
          <div className="settings-action-buttons">
            <button
              type="button"
              onClick={() => {
                setFields((current) => resetFieldDrafts(current));
                setProviderRegistry((current) =>
                  resetProviderRegistryDrafts(current),
                );
                setModelOptions((current) =>
                  resetModelOptionDrafts(current),
                );
                setNotice(null);
              }}
              disabled={!dirty || status === "saving"}
            >
              Revert
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void saveSettings()}
              disabled={!dirty || status === "saving"}
            >
              {status === "saving" ? "Saving" : "Save"}
            </button>
          </div>
        </footer>
      </section>
    </div>
  );

  return createPortal(panel, document.body);
}

export function applyAndroidConnectionInfoDraft(
  draft: AndroidConnectionDraft,
  connectionInfo: AndroidConnectionInfo,
): AndroidConnectionDraft {
  if (!connectionInfo.enabled) {
    return draft;
  }
  const followsEndpoint =
    draft.endpoint === "" || draft.endpoint === draft.autoEndpoint;
  const followsToken = draft.token === "" || draft.token === draft.autoToken;
  return {
    endpoint: followsEndpoint ? connectionInfo.endpoint : draft.endpoint,
    token: followsToken ? connectionInfo.token : draft.token,
    autoEndpoint: connectionInfo.endpoint,
    autoToken: connectionInfo.token,
  };
}

export function AndroidConnectionSection({
  connectionInfo,
  endpoint,
  onEndpointChange,
  onTokenChange,
  token,
}: {
  connectionInfo?: AndroidConnectionInfo | null;
  endpoint: string;
  onEndpointChange: (value: string) => void;
  onTokenChange: (value: string) => void;
  token: string;
}) {
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [qrError, setQrError] = useState<string | null>(null);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">(
    "idle",
  );
  const endpointError = validateAndroidConnectionEndpoint(endpoint);
  const payload = useMemo(
    () =>
      endpointError
        ? ""
        : buildAndroidConnectionPayload({
            endpoint,
            token,
          }),
    [endpoint, endpointError, token],
  );

  useEffect(() => {
    let cancelled = false;
    setCopyState("idle");
    setQrError(null);
    if (!payload) {
      setQrDataUrl(null);
      return;
    }
    void QRCode.toDataURL(payload, {
      errorCorrectionLevel: "M",
      margin: 1,
      width: 192,
    })
      .then((dataUrl) => {
        if (!cancelled) {
          setQrDataUrl(dataUrl);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setQrDataUrl(null);
          setQrError(error instanceof Error ? error.message : String(error));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [payload]);

  async function copyPayload() {
    if (!payload || !navigator.clipboard) {
      setCopyState("failed");
      return;
    }
    try {
      await navigator.clipboard.writeText(payload);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  }

  return (
    <div className="settings-section" id="settings-android-companion">
      <div className="settings-section-heading">
        <h3>Android Companion</h3>
      </div>
      <section className="settings-provider-card android-connect-card">
        <header className="settings-provider-header">
          <div className="settings-provider-title">
            <strong>Mobile connect QR</strong>
            <span>Pair the Android companion with this desktop app-server runtime.</span>
          </div>
        </header>
        {connectionInfo?.enabled ? (
          <div className="settings-auth-status authenticated">
            <span>Same-runtime listener ready</span>
            <small>
              Bound at {connectionInfo.bindEndpoint}; QR uses {connectionInfo.endpoint}
            </small>
          </div>
        ) : connectionInfo ? (
          <div className="settings-auth-error">
            Mobile listener unavailable: {connectionInfo.reason}
          </div>
        ) : (
          <div className="settings-state compact">Checking mobile listener...</div>
        )}
        <div className="android-connect-grid">
          <div className="settings-form-grid android-connect-fields">
            <TextInput
              label="WebSocket endpoint"
              onChange={onEndpointChange}
              value={endpoint}
            />
            <TextInput
              label="Bearer token"
              onChange={onTokenChange}
              type="password"
              value={token}
            />
          </div>
          <div className="android-connect-qr-panel">
            {payload && qrDataUrl ? (
              <img
                alt="Android companion connection QR"
                className="android-connect-qr"
                src={qrDataUrl}
              />
            ) : (
              <div className="android-connect-qr-placeholder">
                {endpointError ?? qrError ?? "Generating QR..."}
              </div>
            )}
          </div>
        </div>
        <div className="android-connect-payload-row">
          <code>{payload || "Enter a ws:// or wss:// endpoint to generate a typed payload."}</code>
          <button type="button" onClick={() => void copyPayload()} disabled={!payload}>
            {copyState === "copied" ? "Copied" : "Copy Payload"}
          </button>
        </div>
        {copyState === "failed" ? (
          <div className="settings-auth-error">
            Could not copy the payload. Select the text and copy it manually.
          </div>
        ) : null}
        <div className="settings-provider-note">
          This QR uses the WebSocket listener on the same app-server process as
          the desktop client. Replace the endpoint with your tunnel URL when
          exposing that listener outside the local network.
        </div>
      </section>
    </div>
  );
}

function buildSaveNotice(status: ConfigWriteResponse["status"], reloaded: boolean) {
  const reloadNotice = reloaded
    ? "Runtime-refreshable settings were reloaded."
    : "Model defaults, provider, and catalog changes apply to new threads or explicit run config updates.";
  return status === "okOverridden"
    ? `Saved to user config. ${reloadNotice} A higher-precedence layer still overrides one value.`
    : `Saved to user config. ${reloadNotice}`;
}

function OpenAiProviderCard({
  accountStatus,
  apiKeyDraft,
  authError,
  authState,
  onApiKeyChange,
  onCancelLogin,
  onStartApiKeyLogin,
  onStartChatgptLogin,
  onStartDeviceLogin,
  pendingLogin,
}: {
  accountStatus: AccountPanelStatus;
  apiKeyDraft: string;
  authError: string | null;
  authState: ReturnType<typeof resolveOpenAiAuthState>;
  onApiKeyChange: (value: string) => void;
  onCancelLogin: () => void;
  onStartApiKeyLogin: () => void;
  onStartChatgptLogin: () => void;
  onStartDeviceLogin: () => void;
  pendingLogin: PendingOpenAiLogin | null;
}) {
  return (
    <section className="settings-provider-card">
      <header className="settings-provider-header">
        <div className="settings-provider-title">
          <strong>OpenAI</strong>
          <span>Built-in provider authentication.</span>
        </div>
      </header>

      <OpenAiAuthControls
        accountStatus={accountStatus}
        apiKeyDraft={apiKeyDraft}
        authError={authError}
        authState={authState}
        onApiKeyChange={onApiKeyChange}
        onCancelLogin={onCancelLogin}
        onStartApiKeyLogin={onStartApiKeyLogin}
        onStartChatgptLogin={onStartChatgptLogin}
        onStartDeviceLogin={onStartDeviceLogin}
        pendingLogin={pendingLogin}
      />
    </section>
  );
}

function ModelOptionCard({
  entry,
  onChange,
}: {
  entry: ModelOptionEntry;
  onChange: (id: string, patch: Partial<ModelOptionEntry>) => void;
}) {
  return (
    <section className="settings-provider-card">
      <header className="settings-provider-header">
        <div className="settings-provider-title">
          <strong>{entry.model || "New configured model"}</strong>
          <span>
            {entry.provider || "Provider id"} · appears in the thread model picker
          </span>
        </div>
        <button
          type="button"
          onClick={() => onChange(entry.id, { isDeleted: true })}
        >
          Remove
        </button>
      </header>
      <div className="settings-form-grid">
        <TextInput label="Model ID" value={entry.model} onChange={(model) => onChange(entry.id, { model })} />
        <TextInput label="Provider ID" value={entry.provider} onChange={(provider) => onChange(entry.id, { provider })} />
        <TextInput label="Base URL" value={entry.baseUrl} onChange={(baseUrl) => onChange(entry.id, { baseUrl })} />
        <SelectInput
          label="Wire API"
          value={entry.wireApi}
          options={[
            ["azure_chat_completions", "Azure chat completions"],
            ["responses", "Responses"],
            ["chat_completions", "Chat completions"],
          ]}
          onChange={(wireApi) => onChange(entry.id, { wireApi })}
        />
        <TextInput label="AK query param" value={entry.ak} onChange={(ak) => onChange(entry.id, { ak })} />
        <TextInput label="Env key" value={entry.envKey} onChange={(envKey) => onChange(entry.id, { envKey })} />
        <TextInput label="Context window" value={entry.contextWindow} inputMode="numeric" onChange={(contextWindow) => onChange(entry.id, { contextWindow })} />
        <TextInput label="Max context" value={entry.maxContextWindow} inputMode="numeric" onChange={(maxContextWindow) => onChange(entry.id, { maxContextWindow })} />
        <TextInput label="Auto compact" value={entry.autoCompactTokenLimit} inputMode="numeric" onChange={(autoCompactTokenLimit) => onChange(entry.id, { autoCompactTokenLimit })} />
        <TextInput label="Max tokens" value={entry.maxTokens} inputMode="numeric" onChange={(maxTokens) => onChange(entry.id, { maxTokens })} />
      </div>
    </section>
  );
}

function ProviderRegistryCard({
  entry,
  onChange,
}: {
  entry: ProviderRegistryEntry;
  onChange: (id: string, patch: Partial<ProviderRegistryEntry>) => void;
}) {
  return (
    <section className={`settings-provider-card ${entry.isReadonly ? "readonly" : ""}`}>
      <header className="settings-provider-header">
        <div className="settings-provider-title">
          <strong>{entry.draftId || "New provider"}</strong>
          <span>{entry.readonlyReason ?? "User provider registry entry."}</span>
        </div>
        <button
          type="button"
          disabled={entry.isReadonly}
          onClick={() => onChange(entry.id, { isDeleted: true })}
        >
          Remove
        </button>
      </header>
      {entry.isReadonly ? (
        <div className="settings-readonly-value">
          {JSON.stringify(entry.raw)}
        </div>
      ) : (
        <div className="settings-form-grid">
          <TextInput label="Provider ID" value={entry.draftId} onChange={(draftId) => onChange(entry.id, { draftId })} />
          <TextInput label="Name" value={entry.name} onChange={(name) => onChange(entry.id, { name })} />
          <TextInput label="Base URL" value={entry.baseUrl} onChange={(baseUrl) => onChange(entry.id, { baseUrl })} />
          <SelectInput
            label="Wire API"
            value={entry.wireApi}
            options={[
              ["responses", "Responses"],
              ["azure_chat_completions", "Azure chat completions"],
              ["chat_completions", "Chat completions"],
            ]}
            onChange={(wireApi) => onChange(entry.id, { wireApi })}
          />
          <TextInput label="Env key" value={entry.envKey} onChange={(envKey) => onChange(entry.id, { envKey })} />
        </div>
      )}
    </section>
  );
}

function TextInput({
  inputMode,
  label,
  onChange,
  type = "text",
  value,
}: {
  inputMode?: "numeric";
  label: string;
  onChange: (value: string) => void;
  type?: "password" | "text";
  value: string;
}) {
  return (
    <label className="settings-inline-field">
      <span>{label}</span>
      <input
        inputMode={inputMode}
        type={type}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}

function SelectInput({
  label,
  onChange,
  options,
  value,
}: {
  label: string;
  onChange: (value: string) => void;
  options: Array<[string, string]>;
  value: string;
}) {
  return (
    <label className="settings-inline-field">
      <span>{label}</span>
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        {options.map(([optionValue, optionLabel]) => (
          <option key={optionValue} value={optionValue}>
            {optionLabel}
          </option>
        ))}
      </select>
    </label>
  );
}

function OpenAiAuthControls({
  accountStatus,
  apiKeyDraft,
  authError,
  authState,
  onApiKeyChange,
  onCancelLogin,
  onStartApiKeyLogin,
  onStartChatgptLogin,
  onStartDeviceLogin,
  pendingLogin,
}: {
  accountStatus: AccountPanelStatus;
  apiKeyDraft: string;
  authError: string | null;
  authState: ReturnType<typeof resolveOpenAiAuthState>;
  onApiKeyChange: (value: string) => void;
  onCancelLogin: () => void;
  onStartApiKeyLogin: () => void;
  onStartChatgptLogin: () => void;
  onStartDeviceLogin: () => void;
  pendingLogin: PendingOpenAiLogin | null;
}) {
  const isBusy = accountStatus === "saving";

  return (
    <div className="settings-auth-panel">
      <div className={`settings-auth-status ${authState.status}`}>
        <span>{authState.label}</span>
        <small>{accountStatus === "loading" ? "Checking..." : authState.detail}</small>
      </div>

      {authError ? <div className="settings-auth-error">{authError}</div> : null}

      {pendingLogin ? (
        <div className="settings-auth-pending">
          <div>
            <strong>Sign-in pending</strong>
            {pendingLogin.mode === "device" ? (
              <span>
                {pendingLogin.verificationUrl} · code {pendingLogin.userCode}
              </span>
            ) : (
              <span>Complete the browser sign-in window.</span>
            )}
          </div>
          <button type="button" onClick={onCancelLogin} disabled={isBusy}>
            Cancel
          </button>
        </div>
      ) : null}

      {authState.status !== "authenticated" ? (
        <div className="settings-auth-actions">
          <button type="button" onClick={onStartChatgptLogin} disabled={isBusy}>
            ChatGPT
          </button>
          <button type="button" onClick={onStartDeviceLogin} disabled={isBusy}>
            Device code
          </button>
        </div>
      ) : null}

      {authState.status !== "authenticated" ? (
        <div className="settings-api-key-row">
          <input
            value={apiKeyDraft}
            onChange={(event) => onApiKeyChange(event.target.value)}
            placeholder="OpenAI API key"
            type="password"
          />
          <button type="button" onClick={onStartApiKeyLogin} disabled={isBusy}>
            Connect
          </button>
        </div>
      ) : null}
    </div>
  );
}

function ReadonlyConfigSection({ rows }: { rows: ConfigInventoryRow[] }) {
  return (
    <div className="settings-section" id="settings-all-config">
      <div className="settings-section-heading">
        <h3>All Config</h3>
      </div>
      {rows.length === 0 ? (
        <div className="settings-state compact">No effective config values.</div>
      ) : (
        <div className="settings-inventory-list">
          {rows.map((row) => (
            <ReadonlyConfigRow key={row.keyPath} row={row} />
          ))}
        </div>
      )}
    </div>
  );
}

function ReadonlyConfigRow({ row }: { row: ConfigInventoryRow }) {
  return (
    <div className="settings-inventory-row">
      <div className="settings-inventory-key">
        <span>{row.keyPath}</span>
        <small>
          {row.valueType} · {row.originLabel}
          {row.isEditable ? " · editable above" : ""}
        </small>
      </div>
      <div className="settings-inventory-value">
        <span title={row.summary}>{row.summary}</span>
        {row.detail ? <pre>{row.detail}</pre> : null}
      </div>
    </div>
  );
}

function ResourceOverview({
  sections,
}: {
  sections: ResourceOverviewSection[];
}) {
  return (
    <div className="settings-section" id="settings-resources">
      <div className="settings-section-heading">
        <h3>Resources</h3>
      </div>
      <div className="settings-resource-sections">
        {sections.map((section) => (
          <section className="settings-resource-section" key={section.id}>
            <h4>{section.title}</h4>
            <div className="settings-resource-list">
              {section.rows.map((row, index) => (
                <ResourceOverviewRowView
                  key={`${row.sourceLabel}:${row.keyPath || index}`}
                  row={row}
                />
              ))}
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}

function ResourceOverviewRowView({ row }: { row: ResourceOverviewRow }) {
  return (
    <div className={`settings-resource-row ${row.isEmpty ? "empty" : ""}`}>
      <div className="settings-resource-meta">
        <span>{row.label}</span>
        <small>
          {row.sourceLabel}
          {row.keyPath ? ` · ${row.keyPath}` : ""}
        </small>
      </div>
      <div className="settings-resource-value">
        <span title={row.summary}>{row.summary}</span>
        {row.detail ? <pre>{row.detail}</pre> : null}
      </div>
    </div>
  );
}

function SettingsSectionFields({
  onChange,
  section,
  unsetValue,
}: {
  onChange: (keyPath: string, value: string) => void;
  section: SettingsFieldSection;
  unsetValue: string;
}) {
  return (
    <div className="settings-section">
      <div className="settings-section-heading">
        <h3>{section.title}</h3>
      </div>
      <div className="settings-field-list">
        {section.fields.map((field) => (
          <SettingsField
            field={field}
            key={field.keyPath}
            onChange={onChange}
            unsetValue={unsetValue}
          />
        ))}
      </div>
    </div>
  );
}

function SettingsField({
  field,
  onChange,
  unsetValue,
}: {
  field: ConfigFieldState;
  onChange: (keyPath: string, value: string) => void;
  unsetValue: string;
}) {
  const changed = field.draftValue !== field.effectiveValue;
  const effectiveLabel = formatDraftValue(field, field.effectiveValue, unsetValue);

  return (
    <label className={`settings-field ${changed ? "dirty" : ""}`}>
      <div className="settings-field-meta">
        <span>{field.label}</span>
        <small>
          {field.keyPath} · {field.originLabel}
        </small>
      </div>
      <div className="settings-control">
        {field.isUnsupported ? (
          <div className="settings-readonly-value">
            {field.unsupportedValue ?? "Unsupported value"}
          </div>
        ) : field.kind === "select" ? (
          <select
            value={field.draftValue}
            onChange={(event: ChangeEvent<HTMLSelectElement>) =>
              onChange(field.keyPath, event.target.value)
            }
          >
            <option value={unsetValue}>{field.unsetLabel ?? "Unset"}</option>
            {field.options?.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        ) : (
          <input
            value={field.draftValue === unsetValue ? "" : field.draftValue}
            onChange={(event: ChangeEvent<HTMLInputElement>) =>
              onChange(
                field.keyPath,
                event.target.value.trim() ? event.target.value : unsetValue,
              )
            }
            placeholder={field.placeholder}
          />
        )}
        <small title={effectiveLabel}>Effective: {effectiveLabel}</small>
      </div>
    </label>
  );
}

function formatDraftValue(
  field: ConfigFieldState,
  value: string,
  unsetValue: string,
) {
  if (value === unsetValue) {
    return field.unsetLabel ?? "Unset";
  }
  return field.options?.find((option) => option.value === value)?.label ?? value;
}

export function countDirtySettingsFields(fields: ConfigFieldState[]) {
  return fields.filter(
    (field) => !field.isUnsupported && field.draftValue !== field.effectiveValue,
  ).length;
}
