// The Extensions page: what the agent is offered, and which of those this build actually runs.
//
// "Extensions" in a cloud app means an add-on store. This app has no add-on format and nothing to
// install from, so the row's local meaning is the one thing that occupies the same place: the list of
// capabilities the model is handed when it is allowed to act. That list exists in the host already —
// `freecode_tools::registry` is what the Coding loop offers — and this page reads it rather than
// restating it, so a tool added there shows up here without a second edit.
//
// The two facts worth a page are the ones no other surface shows. Which calls reach a runner in
// this build (the four reads and four writes inside the workspace, the nine page calls, the two web
// calls the privacy switch has to be on for, the command and its three runners, the repository's
// eight and the one delegation — the registry holds nothing else, so a row here is a capability
// rather than an intention), and what the run would do with each one right now. The second is
// computed by `freecode_core::approvals::evaluate` against the settings and sandbox in force — the
// same engine that decides it mid-run, so the page cannot tell a friendlier story than the loop acts.
import { useCallback, useEffect, useState } from "react";

import { Row, Section } from "./controls";
import { Icon } from "./icons";
import { translate, type Locale } from "../i18n";
import {
  ipc,
  type ExtensionInventory,
  type ExtensionTool,
  type McpInventory,
  type McpPublished,
  type McpReport,
  type McpServer,
  type McpTool,
  type SkillPack,
} from "../lib/ipc";
import { shortcutLabel } from "../lib/shortcuts";
import { useSkills } from "../state/skills";

type T = (key: string, params?: Record<string, string | number>) => string;

/** A label and its value, right-aligned. The nav column is a list of these, so the figures the page
 * leads with read as measurements rather than prose. */
function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-[var(--space-2)] py-1 text-quiet">
      <span className="text-muted">{label}</span>
      <span className="text-right font-medium tabular-nums text-fg-strong">{value}</span>
    </div>
  );
}

/**
 * What a run would do with this call — the approval answer, not the execution answer, which is the
 * section the row sits in. Only "Auto-approve" gets the accent's soft fill: a call that happens
 * without interrupting anyone is the thing this app grants narrowly, and the other two are the row
 * saying it has not been granted.
 */
function Decision({ label, granted }: { label: string; granted: boolean }) {
  return (
    <span
      className={`shrink-0 rounded-[var(--radius-pill)] px-2 py-0.5 text-fine ${
        granted ? "bg-accent-soft text-fg-strong" : "bg-sunken text-muted"
      }`}
    >
      {label}
    </span>
  );
}

/**
 * The row's two labels are looked up by the host's own wire spellings — the same convention the
 * refusal cards and the Settings options use, and the reason there is no copy of either vocabulary
 * here to drift. `extensions::tests` asserts the spellings that appear on the wire.
 */
function Tool({ tool, t }: { tool: ExtensionTool; t: T }) {
  return (
    <Row title={tool.title} description={`${tool.id} — ${tool.description}`}>
      <span className="flex shrink-0 items-center gap-[var(--space-2)]">
        <span className="pill shrink-0">{t(`class.${tool.permission}`)}</span>
        <Decision label={t(`decision.${tool.decision}`)} granted={tool.decision === "auto"} />
      </span>
    </Row>
  );
}

/**
 * A pack as a row. The title is the name a prompt types after the slash — the folder's own — rather
 * than the prettified one, because the page's job here is to list what can be invoked, and a row that
 * led with `Tidy commits` would name a command that resolves to nothing. The display name and the
 * characters the run is given ride the description line with it.
 */
function Skill({ pack, t }: { pack: SkillPack; t: T }) {
  const head = pack.name === pack.id ? pack.description : `${pack.name} — ${pack.description}`;
  return (
    <Row title={`/${pack.id}`} description={head}>
      <span className="flex shrink-0 items-center gap-[var(--space-2)]">
        {pack.cut && <span className="pill shrink-0">{t("skills.tooLong")}</span>}
        <span className="pill shrink-0">{t(`skill.${pack.source}`)}</span>
        <span className="shrink-0 tabular-nums text-fine text-muted">
          {pack.chars.toLocaleString()}
        </span>
      </span>
    </Row>
  );
}

/** Where a pack goes, said by the host's own resolved paths rather than a description of them: the
 * folder this project picks is the folder the run reads, and it is not always the one the page's name
 * implies. One line per place, and only the machine line when this host has no separate project folder
 * to name — an empty path printed under a heading is worse than the heading missing.
 */
function WhereToLook({ label, path }: { label: string; path: string }) {
  if (!path) return null;
  return (
    <p className="mt-[var(--space-3)] text-fine text-faint">
      {label}
      <br />
      <span className="font-mono break-words">{path}</span>
    </p>
  );
}

/* ---- the MCP servers this machine declares ---- */

/** What one server's press came back with, or why it did not. */
type Check = { report?: McpReport; error?: string };

/**
 * The calls one server answered with, as rows of the same vocabulary the tool list above uses. The
 * title is the name a call would go by and the description carries the server's own display name and
 * prose, so a user reading the page sees the two spellings side by side — which is the only way a
 * server answering for `list_dir` is visible rather than silent.
 */
function Published({ report, t }: { report: McpReport; t: T }) {
  return (
    <>
      <p className="px-[var(--space-4)] pb-[var(--space-2)] text-fine text-faint">
        {t("mcp.answered", {
          server: report.serverName || report.name,
          version: report.serverVersion,
          protocol: report.protocol,
          count: report.tools.length,
        })}
      </p>
      {report.tools.map((tool) => (
        <ToolRow key={tool.name} tool={tool} t={t} />
      ))}
    </>
  );
}

function ToolRow({ tool, t }: { tool: McpTool; t: T }) {
  const head = [tool.title !== tool.name ? tool.title : "", tool.description]
    .filter(Boolean)
    .join(" — ");
  return (
    <Row title={tool.name} description={head}>
      <span className="flex shrink-0 items-center gap-[var(--space-2)]">
        {tool.collides && <span className="pill shrink-0">{t("mcp.collides")}</span>}
        {tool.offered && (
          <span className="pill shrink-0" title={t("mcp.offeredName")}>
            {t("mcp.offered", { name: tool.offered })}
          </span>
        )}
      </span>
    </Row>
  );
}

/**
 * One declared server as a row. The title is the key written under `mcpServers`, because that is the
 * name the press is made against, and the command line rides the description so what the page shows is
 * what the press would start. Only the names of the variables the entry sets are shown, never their
 * values: a declaration is where an API key goes, and a row that echoed it would put the secret on the
 * screen for whoever is looking at the page.
 *
 * A server this build cannot start — a URL-shaped one, or one switched off in the file — keeps its row
 * and shows the reason in place of the press, because the entry exists and pretending otherwise would
 * read as a file the app ignored.
 */
function Server({
  server,
  t,
  outcome,
  checking,
  connected,
  onCheck,
}: {
  server: McpServer;
  t: T;
  outcome: Check | undefined;
  checking: boolean;
  /** Whether a run may ask for what this server publishes — that is, whether it has answered once. */
  connected: boolean;
  onCheck: () => void;
}) {
  const line = [server.command, ...server.args].filter(Boolean).join(" ");
  const description = server.envKeys.length
    ? `${line} · ${t("mcp.env", { keys: server.envKeys.join(", ") })}`
    : line;
  return (
    <>
      <Row title={server.name} description={description}>
        <span className="flex shrink-0 items-center gap-[var(--space-2)]">
          <span className="pill shrink-0">{server.transport}</span>
          {connected && <span className="pill shrink-0">{t("mcp.connected")}</span>}
          {server.runnable ? (
            <button type="button" className="field" disabled={checking} onClick={onCheck}>
              {checking ? t("mcp.asking") : t("mcp.check")}
            </button>
          ) : (
            <span className="pill shrink-0">
              {t(server.disabled ? "mcp.off" : "mcp.unrunnable")}
            </span>
          )}
        </span>
      </Row>
      {outcome?.error && (
        <p role="alert" className="px-[var(--space-4)] pb-[var(--space-3)] text-fine text-danger">
          {t("mcp.failed", { error: outcome.error })}
        </p>
      )}
      {outcome?.report && <Published report={outcome.report} t={t} />}
    </>
  );
}

export function Extensions({ locale, onBack }: { locale: Locale; onBack: () => void }) {
  const [inv, setInv] = useState<ExtensionInventory | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mcp, setMcp] = useState<McpInventory | null>(null);
  const [mcpError, setMcpError] = useState<string | null>(null);
  // One entry per server that has been asked, so a second press on a different row cannot answer over
  // the first one's row. `asking` is the name whose press is still out.
  const [checks, setChecks] = useState<Record<string, Check>>({});
  const [asking, setAsking] = useState<string | null>(null);
  // The calls a run may ask for, as the host's registry holds them.
  const [connected, setConnected] = useState<McpPublished[]>([]);
  const t: T = (key, params) => translate(locale, `extensions.${key}`, params);
  // The packs come from the store the composer reads, so the page and the slash strip cannot show two
  // different vocabularies. `force` asks for the folder as it is now: a page that reports what is on
  // disk should not answer from a list this window read before the last pack was saved.
  const skills = useSkills((s) => s.skills);
  const machineDir = useSkills((s) => s.machineDir);
  const projectDir = useSkills((s) => s.projectDir);
  const skillsError = useSkills((s) => s.error);
  const loadSkills = useSkills((s) => s.load);

  useEffect(() => {
    void loadSkills(true);
  }, [loadSkills]);

  useEffect(() => {
    let live = true;
    void ipc
      .extensionsInventory()
      .then((found) => {
        if (live) setInv(found);
      })
      .catch((e: unknown) => {
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, []);

  // The declaration file is read the same way the tool list is: once, on the host's own answer, with
  // nothing started on the way. A failure here is about the file, so it gets its own line rather than
  // the page-level "the registry could not be read", which would name the wrong thing.
  useEffect(() => {
    let live = true;
    void ipc
      .mcpInventory()
      .then((found) => {
        if (live) setMcp(found);
      })
      .catch((e: unknown) => {
        if (live) setMcpError(String(e));
      });
    return () => {
      live = false;
    };
  }, []);

  // What a run may ask for, read from the host's own registry rather than from this page's memory of
  // the last press: a server that answered once keeps its calls until it is asked again, so the page
  // has to be able to say which ones are reachable after a reload.
  useEffect(() => {
    let live = true;
    void ipc
      .mcpPublished()
      .then((found) => {
        if (live) setConnected(found);
      })
      .catch(() => {
        if (live) setConnected([]);
      });
    return () => {
      live = false;
    };
  }, []);

  // The press the row offers. It starts the command, takes the handshake and the tool list, and stops
  // the process again — so it is stated as the question it answers, and its failure is the host's own
  // sentence about why, not a spinner that never clears.
  const ask = useCallback(async (name: string) => {
    setAsking(name);
    setChecks((current) => ({ ...current, [name]: {} }));
    try {
      const report = await ipc.mcpCheck(name);
      setChecks((current) => ({ ...current, [name]: { report } }));
      setConnected(await ipc.mcpPublished());
    } catch (reason) {
      setChecks((current) => ({ ...current, [name]: { error: String(reason) } }));
    } finally {
      setAsking(null);
    }
  }, []);

  // `?tool=<id>` is the deep link to one row of this list, the way `?row=` is the deep link to a
  // settings control: the list is long and this window cannot be scrolled or clicked by the harness.
  // Dev-only, and it moves the page rather than changing it — the row it finds is the row the
  // engine already decided.
  useEffect(() => {
    if (!inv) return;
    const wanted = import.meta.env.DEV
      ? new URLSearchParams(window.location.search).get("tool")
      : null;
    if (!wanted) return;
    const rows = [...document.querySelectorAll<HTMLElement>(".card-row")];
    const hit = rows.find((row) =>
      row.querySelector(".row-desc")?.textContent?.trim().startsWith(`${wanted} —`),
    );
    if (!hit) return;
    hit.scrollIntoView({ block: "center" });
    hit.dataset.flash = "true";
    const id = window.setTimeout(() => delete hit.dataset.flash, 1400);
    return () => window.clearTimeout(id);
  }, [inv]);

  // `?skill=<anything>` is the same door for the packs, which sit under the whole tool list: the
  // section is the last thing on a page longer than the window, and this window cannot be scrolled by
  // the harness. The value is not a filter — the packs are the folder's, all of them — so any value
  // takes the capture to where they are listed, which is also where the empty state and its two paths
  // sit. It waits for the inventory because the rows above are what move the section out of the frame.
  useEffect(() => {
    if (!inv) return;
    if (!import.meta.env.DEV) return;
    if (!new URLSearchParams(window.location.search).get("skill")) return;
    document.getElementById("skills")?.scrollIntoView({ block: "start" });
  }, [inv]);

  // `?mcp=<anything>` takes the capture to the servers, which sit under the whole tool list, and
  // `?mcpcheck=<name>` presses the named row's Check for it. This window takes no injected clicks and
  // the answer is a round trip to somebody else's process, so the state a press puts on the page — the
  // protocol the server chose, its calls, the collision flag — is otherwise unreachable from a harness.
  // Both are dev-only, and the seed only fires for a server the file actually declares.
  useEffect(() => {
    if (!mcp || !import.meta.env.DEV) return;
    const params = new URLSearchParams(window.location.search);
    if (!params.get("mcp")) return;
    document.getElementById("mcp")?.scrollIntoView({ block: "start" });
    const wanted = params.get("mcpcheck");
    if (wanted && mcp.servers.some((s) => s.name === wanted)) void ask(wanted);
  }, [mcp, ask]);

  const blocked = inv?.tools.filter((r) => r.decision === "blocked").length ?? 0;
  // The registry holds only calls that reach a runner, so this is empty today. The section stays
  // because it is what an unimplemented declaration would read as — a row that says so out loud
  // rather than a capability the page implies.
  const declaredOnly = inv?.tools.filter((r) => !r.runnable) ?? [];

  return (
    <div className="flex h-full min-h-0 bg-bg text-fg">
      <nav
        aria-label={t("title")}
        className="flex h-full w-[var(--inspector-width)] shrink-0 flex-col border-r border-line bg-panel"
      >
        <div
          className="pt-[var(--space-3)] pb-1 pr-[var(--space-3)]"
          style={{ paddingLeft: "max(var(--space-4), var(--traffic-light-inset))" }}
        >
          <button
            type="button"
            title={shortcutLabel("escape")}
            className="nav-item"
            onClick={onBack}
          >
            <Icon name="back" />
            <span>{t("back")}</span>
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-[var(--space-3)] pb-[var(--space-3)]">
          {!inv && !error && (
            // The column holds its shape through the round trip rather than arriving empty and
            // filling, the way the status card does for the same reason.
            <div className="flex flex-col gap-1">
              {[0, 1, 2].map((i) => (
                <span key={i} className="skeleton-line block h-7" />
              ))}
            </div>
          )}
          {inv && (
            <>
              <p className="section-label">{t("thisBuild")}</p>
              <Stat label={t("declared")} value={String(inv.tools.length)} />
              <Stat label={t("runnable")} value={String(inv.runnable)} />
              <Stat label={t("blocked")} value={String(blocked)} />

              <p className="section-label mt-[var(--space-4)]">{t("policy")}</p>
              {/* The approval dial's own option strings, from Settings: one vocabulary for the value,
                  so the page and the control it reports cannot word it differently. */}
              <Stat
                label={t("mode")}
                value={translate(locale, `settings.opt.${inv.approvalMode}`)}
              />
              <Stat
                label={t("network")}
                value={t(inv.networkAllowed ? "network.on" : "network.off")}
              />
              <p className="mt-[var(--space-3)] text-fine text-faint">
                {t("workspace")}
                <br />
                <span className="font-mono break-words">{inv.workspaceRoot}</span>
              </p>
            </>
          )}
        </div>
      </nav>

      <main className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-[980px] px-[var(--space-6)] py-[var(--space-6)]">
          <h1 className="text-page font-semibold text-fg-strong">{t("title")}</h1>
          <p className="mt-1 text-muted">{inv ? t("about") : t("loading")}</p>

          {error && (
            <p
              role="alert"
              className="mt-[var(--space-4)] rounded-[var(--radius-md)] border border-danger px-[var(--space-3)] py-[var(--space-2)] text-quiet text-danger"
            >
              {t("loadFailed", { error })}
            </p>
          )}

          {inv && (
            <>
              <Section label={t("runs")} description={t("runs.desc", { count: inv.runnable })}>
                {inv.tools
                  .filter((r) => r.runnable)
                  .map((r) => (
                    <Tool key={r.id} tool={r} t={t} />
                  ))}
              </Section>

              {declaredOnly.length > 0 && (
                <Section
                  label={t("declaredOnly")}
                  description={t("declaredOnly.desc", { count: declaredOnly.length })}
                >
                  {declaredOnly.map((r) => (
                    <Tool key={r.id} tool={r} t={t} />
                  ))}
                </Section>
              )}

              {/* The packs a prompt can be worked under. Read from disk by the same host that will read
                  one back during a run, so this is the vocabulary the slash command actually has. */}
              <Section
                label={t("skills")}
                id="skills"
                description={
                  skills.length === 0 ? undefined : t("skills.desc", { count: skills.length })
                }
              >
                {skills.length === 0 ? (
                  <>
                    <p className="px-[var(--space-4)] py-[var(--space-3)] text-quiet text-muted">
                      {t("skills.none")}
                    </p>
                    <div className="px-[var(--space-4)] pb-[var(--space-3)]">
                      <WhereToLook label={t("skills.machine")} path={machineDir} />
                      <WhereToLook label={t("skills.project")} path={projectDir} />
                      {skillsError && (
                        <p role="alert" className="mt-[var(--space-3)] text-fine text-danger">
                          {t("skills.failed", { error: skillsError })}
                        </p>
                      )}
                    </div>
                  </>
                ) : (
                  skills.map((pack) => <Skill key={pack.id} pack={pack} t={t} />)
                )}
              </Section>

              {/* The servers this machine declares. Listed before anything is asked of them, and a press
                  that starts one answers only the question "what do you publish" — which is why the note
                  under the rows says what a run cannot do with the answer yet. */}
              {(mcp || mcpError) && (
                <Section
                  label={t("mcp")}
                  id="mcp"
                  description={
                    mcp && mcp.servers.length === 0
                      ? undefined
                      : t("mcp.desc", { count: mcp?.servers.length ?? 0 })
                  }
                >
                  {mcpError && (
                    <p
                      role="alert"
                      className="px-[var(--space-4)] py-[var(--space-3)] text-fine text-danger"
                    >
                      {t("mcp.loadFailed", { error: mcpError })}
                    </p>
                  )}
                  {mcp && mcp.error && (
                    <p
                      role="alert"
                      className="px-[var(--space-4)] py-[var(--space-3)] text-fine text-danger"
                    >
                      {t("mcp.broken", { error: mcp.error })}
                    </p>
                  )}
                  {mcp &&
                    (mcp.servers.length === 0 ? (
                      <>
                        <p className="px-[var(--space-4)] py-[var(--space-3)] text-quiet text-muted">
                          {t("mcp.none")}
                        </p>
                        <div className="px-[var(--space-4)] pb-[var(--space-3)]">
                          <WhereToLook label={t("mcp.file")} path={mcp.path} />
                        </div>
                      </>
                    ) : (
                      mcp.servers.map((server) => (
                        <Server
                          key={server.name}
                          server={server}
                          t={t}
                          outcome={checks[server.name]}
                          checking={asking === server.name}
                          connected={connected.some((call) => call.server === server.name)}
                          onCheck={() => void ask(server.name)}
                        />
                      ))
                    ))}
                  <p className="px-[var(--space-4)] pb-[var(--space-3)] text-fine text-faint">
                    {t("mcp.note")}
                  </p>
                </Section>
              )}

              <p className="text-fine text-faint">{t("note")}</p>
            </>
          )}
        </div>
      </main>
    </div>
  );
}
