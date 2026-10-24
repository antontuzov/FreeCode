import { useState } from "react";

import { translate, type Locale } from "../i18n";
import { ipc } from "../lib/ipc";
import { useNav } from "../state/nav";
import { Icon } from "./icons";

/**
 * The in-app browser: the right column's third report, after the inspector and the review. It frames
 * the `preview://` address the host answered for one of the run's pages, so the game or page the run
 * built plays right here beside the conversation instead of taking over the window or being handed to
 * the desktop. The header keeps the two ways out — back to the review that opened it, or on to the
 * default browser — and names the file that is being shown.
 */
export function Preview({ locale }: { locale: Locale }) {
  const preview = useNav((s) => s.preview);
  const close = useNav((s) => s.closePreview);
  // A press of the desktop-open control the OS refused, kept so the sentence lands under the file it
  // is about and a later press that works clears it.
  const [refused, setRefused] = useState<string | null>(null);
  if (!preview) return null;
  const name = preview.path.slice(preview.path.lastIndexOf("/") + 1);
  const openExternal = async () => {
    setRefused(null);
    try {
      await ipc.openFile(preview.path);
    } catch (why) {
      setRefused(String(why));
    }
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-bg text-fg">
      <header className="flex items-center gap-[var(--space-2)] border-b border-line px-[var(--space-3)] py-[var(--space-2)]">
        <button
          type="button"
          onClick={close}
          className="nav-item"
          title={translate(locale, "preview.close")}
        >
          <Icon name="back" size={13} />
          <span>{translate(locale, "back")}</span>
        </button>
        <span className="min-w-0 flex-1 truncate px-[var(--space-2)] text-left text-quiet text-fg">
          {name}
        </span>
        <button
          type="button"
          onClick={() => void openExternal()}
          className="icon-btn shrink-0"
          aria-label={translate(locale, "preview.external")}
          title={translate(locale, "preview.external")}
        >
          <Icon name="globe" size={13} />
        </button>
        <button
          type="button"
          onClick={close}
          className="icon-btn shrink-0"
          aria-label={translate(locale, "preview.close")}
          title={translate(locale, "preview.close")}
        >
          <Icon name="close" size={13} />
        </button>
      </header>
      {refused && (
        <p className="border-b border-line px-[var(--space-3)] py-[var(--space-2)] text-fine text-danger">
          {translate(locale, "preview.externalFailed", { why: refused })}
        </p>
      )}
      <iframe src={preview.url} title={name} className="min-h-0 w-full flex-1 border-0 bg-white" />
    </div>
  );
}
