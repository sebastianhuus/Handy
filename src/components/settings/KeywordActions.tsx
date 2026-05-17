import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface KeywordActionsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const KeywordActions: React.FC<KeywordActionsProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("keyword_actions_enabled") ?? false;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("keyword_actions_enabled", value)}
        isUpdating={isUpdating("keyword_actions_enabled")}
        label={t("settings.advanced.keywordActions.label")}
        description={t("settings.advanced.keywordActions.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  }
);
