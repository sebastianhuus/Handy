import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { PathDisplay } from "../ui/PathDisplay";
import { ToggleSwitch } from "../ui/ToggleSwitch";

interface TranscriptionLogProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const TranscriptionLog: React.FC<TranscriptionLogProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const [logPath, setLogPath] = useState<string | null>(null);

    const enabled = getSetting("log_transcriptions") ?? false;

    useEffect(() => {
      if (!enabled) return;
      commands.getTranscriptionLogPath().then((result) => {
        if (result.status === "ok") setLogPath(result.data);
      });
    }, [enabled]);

    const openFolder = async () => {
      await commands.openAppDataDir();
    };

    return (
      <div>
        <ToggleSwitch
          checked={enabled}
          onChange={(value) => updateSetting("log_transcriptions", value)}
          isUpdating={isUpdating("log_transcriptions")}
          label={t("settings.debug.transcriptionLog.label")}
          description={t("settings.debug.transcriptionLog.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        />
        {enabled && logPath && (
          <div className="px-4 pb-2">
            <p className="text-xs text-mid-gray mb-1">
              {t("settings.debug.transcriptionLog.filePath")}
            </p>
            <PathDisplay path={logPath} onOpen={openFolder} />
          </div>
        )}
      </div>
    );
  },
);

TranscriptionLog.displayName = "TranscriptionLog";
