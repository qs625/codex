import type { DraftSkill, ThreadSkill, WorkflowSummary } from "../types";

export type ComposerSlashCommandId =
  | "clear"
  | "goalCreate"
  | "goalPause"
  | "goalResume"
  | "goalCancel";

export type BuiltInSlashCommand = {
  type: "command";
  commandId: ComposerSlashCommandId;
  token: string;
  label: string;
  description: string;
  aliases: string[];
  draftText?: string;
};

export type SkillSlashSuggestion = {
  type: "skill";
  skill: ThreadSkill;
};

export type WorkflowSlashSuggestion = {
  type: "workflow";
  workflow: WorkflowSummary;
  draftText: string;
};

export type ComposerSlashSuggestion =
  | BuiltInSlashCommand
  | SkillSlashSuggestion
  | WorkflowSlashSuggestion;

type ActiveSlashDraft = {
  firstLine: string;
  lowerFirstLine: string;
};

type ComposerSlashSuggestionRequest = {
  availableSkills: ThreadSkill[];
  availableWorkflows: WorkflowSummary[];
  commandsEnabled: boolean;
  normalizedQuery: string;
  selectedSkillPaths: Set<string>;
};

type SlashSuggestionSection = {
  suggestions: ComposerSlashSuggestion[];
};

export const BUILT_IN_SLASH_COMMANDS: BuiltInSlashCommand[] = [
  {
    type: "command",
    commandId: "clear",
    token: "clear",
    label: "/clear",
    description: "Archive this project session and start a fresh project chat",
    aliases: ["reset", "new"],
  },
  {
    type: "command",
    commandId: "goalCreate",
    token: "goal objective",
    label: "/goal <objective>",
    description: "Create or update this thread goal",
    aliases: ["goal", "objective", "create goal", "set goal"],
    draftText: "/goal ",
  },
  {
    type: "command",
    commandId: "goalPause",
    token: "goal pause",
    label: "/goal pause",
    description: "Pause the current thread goal",
    aliases: ["pause goal", "goal"],
    draftText: "/goal pause",
  },
  {
    type: "command",
    commandId: "goalResume",
    token: "goal resume",
    label: "/goal resume",
    description: "Resume the current thread goal",
    aliases: ["resume goal", "goal"],
    draftText: "/goal resume",
  },
  {
    type: "command",
    commandId: "goalCancel",
    token: "goal cancel",
    label: "/goal cancel",
    description: "Cancel the current thread goal",
    aliases: ["cancel-goal", "goal clear", "clear goal", "goal", "cancel"],
    draftText: "/goal cancel",
  },
];

export function getActiveComposerSlashQuery(draft: string) {
  const target = getActiveSlashDraft(draft);
  if (!target) {
    return null;
  }
  const goalSubcommandQuery = getGoalSubcommandSlashQuery(target);
  if (goalSubcommandQuery !== null) {
    return goalSubcommandQuery;
  }
  if (target.firstLine.includes(" ")) {
    return null;
  }
  return target.firstLine.slice(1);
}

function getActiveSlashDraft(draft: string): ActiveSlashDraft | null {
  const firstLine = draft.trimStart().split("\n", 1)[0] ?? "";
  if (!firstLine.startsWith("/")) {
    return null;
  }

  return {
    firstLine,
    lowerFirstLine: firstLine.toLowerCase(),
  };
}

function getGoalSubcommandSlashQuery(target: ActiveSlashDraft) {
  if (!target.lowerFirstLine.startsWith("/goal ")) {
    return null;
  }

  const query = target.lowerFirstLine.slice("/goal ".length);
  if (query.includes(" ")) {
    return null;
  }
  if (!query) {
    return "goal ";
  }
  if (["pause", "resume", "cancel", "clear"].includes(query)) {
    return null;
  }
  if (
    ["pause", "resume", "cancel", "clear"].some((subcommand) =>
      subcommand.startsWith(query),
    )
  ) {
    return `goal ${query}`;
  }
  return null;
}

export function buildComposerSlashSuggestions({
  availableSkills,
  availableWorkflows = [],
  commandsEnabled = true,
  draftSkills,
  query,
}: {
  availableSkills: ThreadSkill[];
  availableWorkflows?: WorkflowSummary[];
  commandsEnabled?: boolean;
  draftSkills: DraftSkill[];
  query: string | null;
}): ComposerSlashSuggestion[] {
  const request = buildComposerSlashSuggestionRequest({
    availableSkills,
    availableWorkflows,
    commandsEnabled,
    draftSkills,
    query,
  });
  if (!request) {
    return [];
  }

  return buildComposerSlashSuggestionSections(request).flatMap(
    (section) => section.suggestions,
  );
}

function buildComposerSlashSuggestionRequest({
  availableSkills,
  availableWorkflows,
  commandsEnabled,
  draftSkills,
  query,
}: {
  availableSkills: ThreadSkill[];
  availableWorkflows: WorkflowSummary[];
  commandsEnabled: boolean;
  draftSkills: DraftSkill[];
  query: string | null;
}): ComposerSlashSuggestionRequest | null {
  if (query === null) {
    return null;
  }

  return {
    availableSkills,
    availableWorkflows,
    commandsEnabled,
    normalizedQuery: query.trim().toLowerCase(),
    selectedSkillPaths: new Set(draftSkills.map((skill) => skill.path)),
  };
}

function buildComposerSlashSuggestionSections(
  request: ComposerSlashSuggestionRequest,
): SlashSuggestionSection[] {
  return [
    buildBuiltInCommandSuggestionSection(request),
    buildWorkflowSuggestionSection(request),
    buildSkillSuggestionSection(request),
  ];
}

function buildBuiltInCommandSuggestionSection({
  commandsEnabled,
  normalizedQuery,
}: ComposerSlashSuggestionRequest): SlashSuggestionSection {
  if (!commandsEnabled) {
    return { suggestions: [] };
  }

  return {
    suggestions: BUILT_IN_SLASH_COMMANDS.filter((command) =>
      matchesSearchableValues(
        [command.token, command.label, command.description, ...command.aliases],
        normalizedQuery,
      ),
    ),
  };
}

function buildWorkflowSuggestionSection({
  availableWorkflows,
  normalizedQuery,
}: ComposerSlashSuggestionRequest): SlashSuggestionSection {
  return {
    suggestions: availableWorkflows
      .filter((workflow) =>
        matchesSearchableValues(
          workflowSearchableValues(workflow),
          normalizedQuery,
        ),
      )
      .map((workflow) => ({
        type: "workflow" as const,
        workflow,
        draftText: buildWorkflowDraftText(workflow),
      })),
  };
}

function buildSkillSuggestionSection({
  availableSkills,
  normalizedQuery,
  selectedSkillPaths,
}: ComposerSlashSuggestionRequest): SlashSuggestionSection {
  return {
    suggestions: availableSkills
      .filter((skill) => !selectedSkillPaths.has(skill.path))
      .filter((skill) =>
        matchesSearchableValues(
          [skill.name, skill.kind, skill.path],
          normalizedQuery,
        ),
      )
      .map((skill) => ({
        type: "skill" as const,
        skill,
      })),
  };
}

function matchesSearchableValues(values: string[], normalizedQuery: string) {
  if (!normalizedQuery) {
    return true;
  }
  return values.join(" ").toLowerCase().includes(normalizedQuery);
}

function workflowSearchableValues(workflow: WorkflowSummary) {
  return [
    `workflow ${workflow.id}`,
    workflow.id,
    workflow.name,
    workflow.description,
    workflow.source,
    workflow.path,
    ...workflow.whenToUse,
    ...Object.keys(workflow.inputs),
  ];
}

function buildWorkflowDraftText(workflow: WorkflowSummary) {
  const inputNames = Object.keys(workflow.inputs);
  if (inputNames.length === 0) {
    return `Use the ${workflow.id} workflow.`;
  }

  const inputList = inputNames.map((name) => `${name}: `).join(", ");
  return `Use the ${workflow.id} workflow with ${inputList}`;
}
