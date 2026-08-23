import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { CorrectionPair } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { SettingContainer } from "../ui/SettingContainer";

interface CorrectionPairsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const CorrectionPairs: React.FC<CorrectionPairsProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const [fromWord, setFromWord] = useState("");
    const [toWord, setToWord] = useState("");
    const pairs = getSetting("correction_pairs") || [];

    const sanitize = (s: string) => s.trim().replace(/[<>"'&]/g, "");

    const handleAdd = () => {
      const from = sanitize(fromWord);
      const to = sanitize(toWord);
      if (!from || !to || from.length > 100 || to.length > 100) return;
      if (pairs.some((p) => p.from === from)) {
        toast.error(
          t("settings.advanced.correctionPairs.duplicate", { word: from }),
        );
        return;
      }
      updateSetting("correction_pairs", [...pairs, { from, to }]);
      setFromWord("");
      setToWord("");
    };

    const handleRemove = (pair: CorrectionPair) => {
      updateSetting(
        "correction_pairs",
        pairs.filter((p) => p.from !== pair.from || p.to !== pair.to),
      );
    };

    const handleKeyDown = (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleAdd();
      }
    };

    const isDisabled = isUpdating("correction_pairs");
    const canAdd =
      sanitize(fromWord).length > 0 &&
      sanitize(toWord).length > 0 &&
      sanitize(fromWord).length <= 100 &&
      sanitize(toWord).length <= 100 &&
      !isDisabled;

    return (
      <>
        <SettingContainer
          title={t("settings.advanced.correctionPairs.title")}
          description={t("settings.advanced.correctionPairs.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        >
          <div className="flex items-center gap-2">
            <Input
              type="text"
              className="max-w-36"
              value={fromWord}
              onChange={(e) => setFromWord(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={t(
                "settings.advanced.correctionPairs.placeholderFrom",
              )}
              variant="compact"
              disabled={isDisabled}
            />
            <span className="text-sm text-mid-gray">→</span>
            <Input
              type="text"
              className="max-w-36"
              value={toWord}
              onChange={(e) => setToWord(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={t("settings.advanced.correctionPairs.placeholderTo")}
              variant="compact"
              disabled={isDisabled}
            />
            <Button
              onClick={handleAdd}
              disabled={!canAdd}
              variant="primary"
              size="md"
            >
              {t("settings.advanced.correctionPairs.add")}
            </Button>
          </div>
        </SettingContainer>
        {pairs.length > 0 && (
          <div
            className={`px-4 p-2 ${grouped ? "" : "rounded-lg border border-mid-gray/20"} flex flex-wrap gap-1`}
          >
            {pairs.map((pair) => (
              <Button
                key={`${pair.from}->${pair.to}`}
                onClick={() => handleRemove(pair)}
                disabled={isDisabled}
                variant="secondary"
                size="sm"
                className="inline-flex items-center gap-1 cursor-pointer"
                aria-label={t("settings.advanced.correctionPairs.remove", {
                  from: pair.from,
                  to: pair.to,
                })}
              >
                <span>
                  {pair.from} → {pair.to}
                </span>
                <svg
                  className="w-3 h-3"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M6 18L18 6M6 6l12 12"
                  />
                </svg>
              </Button>
            ))}
          </div>
        )}
      </>
    );
  },
);
