import type { ReactNode } from "react";
import { useState } from "react";

import { translate, type Locale } from "../i18n";
import { isWebAddress, openExternal } from "../lib/external";
import { Icon } from "./icons";

/**
 * A link that leaves the app for the browser.
 *
 * An unclickable `https://…` under a refusal reads as a bug report rather than as an instruction, so
 * this renders the words the host or the answer chose and gives the address to the system when it is
 * pressed. It falls back to the bare address in the two cases where pressing cannot work — a non-web
 * scheme, or a machine with nowhere to send the link — because a control that does nothing silently
 * is the one failure this app has already promised not to ship.
 */
export function ExternalLink({
  url,
  label,
  locale,
  inline,
}: {
  url: string;
  label: ReactNode;
  locale: Locale;
  /** The prose shape: a link sitting inside a line of text rather than under a paragraph. */
  inline?: boolean;
}) {
  const [failed, setFailed] = useState(false);
  if (failed || !isWebAddress(url)) {
    return (
      <span className="external-stuck" title={translate(locale, "link.stuck")}>
        {url}
      </span>
    );
  }
  return (
    <button
      type="button"
      onClick={() => void openExternal(url).then((opened) => !opened && setFailed(true))}
      title={translate(locale, "link.opens")}
      className={inline ? "external-link external-link-inline" : "external-link"}
    >
      {label}
      <Icon name="external" size={inline ? 11 : 12} />
    </button>
  );
}
