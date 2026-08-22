import { Claude, Codex, Cursor, Grok, OpenCode } from "@lobehub/icons";
import type { ComponentType } from "react";
import type { ProviderId } from "../../types";

type BrandIcon = ComponentType<{ size: number }>;

export const providerIcons: Record<ProviderId, BrandIcon> = {
  claude: Claude.Color,
  codex: Codex.Color,
  cursor: Cursor.Avatar,
  grok: Grok.Avatar,
  opencode: OpenCode.Avatar,
};
