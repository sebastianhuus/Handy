import React, { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Cog, FlaskConical, History, Info, Sparkles, Cpu } from "lucide-react";
import { type } from "@tauri-apps/plugin-os";
import HandyTextLogo from "./icons/HandyTextLogo";
import HandyHand from "./icons/HandyHand";
import { useSettings } from "../hooks/useSettings";
import { Tooltip } from "./ui/Tooltip";
import {
  GeneralSettings,
  AdvancedSettings,
  HistorySettings,
  DebugSettings,
  AboutSettings,
  PostProcessingSettings,
  ModelsSettings,
} from "./settings";

const IS_MACOS = type() === "macos";

const PANEL_SHORTCUT_INDEX: Partial<Record<string, number>> = {
  general: 1,
  models: 2,
  advanced: 3,
  history: 4,
  about: 5,
};

export type SidebarSection = keyof typeof SECTIONS_CONFIG;

interface IconProps {
  width?: number | string;
  height?: number | string;
  size?: number | string;
  className?: string;
  [key: string]: any;
}

interface SectionConfig {
  labelKey: string;
  icon: React.ComponentType<IconProps>;
  component: React.ComponentType;
  enabled: (settings: any) => boolean;
}

export const SECTIONS_CONFIG = {
  general: {
    labelKey: "sidebar.general",
    icon: HandyHand,
    component: GeneralSettings,
    enabled: () => true,
  },
  models: {
    labelKey: "sidebar.models",
    icon: Cpu,
    component: ModelsSettings,
    enabled: () => true,
  },
  advanced: {
    labelKey: "sidebar.advanced",
    icon: Cog,
    component: AdvancedSettings,
    enabled: () => true,
  },
  history: {
    labelKey: "sidebar.history",
    icon: History,
    component: HistorySettings,
    enabled: () => true,
  },
  postprocessing: {
    labelKey: "sidebar.postProcessing",
    icon: Sparkles,
    component: PostProcessingSettings,
    enabled: (settings) => settings?.post_process_enabled ?? false,
  },
  debug: {
    labelKey: "sidebar.debug",
    icon: FlaskConical,
    component: DebugSettings,
    enabled: (settings) => settings?.debug_mode ?? false,
  },
  about: {
    labelKey: "sidebar.about",
    icon: Info,
    component: AboutSettings,
    enabled: () => true,
  },
} as const satisfies Record<string, SectionConfig>;

interface SidebarItemProps {
  section: { id: SidebarSection; labelKey: string; icon: React.ComponentType<IconProps> };
  isActive: boolean;
  onSectionChange: (section: SidebarSection) => void;
}

const SidebarItem: React.FC<SidebarItemProps> = ({
  section,
  isActive,
  onSectionChange,
}) => {
  const { t } = useTranslation();
  const [showTooltip, setShowTooltip] = useState(false);
  const itemRef = useRef<HTMLDivElement>(null);
  const Icon = section.icon;
  const shortcutIndex = PANEL_SHORTCUT_INDEX[section.id];
  const shortcutLabel = shortcutIndex
    ? `${IS_MACOS ? "⌘" : "Ctrl+"}${shortcutIndex}`
    : null;

  return (
    <>
      <div
        ref={itemRef}
        className={`flex gap-2 items-center p-2 w-full rounded-lg cursor-pointer transition-colors ${
          isActive
            ? "bg-logo-primary/80"
            : "hover:bg-mid-gray/20 hover:opacity-100 opacity-85"
        }`}
        onClick={() => onSectionChange(section.id)}
        onMouseEnter={() => setShowTooltip(true)}
        onMouseLeave={() => setShowTooltip(false)}
      >
        <Icon width={24} height={24} className="shrink-0" />
        <p className="text-sm font-medium truncate" title={t(section.labelKey)}>
          {t(section.labelKey)}
        </p>
      </div>
      {showTooltip && shortcutLabel && (
        <Tooltip targetRef={itemRef} position="bottom">
          <p className="text-xs text-center text-mid-gray">{shortcutLabel}</p>
        </Tooltip>
      )}
    </>
  );
};

interface SidebarProps {
  activeSection: SidebarSection;
  onSectionChange: (section: SidebarSection) => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeSection,
  onSectionChange,
}) => {
  const { settings } = useSettings();

  const availableSections = Object.entries(SECTIONS_CONFIG)
    .filter(([_, config]) => config.enabled(settings))
    .map(([id, config]) => ({ id: id as SidebarSection, ...config }));

  return (
    <div className="flex flex-col w-40 h-full border-e border-mid-gray/20 items-center px-2">
      <HandyTextLogo width={120} className="m-4" />
      <div className="flex flex-col w-full items-center gap-1 pt-2 border-t border-mid-gray/20">
        {availableSections.map((section) => (
          <SidebarItem
            key={section.id}
            section={section}
            isActive={activeSection === section.id}
            onSectionChange={onSectionChange}
          />
        ))}
      </div>
    </div>
  );
};
