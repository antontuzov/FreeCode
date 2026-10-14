import { useState } from "react";

import { translate, type Locale } from "../i18n";
import { useChat } from "../state/chat";
import { useTranscript } from "../state/transcript";
import { Icon } from "./icons";

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
  const notInBuild = translate(locale, "common.notInBuild");

  const submit = () => {
    if (!draft.trim()) return;
    void send(draft);
    setDraft("");
  };

  return (
    <div className="shrink-0 px-[var(--space-6)] pb-[var(--space-3)]">
      <div className="mx-auto max-w-[720px] rounded-[var(--radius-lg)] border border-line bg-elevated">
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
          <p
            role="alert"
            className="px-[var(--space-4)] pb-1 text-[calc(var(--font-ui)-2px)] text-danger"
          >
            {error}
          </p>
        )}
        <div className="flex items-center gap-2 px-[var(--space-3)] pb-[var(--space-3)]">
          {/* Nothing this build can add to a prompt yet: attachments wait on the vision path, voice
              on the input it would transcribe into. Named on hover, like the sidebar's unbuilt rows. */}
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
            className="icon-btn"
            aria-label={translate(locale, "composer.attach")}
            aria-disabled
            title={notInBuild}
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
            className="flex items-center gap-1.5 rounded-[var(--radius-pill)] px-2 py-1 text-[calc(var(--font-ui)-2px)] text-muted hover:bg-hover hover:text-fg"
          >
            <Icon name="review" size={13} />
            {translate(locale, `composer.approve.${mode}`)}
            <Icon name="chevronDown" size={12} />
          </button>

          <span className="flex-1" />

          <button
            type="button"
            onClick={onPickModel}
            className="flex items-center gap-1.5 rounded-[var(--radius-pill)] px-2 py-1 text-[calc(var(--font-ui)-2px)] text-muted hover:bg-hover hover:text-fg"
          >
            <Icon name="bolt" size={13} />
            {translate(locale, "composer.model")}
            <Icon name="chevronDown" size={12} />
          </button>
          <button
            type="button"
            className="icon-btn"
            aria-label={translate(locale, "composer.mic", { shortcut: "⌥V" })}
            aria-disabled
            title={notInBuild}
          >
            <Icon name="mic" />
          </button>
          <button
            type="button"
            onClick={() => (running ? void stop() : submit())}
            disabled={!running && !draft.trim()}
            aria-label={translate(locale, running ? "composer.stop" : "composer.send")}
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
