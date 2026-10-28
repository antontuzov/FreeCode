import { useEffect, useRef, useState } from "react";

import { translate, type Locale } from "../i18n";
import { ipc } from "../lib/ipc";
import { attachPage } from "../lib/page";
import { previewPage, previewPathOf, previewTab, useNav } from "../state/nav";
import { Icon } from "./icons";

/** The name a tab wears: the page's own file name, which is the part of the path that tells two
 * pages apart without spending the strip on a directory both of them share. */
function nameOf(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/**
 * The in-app browser: the right column's third report, after the inspector and the review.
 *
 * It frames the `preview://` addresses the host serves from the project folder, so the game or page
 * a run built plays beside the conversation instead of taking over the window or being handed to the
 * desktop. The chrome is the reference's: a strip of tabs over a toolbar with the few things this
 * browser can honestly do — a new tab, the tab's own history, a reload, the address the tab is
 * standing on, and the way out to the default browser. A tab with no page is the reference's empty
 * state in this build's words: what the panel reads, and Start browser for the project's own index
 * page. The reference also draws download, edit-page and send glyphs; this build has no answer for
 * any of them yet, and a control that pressed and did nothing would be the lie the rest of this
 * window is written to avoid.
 */
export function Preview({ locale }: { locale: Locale }) {
  const tabs = useNav((s) => s.previewTabs);
  const active = useNav((s) => s.previewActive);
  const selectTab = useNav((s) => s.selectPreviewTab);
  const addTab = useNav((s) => s.newPreviewTab);
  const closeTab = useNav((s) => s.closePreviewTab);
  const navigate = useNav((s) => s.navigatePreview);
  const back = useNav((s) => s.previewBack);
  const forward = useNav((s) => s.previewForward);
  const reload = useNav((s) => s.reloadPreview);

  const tab = previewTab({ previewTabs: tabs, previewActive: active });
  const page = previewPage(tab);
  const [address, setAddress] = useState(page?.path ?? "");
  // A press the host refused — a path outside the folder, a file that is gone — kept under the
  // toolbar until the next address that works or the next page the panel lands on.
  const [refused, setRefused] = useState<string | null>(null);
  const field = useRef<HTMLInputElement>(null);

  const at = page?.path ?? "";
  const url = page?.url ?? "";
  // The frame the agent's page calls act on. It is registered from an effect rather than through a
  // ref callback so a new document — a tab switch, a reload, a page the run opened — replaces it in
  // one place, and an empty tab or a panel closing leaves the bridge nothing to post to.
  const frame = useRef<HTMLIFrameElement>(null);
  useEffect(() => {
    attachPage(frame.current, url);
    return () => attachPage(null, "");
  }, [url, tab?.reloads]);
  // The field says where the frame actually is. Back, a tab switch or a page the review opened are
  // the panel moving, not the user typing, and an address left behind would name a page the column
  // is not showing.
  useEffect(() => {
    setAddress(at);
    setRefused(null);
  }, [url, at]);

  const open = async (raw: string) => {
    // Enter in an empty field asks for nothing, so it is not answered with the refusal a web address
    // earns: there is no address there to refuse.
    if (!raw.trim()) return;
    const path = previewPathOf(raw);
    if (path === null) {
      setRefused(translate(locale, "preview.outside"));
      return;
    }
    try {
      const resolved = await ipc.previewUrl(path);
      setRefused(null);
      navigate(resolved, path);
    } catch (why) {
      setRefused(translate(locale, "preview.addressFailed", { why: String(why) }));
    }
  };

  const openExternal = async () => {
    if (!page) return;
    setRefused(null);
    try {
      await ipc.openFile(page.path);
    } catch (why) {
      setRefused(translate(locale, "preview.externalFailed", { why: String(why) }));
    }
  };

  // The empty tab's one control: ask the host for the project's entry page and show it here. The
  // address it answers with is the same `preview://` form every other page in this panel arrives
  // as, so it goes back through the same parser — and a project without an entry page says so in
  // the host's own sentence rather than sitting behind a button that never answers.
  const start = async () => {
    setRefused(null);
    try {
      const url = await ipc.previewEntry();
      const path = previewPathOf(url);
      if (path === null) return;
      navigate(url, path);
    } catch (why) {
      setRefused(String(why));
    }
  };

  const newTabLabel = translate(locale, "preview.newTab");
  const startTab = () => {
    addTab();
    field.current?.focus();
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-bg text-fg">
      <div
        role="tablist"
        aria-label={translate(locale, "preview.title")}
        className="flex h-[var(--header-height)] shrink-0 items-center gap-[var(--space-1)] overflow-x-auto border-b border-line px-[var(--space-2)]"
      >
        {tabs.map((one, i) => {
          const shown = previewPage(one);
          const label = shown ? nameOf(shown.path) : translate(locale, "preview.newTab");
          return (
            <div
              key={i}
              data-active={i === active}
              className={`flex h-8 min-w-0 max-w-[190px] shrink-0 items-center rounded-[var(--radius-md)] pl-[var(--space-2)] pr-0.5 ${
                i === active ? "bg-elevated" : "hover:bg-hover"
              }`}
            >
              <button
                type="button"
                role="tab"
                aria-selected={i === active}
                onClick={() => selectTab(i)}
                title={label}
                className="flex min-w-0 items-center gap-[var(--space-2)] py-1 text-quiet"
              >
                <Icon name="globe" size={13} className="shrink-0" />
                <span className="truncate">{label}</span>
              </button>
              <button
                type="button"
                onClick={() => closeTab(i)}
                aria-label={translate(locale, "preview.closeTab")}
                title={translate(locale, "preview.closeTab")}
                className="icon-btn shrink-0"
              >
                <Icon name="close" size={11} />
              </button>
            </div>
          );
        })}
        <button
          type="button"
          onClick={startTab}
          aria-label={newTabLabel}
          title={newTabLabel}
          className="icon-btn shrink-0"
        >
          <Icon name="plus" size={15} />
        </button>
      </div>

      <div className="flex h-[var(--header-height)] shrink-0 items-center gap-[var(--space-1)] border-b border-line px-[var(--space-2)]">
        <button
          type="button"
          onClick={startTab}
          aria-label={newTabLabel}
          title={newTabLabel}
          className="icon-btn shrink-0"
        >
          <Icon name="plus" size={15} />
        </button>
        <button
          type="button"
          onClick={back}
          disabled={!tab || tab.cursor <= 0}
          aria-label={translate(locale, "preview.back")}
          title={translate(locale, "preview.back")}
          className="icon-btn shrink-0"
        >
          <Icon name="back" size={14} />
        </button>
        <button
          type="button"
          onClick={forward}
          disabled={!tab || tab.cursor >= tab.trail.length - 1}
          aria-label={translate(locale, "preview.forward")}
          title={translate(locale, "preview.forward")}
          className="icon-btn shrink-0"
        >
          <Icon name="forward" size={14} />
        </button>
        <button
          type="button"
          onClick={reload}
          disabled={!page}
          aria-label={translate(locale, "preview.reload")}
          title={translate(locale, "preview.reload")}
          className="icon-btn shrink-0"
        >
          <Icon name="reload" size={14} />
        </button>
        <form
          className="min-w-0 flex-1"
          onSubmit={(event) => {
            event.preventDefault();
            void open(address);
          }}
        >
          <div className="flex h-8 min-w-0 items-center rounded-[var(--radius-md)] border border-line bg-elevated px-[var(--space-3)] focus-within:border-accent-line">
            <input
              ref={field}
              value={address}
              onChange={(event) => setAddress(event.target.value)}
              // Esc belongs to the field while the caret is in it: the window's own Esc closes the
              // panel, and a user reaching for "put back the address" must not lose the page instead.
              onKeyDown={(event) => {
                if (event.key !== "Escape") return;
                event.preventDefault();
                setAddress(at);
                setRefused(null);
                event.currentTarget.blur();
              }}
              placeholder={translate(locale, "preview.addressHint")}
              aria-label={translate(locale, "preview.address")}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              className={`field-input ${address ? "" : "text-center placeholder:text-faint"}`}
            />
          </div>
        </form>
        <button
          type="button"
          onClick={() => void openExternal()}
          disabled={!page}
          aria-label={translate(locale, "preview.external")}
          title={translate(locale, "preview.external")}
          className="icon-btn shrink-0"
        >
          <Icon name="external" size={14} />
        </button>
      </div>

      {refused && (
        <p
          role="alert"
          className="shrink-0 border-b border-line px-[var(--space-3)] py-[var(--space-2)] text-fine text-danger"
        >
          {refused}
        </p>
      )}

      {page ? (
        <iframe
          ref={frame}
          // The reload counter is part of the key: the same address with a new key is a new document,
          // which is what the reload control asks the scheme for.
          key={`${page.url}#${tab?.reloads ?? 0}`}
          src={page.url}
          title={nameOf(page.path)}
          className="min-h-0 w-full flex-1 border-0 bg-white"
        />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-[var(--space-3)] px-[var(--space-6)] text-center">
          <Icon name="globe" size={46} className="text-faint" strokeWidth={1.2} />
          <p className="text-ui text-fg">{translate(locale, "preview.title")}</p>
          <p className="max-w-[36ch] text-fine text-muted">
            {translate(locale, "preview.emptyBody")}
          </p>
          <button
            type="button"
            onClick={() => void start()}
            className="mt-[var(--space-1)] inline-flex items-center gap-1.5 rounded-[var(--radius-md)] border border-line px-[var(--space-3)] py-1 text-quiet text-muted hover:bg-hover hover:text-fg"
          >
            {translate(locale, "preview.start")}
          </button>
        </div>
      )}
    </div>
  );
}
