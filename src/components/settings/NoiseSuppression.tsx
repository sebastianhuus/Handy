import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface NoiseSuppressionToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const NoiseSuppression: React.FC<NoiseSuppressionToggleProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("noise_suppression") ?? false;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("noise_suppression", value)}
        isUpdating={isUpdating("noise_suppression")}
        label={t("settings.sound.noiseSuppression.label")}
        description={t("settings.sound.noiseSuppression.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
