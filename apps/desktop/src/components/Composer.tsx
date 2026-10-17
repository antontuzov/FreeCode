import { useRef, useState } from "react";

import { translate, type Locale } from "../i18n";
import { toPictureDataUrl } from "../lib/image";
import { shortcutLabel } from "../lib/shortcuts";
import { useChat } from "../state/chat";
import { useTranscript } from "../state/transcript";
import { Icon } from "./icons";
import { ModelPicker } from "./ModelPicker";

const APPROVAL_MODES = ["auto", "ask", "plan"] as const;
type ApprovalMode = (typeof APPROVAL_MODES)[number];

/**
 * The composer's approval selector is the agent loop's permission dial, so its default is the
 * conservative one and every mode is named out loud rather than shown as an icon the user has to
 * decode. Wiring it to the loop is Phase 4; until then it holds state and nothing more.
 */
export function Composer({ locale, onPickModel }: { locale: Locale; onPickModel: () => void }) {
  const running = useTranscript((s) => s.running);
  const send = useChat((s) => s.send);
  const stop = useChat((s) => s.stop);
  const error = useChat((s) => s.error);
  const [draft, setDraft] = useState("");
  const [mode, setMode] = useState<ApprovalMode>("ask");
  const [picture, setPicture] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [attachError, setAttachError] = useState<string | null>(null);
  const picker = useRef<HTMLInputElement>(null);
  const notInBuild = translate(locale, "common.notInBuild");

  // The picture is shrunk and re-encoded here, before it ever crosses to the host, because a phone
  // photo's bytes are far past what a vision projector samples — and the host charges the attached
  // picture for the engine's whole image-token budget either way.
  const pickPicture = async (picked: File | undefined) => {
    if (!picked) return;
    setPreparing(true);
    setAttachError(null);
    try {
      setPicture(await toPictureDataUrl(picked));
    } catch {
      // A file the browser cannot decode as an image is the one way this fails without the engine
      // being involved, so it is said here rather than left as a picture button that did nothing.
      setPicture(null);
      setAttachError(translate(locale, "composer.picture.failed"));
    } finally {
      setPreparing(false);
      // Cleared so choosing the same file twice fires `change` again, which the browser otherwise
      // suppresses and the user reads as the button having stopped working.
      if (picker.current) picker.current.value = "";
    }
  };

  const submit = () => {
    if (!draft.trim() && !picture) return;
    // A picture with no words is a question — "what is in this?" needs no saying.
    void send(draft, picture ?? undefined);
    setDraft("");
    setPicture(null);
  };

  return (
    <div className="shrink-0 px-[var(--space-6)] pb-[var(--space-3)]">
      <div className="mx-auto w-full max-w-[var(--content-width)] rounded-[var(--radius-lg)] border border-line bg-elevated shadow-raised transition-colors focus-within:border-accent-line">
        <label className="sr-only" htmlFor="composer-input">
          {translate(locale, "composer.placeholder")}
        </label>
        <textarea
          id="composer-input"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            // Enter sends, Shift+Enter adds the newline a multi-line task needs.
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              submit();
            }
          }}
          rows={2}
          placeholder={translate(locale, "composer.placeholder")}
          className="w-full resize-none bg-transparent px-[var(--space-4)] pt-[var(--space-3)] text-fg outline-none placeholder:text-faint"
        />
        {error && (
          <p role="alert" className="px-[var(--space-4)] pb-1 text-fine text-danger">
            {error}
          </p>
        )}
        {attachError && (
          <p role="alert" className="px-[var(--space-4)] pb-1 text-fine text-danger">
            {attachError}
          </p>
        )}
        {picture && (
          <div className="flex items-center gap-2 px-[var(--space-4)] pb-2">
            {/* The shrunk bytes, not the file the user picked: what is on screen is exactly what the
                model will be given, so a picture that resized badly is visible before it is sent. */}
            <img
              src={picture}
              alt=""
              className="size-12 rounded-[var(--radius-md)] border border-line object-cover"
            />
            <span className="text-fine text-muted">{translate(locale, "composer.picture")}</span>
            <button
              type="button"
              onClick={() => setPicture(null)}
              className="icon-btn"
              aria-label={translate(locale, "composer.picture.remove")}
            >
              <Icon name="close" size={12} />
            </button>
          </div>
        )}
        <div className="flex items-center gap-2 px-[var(--space-3)] pb-[var(--space-3)]">
          {/* A plain file input, because the webview answers one with the native open panel — no
              dialog plugin and no filesystem capability handed to the interface. Only a picture is
              asked for: the engine reads images, and nothing else has a path to an answer. */}
          <input
            ref={picker}
            type="file"
            accept="image/*"
            className="sr-only"
            id="composer-picture"
            onChange={(e) => void pickPicture(e.target.files?.[0])}
          />
          {/* Still nothing to add a prompt but a picture: context attachments wait on the retrieval
              path, voice on the input it would transcribe into. Named on hover, like the sidebar's
              unbuilt rows. */}
          <button
            type="button"
            className="icon-btn"
            aria-label={translate(locale, "composer.add")}
            aria-disabled
            title={notInBuild}
          >
            <Icon name="plus" />
          </button>
          <button
            type="button"
            onClick={() => picker.current?.click()}
            className="icon-btn"
            aria-label={translate(locale, "composer.attach")}
            aria-busy={preparing || undefined}
            title={
              preparing
                ? translate(locale, "composer.picture.busy")
                : translate(locale, "composer.attach")
            }
          >
            <Icon name="attach" />
          </button>
          <button
            type="button"
            onClick={() =>
              setMode(
                (m) =>
                  APPROVAL_MODES[(APPROVAL_MODES.indexOf(m) + 1) % APPROVAL_MODES.length] ?? "ask",
              )
            }
            // The dial works and the label changes; what it dials into does not answer differently
            // yet, and a control that quietly rewrites its own name would claim otherwise.
            title={translate(locale, "composer.approve.inert")}
            className="flex items-center gap-1.5 rounded-[var(--radius-pill)] px-2 py-1 text-fine text-muted hover:bg-hover hover:text-fg"
          >
            <Icon name="review" size={13} />
            {translate(locale, `composer.approve.${mode}`)}
            <Icon name="chevronDown" size={12} />
          </button>

          <span className="flex-1" />

          <ModelPicker locale={locale} onOpenSettings={onPickModel} />
          <button
            type="button"
            className="icon-btn"
            aria-label={translate(locale, "composer.mic")}
            aria-disabled
            title={notInBuild}
          >
            <Icon name="mic" />
          </button>
          <button
            type="button"
            onClick={() => (running ? void stop() : submit())}
            disabled={!running && !draft.trim() && !picture}
            aria-label={translate(locale, running ? "composer.stop" : "composer.send")}
            // The key the interrupt answers to, named only while there is a reply to interrupt.
            title={running ? shortcutLabel("escape") : undefined}
            className="grid size-8 place-items-center rounded-[var(--radius-md)] bg-accent text-accent-contrast disabled:opacity-60"
          >
            {/* Interrupting a reply cuts its stream and keeps the server, so the next prompt is
                answered without reloading the weights. */}
            <Icon name={running ? "stop" : "send"} size={15} />
          </button>
        </div>
      </div>
    </div>
  );
}
