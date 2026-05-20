import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface NumberWordConversionProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const NumberWordConversion: React.FC<NumberWordConversionProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("convert_number_words") ?? false;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("convert_number_words", value)}
        isUpdating={isUpdating("convert_number_words")}
        label={t("settings.sound.convertNumberWords.label")}
        description={t("settings.sound.convertNumberWords.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
