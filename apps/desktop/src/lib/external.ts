import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * Handing an address to the user's browser.
 *
 * Two things gate this, and both are needed. The capability scopes the host's `open_url` to `http`
 * and `https`, so a `file://` path or a `javascript:` address cannot launch anything even if it
 * reaches the command — and this check is what keeps the app from asking at all, which is the only
 * way a link that will never open can be recognised early enough to render it differently.
 */
const WEB_ADDRESS = /^https?:\/\/\S+$/i;

export function isWebAddress(url: string): boolean {
  return WEB_ADDRESS.test(url.trim());
}

/**
 * `false` when nothing was opened — the address is not a web one, or the OS had nowhere to send it.
 * A link that silently did nothing is the failure this returns a signal for, so the caller can show
 * the address instead and leave the user something to copy.
 */
export async function openExternal(url: string): Promise<boolean> {
  const address = url.trim();
  if (!isWebAddress(address)) return false;
  try {
    await openUrl(address);
    return true;
  } catch {
    return false;
  }
}
