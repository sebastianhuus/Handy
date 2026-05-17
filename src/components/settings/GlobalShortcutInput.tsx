import React, { useEffect, useState, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  getKeyName,
  formatKeyCombination,
  normalizeKey,
} from "../../lib/utils/keyboard";
import { ResetButton } from "../ui/ResetButton";
import TrashIcon from "../icons/TrashIcon";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";
import { commands } from "@/bindings";
import { toast } from "sonner";

interface GlobalShortcutInputProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  shortcutId: string;
  disabled?: boolean;
}

export const GlobalShortcutInput: React.FC<GlobalShortcutInputProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
  shortcutId,
  disabled = false,
}) => {
  const { t } = useTranslation();
  const {
    getSetting,
    addBinding,
    removeBinding,
    resetBinding,
    clearBinding,
    isUpdating,
    isLoading,
  } = useSettings();
  const [keyPressed, setKeyPressed] = useState<string[]>([]);
  const [recordedKeys, setRecordedKeys] = useState<string[]>([]);
  const [isRecording, setIsRecording] = useState(false);
  const recordingRef = useRef<HTMLDivElement | null>(null);
  const suspendedIdsRef = useRef<string[]>([]);
  const osType = useOsType();

  const bindings = getSetting("bindings") || {};

  // Keyboard listeners — only active while recording a new hotkey
  useEffect(() => {
    if (!isRecording) return;
    let cleanup = false;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (cleanup || e.repeat) return;
      e.preventDefault();
      const key = normalizeKey(getKeyName(e, osType));
      if (!keyPressed.includes(key)) {
        setKeyPressed((prev) => [...prev, key]);
        if (!recordedKeys.includes(key)) {
          setRecordedKeys((prev) => [...prev, key]);
        }
      }
    };

    const handleKeyUp = async (e: KeyboardEvent) => {
      if (cleanup) return;
      e.preventDefault();
      const key = normalizeKey(getKeyName(e, osType));
      setKeyPressed((prev) => prev.filter((k) => k !== key));

      const updatedKeyPressed = keyPressed.filter((k) => k !== key);
      if (updatedKeyPressed.length === 0 && recordedKeys.length > 0) {
        const modifiers = [
          "ctrl",
          "control",
          "shift",
          "alt",
          "option",
          "meta",
          "command",
          "cmd",
          "super",
          "win",
          "windows",
        ];
        const sortedKeys = [...recordedKeys].sort((a, b) => {
          const aMod = modifiers.includes(a.toLowerCase());
          const bMod = modifiers.includes(b.toLowerCase());
          if (aMod && !bMod) return -1;
          if (!aMod && bMod) return 1;
          return 0;
        });
        const newShortcut = sortedKeys.join("+");

        const conflict = Object.entries(bindings).find(
          ([otherId, b]) =>
            otherId !== shortcutId && b?.current_bindings.includes(newShortcut),
        );
        if (conflict) {
          toast.error(
            t("settings.general.shortcut.errors.duplicate", {
              shortcut: formatKeyCombination(newShortcut, osType),
              name: t(
                `settings.general.shortcut.bindings.${conflict[0]}.name`,
                conflict[1]?.name ?? conflict[0],
              ),
            }),
          );
        } else {
          try {
            await addBinding(shortcutId, newShortcut);
          } catch (error) {
            console.error("Failed to add binding:", error);
            toast.error(
              t("settings.general.shortcut.errors.set", {
                error: String(error),
              }),
            );
          }
        }
        suspendedIdsRef.current.forEach((id) =>
          commands.resumeBinding(id).catch(console.error),
        );
        suspendedIdsRef.current = [];
        setIsRecording(false);
        setKeyPressed([]);
        setRecordedKeys([]);
      }
    };

    const handleClickOutside = (e: MouseEvent) => {
      if (cleanup) return;
      if (
        recordingRef.current &&
        !recordingRef.current.contains(e.target as Node)
      ) {
        suspendedIdsRef.current.forEach((id) =>
          commands.resumeBinding(id).catch(console.error),
        );
        suspendedIdsRef.current = [];
        setIsRecording(false);
        setKeyPressed([]);
        setRecordedKeys([]);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("keyup", handleKeyUp);
    window.addEventListener("click", handleClickOutside);
    return () => {
      cleanup = true;
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("keyup", handleKeyUp);
      window.removeEventListener("click", handleClickOutside);
    };
  }, [
    isRecording,
    keyPressed,
    recordedKeys,
    shortcutId,
    osType,
    addBinding,
    t,
  ]);

  const startRecording = async () => {
    if (isRecording) return;
    // Suspend all bindings so nothing fires while recording a new hotkey
    const ids = Object.keys(bindings);
    suspendedIdsRef.current = ids;
    await Promise.all(ids.map((id) => commands.suspendBinding(id).catch(console.error)));
    setIsRecording(true);
    setKeyPressed([]);
    setRecordedKeys([]);
  };

  const formatCurrentKeys = (): string => {
    if (recordedKeys.length === 0)
      return t("settings.general.shortcut.pressKeys");
    return formatKeyCombination(recordedKeys.join("+"), osType);
  };

  if (isLoading) {
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.loading")}
        </div>
      </SettingContainer>
    );
  }

  if (Object.keys(bindings).length === 0) {
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.none")}
        </div>
      </SettingContainer>
    );
  }

  const binding = bindings[shortcutId];
  if (!binding) {
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.notFound")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.none")}
        </div>
      </SettingContainer>
    );
  }

  const translatedName = t(
    `settings.general.shortcut.bindings.${shortcutId}.name`,
    binding.name,
  );
  const translatedDescription = t(
    `settings.general.shortcut.bindings.${shortcutId}.description`,
    binding.description,
  );

  const hotkeys = binding.current_bindings ?? [];
  const updating = isUpdating(`binding_${shortcutId}`);

  return (
    <SettingContainer
      title={translatedName}
      description={translatedDescription}
      descriptionMode={descriptionMode}
      grouped={grouped}
      disabled={disabled}
      layout="horizontal"
    >
      <div className="flex flex-wrap items-center gap-1.5 justify-end">
        {hotkeys.map((hk) => (
          <div
            key={hk}
            className="group inline-flex items-center gap-1.5 pl-2.5 pr-1.5 py-1 text-sm font-semibold bg-mid-gray/10 border border-mid-gray/80 rounded-md hover:border-mid-gray/60 transition-colors"
          >
            <span>{formatKeyCombination(hk, osType)}</span>
            <button
              onClick={async () => {
                try {
                  await removeBinding(shortcutId, hk);
                } catch (error) {
                  console.error("Failed to remove binding:", error);
                  toast.error(
                    t("settings.general.shortcut.errors.set", {
                      error: String(error),
                    }),
                  );
                }
              }}
              disabled={updating}
              className="opacity-0 group-hover:opacity-100 transition-opacity text-mid-gray hover:text-logo-primary disabled:cursor-not-allowed rounded p-0.5"
              title={t("settings.general.shortcut.remove", "Remove")}
            >
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                <path
                  d="M1.5 1.5l7 7M8.5 1.5l-7 7"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  strokeLinecap="round"
                />
              </svg>
            </button>
          </div>
        ))}
        {isRecording ? (
          <div
            ref={recordingRef}
            className="inline-flex items-center px-2.5 py-1 text-sm font-semibold border border-logo-primary bg-logo-primary/20 rounded-md"
          >
            {formatCurrentKeys()}
          </div>
        ) : (
          <button
            className={`px-2.5 py-1 text-sm rounded-md transition-colors disabled:opacity-50 ${
              hotkeys.length === 0
                ? "font-semibold bg-mid-gray/10 border border-mid-gray/80 hover:bg-logo-primary/10 hover:border-logo-primary"
                : "font-medium border border-dashed border-mid-gray/50 text-mid-gray hover:border-logo-primary hover:text-logo-primary hover:bg-logo-primary/5"
            }`}
            onClick={startRecording}
            disabled={updating}
          >
            {hotkeys.length === 0
              ? t("settings.general.shortcut.notSet")
              : t("settings.general.shortcut.add", "+ Add")}
          </button>
        )}
        {binding.default_binding ? (
          <ResetButton
            onClick={() => resetBinding(shortcutId)}
            disabled={updating}
          />
        ) : null}
        {hotkeys.length > 0 && (
          <ResetButton
            onClick={() => clearBinding(shortcutId)}
            disabled={updating}
          >
            <TrashIcon width={16} height={16} />
          </ResetButton>
        )}
      </div>
    </SettingContainer>
  );
};
