import { toast } from "sonner";
import type { TFunction } from "i18next";
import { commands } from "@/bindings";

/**
 * If the given hotkey string contains the Fn/Globe key and macOS is
 * configured to do something on Globe key press (anything other than
 * "Do Nothing"), show a one-time warning toast with an action button
 * that opens System Settings → Keyboard.
 *
 * Dedupes on the toast id so spamming Fn-binding adds doesn't stack
 * warnings. Safe to call on any platform; non-macOS returns -1 from
 * the backend command and we no-op.
 */
export async function maybeWarnAboutGlobeKey(
  hotkeyString: string,
  t: TFunction,
): Promise<void> {
  const containsFn = hotkeyString
    .split("+")
    .some((k) => k.trim().toLowerCase() === "fn");
  if (!containsFn) return;

  const setting = await commands.getGlobeKeySetting();
  // 0 = Do Nothing (safe); 1/2/3 = Change Input Source / Emoji / Dictation;
  // -1 = unset or non-macOS. We only warn for known non-zero values.
  if (setting <= 0) return;

  toast.warning(t("settings.general.shortcut.globeKeyWarning.title"), {
    id: "globe-key-warning",
    duration: 10000,
    description: t("settings.general.shortcut.globeKeyWarning.description"),
    action: {
      label: t("settings.general.shortcut.globeKeyWarning.action"),
      onClick: () => {
        commands.openKeyboardSettings().catch(console.error);
      },
    },
  });
}
