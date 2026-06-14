import type { DraftSkill, ThreadSkill } from "../types";

export type ComposerSlashCommandId = "clear";

export type BuiltInSlashCommand = {
  type: "command";
  commandId: ComposerSlashCommandId;
  token: string;
  label: string;
  description: string;
  aliases: string[];
};

export type SkillSlashSuggestion = {
  type: "skill";
  skill: ThreadSkill;
};

export type ComposerSlashSuggestion = BuiltInSlashCommand | SkillSlashSuggestion;

export const BUILT_IN_SLASH_COMMANDS: BuiltInSlashCommand[] = [
  {
    type: "command",
    commandId: "clear",
    token: "clear",
    label: "/clear",
    description: "Archive this root session and start a fresh root",
    aliases: ["reset", "new"],
  },
];

export function getActiveComposerSlashQuery(draft: string) {
  const firstLine = draft.trimStart().split("\n", 1)[0] ?? "";
  if (!firstLine.startsWith("/") || firstLine.includes(" ")) {
    return null;
  }
  return firstLine.slice(1);
}

export function buildComposerSlashSuggestions({
  availableSkills,
  commandsEnabled = true,
  draftSkills,
  query,
}: {
  availableSkills: ThreadSkill[];
  commandsEnabled?: boolean;
  draftSkills: DraftSkill[];
  query: string | null;
}): ComposerSlashSuggestion[] {
  if (query === null) {
    return [];
  }

  return [
    ...(commandsEnabled ? filterBuiltInSlashCommands(query) : []),
    ...filterSkillSlashSuggestions(availableSkills, draftSkills, query).map(
      (skill) => ({
        type: "skill" as const,
        skill,
      }),
    ),
  ];
}

function filterBuiltInSlashCommands(query: string) {
  const normalizedQuery = query.trim().toLowerCase();
  return BUILT_IN_SLASH_COMMANDS.filter((command) => {
    if (!normalizedQuery) {
      return true;
    }

    const searchable = [
      command.token,
      command.label,
      command.description,
      ...command.aliases,
    ]
      .join(" ")
      .toLowerCase();
    return searchable.includes(normalizedQuery);
  });
}

function filterSkillSlashSuggestions(
  availableSkills: ThreadSkill[],
  draftSkills: DraftSkill[],
  query: string,
) {
  const normalizedQuery = query.trim().toLowerCase();
  const selectedPaths = new Set(draftSkills.map((skill) => skill.path));

  return availableSkills.filter((skill) => {
    if (selectedPaths.has(skill.path)) {
      return false;
    }

    if (!normalizedQuery) {
      return true;
    }

    const searchable = [skill.name, skill.kind, skill.path]
      .join(" ")
      .toLowerCase();
    return searchable.includes(normalizedQuery);
  });
}
