// The window's half of the agent's page bridge.
//
// A `browser_*` call runs on the host's tool thread, which emits `page-request` and blocks until
// `page_reply` arrives. The page itself lives in a cross-origin frame — the `preview` scheme — so the
// window cannot reach into it: it relays each request to the script the scheme injects into every
// HTML page it serves, and hands the script's answer back. Nothing else in the app speaks this
// protocol, and the page accepts only messages stamped `freecode-agent`.
import { listen } from "@tauri-apps/api/event";

import { useNav } from "../state/nav";
import { ipc, PAGE_EVENT, type PageExpect, type PageRequest } from "./ipc";

/** The frame the panel is showing, the address it was given, and — once the page's script can
 * answer — whether it is loaded far enough to speak. */
type Shown = { frame: HTMLIFrameElement; url: string };

let shown: Shown | null = null;
/** How long the panel waits for the frame to mount and to answer a request. Both stay under the
 * host's own patience (`page::patience`), so the model reads the specific sentence instead of a
 * generic timeout. */
const MOUNT_MS = 6000;
const REPLY_MS = 6000;
/** A page's own `wait` polls for up to 15 s; the answer, not the wait, has to outlast it. */
const WAIT_SLACK_MS = 2500;
/** Ids are the window's own; the page caches its answer under the id, so a retried post cannot make
 * a click happen twice. */
let next = 1;
const pending = new Map<number, (body: Record<string, unknown>) => void>();

/**
 * Tell the bridge which frame the panel is showing. Called from the preview panel's own effect: the
 * frame exists only while the panel is open on a page, so `null` is the panel closing or an empty
 * tab, and a page call then has nothing to act on.
 */
export function attachPage(frame: HTMLIFrameElement | null, url: string): void {
  shown = frame ? { frame, url } : null;
}

const sleep = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

function message(event: MessageEvent): void {
  const data = event.data as { source?: unknown; id?: unknown; reply?: unknown } | null;
  if (!data || data.source !== "freecode-page" || typeof data.id !== "number") return;
  // Only the frame the panel is showing may answer, and only for a request still waiting.
  if (!shown || event.source !== shown.frame.contentWindow) return;
  const answer = pending.get(data.id);
  if (!answer) return;
  pending.delete(data.id);
  answer((data.reply ?? {}) as Record<string, unknown>);
}

/** The frame showing `url`, once the panel has mounted it. The panel opens through the store, so
 * the frame arrives a render later than the request. */
async function mounted(url: string): Promise<Shown> {
  const deadline = Date.now() + MOUNT_MS;
  while (Date.now() < deadline) {
    if (shown && shown.url === url) return shown;
    await sleep(40);
  }
  throw new Error(
    `the browser panel did not open ${url} in time. It must be able to show the page for a browser ` +
      `call to act on it.`,
  );
}

/**
 * One operation, sent until the page answers. A frame still parsing its document has not installed
 * the listener yet, and a post to it is simply lost — so the request is repeated while the panel
 * still shows that frame, which is what the page's own reply cache makes safe to do.
 */
async function ask(
  target: Shown,
  op: string,
  fields: Record<string, unknown>,
  deadline: number,
): Promise<Record<string, unknown>> {
  const id = next++;
  return new Promise((resolve, reject) => {
    let settled = false;
    let retry = 0;
    let expiry = 0;
    const finish = (why?: string, body?: Record<string, unknown>) => {
      if (settled) return;
      settled = true;
      window.clearInterval(retry);
      window.clearTimeout(expiry);
      pending.delete(id);
      if (why !== undefined) reject(new Error(why));
      else resolve(body ?? {});
    };
    const send = () => {
      if (shown !== target) {
        finish("the browser panel moved to another page before it answered");
        return;
      }
      target.frame.contentWindow?.postMessage({ source: "freecode-agent", id, op, fields }, "*");
    };
    pending.set(id, (body) => finish(body.error ? String(body.error) : undefined, body));
    retry = window.setInterval(send, 250);
    expiry = window.setTimeout(
      () => finish(`the page did not answer "${op}" in time`),
      Math.max(0, deadline - Date.now()),
    );
    send();
  });
}

/**
 * Open a page in the panel and answer with what it shows: the panel is the store's, so opening is
 * the same press the review's control makes, and the frame that follows is the one every later call
 * acts on.
 */
async function open(fields: Record<string, unknown>): Promise<Record<string, unknown>> {
  const url = typeof fields.url === "string" ? fields.url : "";
  const path = typeof fields.path === "string" ? fields.path : "";
  if (!url) throw new Error("the browser was handed no address to open");
  useNav.getState().openPreview(url, path);
  const target = await mounted(url);
  const body = await ask(target, "read", {}, Date.now() + REPLY_MS);
  return { ...body, url: body.url || url };
}

/**
 * Save what the panel's frame draws. The page's own script is not asked: the pixels are the webview's
 * own render, so what lands on disk is the frame as the window shows it, and only the window knows
 * where that frame sits in the layout.
 */
async function capture(target: Shown): Promise<Record<string, unknown>> {
  const box = target.frame.getBoundingClientRect();
  return ipc.capturePage({
    x: box.left,
    y: box.top,
    width: box.width,
    height: box.height,
  });
}

/**
 * Take the same picture a capture takes and let the host assert on it. The page's own script is not
 * asked: the verdict is measured on the frame the webview drew, so a run can check what a page shows
 * without a model looking at it — and a failed check comes back as a verdict, not as an error.
 */
async function check(
  target: Shown,
  fields: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const box = target.frame.getBoundingClientRect();
  return ipc.checkPage(
    { x: box.left, y: box.top, width: box.width, height: box.height },
    fields as PageExpect,
  );
}

async function perform(op: string, fields: Record<string, unknown>): Promise<unknown> {
  if (op === "open") return open(fields);
  if (!shown) {
    throw new Error(
      "the browser panel is closed in this window. browser_open opens a page and the panel with it, " +
        "and the other browser calls act on that page.",
    );
  }
  if (op === "capture") return capture(shown);
  if (op === "check") return check(shown, fields);
  const waited = typeof fields.timeout_ms === "number" ? fields.timeout_ms : 0;
  const deadline = Date.now() + (op === "wait" ? waited + WAIT_SLACK_MS : REPLY_MS);
  return ask(shown, op, fields, deadline);
}

async function answer(request: PageRequest): Promise<void> {
  try {
    const value = await perform(request.op, request.arguments ?? {});
    await ipc.pageReply(request.id, { value });
  } catch (why) {
    await ipc.pageReply(request.id, {
      error: why instanceof Error ? why.message : String(why),
    });
  }
}

/**
 * Wire the bridge for the life of the window. Called at module scope in `main.tsx` for the same
 * reason the run's own channel is: a listener registered inside the React tree would be doubled by
 * StrictMode's dev mount, and a request arriving in that gap would have nobody to answer it.
 */
export function startPageBridge(): void {
  window.addEventListener("message", message);
  void listen<PageRequest>(PAGE_EVENT, (event) => void answer(event.payload));
}
