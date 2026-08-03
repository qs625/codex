import { useEffect, useMemo, useState, type ChangeEvent } from "react";
import { createPortal } from "react-dom";

import {
  CONFIG_SECTION_LABELS,
  buildConfigSaveParams,
  buildSettingsConfigState,
  getUnsetDraftValue,
  isSettingsDirty,
  resetFieldDrafts,
  updateFieldDraft,
  type ConfigFieldState,
} from "../lib/configSettings";
import { toErrorMessage } from "../lib/shared";
import type {
  ConfigReadResponse,
  ConfigWriteResponse,
} from "../types";

type SettingsPanelStatus = "loading" | "ready" | "saving";

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
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const dirty = useMemo(() => isSettingsDirty(fields), [fields]);
  const unsetValue = getUnsetDraftValue();

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

  const groupedFields = useMemo(
    () =>
      fields.reduce<Record<string, ConfigFieldState[]>>((groups, field) => {
        groups[field.section] = [...(groups[field.section] ?? []), field];
        return groups;
      }, {}),
    [fields],
  );

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
            (["model", "execution", "desktop"] as const).map((section) => {
              const sectionFields = groupedFields[section] ?? [];
              if (sectionFields.length === 0) {
                return null;
              }
              return (
                <div className="settings-section" key={section}>
                  <div className="settings-section-heading">
                    <h3>{CONFIG_SECTION_LABELS[section]}</h3>
                  </div>
                  <div className="settings-field-list">
                    {sectionFields.map((field) => (
                      <SettingsField
                        field={field}
                        key={field.keyPath}
                        onChange={updateDraft}
                        unsetValue={unsetValue}
                      />
                    ))}
                  </div>
                </div>
              );
            })
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
