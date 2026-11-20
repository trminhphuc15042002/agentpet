//! Writes/removes AgentPet's hook entries in each agent's config, using Windows
//! paths (%USERPROFILE%\.claude\settings.json, ...). Ported from the macOS app's
//! AgentHooks + HookInstaller. Our entries are identified by their command
//! string so install is idempotent and foreign hooks are never touched.

use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Serialize, Clone)]
pub struct AgentInfo {
    pub kind: String,
    pub display_name: String,
    pub installed: bool,
    pub note: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    ClaudeNested,      // {"hooks": {Event: [{"hooks": [{"type":"command","command":..}]}]}}
    CursorFlat,        // {"version":1,"hooks":{event:[{"command":..,"type":"command"}]}}
    WindsurfFlat,      // {"hooks":{event:[{"command":..,"show_output":false}]}}
    KiroFlat,          // agent file: {"name":..,"hooks":{event:[{"command":..}]}}
    AntigravityNested, // {"agentpet": {Event: [..]}} (matcher events vs bare handlers)
    OpencodePlugin,    // a JS plugin file
    PiExtension,       // a TS extension file for Pi (~/.pi/agent/extensions)
    HermesYaml,        // Hermes config.yaml `hooks:` block: event -> [{command:..}]
}

struct Spec {
    style: Style,
    rel_path: &'static [&'static str],
    events: &'static [&'static str],
}

fn spec(kind: &str) -> Option<Spec> {
    Some(match kind {
        "claude" => Spec { style: Style::ClaudeNested, rel_path: &[".claude", "settings.json"],
            events: &["SessionStart", "UserPromptSubmit", "PreToolUse", "Notification", "Stop", "SubagentStop", "SessionEnd"] },
        "codex" => Spec { style: Style::ClaudeNested, rel_path: &[".codex", "hooks.json"],
            events: &["SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest", "Stop", "Interrupt", "SessionEnd", "SubagentStart", "SubagentStop"] },
        "gemini" => Spec { style: Style::ClaudeNested, rel_path: &[".gemini", "settings.json"],
            events: &["SessionStart", "BeforeAgent", "BeforeTool", "AfterTool", "Notification", "AfterAgent", "SessionEnd"] },
        "cursor" => Spec { style: Style::CursorFlat, rel_path: &[".cursor", "hooks.json"],
            events: &["sessionStart", "beforeSubmitPrompt", "preToolUse", "stop", "subagentStop", "sessionEnd"] },
        "copilot" => Spec { style: Style::CursorFlat, rel_path: &[".copilot", "hooks", "agentpet.json"],
            events: &["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"] },
        "windsurf" => Spec { style: Style::WindsurfFlat, rel_path: &[".codeium", "windsurf", "hooks.json"],
            events: &["pre_user_prompt", "post_cascade_response"] },
        "antigravity" => Spec { style: Style::AntigravityNested, rel_path: &[".gemini", "config", "hooks.json"],
            events: &["PreInvocation", "PreToolUse", "PostToolUse", "Stop"] },
        "kiro" => Spec { style: Style::KiroFlat, rel_path: &[".kiro", "agents", "default.json"],
            events: &["agentSpawn", "userPromptSubmit", "postToolUse", "stop"] },
        "opencode" => Spec { style: Style::OpencodePlugin, rel_path: &[".config", "opencode", "plugins", "agentpet.js"],
            events: &[] },
        "droid" => Spec { style: Style::ClaudeNested, rel_path: &[".factory", "hooks.json"],
            events: &["SessionStart", "UserPromptSubmit", "PreToolUse", "Notification", "Stop", "SubagentStop", "SessionEnd"] },
        "pi" => Spec { style: Style::PiExtension, rel_path: &[".pi", "agent", "extensions", "agentpet.ts"],
            events: &[] },
        // xAI Grok Build: ~/.grok/hooks/agentpet.json, Claude-compatible nested
        // shape (PascalCase config keys). PreToolUse is omitted , Grok treats a
        // hook's exit 2 as deny/keep-working, and our hook always exits 0.
        "grok" => Spec { style: Style::ClaudeNested, rel_path: &[".grok", "hooks", "agentpet.json"],
            events: &["SessionStart", "UserPromptSubmit", "PostToolUse", "Notification", "Stop", "SessionEnd"] },
        // Hermes Agent (Nous Research): shell hooks in config.yaml, one home per
        // profile. `pre_tool_call` is the only blocking event, so the pet uses
        // observers only (turn start/tool end/done) rather than gating tools.
        "hermes" => Spec { style: Style::HermesYaml, rel_path: &["hermes", "config.yaml"],
            events: &["on_session_start", "pre_llm_call", "post_tool_call", "pre_approval_request",
                "on_human_input_request", "post_llm_call", "on_session_end"] },
        _ => return None,
    })
}

pub fn catalog() -> Vec<AgentInfo> {
    let entries: &[(&str, &str, Option<&str>)] = &[
        ("claude", "Claude Code", None),
        ("codex", "Codex", Some("After enabling, run /hooks in Codex and Trust the AgentPet hook")),
        ("gemini", "Gemini CLI", None),
        ("cursor", "Cursor", None),
        ("opencode", "opencode", None),
        ("windsurf", "Windsurf", Some("No \"needs input\" alerts (Windsurf has no such hook)")),
        ("antigravity", "Antigravity", Some("No \"needs input\" alerts (Antigravity has no notification hook)")),
        ("copilot", "GitHub Copilot", Some("Copilot CLI only (~/.copilot/hooks)")),
        ("kiro", "Kiro CLI", Some("Hooks the default Kiro CLI agent")),
        ("droid", "Factory Droid", Some("Factory Droid CLI (~/.factory/hooks.json)")),
        ("pi", "Pi", Some("Pi extension (~/.pi/agent/extensions). No \"needs input\" alerts")),
        ("grok", "Grok Build", Some("xAI Grok Build CLI (~/.grok/hooks/agentpet.json)")),
        ("jcode", "jcode", Some("jcode lifecycle hooks (~/.jcode/config.toml, in WSL when present)")),
        ("hermes", "Hermes Agent", Some("Hermes Agent CLI (%LOCALAPPDATA%\\hermes\\config.yaml, all profiles)")),
    ];
    entries.iter().map(|(kind, name, note)| AgentInfo {
        kind: kind.to_string(),
        display_name: name.to_string(),
        installed: is_installed(kind),
        note: note.map(|s| s.to_string()),
    }).collect()
}

fn config_path(kind: &str) -> Option<PathBuf> {
    // Hermes keeps one home per profile, so it has its own path resolver.
    if kind == "hermes" { return None; }
    let mut p = dirs::home_dir()?;
    for part in spec(kind)?.rel_path { p.push(part); }
    Some(p)
}

/// Every config file this agent reads. Install/uninstall touch all of them:
/// Hermes gives each profile its own home, so hooking only the base home would
/// miss `forge`/`worker-*`/... sessions.
fn config_paths(kind: &str) -> Vec<PathBuf> {
    if kind == "hermes" { return hermes::config_paths(); }
    config_path(kind).into_iter().collect()
}

fn hook_command() -> String {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "agentpet".into());
    format!("\"{}\" hook --agent", exe)
}
fn full_command(kind: &str) -> String { format!("{} {}", hook_command(), kind) }

fn is_ours(cmd: &str) -> bool {
    let l = cmd.to_lowercase();
    l.contains("agentpet") && l.contains("hook")
}

fn read_json(path: &PathBuf) -> Value {
    std::fs::read_to_string(path).ok()
        .and_then(|s| if s.trim().is_empty() { None } else { serde_json::from_str(&s).ok() })
        .unwrap_or_else(|| json!({}))
}
fn write_json(path: &PathBuf, v: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    std::fs::write(path, serde_json::to_string_pretty(v).unwrap_or_default())
}

// ----- per-style entry helpers --------------------------------------------
fn container_key(style: Style) -> &'static str {
    if style == Style::AntigravityNested { "agentpet" } else { "hooks" }
}
fn antigravity_matcher(event: &str) -> bool {
    matches!(event, "PreToolUse" | "PostToolUse")
}
fn group_is_ours(entry: &Value) -> bool {
    entry.get("hooks").and_then(|h| h.as_array())
        .map(|a| a.iter().any(|h| h.get("command").and_then(|c| c.as_str()).map(is_ours).unwrap_or(false)))
        .unwrap_or(false)
}
fn flat_is_ours(entry: &Value) -> bool {
    entry.get("command").and_then(|c| c.as_str()).map(is_ours).unwrap_or(false)
}
fn entry_is_ours(style: Style, event: &str, entry: &Value) -> bool {
    match style {
        Style::ClaudeNested => group_is_ours(entry),
        Style::AntigravityNested => if antigravity_matcher(event) { group_is_ours(entry) } else { flat_is_ours(entry) },
        _ => flat_is_ours(entry),
    }
}
fn make_entry(style: Style, event: &str, cmd: &str) -> Value {
    match style {
        Style::ClaudeNested => json!({ "hooks": [{ "type": "command", "command": cmd }] }),
        Style::CursorFlat => json!({ "command": cmd, "type": "command" }),
        Style::WindsurfFlat => json!({ "command": cmd, "show_output": false }),
        Style::KiroFlat => json!({ "command": cmd }),
        Style::AntigravityNested => if antigravity_matcher(event) {
            json!({ "matcher": "*", "hooks": [{ "type": "command", "command": cmd }] })
        } else {
            json!({ "type": "command", "command": cmd })
        },
        Style::OpencodePlugin | Style::PiExtension | Style::HermesYaml => Value::Null,
    }
}

// ----- public API ----------------------------------------------------------
pub fn is_installed(kind: &str) -> bool {
    if kind == "jcode" {
        return jcode_target()
            // A read error here only means "not detected": nothing is written.
            .map(|t| crate::jcode::is_installed(&std::fs::read_to_string(&t.config).unwrap_or_default(), crate::jcode::EVENTS))
            .unwrap_or(false);
    }
    let Some(s) = spec(kind) else { return false };
    if s.style == Style::HermesYaml {
        // Installed only when every home Hermes reads carries the hook; a new
        // profile otherwise leaves the pet blind with the toggle still "on".
        let paths = config_paths(kind);
        return !paths.is_empty() && paths.iter().all(|p| {
            std::fs::read_to_string(p)
                .map(|text| hermes::is_installed(&text, s.events))
                .unwrap_or(false)
        });
    }
    let Some(path) = config_path(kind) else { return false };
    if s.style == Style::OpencodePlugin || s.style == Style::PiExtension {
        return std::fs::read_to_string(&path).map(|c| is_ours(&c)).unwrap_or(false);
    }
    let v = read_json(&path);
    let Some(map) = v.get(container_key(s.style)).and_then(|h| h.as_object()) else { return false };
    s.events.iter().any(|event| {
        map.get(*event).and_then(|a| a.as_array())
            .map(|arr| arr.iter().any(|e| entry_is_ours(s.style, event, e)))
            .unwrap_or(false)
    })
}

pub fn toggle(kind: &str) -> Result<bool, String> {
    if is_installed(kind) {
        uninstall(kind).map_err(|e| e.to_string())?;
        Ok(false)
    } else {
        install(kind).map_err(|e| e.to_string())?;
        Ok(true)
    }
}

fn install(kind: &str) -> std::io::Result<()> {
    if kind == "jcode" { return jcode_install(); }
    let Some(s) = spec(kind) else {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "unknown agent"));
    };
    let cmd = full_command(kind);

    if s.style == Style::HermesYaml {
        return hermes::install_all(&config_paths(kind), &cmd, s.events);
    }

    let Some(path) = config_path(kind) else {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "unknown agent"));
    };

    if s.style == Style::OpencodePlugin {
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
        std::fs::write(&path, opencode_plugin(&binary_from(&cmd)))?;
        // Migration: V1 stored the plugin in the singular `plugin/` dir, which
        // OpenCode V2 ignores; remove it so it can never shadow the V2 file.
        if let Some(home) = dirs::home_dir() {
            let legacy = home.join(".config").join("opencode").join("plugin").join("agentpet.js");
            let _ = std::fs::remove_file(legacy);
        }
        return Ok(());
    }
    if s.style == Style::PiExtension {
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
        return std::fs::write(&path, pi_extension(&binary_from(&cmd)));
    }

    let mut v = read_json(&path);
    let Some(obj) = v.as_object_mut() else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,
            format!("{} is not a JSON object; fix or remove it and try again", path.display())));
    };
    if s.style == Style::CursorFlat { obj.entry("version").or_insert(json!(1)); }
    // A fresh Kiro agent file needs a name to be a valid agent.
    if s.style == Style::KiroFlat && obj.get("name").is_none() {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "default".into());
        obj.insert("name".to_string(), json!(name));
    }
    let key = container_key(s.style);
    if !obj.get(key).map_or(false, |h| h.is_object()) { obj.insert(key.to_string(), json!({})); }
    let map = obj.get_mut(key).and_then(|h| h.as_object_mut()).unwrap();
    for event in s.events {
        let mut kept: Vec<Value> = map.get(*event).and_then(|a| a.as_array())
            .map(|a| a.iter().filter(|e| !entry_is_ours(s.style, event, e)).cloned().collect())
            .unwrap_or_default();
        kept.push(make_entry(s.style, event, &cmd));
        map.insert((*event).to_string(), Value::Array(kept));
    }
    write_json(&path, &v)?;
    if kind == "codex" { enable_codex_hooks(); }
    Ok(())
}

fn uninstall(kind: &str) -> std::io::Result<()> {
    if kind == "jcode" { return jcode_uninstall(); }
    let Some(s) = spec(kind) else { return Ok(()) };
    let cmd = full_command(kind);
    if s.style == Style::HermesYaml {
        return hermes::uninstall_all(&config_paths(kind), &cmd, s.events);
    }
    let Some(path) = config_path(kind) else { return Ok(()) };
    // Files we own outright: just delete.
    if kind == "copilot" || s.style == Style::OpencodePlugin || s.style == Style::PiExtension {
        let _ = std::fs::remove_file(&path);
        if s.style == Style::OpencodePlugin {
            if let Some(home) = dirs::home_dir() {
                let legacy = home.join(".config").join("opencode").join("plugin").join("agentpet.js");
                let _ = std::fs::remove_file(legacy);
            }
        }
        return Ok(());
    }
    let mut v = read_json(&path);
    let Some(obj) = v.as_object_mut() else { return Ok(()) };
    let key = container_key(s.style);
    if let Some(map) = obj.get_mut(key).and_then(|h| h.as_object_mut()) {
        for event in s.events {
            if let Some(arr) = map.get(*event).and_then(|a| a.as_array()) {
                let kept: Vec<Value> = arr.iter().filter(|e| !entry_is_ours(s.style, event, e)).cloned().collect();
                if kept.is_empty() { map.remove(*event); } else { map.insert((*event).to_string(), Value::Array(kept)); }
            }
        }
        if map.is_empty() { obj.remove(key); }
    }
    write_json(&path, &v)
}

/// Extracts the quoted binary path from a hook command for the opencode plugin.
fn binary_from(cmd: &str) -> String {
    if let Some(start) = cmd.find('"') {
        if let Some(end) = cmd[start + 1..].find('"') {
            return cmd[start + 1..start + 1 + end].to_string();
        }
    }
    cmd.split(' ').next().unwrap_or(cmd).to_string()
}

fn opencode_plugin(binary: &str) -> String {
    let bin = serde_json::to_string(binary).unwrap_or_else(|_| format!("\"{}\"", binary));
    let template = r##"// AgentPet integration (auto-generated, safe to delete to uninstall).
// OpenCode V2 plugin: a default-exported definition with { id, setup }.
// V1 plugin modules (named exports returning a hooks object) are rejected by
// the V2 loader, so this file targets V2 only.
import { spawn } from "node:child_process"

const AGENTPET_BIN = __BIN__

function getString(value) {
  return typeof value === "string" && value.length > 0 ? value : ""
}
function propsOf(event) {
  return (event && (event.properties || event.data)) || {}
}
function extractSessionID(event) {
  const p = propsOf(event)
  return (
    getString(p.info && p.info.id) ||
    getString(p.sessionID) || getString(p.sessionId) || getString(p.session_id) ||
    getString(p.id) ||
    getString(event && event.sessionID) || getString(event && event.sessionId) ||
    getString(event && event.session_id) ||
    getString(event && event.session && event.session.id) ||
    getString(event && event.id)
  )
}

function modelName(model) {
  if (!model) return ""
  if (typeof model === "string") return model
  return (
    getString(model.modelID) ||
    getString(model.id) ||
    getString(model.name) ||
    getString(model.displayName) ||
    ""
  )
}

export default {
  id: "agentpet",
  async setup(ctx) {
    // `ctx.location.directory` is the process cwd captured once per plugin
    // instance, so a session opened in another project was labelled with the
    // server's directory (e.g. "rutting-laser"). Prefer the directory carried
    // by each event's session info, remember it per session, and only fall back
    // to the setup value when it looks like a real path.
    const setupDir = (ctx && ctx.location && ctx.location.directory) || ""
    const looksLikePath = (s) => s.indexOf("/") >= 0 || s.indexOf("\\") >= 0
    const baseDir = looksLikePath(setupDir) ? setupDir : ""
    const roles = new Map()
    const models = new Map()
    const dirs = new Map()
    const extractDirectory = (event) => {
      const p = propsOf(event)
      const info = p.info || {}
      return (
        getString(info.location && info.location.directory) ||
        getString(info.directory) ||
        getString(p.location && p.location.directory) ||
        getString(p.directory) ||
        getString(p.cwd) ||
        getString(event && event.location && event.location.directory) ||
        getString(event && event.directory)
      )
    }
    const projectFor = (event, sid) => {
      const found = extractDirectory(event)
      if (found) dirs.set(sid, found)
      return dirs.get(sid) || found || baseDir
    }
    // The model can arrive on session.model.selected, but also on session.updated
    // / message info (info.model / modelID). Read it from any event so a session
    // that started before this plugin loaded still reports its model.
    const extractModel = (event) => {
      const p = propsOf(event)
      const info = p.info || {}
      return (
        modelName(p.model) ||
        modelName(info.model) ||
        getString(p.modelID) ||
        getString(info.modelID) ||
        ""
      )
    }
    const send = (state, sid, tokens, cost, event) => {
      try {
        const project = projectFor(event, sid)
        const args = ["hook", "--agent", "opencode", "--event", state, "--session", sid, "--project", project]
        const role = roles.get(sid)
        if (role) args.push("--role", role)
        const model = models.get(sid)
        if (model) args.push("--model", model)
        if (tokens > 0) args.push("--tokens", String(tokens))
        if (cost > 0) args.push("--cost", String(cost))
        const child = spawn(AGENTPET_BIN, args,
          { stdio: "ignore", detached: true, windowsHide: true })
        child.on("error", () => {})
        if (child.unref) child.unref()
      } catch (e) {}
    }
    const sidFor = (event) => "opencode:" + (extractSessionID(event) || baseDir || "default")

    if (ctx && ctx.tool && ctx.tool.hook) {
      await ctx.tool.hook("execute.before", (event) => send("working", sidFor(event), 0, 0, event))
    }

    if (ctx && ctx.event && ctx.event.subscribe) {
      const controller = new AbortController()
      void (async () => {
        try {
          for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
            const type = (event && event.type) || ""
            const sid = sidFor(event)
            const mdl = extractModel(event)
            if (mdl) models.set(sid, mdl)
            // OpenCode V2 event names. V1 names (session.status /
            // session.idle / session.updated) are no longer emitted, so the
            // turn end comes from session.execution.*. Remember the driving
            // agent/role so state reports carry it.
            if (type === "session.agent.selected") {
              const role = (propsOf(event).agent) || ""
              if (role) roles.set(sid, role)
            } else if (type === "permission.asked" || type === "session.permission.create") {
              send("waiting", sid, 0, 0, event)
            } else if (type === "session.execution.succeeded" ||
                       type === "session.execution.failed" ||
                       type === "session.execution.interrupted" ||
                       type === "session.deleted") {
              send("done", sid, 0, 0, event)
            } else if (type === "session.execution.started" ||
                       type === "session.tool.called" ||
                       type === "session.tool.input.started") {
              send("working", sid, 0, 0, event)
            } else if (type === "session.usage.updated") {
              // Cumulative usage; the server turns it into a delta and de-dupes.
              // Pet XP counts input + output, not cache-read.
              const p = propsOf(event)
              const t = p.tokens || {}
              const total = (t.input || 0) + (t.output || 0)
              const cost = typeof p.cost === "number" ? p.cost : 0
              if (total > 0 || cost > 0) send("usage", sid, total, cost, event)
            }
          }
        } catch (e) {}
      })()
      return () => controller.abort()
    }
  },
}
"##;
    template.replace("__BIN__", &bin)
}

/// The Pi extension: reports session lifecycle through the `agentpet hook` CLI.
fn pi_extension(binary: &str) -> String {
    let bin = serde_json::to_string(binary).unwrap_or_else(|_| format!("\"{}\"", binary));
    format!(
        "// AgentPet integration (auto-generated, safe to delete to uninstall).\n\
         // Reports Pi session lifecycle to AgentPet's menu bar app.\n\
         import {{ spawn }} from \"node:child_process\"\n\
         const AGENTPET_BIN = {bin}\n\
         export default function (pi) {{\n\
         \x20 const send = (state, ctx) => {{\n\
         \x20   try {{\n\
         \x20     const cwd = (ctx && ctx.cwd) || process.cwd()\n\
         \x20     const file = ctx && ctx.sessionManager && ctx.sessionManager.getSessionFile ? ctx.sessionManager.getSessionFile() : null\n\
         \x20     const sid = \"pi:\" + (file || cwd)\n\
         \x20     const p = spawn(AGENTPET_BIN, [\"hook\", \"--agent\", \"pi\", \"--event\", state, \"--session\", sid, \"--project\", cwd], {{ stdio: \"ignore\" }})\n\
         \x20     if (p && p.unref) p.unref()\n\
         \x20   }} catch (e) {{}}\n\
         \x20 }}\n\
         \x20 pi.on(\"session_start\", async (_e, ctx) => send(\"registered\", ctx))\n\
         \x20 pi.on(\"agent_start\", async (_e, ctx) => send(\"working\", ctx))\n\
         \x20 pi.on(\"agent_end\", async (_e, ctx) => send(\"done\", ctx))\n\
         \x20 pi.on(\"session_shutdown\", async (_e, ctx) => send(\"done\", ctx))\n\
         }}\n"
    )
}

/// Ensure `[features] hooks = true` in ~/.codex/config.toml (modern key; the
/// `codex_hooks` alias is ignored by recent Codex).
fn enable_codex_hooks() {
    let Some(home) = dirs::home_dir() else { return };
    let path = home.join(".codex").join("config.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let already = text.lines().any(|l| {
        let c = l.trim().replace(' ', "");
        !c.starts_with('#') && c.starts_with("hooks=true")
    });
    if already { return; }
    let updated = if let Some(idx) = text.lines().position(|l| l.trim() == "[features]") {
        let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        lines.insert(idx + 1, "hooks = true".into());
        lines.join("\n")
    } else {
        let mut t = text;
        if !t.is_empty() && !t.ends_with('\n') { t.push('\n'); }
        t.push_str("\n[features]\nhooks = true\n");
        t
    };
    if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
    let _ = std::fs::write(&path, updated);
}

// ----- jcode (TOML [hooks], usually inside WSL) -----------------------------

/// Where jcode's config lives and what hook command jcode should run.
struct JcodeTarget {
    config: PathBuf,
    command: String,
}

/// Prefer the default WSL distro's `~/.jcode` (jcode is a Linux/macOS CLI and on
/// Windows it typically runs under WSL), else a native `%USERPROFILE%\.jcode`.
/// simplify: default distro only; a per-distro picker is the upgrade path.
fn jcode_target() -> Option<JcodeTarget> {
    let exe = std::env::current_exe().ok()?;
    let (home, via_wsl) = jcode_home()?;
    let config = home.join(".jcode").join("config.toml");
    let command = if via_wsl {
        crate::jcode::hook_command(&windows_to_wsl_path(&exe.to_string_lossy()), true)
    } else {
        crate::jcode::hook_command(&exe.to_string_lossy(), false)
    };
    Some(JcodeTarget { config, command })
}

/// `C:\Users\A\x.exe` -> `/mnt/c/Users/A/x.exe` (WSL's default automount root).
/// simplify: assumes automount root /mnt; a custom root in /etc/wsl.conf would
/// need `wslpath -u` instead.
pub fn windows_to_wsl_path(p: &str) -> String {
    let p = p.trim_start_matches(r"\\?\");
    let mut chars = p.chars();
    match (chars.next(), chars.next()) {
        (Some(d), Some(':')) if d.is_ascii_alphabetic() => {
            format!("/mnt/{}{}", d.to_ascii_lowercase(), p[2..].replace('\\', "/"))
        }
        _ => p.replace('\\', "/"),
    }
}

fn jcode_write(path: &PathBuf, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    // Write-then-rename: jcode re-reads config.toml on change, never a partial one.
    let tmp = path.with_extension("toml.agentpet-tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Reads config.toml. A missing file is an empty config (first install); ANY
/// other error (a `\\wsl.localhost` hiccup while the VM starts, a permission
/// error, non-UTF-8 content) is returned so the caller aborts. Treating those as
/// "empty" would make install rewrite the file with only the `[hooks]` block and
/// wipe the user's jcode config.
fn read_config(path: &PathBuf) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

fn jcode_install() -> std::io::Result<()> {
    // Enabling jcode is the one moment we look for it inside WSL (see the note on
    // jcode_home): opt-in, with a timeout, and we are already off the UI thread.
    probe_jcode_wsl();
    let t = jcode_target().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "jcode config not found"))?;
    let existing = read_config(&t.config)?;
    let updated = crate::jcode::install(&existing, &t.command, crate::jcode::EVENTS)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    if updated != existing { jcode_write(&t.config, &updated)?; }
    Ok(())
}

fn jcode_uninstall() -> std::io::Result<()> {
    // No probe here: uninstall edits the config where install put it (the cached
    // WSL home). If WSL is down the read fails and we abort instead of editing
    // the native config and leaving the WSL hooks behind.
    let Some(t) = jcode_target() else { return Ok(()) };
    let existing = read_config(&t.config)?;
    let updated = crate::jcode::uninstall(&existing, crate::jcode::EVENTS);
    if updated != existing { jcode_write(&t.config, &updated)?; }
    Ok(())
}

#[cfg(test)]
mod jcode_path_tests {
    use super::windows_to_wsl_path;

    #[test]
    fn converts_drive_paths() {
        assert_eq!(windows_to_wsl_path(r"C:\Users\HocSys\AppData\Local\AgentPet\agentpet.exe"),
                   "/mnt/c/Users/HocSys/AppData/Local/AgentPet/agentpet.exe");
        assert_eq!(windows_to_wsl_path(r"\\?\D:\A B\x.exe"), "/mnt/d/A B/x.exe");
    }
}

// ----- jcode home (shared by limits and the jcode hooks) --------------------
//
// jcode usually runs inside WSL, so its home is `\\wsl.localhost\<distro>\<home>`.
// Finding it needs `wsl.exe`, which boots the WSL VM (vmmem) when it is off. So:
//   * `jcode_home()` NEVER spawns anything: it answers from a cache (memory,
//     seeded from a small marker file) and falls back to the native home.
//   * `probe_jcode_wsl()` is the only place that spawns `wsl.exe`. It has a
//     timeout and is meant to run only once jcode is known to be in use (the
//     user enables jcode in Settings > Agents), always off the UI thread. A hung
//     `wsl.exe` (broken distro, first-run prompt) can therefore never block a
//     caller of `jcode_home()`.
// Claude-only users, with or without WSL, never trigger a probe.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long `wsl.exe` gets before it is killed. Booting a cold VM takes a few seconds.
#[cfg_attr(not(windows), allow(dead_code))]
const WSL_PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// `%APPDATA%\AgentPet\jcode_wsl_home`: one line, the UNC home of the WSL distro
/// where jcode lives. Written by a successful probe so later launches know
/// without spawning `wsl.exe`. Delete the file to forget it.
fn wsl_marker() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("AgentPet").join("jcode_wsl_home"))
}

fn wsl_cache() -> &'static Mutex<Option<PathBuf>> {
    static C: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    C.get_or_init(|| {
        let saved = wsl_marker()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        Mutex::new(saved)
    })
}

/// The home directory jcode uses, as a path this process can open, plus whether
/// it is inside WSL. Cache lookup only (see the block comment above): the WSL
/// home is known after `probe_jcode_wsl` found one, else the native home.
pub fn jcode_home() -> Option<(PathBuf, bool)> {
    if let Some(wsl) = wsl_cache().lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return Some((wsl, true));
    }
    dirs::home_dir().map(|h| (h, false))
}

/// Looks for jcode in the default WSL distro and remembers the answer (also
/// forgets a stale one). BLOCKS up to `WSL_PROBE_TIMEOUT` plus a UNC lookup, so
/// call it from a worker (`spawn_blocking` / a thread), never from the UI thread.
pub fn probe_jcode_wsl() {
    #[cfg(windows)]
    {
        let found = wsl_default_home().and_then(|(distro, home)| {
            let unc = PathBuf::from(format!(r"\\wsl.localhost\{}{}", distro, home.replace('/', r"\")));
            unc.join(".jcode").is_dir().then_some(unc)
        });
        *wsl_cache().lock().unwrap_or_else(|e| e.into_inner()) = found.clone();
        if let Some(m) = wsl_marker() {
            match found {
                Some(unc) => {
                    if let Some(dir) = m.parent() { let _ = std::fs::create_dir_all(dir); }
                    let _ = std::fs::write(m, unc.to_string_lossy().as_bytes());
                }
                None => { let _ = std::fs::remove_file(m); }
            }
        }
    }
}

/// Runs `cmd` with stdout captured and kills it after `timeout`. `None` on a
/// spawn error, a timeout, or a non-zero exit. Std only, so it is unit-tested.
#[cfg_attr(not(windows), allow(dead_code))]
fn run_with_timeout(mut cmd: std::process::Command, timeout: Duration) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    // Read on a thread so a full pipe can't stall the child while we poll it.
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(40)),
            // Timed out (or wait failed): kill it. The reader thread ends by
            // itself once the pipe closes, so it is not joined here.
            _ => { let _ = child.kill(); let _ = child.wait(); return None; }
        }
    };
    if !status.success() { return None; }
    reader.join().ok()
}

/// (distro, $HOME) of the default WSL distro, via `wsl.exe`. None without WSL
/// or when it does not answer in time.
#[cfg(windows)]
fn wsl_default_home() -> Option<(String, String)> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new("wsl.exe");
    cmd.args(["-e", "sh", "-c", "printf '%s\\n%s' \"$WSL_DISTRO_NAME\" \"$HOME\""])
        .creation_flags(CREATE_NO_WINDOW);
    let out = run_with_timeout(cmd, WSL_PROBE_TIMEOUT)?;
    let text = String::from_utf8_lossy(&out).replace('\r', "");
    let mut it = text.lines();
    let distro = it.next()?.trim().to_string();
    let home = it.next()?.trim().to_string();
    (!distro.is_empty() && home.starts_with('/')).then_some((distro, home))
}

#[cfg(test)]
mod read_config_tests {
    use super::*;

    #[test]
    fn missing_file_is_empty_but_other_errors_abort() {
        let dir = std::env::temp_dir().join(format!("ap-readcfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_config(&dir.join("nope.toml")).unwrap(), "");
        // Non-UTF-8 content must be an error, never silently "empty".
        let bad = dir.join("bad.toml");
        std::fs::write(&bad, [0xff, 0xfe, 0x00]).unwrap();
        assert!(read_config(&bad).is_err());
        // A directory where a file is expected is not NotFound either.
        assert!(read_config(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Hermes Agent (Nous Research) integration. Unlike the other agents Hermes
/// keeps a YAML config per profile and refuses to run a shell hook that is not
/// in its per-home consent allowlist, so install covers every home and approves
/// the hook there too. `config.yaml` is user-owned and comment-heavy, so it is
/// edited line-wise (same approach as jcode's TOML) and foreign hooks are kept.
mod hermes {
    use super::is_ours;
    use std::sync::atomic::{AtomicU64, Ordering};
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};

    pub const ALLOWLIST: &str = "shell-hooks-allowlist.json";

    static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

    pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config.toml".into());
        let tmp = path.with_file_name(format!(".{name}.agentpet-{}-{seq}.tmp", std::process::id()));
        if let Err(e) = std::fs::write(&tmp, contents.as_bytes()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        match replace_file(&tmp, path) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
            const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
            extern "system" {
                fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
            }
            let from_w: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
            let to_w: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
            let ok = unsafe {
                MoveFileExW(from_w.as_ptr(), to_w.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
            };
            if ok == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
        #[cfg(not(windows))]
        {
            std::fs::rename(from, to)
        }
    }

    /// Hermes home: `HERMES_HOME` override, else `%LOCALAPPDATA%\hermes`
    /// (`~/.hermes` on macOS/Linux).
    pub fn root() -> Option<PathBuf> {
        if let Ok(home) = std::env::var("HERMES_HOME") {
            let home = home.trim();
            if !home.is_empty() { return Some(PathBuf::from(home)); }
        }
        dirs::data_local_dir().map(|p| p.join("hermes"))
    }

    /// Base home first, then every profile home that already has a config file.
    /// Profiles without one are left alone rather than created.
    pub fn config_paths() -> Vec<PathBuf> {
        let Some(root) = root() else { return Vec::new() };
        let mut paths = vec![root.join("config.yaml")];
        let mut profiles: Vec<PathBuf> = std::fs::read_dir(root.join("profiles"))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_dir() && p.join("config.yaml").is_file())
                    .collect()
            })
            .unwrap_or_default();
        profiles.sort();
        paths.extend(profiles.into_iter().map(|p| p.join("config.yaml")));
        paths
    }

    // ----- config.yaml `hooks:` block -------------------------------------

    fn strip_cr(line: &str) -> &str { line.strip_suffix('\r').unwrap_or(line) }

    /// Top-level `hooks:` with no inline value (a value on the same line is a
    /// representation we do not edit).
    fn is_hooks_header(line: &str) -> bool {
        if line.starts_with(' ') || line.starts_with('\t') { return false; }
        let t = strip_cr(line).trim_end();
        t.split('#').next().unwrap_or("").trim_end() == "hooks:"
    }

    /// `hooks: {...}` / `hooks: null` on one line.
    fn inline_value(line: &str) -> Option<String> {
        if line.starts_with(' ') || line.starts_with('\t') { return None; }
        let t = strip_cr(line).trim_end();
        let rest = t.strip_prefix("hooks:")?.trim_start();
        if rest.is_empty() || rest.starts_with('#') { None } else { Some(rest.to_string()) }
    }

    fn hooks_block(lines: &[String]) -> Option<(usize, usize)> {
        let header = lines.iter().position(|l| is_hooks_header(l))?;
        let mut end = header + 1;
        while end < lines.len() {
            let t = strip_cr(&lines[end]);
            let trimmed = t.trim();
            let indented = t.starts_with(' ') || t.starts_with('\t');
            if !trimmed.is_empty() && !trimmed.starts_with('#') && !indented { break; }
            end += 1;
        }
        Some((header, end))
    }

    fn child_indent(lines: &[String], header: usize, end: usize) -> String {
        for line in &lines[header + 1..end] {
            let t = strip_cr(line);
            let trimmed = t.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') { continue; }
            let lead = &t[..t.len() - trimmed.len()];
            if !lead.is_empty() { return lead.to_string(); }
        }
        "  ".to_string()
    }

    fn is_event_key(line: &str, indent: &str, event: &str) -> bool {
        let t = strip_cr(line);
        let Some(rest) = t.strip_prefix(indent) else { return false };
        if rest.starts_with(' ') || rest.starts_with('\t') { return false; }
        let Some(after) = rest.strip_prefix(event) else { return false };
        let Some(value) = after.strip_prefix(':') else { return false };
        value.is_empty()
            || value.starts_with(' ')
            || value.starts_with('\t')
            || value.starts_with('#')
            || value.starts_with('[')
    }

    fn event_value(line: &str, indent: &str, event: &str) -> String {
        let t = strip_cr(line);
        t[indent.len() + event.len() + 1..].trim().to_string()
    }

    /// First line after a key that is not indented deeper than it.
    fn list_end(lines: &[String], key: usize, indent: &str) -> usize {
        let mut i = key + 1;
        while i < lines.len() {
            let t = strip_cr(&lines[i]);
            if t.trim().is_empty() { i += 1; continue; }
            let deeper = t.strip_prefix(indent)
                .map(|rest| rest.starts_with(' ') || rest.starts_with('\t'))
                .unwrap_or(false);
            if !deeper { break; }
            i += 1;
        }
        i
    }

    /// The command a list item runs, without YAML quoting or a trailing comment.
    fn item_command(line: &str) -> Option<String> {
        let t = strip_cr(line).trim_start().strip_prefix('-')?.trim_start();
        let value = t.strip_prefix("command:")?.trim_start();
        let unquoted = if let Some(rest) = value.strip_prefix('\'') {
            rest.split('\'').next().unwrap_or("")
        } else if let Some(rest) = value.strip_prefix('"') {
            rest.split('"').next().unwrap_or("")
        } else {
            value.split('#').next().unwrap_or("").trim()
        };
        Some(unquoted.trim().to_string())
    }

    /// A list item that runs AgentPet's hook (a comment merely mentioning it is
    /// not ours, and a foreign command is never overwritten).
    fn ours_item(line: &str) -> bool {
        item_command(line).map(|c| is_ours(&c)).unwrap_or(false)
    }

    fn yaml_string(command: &str) -> String {
        if !command.contains('\'') && !command.contains('\n') {
            format!("'{command}'")
        } else {
            let escaped = command.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
            format!("\"{escaped}\"")
        }
    }

    pub fn is_installed(text: &str, _events: &[&str]) -> bool {
        let lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        let Some((header, end)) = hooks_block(&lines) else { return false };
        (header + 1..end).any(|i| ours_item(&lines[i]))
    }

    fn create_block(lines: &mut Vec<String>, eol: &str, had_trailing_nl: bool) {
        if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            lines.pop();
        } else if eol == "\r" {
            if let Some(last) = lines.last_mut() {
                if !last.ends_with('\r') { last.push('\r'); }
            }
        }
        if !lines.is_empty() { lines.push(eol.to_string()); }
        lines.push(format!("hooks:{eol}"));
        if had_trailing_nl { lines.push(String::new()); }
    }

    fn install_event(
        lines: &mut Vec<String>,
        event: &str,
        command: &str,
        eol: &str,
        path: &str,
    ) -> Result<(), String> {
        let (header, end) = hooks_block(lines).ok_or_else(|| format!("{path}: no hooks block"))?;
        let indent = child_indent(lines, header, end);
        let item = format!("{indent}  - command: {}{eol}", yaml_string(command));
        if let Some(key) = (header + 1..end).find(|&i| is_event_key(&lines[i], &indent, event)) {
            let raw = event_value(&lines[key], &indent, event);
            let value = raw.split('#').next().unwrap_or("").trim().to_string();
            if value.is_empty() {
                let stop = list_end(lines, key, &indent);
                if let Some(i) = (key + 1..stop).find(|&i| ours_item(&lines[i])) {
                    lines[i] = item;
                } else {
                    let at = (key + 1..stop)
                        .rev()
                        .find(|&i| !strip_cr(&lines[i]).trim().is_empty())
                        .map(|i| i + 1)
                        .unwrap_or(key + 1);
                    lines.insert(at, item);
                }
                return Ok(());
            }
            if value == "[]" {
                lines[key] = format!("{indent}{event}:{eol}");
                lines.insert(key + 1, item);
                return Ok(());
            }
            return Err(format!(
                "{path} already sets hooks.{event} inline; remove that inline value and retry."
            ));
        }
        let mut at = end;
        while at > header + 1 && strip_cr(&lines[at - 1]).trim().is_empty() { at -= 1; }
        lines.insert(at, format!("{indent}{event}:{eol}"));
        lines.insert(at + 1, item);
        Ok(())
    }

    pub fn install(text: &str, command: &str, events: &[&str], path: &str) -> Result<String, String> {
        if text.lines().any(|l| inline_value(l).is_some()) {
            return Err(format!(
                "{path} has an inline `hooks:` value; AgentPet does not rewrite it."
            ));
        }
        let crlf = text.contains("\r\n");
        let eol = if crlf { "\r" } else { "" };
        let had_trailing_nl = text.ends_with('\n');
        let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        if hooks_block(&lines).is_none() {
            create_block(&mut lines, eol, had_trailing_nl);
        }
        for event in events {
            install_event(&mut lines, event, command, eol, path)?;
        }
        Ok(join_preserving_final_newline(lines, had_trailing_nl))
    }

    fn uninstall_event(lines: &mut Vec<String>, event: &str) -> bool {
        let Some((header, end)) = hooks_block(lines) else { return false };
        let indent = child_indent(lines, header, end);
        let Some(key) = (header + 1..end).find(|&i| is_event_key(&lines[i], &indent, event)) else {
            return false;
        };
        let raw = event_value(&lines[key], &indent, event);
        let value = raw.split('#').next().unwrap_or("").trim().to_string();
        if !value.is_empty() && value != "[]" { return false; }
        let stop = list_end(lines, key, &indent);
        let mut removed = false;
        let mut i = stop;
        while i > key + 1 {
            i -= 1;
            if ours_item(&lines[i]) {
                lines.remove(i);
                removed = true;
            }
        }
        if !removed { return false; }
        // Drop the event key when nothing is listed under it any more.
        let Some((header, end)) = hooks_block(lines) else { return removed };
        let indent = child_indent(lines, header, end);
        if let Some(key) = (header + 1..end).find(|&i| is_event_key(&lines[i], &indent, event)) {
            let stop = list_end(lines, key, &indent);
            let has_item = (key + 1..stop).any(|i| strip_cr(&lines[i]).trim_start().starts_with('-'));
            if !has_item { lines.remove(key); }
        }
        removed
    }

    pub fn uninstall(text: &str, events: &[&str]) -> String {
        let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        if hooks_block(&lines).is_none() { return text.to_string(); }
        let mut removed = false;
        for event in events {
            removed |= uninstall_event(&mut lines, event);
        }
        // An emptied block goes away entirely, along with the separator line we
        // added before it, so removal restores the user's file.
        if removed {
            if let Some((header, end)) = hooks_block(&lines) {
                let empty = lines[header + 1..end].iter().all(|l| {
                    let t = strip_cr(l);
                    let trimmed = t.trim();
                    trimmed.is_empty() || trimmed.starts_with('#')
                });
                if empty {
                    lines.remove(header);
                    if header > 0 && strip_cr(&lines[header - 1]).trim().is_empty() {
                        lines.remove(header - 1);
                    }
                }
            }
        }
        lines.join("\n")
    }

    fn join_preserving_final_newline(mut lines: Vec<String>, had_trailing_nl: bool) -> String {
        if !had_trailing_nl {
            if let Some(last) = lines.last_mut() {
                if last.ends_with('\r') { last.pop(); }
            }
        }
        lines.join("\n")
    }

    fn install_to_disk(command: &str, path: &Path, events: &[&str]) -> std::io::Result<()> {
        let existing = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let updated = install(&existing, command, events, &path.display().to_string())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        if updated != existing { write_atomic(path, &updated)?; }
        Ok(())
    }

    pub fn install_all(paths: &[PathBuf], command: &str, events: &[&str]) -> std::io::Result<()> {
        let mut first_err: Option<String> = None;
        for path in paths {
            match install_to_disk(command, path, events) {
                Ok(()) => {
                    if let Some(home) = path.parent() {
                        if let Err(e) = approve(&allowlist_path(home), events, command) {
                            first_err.get_or_insert(e);
                        }
                    }
                }
                Err(e) => { first_err.get_or_insert(e.to_string()); }
            }
        }
        match first_err {
            Some(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
            None => Ok(()),
        }
    }

    pub fn uninstall_all(paths: &[PathBuf], _command: &str, events: &[&str]) -> std::io::Result<()> {
        let mut first_err: Option<String> = None;
        for path in paths {
            let existing = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => { first_err.get_or_insert(e.to_string()); continue; }
            };
            let updated = uninstall(&existing, events);
            if updated != existing {
                if let Err(e) = write_atomic(path, &updated) {
                    first_err.get_or_insert(e.to_string());
                    continue;
                }
            }
            if let Some(home) = path.parent() {
                if let Err(e) = revoke(&allowlist_path(home)) {
                    first_err.get_or_insert(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
            None => Ok(()),
        }
    }

    // ----- consent allowlist ----------------------------------------------

    pub fn allowlist_path(home: &Path) -> PathBuf { home.join(ALLOWLIST) }

    fn read_allowlist(path: &Path) -> Result<Value, String> {
        match std::fs::read_to_string(path) {
            Ok(s) if s.trim().is_empty() => Ok(json!({ "approvals": [] })),
            Ok(s) => serde_json::from_str(&s).map_err(|e| {
                format!("{} is not valid JSON ({}); fix or remove it and retry", path.display(), e)
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({ "approvals": [] })),
            Err(e) => Err(e.to_string()),
        }
    }

    fn with_approvals<F>(path: &Path, edit: F) -> Result<(), String>
    where F: FnOnce(&mut Vec<Value>) -> Result<(), String> {
        let mut data = read_allowlist(path)?;
        let obj = data.as_object_mut()
            .ok_or_else(|| format!("{} is not a JSON object; fix or remove it and retry", path.display()))?;
        let approvals = obj.entry("approvals").or_insert_with(|| json!([]));
        let arr = approvals.as_array_mut()
            .ok_or_else(|| format!("{}: approvals is not an array", path.display()))?;
        edit(arr)?;
        let text = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
        write_atomic(path, &text).map_err(|e| e.to_string())
    }

    fn approval_is_ours(entry: &Value) -> bool {
        entry.get("command").and_then(|c| c.as_str()).map(is_ours).unwrap_or(false)
    }

    /// Record consent for every event. Hermes skips (and never prompts for) a
    /// hook that is not allowlisted in a non-interactive run, so installing the
    /// config alone would look like a broken pet.
    pub fn approve(path: &Path, events: &[&str], command: &str) -> Result<(), String> {
        let now = iso8601_now();
        with_approvals(path, |arr| {
            arr.retain(|e| !approval_is_ours(e));
            for event in events {
                arr.push(json!({
                    "event": event,
                    "command": command,
                    "approved_at": now,
                    // No script mtime: Hermes only checks drift when this is set,
                    // and agentpet.exe is replaced by every update.
                    "script_mtime_at_approval": Value::Null,
                }));
            }
            Ok(())
        })
    }

    pub fn revoke(path: &Path) -> Result<(), String> {
        if !path.is_file() { return Ok(()); }
        with_approvals(path, |arr| {
            arr.retain(|e| !approval_is_ours(e));
            Ok(())
        })
    }

    /// ISO-8601 UTC, matching the timestamp Hermes records itself.
    fn iso8601_now() -> String {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as i64;
        let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
        let rem = secs.rem_euclid(86_400);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            rem / 3600,
            (rem % 3600) / 60,
            rem % 60
        )
    }

    /// Howard Hinnant's days-from-civil, inverted.
    pub(super) fn civil_from_days(days: i64) -> (i64, u32, u32) {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
        (year, month, day)
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn returns_output_of_a_quick_command() {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "printf hi"]);
        assert_eq!(run_with_timeout(c, Duration::from_secs(5)).as_deref(), Some(&b"hi"[..]));
    }

    #[cfg(unix)]
    #[test]
    fn kills_a_hung_command() {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "sleep 30"]);
        let t = Instant::now();
        assert!(run_with_timeout(c, Duration::from_millis(300)).is_none());
        assert!(t.elapsed() < Duration::from_secs(5), "must not wait for the child");
    }

    #[cfg(unix)]
    #[test]
    fn non_zero_exit_is_none() {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "exit 3"]);
        assert!(run_with_timeout(c, Duration::from_secs(5)).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_registers_interrupt_and_session_end_next_to_stop() {
        let s = spec("codex").expect("codex spec");
        for event in ["Stop", "Interrupt", "SessionEnd", "SubagentStop"] {
            assert!(s.events.contains(&event), "codex must register {event}");
        }
        let mut seen = std::collections::HashSet::new();
        assert!(s.events.iter().all(|e| seen.insert(*e)), "no duplicate codex events");
        for event in s.events {
            let entry = make_entry(s.style, event, "\"/x/agentpet\" hook --agent codex");
            assert!(entry_is_ours(s.style, event, &entry), "{event} entry must be recognised for uninstall");
        }
    }

    #[test]
    fn opencode_plugin_is_v2_shape() {
        let js = opencode_plugin("C:\\bin\\agentpet.exe");
        // V2 loader requires a default-exported { id, setup } definition.
        assert!(js.contains("export default"), "must default-export");
        assert!(js.contains("id: \"agentpet\""), "must carry an id");
        assert!(js.contains("async setup(ctx)"), "must expose setup(ctx)");
        // V1's named export / Bun.spawn must be gone.
        assert!(!js.contains("export const AgentPet"), "must not use V1 shape");
        assert!(!js.contains("Bun.spawn"), "must not rely on Bun");
        assert!(js.contains("node:child_process"), "must import node spawn");
        // Binary path is embedded as a JS string literal.
        assert!(js.contains("C:\\\\bin\\\\agentpet.exe"), "binary path embedded");
    }

    #[test]
    fn opencode_plugin_uses_v2_events() {
        let js = opencode_plugin("/x/agentpet");
        assert!(js.contains("session.execution.succeeded"), "done on execution.succeeded");
        assert!(js.contains("session.execution.started"), "working on execution.started");
        assert!(js.contains("ctx.event.subscribe"), "subscribes to the event bus");
        assert!(js.contains("ctx.tool.hook"), "hooks tool execution");
        // V1 event names are no longer emitted by OpenCode V2.
        assert!(!js.contains("\"session.idle\""), "must not wait for session.idle");
        assert!(!js.contains("\"session.status\""), "must not wait for session.status");
    }

    #[test]
    fn opencode_installs_into_plugins_dir() {
        let spec = spec("opencode").expect("opencode spec");
        assert_eq!(
            spec.rel_path,
            &[".config", "opencode", "plugins", "agentpet.js"]
        );
    }

    const HERMES_CMD: &str = "\"C:\\Users\\me\\AppData\\Local\\Programs\\AgentPet\\agentpet.exe\" hook --agent hermes";
    const HERMES_EVENTS: &[&str] = &[
        "on_session_start", "pre_llm_call", "post_tool_call", "pre_approval_request",
        "on_human_input_request", "post_llm_call", "on_session_end",
    ];
    const HERMES_SAMPLE: &str = "# Hermes Agent CLI Configuration\nmodel: \"anthropic/claude\"\n\n# Gateway\nmultiplex_profiles: true\n";

    #[test]
    fn hermes_spec_uses_observer_events_only() {
        let s = spec("hermes").expect("hermes spec");
        assert!(matches!(s.style, Style::HermesYaml));
        assert!(!s.events.contains(&"pre_tool_call"), "pre_tool_call blocks tools");
        assert_eq!(s.events, HERMES_EVENTS);
        assert!(config_path("hermes").is_none(), "hermes has per-profile homes");
    }

    #[test]
    fn hermes_in_catalog() {
        let row = catalog().into_iter().find(|a| a.kind == "hermes").expect("hermes");
        assert_eq!(row.display_name, "Hermes Agent");
        assert!(row.note.unwrap_or_default().contains("hermes"));
    }

    #[test]
    fn hermes_install_creates_hooks_block_and_removes_it_again() {
        let out = hermes::install(HERMES_SAMPLE, HERMES_CMD, HERMES_EVENTS, "config.yaml").unwrap();
        let lines: Vec<&str> = out.split('\n').collect();
        let hooks = lines.iter().position(|l| *l == "hooks:").expect("hooks block");
        assert!(hooks > 3, "appended after the user's keys");
        for event in HERMES_EVENTS {
            let key = format!("  {event}:");
            let i = lines.iter().position(|l| *l == key.as_str()).expect(event);
            assert_eq!(lines[i + 1], format!("    - command: '{HERMES_CMD}'"));
        }
        assert!(hermes::is_installed(&out, HERMES_EVENTS));
        assert!(!hermes::is_installed(HERMES_SAMPLE, HERMES_EVENTS));
        assert_eq!(hermes::uninstall(&out, HERMES_EVENTS), HERMES_SAMPLE);
        assert!(!hermes::is_installed(&hermes::uninstall(&out, HERMES_EVENTS), HERMES_EVENTS));
    }

    #[test]
    fn hermes_install_is_idempotent_and_updates_the_binary_path() {
        let once = hermes::install(HERMES_SAMPLE, HERMES_CMD, HERMES_EVENTS, "config.yaml").unwrap();
        let twice = hermes::install(&once, HERMES_CMD, HERMES_EVENTS, "config.yaml").unwrap();
        assert_eq!(twice, once);
        let moved = hermes::install(
            &once,
            "\"D:\\Program Files\\AgentPet\\agentpet.exe\" hook --agent hermes",
            HERMES_EVENTS,
            "config.yaml",
        )
        .unwrap();
        assert_eq!(moved.split('\n').count(), once.split('\n').count());
        assert!(moved.contains(r"D:\Program Files\AgentPet\agentpet.exe"));
        assert!(!moved.contains(r"Users\me"));
    }

    #[test]
    fn hermes_keeps_foreign_hooks_and_appends_to_a_shared_event() {
        let src = "hooks:\n  post_tool_call:\n    - command: 'notify-send done'\n";
        let out = hermes::install(src, HERMES_CMD, HERMES_EVENTS, "config.yaml").unwrap();
        assert!(out.contains("    - command: 'notify-send done'"), "{out}");
        assert!(out.contains(&format!("    - command: '{HERMES_CMD}'")));
        // The foreign item survives removal, its key stays, our item goes.
        let removed = hermes::uninstall(&out, HERMES_EVENTS);
        assert_eq!(
            removed,
            "hooks:\n  post_tool_call:\n    - command: 'notify-send done'\n"
        );
    }

    #[test]
    fn hermes_expands_an_inline_empty_list() {
        let src = "hooks:\n  post_tool_call: []\n";
        let out = hermes::install(src, HERMES_CMD, &["post_tool_call"], "config.yaml").unwrap();
        assert!(out.contains("  post_tool_call:\n    - command: "), "{out}");
        // An emptied block is dropped, so the empty list it replaced is gone too.
        assert_eq!(hermes::uninstall(&out, &["post_tool_call"]), "");
    }

    #[test]
    fn hermes_inline_mapping_is_refused_not_overwritten() {
        let src = "hooks: {post_tool_call: [{command: 'x'}]}\n";
        let err = hermes::install(src, HERMES_CMD, HERMES_EVENTS, "config.yaml").unwrap_err();
        assert!(err.contains("inline `hooks:` value"), "{err}");
        assert_eq!(hermes::uninstall(src, HERMES_EVENTS), src);
        let flow_event = "hooks:\n  post_tool_call: [{command: 'x'}]\n";
        let err = hermes::install(flow_event, HERMES_CMD, HERMES_EVENTS, "config.yaml").unwrap_err();
        assert!(err.contains("already sets hooks.post_tool_call inline"), "{err}");
    }

    #[test]
    fn hermes_preserves_crlf_and_trailing_newline() {
        let src = "model: x\r\n";
        let out = hermes::install(src, HERMES_CMD, &["post_llm_call"], "config.yaml").unwrap();
        assert!(out.contains("hooks:\r\n"), "{out:?}");
        assert!(out.contains("  post_llm_call:\r\n"), "{out:?}");
        assert!(out.ends_with("\r\n"), "{out:?}");
        assert_eq!(hermes::uninstall(&out, &["post_llm_call"]), src);
    }

    #[test]
    fn hermes_comment_mentioning_agentpet_is_not_our_item() {
        let src = "hooks:\n  post_llm_call:\n    - command: 'notify'  # agentpet hook\n";
        let out = hermes::install(src, HERMES_CMD, &["post_llm_call"], "config.yaml").unwrap();
        assert_eq!(out.matches("- command: 'notify'").count(), 1);
        let removed = hermes::uninstall(&out, &["post_llm_call"]);
        assert_eq!(removed, src);
    }

    #[test]
    fn hermes_allowlist_approve_and_revoke() {
        let dir = std::env::temp_dir().join(format!(
            "agentpet-hermes-allowlist-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = hermes::allowlist_path(&dir);
        std::fs::write(&path, r#"{"approvals":[{"event":"other","command":"~/bin/notify-send"}]}"#).unwrap();
        hermes::approve(&path, &["pre_llm_call", "post_llm_call"], HERMES_CMD).unwrap();
        let data: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let approvals = data.get("approvals").and_then(|a| a.as_array()).unwrap();
        assert_eq!(approvals.len(), 3, "foreign approval kept");
        assert!(approvals.iter().any(|e| e["command"] == "~/bin/notify-send"));
        let ours = approvals.iter().find(|e| e["event"] == "pre_llm_call").unwrap();
        assert_eq!(ours["command"], HERMES_CMD);
        assert!(ours["approved_at"].as_str().unwrap().ends_with('Z'));
        // Re-approving refreshes instead of duplicating.
        hermes::approve(&path, &["pre_llm_call"], HERMES_CMD).unwrap();
        let data: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(data["approvals"].as_array().unwrap().len(), 2);
        hermes::revoke(&path).unwrap();
        let data: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let approvals = data["approvals"].as_array().unwrap();
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0]["command"], "~/bin/notify-send");
        // Revoking when the file is gone is a no-op, not an error.
        std::fs::remove_file(&path).unwrap();
        hermes::revoke(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hermes_install_leaves_malformed_allowlist_alone() {
        let dir = std::env::temp_dir().join(format!("agentpet-hermes-bad-allowlist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = hermes::allowlist_path(&dir);
        std::fs::write(&path, "{not json").unwrap();
        assert!(hermes::approve(&path, &["pre_llm_call"], HERMES_CMD).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hermes_config_paths_include_every_profile_with_a_config() {
        let root = std::env::temp_dir().join(format!(
            "agentpet-hermes-homes-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(root.join("profiles").join("forge")).unwrap();
        std::fs::create_dir_all(root.join("profiles").join("empty")).unwrap();
        std::fs::write(root.join("profiles").join("forge").join("config.yaml"), "model: x\n").unwrap();
        let previous = std::env::var("HERMES_HOME").ok();
        std::env::set_var("HERMES_HOME", &root);
        let paths = hermes::config_paths();
        match previous {
            Some(v) => std::env::set_var("HERMES_HOME", v),
            None => std::env::remove_var("HERMES_HOME"),
        }
        assert_eq!(
            paths,
            vec![
                root.join("config.yaml"),
                root.join("profiles").join("forge").join("config.yaml"),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hermes_iso8601_matches_a_real_timestamp() {
        // 2026-10-27 is 20753 days after the Unix epoch.
        assert_eq!(hermes::civil_from_days(20_753), (2026, 10, 27));
        assert_eq!(hermes::civil_from_days(0), (1970, 1, 1));
    }
}
