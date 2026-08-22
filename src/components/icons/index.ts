import type { ComponentType, SVGProps } from "react";
import type { ProviderId } from "../../types";
import { ClaudeIcon } from "./ClaudeIcon";
import { CodexIcon } from "./CodexIcon";
import { CursorIcon } from "./CursorIcon";
import { GrokIcon } from "./GrokIcon";
import { OpenCodeIcon } from "./OpenCodeIcon";

export const providerIcons: Record<ProviderId, ComponentType<SVGProps<SVGSVGElement>>> = {
  claude: ClaudeIcon,
  codex: CodexIcon,
  cursor: CursorIcon,
  grok: GrokIcon,
  opencode: OpenCodeIcon,
};
