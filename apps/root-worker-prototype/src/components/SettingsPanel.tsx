import { useEffect, useMemo, useState, type ChangeEvent } from "react";
import { createPortal } from "react-dom";

import {
  pendingLoginFromResponse,
  resolveOpenAiAuthState,
  type PendingOpenAiLogin,
} from "../lib/accountSettings";
import {
  buildConfigSaveParams,
  buildGlobalSettingsSections,
  buildProviderSettingsGroups,
  buildSettingsConfigState,
  getUnsetDraftValue,
  isSettingsDirty,
  resetFieldDrafts,
  updateFieldDraft,
  type ConfigFieldState,
  type ProviderSettingsGroup,
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

export function SettingsPanel({
  onClose,
  workspacePath,
}: {
  onClose: () => void;
  workspacePath: string;
}) {
  const [status, setStatus] = useState<SettingsPanelStatus>("loading");
  const [fields, setFields] = useState<ConfigFieldState[]>([]);
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
  const dirty = useMemo(() => isSettingsDirty(fields), [fields]);
  const unsetValue = getUnsetDraftValue();
  const providerGroups = useMemo(
    () => buildProviderSettingsGroups(fields),
    [fields],
  );
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
    const params = buildConfigSaveParams(fields, userVersion);
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
      setUserConfigPath(state.userConfigPath ?? response.filePath ?? null);
      setUserVersion(state.userVersion ?? response.version ?? null);
      setNotice(
        response.status === "okOverridden"
          ? "Saved, but a higher-precedence layer still overrides one value."
          : "Saved to user config.",
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

  function setProvider(provider: string | null) {
    setFields((current) =>
      updateFieldDraft(current, "model_provider", provider ?? unsetValue),
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

          {fields.length > 0 ? (
            <>
              <div className="settings-section">
                <div className="settings-section-heading">
                  <h3>Providers</h3>
                </div>
                <div className="settings-provider-list">
                  {providerGroups.map((group) => (
                    <ProviderSettingsCard
                      accountStatus={accountStatus}
                      authError={authError}
                      authState={openAiAuthState}
                      apiKeyDraft={apiKeyDraft}
                      group={group}
                      key={group.id}
                      onApiKeyChange={setApiKeyDraft}
                      onCancelLogin={() => void cancelLogin()}
                      onSelectProvider={setProvider}
                      onStartApiKeyLogin={() => void startApiKeyLogin()}
                      onStartChatgptLogin={() => void startChatgptLogin()}
                      onStartDeviceLogin={() => void startDeviceLogin()}
                      onUpdateDraft={updateDraft}
                      pendingLogin={pendingLogin}
                      unsetValue={unsetValue}
                    />
                  ))}
                </div>
              </div>

              {globalSections.map((section) => (
                <SettingsSectionFields
                  key={section.id}
                  onChange={updateDraft}
                  section={section}
                  unsetValue={unsetValue}
                />
              ))}
            </>
          ) : status !== "loading" ? (
            <div className="settings-state">No editable config values found.</div>
          ) : null}
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

function ProviderSettingsCard({
  accountStatus,
  apiKeyDraft,
  authError,
  authState,
  group,
  onApiKeyChange,
  onCancelLogin,
  onSelectProvider,
  onStartApiKeyLogin,
  onStartChatgptLogin,
  onStartDeviceLogin,
  onUpdateDraft,
  pendingLogin,
  unsetValue,
}: {
  accountStatus: AccountPanelStatus;
  apiKeyDraft: string;
  authError: string | null;
  authState: ReturnType<typeof resolveOpenAiAuthState>;
  group: ProviderSettingsGroup;
  onApiKeyChange: (value: string) => void;
  onCancelLogin: () => void;
  onSelectProvider: (provider: string | null) => void;
  onStartApiKeyLogin: () => void;
  onStartChatgptLogin: () => void;
  onStartDeviceLogin: () => void;
  onUpdateDraft: (keyPath: string, value: string) => void;
  pendingLogin: PendingOpenAiLogin | null;
  unsetValue: string;
}) {
  const isOpenAi = group.id === "openai";
  const isActive = group.status === "active" || group.status === "custom";
  const providerButtonLabel =
    group.id === "custom"
      ? "Current"
      : isActive
        ? "Selected"
        : `Use ${group.title}`;

  return (
    <section className={`settings-provider-card ${isActive ? "active" : ""}`}>
      <header className="settings-provider-header">
        <div className="settings-provider-title">
          <strong>{group.title}</strong>
          <span>{group.description}</span>
        </div>
        <button
          type="button"
          disabled={isActive || group.providerValue == null}
          onClick={() => onSelectProvider(group.providerValue)}
        >
          {providerButtonLabel}
        </button>
      </header>

      {isOpenAi ? (
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
      ) : null}

      {group.id === "modelhub" ? (
        <div className="settings-provider-note">
          ModelHub authentication is managed outside this settings panel.
        </div>
      ) : null}

      {isActive ? (
        <div className="settings-field-list">
          {group.fields.map((field) => (
            <SettingsField
              field={field}
              key={field.keyPath}
              onChange={onUpdateDraft}
              unsetValue={unsetValue}
            />
          ))}
        </div>
      ) : null}
    </section>
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
