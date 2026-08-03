import type { Account, GetAccountResponse, LoginAccountResponse } from "../types";

export type OpenAiAuthState = {
  status: "authenticated" | "required" | "notRequired" | "unknown";
  label: string;
  detail: string;
};

export type PendingOpenAiLogin = {
  loginId: string;
  mode: "chatgpt" | "device";
  authUrl?: string;
  verificationUrl?: string;
  userCode?: string;
};

export function resolveOpenAiAuthState(
  accountResponse: GetAccountResponse | null,
): OpenAiAuthState {
  if (!accountResponse) {
    return {
      status: "unknown",
      label: "Unknown",
      detail: "Account status has not loaded.",
    };
  }

  const account = accountResponse.account;
  if (account?.type === "apiKey" || account?.type === "chatgpt") {
    return {
      status: "authenticated",
      label: accountLabel(account),
      detail: accountDetail(account),
    };
  }

  if (accountResponse.requiresOpenaiAuth) {
    return {
      status: "required",
      label: "Authentication required",
      detail: account
        ? "Current account is not an OpenAI credential."
        : "OpenAI credentials are needed for the current provider.",
    };
  }

  return {
    status: "notRequired",
    label: "Not required",
    detail: "Current provider can run without OpenAI credentials.",
  };
}

export function pendingLoginFromResponse(
  response: LoginAccountResponse,
): PendingOpenAiLogin | null {
  if (response.type === "chatgpt") {
    return {
      loginId: response.loginId,
      mode: "chatgpt",
      authUrl: response.authUrl,
    };
  }
  if (response.type === "chatgptDeviceCode") {
    return {
      loginId: response.loginId,
      mode: "device",
      verificationUrl: response.verificationUrl,
      userCode: response.userCode,
    };
  }
  return null;
}

function accountLabel(account: Account) {
  switch (account.type) {
    case "apiKey":
      return "API key connected";
    case "chatgpt":
      return "ChatGPT connected";
    default:
      return "Connected";
  }
}

function accountDetail(account: Account) {
  if (account.type === "chatgpt") {
    return `${account.email} · ${account.planType}`;
  }
  if (account.type === "apiKey") {
    return "Stored by the app server.";
  }
  return "Stored by the app server.";
}
