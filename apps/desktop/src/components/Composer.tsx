import { useEffect, useRef, useState } from "react";

import { translate, type Locale } from "../i18n";
import { toPictureDataUrl } from "../lib/image";
import type { ApprovalMode } from "../lib/ipc";
import { shortcutLabel } from "../lib/shortcuts";
import { useChat } from "../state/chat";
import { useNav } from "../state/nav";
import { useSettings } from "../state/settings";
import { useTranscript } from "../state/transcript";
import { Icon } from "./icons";
import { ModelPicker } from "./ModelPicker";

/**
 * The three modes a press of the dial cycles through, in order. `custom` is deliberately absent: it is
 * a per-tool table only Settings can fill, so a press leaves that mode rather than landing in it.
 */
const DIAL: ApprovalMode[] = ["auto_approve", "ask_always", "plan_only"];

/** `?coding=1` stands in for the press of the Coding control the native window cannot be given, so the
 * control's on state — and the tool rows it earns — can be looked at in the real app. */
const CODING_SEEDED = new URLSearchParams(window.location.search).get("coding") === "1";

/**
 * The composer's approval selector is the agent loop's permission dial, and it edits the one value the
 * loop reads: `approvals.mode` in the settings the host loads when a prompt is sent, which is the same
 * document the Settings page writes. A call the mode refuses comes back as a blocked row naming the
 * policy, so the label has to be exact — and this build has no prompt for a call to wait on, which
 * means the mode that would ask refuses instead. Hover says so per mode, because "Ask every time" that
 * refused everything would be a worse surprise than naming it.
 */
export function Composer({ locale, onPickModel }: { locale: Locale; onPickModel: () => void }) {
  const running = useTranscript((s) => s.running);
  const runDiff = useTranscript((s) => s.runDiff);
  const runRounds = useTranscript((s) => s.runRounds);
  const send = useChat((s) => s.send);
  const stop = useChat((s) => s.stop);
  const error = useChat((s) => s.error);
  // The diff pill and the right column's review are one control at either end of the same press, so
  // the pill reads the column's state rather than holding its own copy of it.
  const review = useNav((s) => s.panel) === "review";
  const toggleReview = useNav((s) => s.toggleReview);
  const [draft, setDraft] = useState("");
  const mode = useSettings((s) => s.doc?.approvals.mode);
  // The shell's switch changes what the Coding tooltip may promise: with it on, a run can also issue
  // one command, and that is the one call the tooltip's "nothing else reaches past this machine" would
  // otherwise be wrong about.
  const shell = useSettings((s) => s.doc?.approvals.shellEnabled ?? false);
  // A delegation is a second model run, so the tooltip has to name it whenever the switch that pays
  // for it is on — the base sentences price a run in tool results and would leave it out.
  const subagents = useSettings((s) => s.doc?.approvals.subagentsEnabled ?? false);
  // The Coding control: whether the next prompt offers the workspace reads. It is per prompt rather
  // than per conversation, because each round costs the context window a tool result, and that is a
  // price the user agrees to one question at a time rather than once per chat.
  const [coding, setCoding] = useState(CODING_SEEDED);
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

  // The dial names a value the host owns, so it is read at launch rather than after the first press:
  // a pill that held nothing until the Settings page was opened would be a control with no state.
  useEffect(() => {
    const store = useSettings.getState();
    if (!store.loaded) void store.load();
  }, []);

  // A press writes the same document the Settings page edits, and the host reads it when the next
  // prompt is sent — so the run already on screen is unaffected, and the one after it is not.
  const cycleApproval = async () => {
    if (!useSettings.getState().loaded) await useSettings.getState().load();
    const current = useSettings.getState().doc?.approvals.mode ?? "auto_approve";
    // `custom` is not in the cycle, so its index is -1 and the next press lands on the first mode.
    const next = DIAL[(DIAL.indexOf(current) + 1) % DIAL.length] ?? "auto_approve";
    await useSettings.getState().update((doc) => void (doc.approvals.mode = next));
  };

  const submit = () => {
    if (!draft.trim() && !picture) return;
    // A picture with no words is a question — "what is in this?" needs no saying.
    void send(draft, picture ?? undefined, coding);
    setDraft("");
    setPicture(null);
  };

  return (
    <div className="shrink-0 px-[var(--space-6)] pb-[var(--space-3)]">
      {(runRounds || runDiff) && (
        // What the run has spent and what it cost, on the line above the box it spent them in. The
        // step pill counts the loop's rounds against the budget the host announced, so a long agent
        // run says where it is as well as where it is going; the diff pill counts the writes.
        <div className="mx-auto mb-[var(--space-2)] flex w-full max-w-[var(--content-width)] flex-wrap items-center gap-[var(--space-2)]">
          {runRounds && (
            <span className="inline-flex items-center gap-[var(--space-2)] rounded-[var(--radius-pill)] border border-line bg-elevated px-[var(--space-3)] py-1 text-fine text-muted">
              <Icon
                name="step"
                size={13}
                className={running ? "animate-spin" : undefined}
                aria-hidden
              />
              {translate(locale, "composer.step", {
                done: runRounds.done,
                max: runRounds.max,
              })}
            </span>
          )}
          {runDiff && (
            // What the run changed, where the composer already is. The count is the host's ledger of
            // distinct files, not the number of edit rows, and it moves while the run goes rather than
            // arriving once it ends — an agent that has changed six files is a different thing to
            // watch than one that has changed none.
            //
            // It is also the control that opens the change. Every write row carries the lines it wrote
            // from the build that stages them, so a pill that counted a diff and could not show one
            // would be a number the user has to go and find in the workspace themselves.
            <button
              type="button"
              onClick={toggleReview}
              aria-expanded={review}
              title={translate(locale, review ? "composer.review.close" : "composer.review.open")}
              className={`inline-flex items-center gap-[var(--space-2)] rounded-[var(--radius-pill)] border px-[var(--space-3)] py-1 text-fine transition-colors ${
                review
                  ? "border-accent-line bg-accent-soft text-fg"
                  : "border-line bg-elevated text-muted hover:bg-hover hover:text-fg"
              }`}
            >
              <Icon name="review" size={13} />
              {translate(
                locale,
                runDiff.files === 1
                  ? "transcript.diff.fileChanged"
                  : "transcript.diff.filesChanged",
                { count: runDiff.files },
              )}
              <span className="font-mono tabular-nums">
                <span className="text-success">+{runDiff.added}</span>{" "}
                <span className="text-danger">−{runDiff.removed}</span>
              </span>
              {/* The one glyph that says which way this press goes: down opens, and the same control
                  with the chevron turned is the one that closes. */}
              <Icon
                name="chevronDown"
                size={12}
                className={`disclosure-icon shrink-0 ${review ? "rotate-180" : ""}`}
                aria-hidden
              />
            </button>
          )}
        </div>
      )}
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
          {/* The Coding control, as the reference draws it: one line icon in the row, not a second
              labelled pill. It reads as pressed because the state is the whole point of the click —
              the title is what says what each side of it costs. */}
          <button
            type="button"
            onClick={() => setCoding((on) => !on)}
            className={`icon-btn ${coding ? "bg-accent-soft text-fg-strong" : ""}`}
            aria-label={translate(locale, "composer.coding")}
            aria-pressed={coding}
            title={[
              translate(
                locale,
                coding
                  ? shell
                    ? "composer.coding.onShell"
                    : "composer.coding.on"
                  : shell
                    ? "composer.coding.offShell"
                    : "composer.coding.off",
              ),
              ...(coding && subagents ? [translate(locale, "composer.coding.subagents")] : []),
            ].join(" ")}
          >
            <Icon name="coding" />
          </button>
          <button
            type="button"
            onClick={() => void cycleApproval()}
            // Until the host's value arrives the pill names the control rather than guessing at what it
            // holds, the way the model pill carries its own title before the engine's answer does.
            title={translate(
              locale,
              mode ? `composer.approve.${mode}` : "settings.row.approvalMode",
            )}
            className="flex items-center gap-1.5 rounded-[var(--radius-pill)] px-2 py-1 text-fine text-muted hover:bg-hover hover:text-fg"
          >
            <Icon name="review" size={13} />
            {translate(locale, mode ? `settings.opt.${mode}` : "settings.section.approvals")}
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
