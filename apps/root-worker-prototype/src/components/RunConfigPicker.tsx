import { useEffect, useMemo, useRef, useState } from "react";

import { ChevronDownIcon } from "./icons";
import {
  ensureCurrentModelVisible,
  getRunModelLabel,
  getSupportedReasoningEfforts,
  normalizeModelListResponse,
  resolveSelectionForModel,
} from "../lib/runConfig";
import { toErrorMessage } from "../lib/shared";
import {
  getThreadModelLabel,
  getThreadReasoningLabel,
} from "../lib/thread";
import type { RunModel, RunModelListResponse, Thread } from "../types";

export function RunConfigPicker({
  disabled,
  onApply,
  selectedThread,
}: {
  disabled: boolean;
  onApply: (selection: { model: string; reasoningEffort: string }) => void;
  selectedThread: Thread | null;
}) {
  const [isOpen, setIsOpen] = useState(false);
  const [models, setModels] = useState<RunModel[]>([]);
  const [hasRequestedModels, setHasRequestedModels] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [draftModel, setDraftModel] = useState<string | null>(null);
  const [draftReasoningEffort, setDraftReasoningEffort] = useState<
    string | null
  >(null);
  const [fallbackMessage, setFallbackMessage] = useState<string | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);
  const openThreadIdRef = useRef<string | null>(null);

  const activeModelLabel = getThreadModelLabel(selectedThread);
  const activeReasoningLabel = getThreadReasoningLabel(selectedThread);
  const selectedModel = useMemo(
    () => models.find((model) => model.model === draftModel) ?? null,
    [draftModel, models],
  );
  const supportedEfforts = selectedModel
    ? getSupportedReasoningEfforts(selectedModel)
    : [];
  const canApply =
    selectedThread != null &&
    selectedModel != null &&
    !selectedModel.current &&
    draftReasoningEffort != null &&
    !disabled;
  const hasChanged =
    selectedThread != null &&
    (draftModel !== selectedThread.model ||
      draftReasoningEffort !== selectedThread.reasoningEffort);

  useEffect(() => {
    if (!isOpen) {
      return;
    }
    openThreadIdRef.current = selectedThread?.id ?? null;
    setFallbackMessage(null);
    setDraftModel(selectedThread?.model ?? null);
    setDraftReasoningEffort(selectedThread?.reasoningEffort ?? null);
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen || openThreadIdRef.current === selectedThread?.id) {
      return;
    }
    closePopover();
  }, [isOpen, selectedThread?.id]);

  useEffect(() => {
    if (!isOpen || hasRequestedModels || isLoading) {
      return;
    }
    void loadModels();
  }, [hasRequestedModels, isLoading, isOpen]);

  useEffect(() => {
    if (!isOpen) {
      return;
    }

    function handlePointerDown(event: MouseEvent) {
      const target = event.target;
      if (!(target instanceof Node)) {
        return;
      }
      if (
        popoverRef.current?.contains(target) ||
        triggerRef.current?.contains(target)
      ) {
        return;
      }
      closePopover();
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        closePopover();
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const focusableElements = getPopoverFocusableElements(popoverRef.current);
      if (focusableElements.length === 0) {
        event.preventDefault();
        return;
      }
      const firstElement = focusableElements[0];
      const lastElement = focusableElements.at(-1);
      if (!firstElement || !lastElement) {
        return;
      }
      if (
        event.shiftKey &&
        (document.activeElement === firstElement ||
          document.activeElement === popoverRef.current)
      ) {
        event.preventDefault();
        lastElement.focus();
        return;
      }
      if (!event.shiftKey && document.activeElement === lastElement) {
        event.preventDefault();
        firstElement.focus();
      }
    }

    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    window.setTimeout(() => popoverRef.current?.focus(), 0);

    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen]);

  async function loadModels() {
    setHasRequestedModels(true);
    setIsLoading(true);
    setLoadError(null);
    try {
      const response =
        (await window.codexDesktop.listModels()) as RunModelListResponse;
      const normalizedModels = ensureCurrentModelVisible(
        normalizeModelListResponse(response),
        selectedThread?.model ?? null,
        selectedThread?.reasoningEffort ?? null,
      );
      setModels(normalizedModels);
      const currentModel =
        normalizedModels.find(
          (model) => model.model === selectedThread?.model,
        ) ?? null;
      if (currentModel && !currentModel.current) {
        syncDraftForModel(currentModel, selectedThread?.reasoningEffort ?? null);
      }
    } catch (error) {
      setLoadError(toErrorMessage(error));
    } finally {
      setIsLoading(false);
    }
  }

  function closePopover() {
    setIsOpen(false);
    triggerRef.current?.focus();
  }

  function syncDraftForModel(model: RunModel, currentEffort: string | null) {
    const selection = resolveSelectionForModel(model, currentEffort);
    setDraftModel(selection.model);
    setDraftReasoningEffort(selection.reasoningEffort);
    setFallbackMessage(
      currentEffort && currentEffort !== selection.reasoningEffort
        ? "已回退到该模型默认 reasoning"
        : null,
    );
  }

  function selectModel(model: RunModel) {
    if (model.current) {
      setDraftModel(model.model);
      setDraftReasoningEffort(selectedThread?.reasoningEffort ?? null);
      setFallbackMessage(null);
      return;
    }
    syncDraftForModel(model, draftReasoningEffort);
  }

  function applySelection() {
    if (!canApply || draftModel == null || draftReasoningEffort == null) {
      return;
    }
    onApply({
      model: draftModel,
      reasoningEffort: draftReasoningEffort,
    });
    closePopover();
  }

  return (
    <div className="run-config-picker">
      <button
        ref={triggerRef}
        type="button"
        className="run-config-trigger"
        aria-label={`运行配置，当前模型 ${activeModelLabel}，reasoning ${activeReasoningLabel}`}
        aria-expanded={isOpen}
        aria-haspopup="dialog"
        disabled={!selectedThread}
        onClick={(event) => {
          event.stopPropagation();
          setIsOpen((current) => !current);
        }}
      >
        <span className="run-config-trigger-title">运行配置</span>
        <span className="run-config-trigger-meta">
          {activeModelLabel} · {activeReasoningLabel}
        </span>
        <ChevronDownIcon />
      </button>

      {isOpen ? (
        <div
          ref={popoverRef}
          className="run-config-popover"
          role="dialog"
          aria-label="运行配置"
          tabIndex={-1}
          onClick={(event) => event.stopPropagation()}
        >
          <RunConfigPopoverContent
            canApply={canApply}
            disabled={disabled}
            draftModel={draftModel}
            draftReasoningEffort={draftReasoningEffort}
            fallbackMessage={fallbackMessage}
            hasChanged={hasChanged}
            isLoading={isLoading}
            loadError={loadError}
            models={models}
            onApply={applySelection}
            onCancel={closePopover}
            onRetry={() => void loadModels()}
            onSelectModel={selectModel}
            onSelectReasoningEffort={setDraftReasoningEffort}
            supportedEfforts={supportedEfforts}
          />
        </div>
      ) : null}
    </div>
  );
}

export function RunConfigPopoverContent({
  canApply,
  disabled,
  draftModel,
  draftReasoningEffort,
  fallbackMessage,
  hasChanged,
  isLoading,
  loadError,
  models,
  onApply,
  onCancel,
  onRetry,
  onSelectModel,
  onSelectReasoningEffort,
  supportedEfforts,
}: {
  canApply: boolean;
  disabled: boolean;
  draftModel: string | null;
  draftReasoningEffort: string | null;
  fallbackMessage: string | null;
  hasChanged: boolean;
  isLoading: boolean;
  loadError: string | null;
  models: RunModel[];
  onApply: () => void;
  onCancel: () => void;
  onRetry: () => void;
  onSelectModel: (model: RunModel) => void;
  onSelectReasoningEffort: (effort: string) => void;
  supportedEfforts: string[];
}) {
  return (
    <>
      <div className="run-config-popover-header">
        <span>运行配置</span>
        <small>更改后仅影响当前 thread 的后续消息</small>
      </div>

      {isLoading ? (
        <div className="run-config-state">正在加载模型...</div>
      ) : null}

      {loadError ? (
        <div className="run-config-error">
          <span>模型列表加载失败，当前配置未受影响。{loadError}</span>
          <button type="button" onClick={onRetry}>
            重试
          </button>
        </div>
      ) : null}

      {!isLoading && !loadError && models.length === 0 ? (
        <div className="run-config-state">
          暂无可用模型，当前配置未受影响。
        </div>
      ) : null}

      {!isLoading && !loadError && models.length > 0 ? (
        <>
          <div className="run-config-field">
            <div className="run-config-label-row">
              <span>模型</span>
              <small>来自 model/list</small>
            </div>
            <div className="run-config-model-list" role="radiogroup">
              {models.map((model) => {
                const selected = model.model === draftModel;
                return (
                  <button
                    key={model.id}
                    type="button"
                    className={selected ? "selected" : ""}
                    role="radio"
                    aria-checked={selected}
                    onClick={() => onSelectModel(model)}
                  >
                    <span className="run-config-radio-dot" />
                    <span className="run-config-model-copy">
                      <span className="run-config-model-name">
                        <span className="run-config-model-name-text">
                          {getRunModelLabel(model)}
                        </span>
                        {model.current ? (
                          <span className="run-config-model-badge current">
                            Current
                          </span>
                        ) : null}
                        {model.configured ? (
                          <span className="run-config-model-badge">
                            Configured
                          </span>
                        ) : null}
                      </span>
                      <span className="run-config-model-description">
                        {model.description}
                      </span>
                    </span>
                    <span className="run-config-model-effort">
                      {model.defaultReasoningEffort}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          <div className="run-config-field">
            <div className="run-config-label-row">
              <span>Reasoning</span>
              <small>随所选模型支持项变化</small>
            </div>
            <div className="run-config-efforts" role="radiogroup">
              {supportedEfforts.map((effort) => (
                <button
                  key={effort}
                  type="button"
                  className={effort === draftReasoningEffort ? "selected" : ""}
                  role="radio"
                  aria-checked={effort === draftReasoningEffort}
                  onClick={() => onSelectReasoningEffort(effort)}
                >
                  {effort}
                </button>
              ))}
            </div>
          </div>

          {fallbackMessage ? (
            <div className="run-config-state">{fallbackMessage}</div>
          ) : null}
        </>
      ) : null}

      {disabled ? (
        <div className="run-config-state">
          当前 turn 正在运行，结束后可应用切换
        </div>
      ) : null}

      <div className="run-config-actions">
        <button type="button" onClick={onCancel}>
          取消
        </button>
        <button
          type="button"
          className="primary"
          disabled={!canApply || !hasChanged}
          onClick={onApply}
        >
          应用
        </button>
      </div>
    </>
  );
}

function getPopoverFocusableElements(popover: HTMLDivElement | null) {
  if (!popover) {
    return [];
  }
  return Array.from(
    popover.querySelectorAll<HTMLButtonElement>(
      'button:not(:disabled), [tabindex]:not([tabindex="-1"])',
    ),
  );
}
