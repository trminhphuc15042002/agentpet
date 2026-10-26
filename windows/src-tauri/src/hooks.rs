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
    JcodeToml,         // ~/.jcode/config.toml [hooks] key = "command"
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
            events: &["SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest", "Stop", "SubagentStart", "SubagentStop"] },
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
        // Observers only: jcode spawns them detached. pre_tool (a blocking gate)
        // is deliberately not used.
        "jcode" => Spec { style: Style::JcodeToml, rel_path: &[".jcode", "config.toml"],
            events: &["session_start", "turn_start", "post_tool", "turn_end", "session_end"] },
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
        ("jcode", "jcode", Some("jcode lifecycle hooks (~/.jcode/config.toml)")),
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
        Style::OpencodePlugin | Style::PiExtension | Style::JcodeToml | Style::HermesYaml => Value::Null,
    }
}

// ----- public API ----------------------------------------------------------
pub fn is_installed(kind: &str) -> bool {
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
    if s.style == Style::JcodeToml {
        return jcode::read_or_empty(&path)
            .map(|text| jcode::is_installed(&text, s.events))
            .unwrap_or(false);
    }
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
    if s.style == Style::JcodeToml {
        return jcode::install_to_disk(&cmd, &path, s.events);
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
    let Some(s) = spec(kind) else { return Ok(()) };
    let cmd = full_command(kind);
    if s.style == Style::HermesYaml {
        return hermes::uninstall_all(&config_paths(kind), &cmd, s.events);
    }
    let Some(path) = config_path(kind) else { return Ok(()) };
    if s.style == Style::JcodeToml {
        return jcode::uninstall_from_disk(&path, s.events);
    }
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

/// Line-preserving editor for jcode's `~/.jcode/config.toml` `[hooks]` table.
/// Comments, unrelated keys, and newline style stay intact. Foreign or array
/// values are refused rather than merged.
mod jcode {
    use super::is_ours;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

    pub fn is_hooks_header(line: &str) -> bool {
        header_compact(line) == "[hooks]"
    }

    /// Full table/array-table header, including every closing `]`. Truncating at
    /// the first `]` would turn `[[hooks]]` into `[[hooks]` and miss it.
    fn header_code(line: &str) -> Option<&str> {
        let t = strip_cr(line).trim();
        if !t.starts_with('[') {
            return None;
        }
        let mut depth = 0i32;
        let mut in_str: Option<char> = None;
        let mut escape = false;
        let mut i = 0;
        while i < t.len() {
            let ch = t[i..].chars().next()?;
            let n = ch.len_utf8();
            if let Some(q) = in_str {
                if escape {
                    escape = false;
                } else if q == '"' && ch == '\\' {
                    escape = true;
                } else if ch == q {
                    in_str = None;
                }
                i += n;
                continue;
            }
            match ch {
                '"' | '\'' => in_str = Some(ch),
                '[' => depth += 1,
                ']' => {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                    i += n;
                    if depth == 0 {
                        let rest = t[i..].trim_start();
                        if rest.is_empty() || rest.starts_with('#') {
                            return Some(&t[..i]);
                        }
                        return None;
                    }
                    continue;
                }
                _ => {}
            }
            i += n;
        }
        None
    }

    fn header_compact(line: &str) -> String {
        header_code(line)
            .map(|h| h.chars().filter(|c| !c.is_whitespace()).collect())
            .unwrap_or_default()
    }

    fn strip_cr(line: &str) -> &str {
        line.strip_suffix('\r').unwrap_or(line)
    }

    fn hooks_body(lines: &[String]) -> Option<(usize, usize)> {
        let header = lines.iter().position(|l| is_hooks_header(l))?;
        let mut end = header + 1;
        while end < lines.len() {
            if strip_cr(&lines[end]).trim().starts_with('[') {
                break;
            }
            end += 1;
        }
        Some((header, end))
    }

    fn split_indent(s: &str) -> (&str, &str) {
        let i = s.find(|c: char| c != ' ' && c != '\t').unwrap_or(s.len());
        (&s[..i], &s[i..])
    }

    fn parse_literal_string(s: &str) -> Option<(String, usize)> {
        if !s.starts_with('\'') {
            return None;
        }
        let rest = &s[1..];
        let end = rest.find('\'')?;
        Some((rest[..end].to_string(), 1 + end + 1))
    }

    fn parse_basic_string(s: &str) -> Option<(String, usize)> {
        if !s.starts_with('"') {
            return None;
        }
        let mut chars = s.char_indices();
        chars.next()?;
        let mut out = String::new();
        let mut escape = false;
        for (idx, ch) in chars {
            if escape {
                out.push(match ch {
                    '\\' => '\\',
                    '"' => '"',
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    _ => return None,
                });
                escape = false;
                continue;
            }
            match ch {
                '\\' => escape = true,
                '"' => return Some((out, idx + ch.len_utf8())),
                '\n' => return None,
                _ => out.push(ch),
            }
        }
        None
    }

    fn parse_quoted(s: &str) -> Option<(String, usize)> {
        if s.starts_with("'''") || s.starts_with("\"\"\"") {
            return None;
        }
        if s.starts_with('\'') {
            parse_literal_string(s)
        } else if s.starts_with('"') {
            parse_basic_string(s)
        } else {
            None
        }
    }

    fn remainder_comment(rest: &str) -> Option<&str> {
        let t = rest.trim_start();
        if t.is_empty() {
            Some("")
        } else if t.starts_with('#') {
            Some(rest)
        } else {
            None
        }
    }

    enum Hit {
        Ours { index: usize, comment: String, leading: String },
        Foreign,
        Ambiguous,
        Missing,
    }

    fn hit_for_event(lines: &[String], header: usize, end: usize, event: &str) -> Hit {
        let mut found: Option<(usize, String, String)> = None;
        for i in header + 1..end {
            match inspect_line(&lines[i], event) {
                LineHit::Skip => {}
                LineHit::Ours { comment, leading } => {
                    if found.is_some() {
                        return Hit::Ambiguous;
                    }
                    found = Some((i, comment, leading));
                }
                LineHit::Foreign => return Hit::Foreign,
                LineHit::Ambiguous => return Hit::Ambiguous,
            }
        }
        match found {
            Some((index, comment, leading)) => Hit::Ours { index, comment, leading },
            None => Hit::Missing,
        }
    }

    enum LineHit {
        Skip,
        Ours { comment: String, leading: String },
        Foreign,
        Ambiguous,
    }

    fn inspect_line(line: &str, event: &str) -> LineHit {
        let raw = strip_cr(line);
        let (leading, rest) = split_indent(raw);
        if rest.is_empty() || rest.starts_with('#') {
            return LineHit::Skip;
        }
        if rest.starts_with('[') {
            return LineHit::Skip;
        }
        let (key, key_quoted, after_key) = match parse_key(rest) {
            Some(v) => v,
            None => {
                return if rest.contains(event) { LineHit::Ambiguous } else { LineHit::Skip };
            }
        };
        if after_key.trim_start().starts_with('.') {
            return if key == event || key == "hooks" { LineHit::Ambiguous } else { LineHit::Skip };
        }
        let after_key = after_key.trim_start();
        if !after_key.starts_with('=') {
            return if key == event { LineHit::Ambiguous } else { LineHit::Skip };
        }
        if key != event {
            return LineHit::Skip;
        }
        if key_quoted {
            return LineHit::Ambiguous;
        }
        let after_eq = after_key[1..].trim_start();
        if after_eq.starts_with('[') || after_eq.starts_with('{')
            || after_eq.starts_with("'''") || after_eq.starts_with("\"\"\"")
        {
            return LineHit::Ambiguous;
        }
        let Some((value, n)) = parse_quoted(after_eq) else {
            return LineHit::Ambiguous;
        };
        let Some(comment) = remainder_comment(&after_eq[n..]) else {
            return LineHit::Ambiguous;
        };
        if is_ours(&value) {
            LineHit::Ours { comment: comment.to_string(), leading: leading.to_string() }
        } else {
            LineHit::Foreign
        }
    }

    fn parse_key(s: &str) -> Option<(String, bool, &str)> {
        if s.starts_with('\'') || s.starts_with('"') {
            let (key, n) = parse_quoted(s)?;
            Some((key, true, &s[n..]))
        } else {
            let n = s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).unwrap_or(s.len());
            if n == 0 {
                return None;
            }
            Some((s[..n].to_string(), false, &s[n..]))
        }
    }

    fn inner_is_hooks_name(inner: &str) -> bool {
        inner == "hooks" || inner == "\"hooks\"" || inner == "'hooks'"
    }

    fn unsupported_hooks_repr(toml: &str) -> bool {
        let mut standard = 0usize;
        for line in toml.split('\n') {
            if line_is_unsupported_hooks(line) {
                return true;
            }
            if is_hooks_header(line) {
                standard += 1;
                if standard > 1 {
                    return true;
                }
            }
        }
        false
    }

    fn line_is_unsupported_hooks(line: &str) -> bool {
        let compact = header_compact(line);
        if compact.starts_with("[[") && compact.ends_with("]]") {
            if inner_is_hooks_name(&compact[2..compact.len() - 2]) {
                return true;
            }
        } else if compact.starts_with('[') && compact.ends_with(']') && compact != "[hooks]" {
            if inner_is_hooks_name(&compact[1..compact.len() - 1]) {
                return true;
            }
        }
        let raw = strip_cr(line);
        let rest = raw.trim_start();
        if rest.is_empty() || rest.starts_with('#') || rest.starts_with('[') {
            return false;
        }
        let Some((key, _, after_key)) = parse_key(rest) else {
            return false;
        };
        if key != "hooks" {
            return false;
        }
        let after = after_key.trim_start();
        after.starts_with('.') || after.starts_with('=')
    }

    fn conflict(path: &str, event: &str) -> String {
        format!("{path} already sets hooks.{event}; remove it or add AgentPet's command to it manually.")
    }

    pub fn toml_string(command: &str) -> String {
        if !command.contains('\'') && !command.contains('\n') {
            format!("'{command}'")
        } else {
            let escaped = command.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
            format!("\"{escaped}\"")
        }
    }

    pub fn is_installed(toml: &str, events: &[&str]) -> bool {
        if unsupported_hooks_repr(toml) {
            return false;
        }
        let lines: Vec<String> = toml.split('\n').map(|s| s.to_string()).collect();
        let Some((header, end)) = hooks_body(&lines) else { return false };
        events.iter().any(|event| matches!(hit_for_event(&lines, header, end, event), Hit::Ours { .. }))
    }

    pub fn install(toml: &str, command: &str, events: &[&str], path: &str) -> Result<String, String> {
        if unsupported_hooks_repr(toml) {
            return Err(format!("{path} has an unsupported hooks representation; refuse to edit it."));
        }
        let crlf = toml.contains("\r\n") || toml.contains('\r');
        let had_trailing_nl = toml.ends_with('\n');
        let eol = if crlf { "\r" } else { "" };
        let mut lines: Vec<String> = toml.split('\n').map(|s| s.to_string()).collect();
        if hooks_body(&lines).is_none() {
            if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
                lines.pop();
            } else if crlf {
                if let Some(last) = lines.last_mut() {
                    if !last.ends_with('\r') {
                        last.push('\r');
                    }
                }
            }
            if !lines.is_empty() {
                lines.push(eol.to_string());
            }
            lines.push(format!("[hooks]{eol}"));
            if had_trailing_nl {
                lines.push(String::new());
            }
        }
        let (header, end) = hooks_body(&lines).expect("hooks table present");
        let mut ours: Vec<(String, usize, String, String)> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        for event in events {
            match hit_for_event(&lines, header, end, event) {
                Hit::Foreign | Hit::Ambiguous => return Err(conflict(path, event)),
                Hit::Ours { index, comment, leading } => {
                    ours.push((event.to_string(), index, comment, leading));
                }
                Hit::Missing => missing.push((*event).to_string()),
            }
        }
        let value = toml_string(command);
        for (event, index, comment, leading) in &ours {
            let line_eol = if lines[*index].ends_with('\r') || crlf { "\r" } else { "" };
            lines[*index] = format!("{leading}{event} = {value}{comment}{line_eol}");
        }
        let new_entries: Vec<String> = missing
            .into_iter()
            .map(|event| format!("{event} = {value}{eol}"))
            .collect();
        let at = header + 1;
        lines.splice(at..at, new_entries);
        Ok(join_preserving_final_newline(lines, had_trailing_nl))
    }

    fn join_preserving_final_newline(mut lines: Vec<String>, had_trailing_nl: bool) -> String {
        if !had_trailing_nl {
            if let Some(last) = lines.last_mut() {
                if last.ends_with('\r') {
                    last.pop();
                }
            }
        }
        lines.join("\n")
    }

    pub fn uninstall(toml: &str, events: &[&str]) -> String {
        if unsupported_hooks_repr(toml) {
            return toml.to_string();
        }
        let mut lines: Vec<String> = toml.split('\n').map(|s| s.to_string()).collect();
        for event in events {
            let Some((header, end)) = hooks_body(&lines) else { continue };
            if let Hit::Ours { index, .. } = hit_for_event(&lines, header, end, event) {
                lines.remove(index);
            }
        }
        lines.join("\n")
    }

    pub fn read_or_empty(path: &Path) -> std::io::Result<String> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    pub fn install_to_disk(command: &str, path: &Path, events: &[&str]) -> std::io::Result<()> {
        let existing = read_or_empty(path)?;
        let updated = install(&existing, command, events, &path.display().to_string())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        if updated != existing {
            write_atomic(path, &updated)?;
        }
        Ok(())
    }

    pub fn uninstall_from_disk(path: &Path, events: &[&str]) -> std::io::Result<()> {
        let existing = read_or_empty(path)?;
        let updated = uninstall(&existing, events);
        if updated != existing {
            write_atomic(path, &updated)?;
        }
        Ok(())
    }

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
}

/// Hermes Agent (Nous Research) integration. Unlike the other agents Hermes
/// keeps a YAML config per profile and refuses to run a shell hook that is not
/// in its per-home consent allowlist, so install covers every home and approves
/// the hook there too. `config.yaml` is user-owned and comment-heavy, so it is
/// edited line-wise (same approach as jcode's TOML) and foreign hooks are kept.
mod hermes {
    use super::{is_ours, jcode::write_atomic};
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};

    pub const ALLOWLIST: &str = "shell-hooks-allowlist.json";

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
mod tests {
    use super::*;

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

    const JCODE_CMD: &str = "\"/x/agentpet\" hook --agent jcode";
    const JCODE_EVENTS: &[&str] = &["session_start", "turn_start", "post_tool", "turn_end", "session_end"];
    const JCODE_SAMPLE: &str = "# user comment\n[display]\ndebug_socket = false\n\n[hooks]\npre_tool_timeout_ms = 5000\n\n[ambient]\nenabled = false\n";

    #[test]
    fn jcode_spec_uses_observers_only() {
        let s = spec("jcode").expect("jcode spec");
        assert!(matches!(s.style, Style::JcodeToml));
        assert_eq!(s.rel_path, &[".jcode", "config.toml"]);
        assert!(!s.events.contains(&"pre_tool"), "pre_tool is a blocking gate");
        assert_eq!(s.events, JCODE_EVENTS);
    }

    #[test]
    fn jcode_in_catalog() {
        let row = catalog().into_iter().find(|a| a.kind == "jcode").expect("jcode");
        assert_eq!(row.display_name, "jcode");
        assert_eq!(row.note.as_deref(), Some("jcode lifecycle hooks (~/.jcode/config.toml)"));
    }

    #[test]
    fn jcode_install_adds_keys_inside_hooks_table_only() {
        let out = jcode::install(JCODE_SAMPLE, JCODE_CMD, JCODE_EVENTS, "config.toml").unwrap();
        let lines: Vec<&str> = out.split('\n').collect();
        let hooks = lines.iter().position(|l| *l == "[hooks]").unwrap();
        let ambient = lines.iter().position(|l| *l == "[ambient]").unwrap();
        for event in JCODE_EVENTS {
            let entry = format!("{event} = '{JCODE_CMD}'");
            let i = lines.iter().position(|l| *l == entry.as_str()).expect(event);
            assert!(i > hooks && i < ambient, "{event} must sit in [hooks]");
        }
        assert_eq!(jcode::uninstall(&out, JCODE_EVENTS), JCODE_SAMPLE);
        assert!(jcode::is_installed(&out, JCODE_EVENTS));
        assert!(!jcode::is_installed(JCODE_SAMPLE, JCODE_EVENTS));
    }

    #[test]
    fn jcode_install_is_idempotent() {
        let once = jcode::install(JCODE_SAMPLE, JCODE_CMD, JCODE_EVENTS, "config.toml").unwrap();
        assert_eq!(jcode::install(&once, JCODE_CMD, JCODE_EVENTS, "config.toml").unwrap(), once);
        let moved = jcode::install(&once, "\"/y/agentpet\" hook --agent jcode", JCODE_EVENTS, "config.toml").unwrap();
        assert_eq!(moved.split('\n').count(), once.split('\n').count());
        assert!(moved.contains("/y/agentpet") && !moved.contains("/x/agentpet"));
    }

    #[test]
    fn jcode_install_creates_hooks_table_when_missing() {
        let out = jcode::install("[display]\nx = 1\n", JCODE_CMD, &["turn_end"], "config.toml").unwrap();
        assert!(out.starts_with("[display]\nx = 1\n\n[hooks]\nturn_end = '"));
        assert!(jcode::install("", JCODE_CMD, &["turn_end"], "config.toml").unwrap().starts_with("[hooks]\n"));
    }

    #[test]
    fn jcode_header_variants_are_found_not_duplicated() {
        for header in ["[hooks] # observers", "[ hooks ]", "\t[hooks]\t", "[hooks]\r"] {
            let src = format!("{header}\npre_tool = \"x\"\n");
            let out = jcode::install(&src, JCODE_CMD, &["turn_end"], "config.toml").unwrap();
            let headers: Vec<&str> = out.split('\n').filter(|l| jcode::is_hooks_header(l)).collect();
            assert_eq!(headers.len(), 1, "header {header:?}");
            assert!(jcode::is_installed(&out, &["turn_end"]));
            assert_eq!(jcode::uninstall(&out, &["turn_end"]), src);
        }
        assert!(!jcode::is_hooks_header("[[hooks]]"));
        assert!(!jcode::is_hooks_header("# [hooks]"));
        assert!(!jcode::is_hooks_header("[hooks.extra]"));
    }

    #[test]
    fn jcode_foreign_hook_is_never_overwritten() {
        let foreign = JCODE_SAMPLE.replace(
            "pre_tool_timeout_ms = 5000",
            "pre_tool_timeout_ms = 5000\nturn_end = \"~/bin/notify\"",
        );
        let err = jcode::install(&foreign, JCODE_CMD, JCODE_EVENTS, "config.toml").unwrap_err();
        assert!(err.contains("already sets hooks.turn_end"));
        assert_eq!(jcode::uninstall(&foreign, JCODE_EVENTS), foreign);
    }

    #[test]
    fn jcode_array_hook_is_never_overwritten() {
        let src = "[hooks]\nturn_end = [\"~/bin/notify\"]\n";
        let err = jcode::install(src, JCODE_CMD, &["turn_end"], "config.toml").unwrap_err();
        assert!(err.contains("already sets hooks.turn_end"));
        assert_eq!(jcode::uninstall(src, &["turn_end"]), src);
        assert!(!jcode::is_installed(src, &["turn_end"]));
    }

    #[test]
    fn jcode_commented_keys_and_similar_names_are_ignored() {
        let toml = "[hooks]\n# turn_end = \"old\"\nturn_end_extra = 1\n";
        let out = jcode::install(toml, JCODE_CMD, &["turn_end"], "config.toml").unwrap();
        assert!(out.contains("# turn_end = \"old\"\nturn_end_extra = 1"));
        assert!(out.contains(&format!("turn_end = '{JCODE_CMD}'")));
    }

    #[test]
    fn jcode_command_quoting() {
        assert_eq!(
            jcode::toml_string(r#""/a b's/agentpet" hook"#),
            r#""\"/a b's/agentpet\" hook""#
        );
        assert_eq!(
            jcode::toml_string(r#""/x/agentpet" hook"#),
            r#"'"/x/agentpet" hook'"#
        );
        let win = r#""C:\Users\me\agentpet.exe" hook --agent jcode"#;
        assert_eq!(jcode::toml_string(win), format!("'{win}'"));
        let apo = r#""C:\Users\O'Brien\agentpet.exe" hook"#;
        assert_eq!(
            jcode::toml_string(apo),
            r#""\"C:\\Users\\O'Brien\\agentpet.exe\" hook""#
        );
    }

    #[test]
    fn jcode_disk_roundtrip() {
        let dir = std::env::temp_dir().join(format!("agentpet-jcode-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, JCODE_SAMPLE).unwrap();
        let installed = jcode::install(JCODE_SAMPLE, JCODE_CMD, JCODE_EVENTS, &path.display().to_string()).unwrap();
        jcode::write_atomic(&path, &installed).unwrap();
        assert!(jcode::is_installed(&std::fs::read_to_string(&path).unwrap(), JCODE_EVENTS));
        let removed = jcode::uninstall(&std::fs::read_to_string(&path).unwrap(), JCODE_EVENTS);
        jcode::write_atomic(&path, &removed).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), JCODE_SAMPLE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jcode_foreign_command_with_agentpet_hook_comment_is_not_ours() {
        let src = "[hooks]\nturn_end = 'notify' # agentpet hook\n";
        let err = jcode::install(src, JCODE_CMD, &["turn_end"], "config.toml").unwrap_err();
        assert!(err.contains("already sets hooks.turn_end"));
        assert_eq!(jcode::uninstall(src, &["turn_end"]), src);
        assert!(!jcode::is_installed(src, &["turn_end"]));
    }

    #[test]
    fn jcode_invalid_utf8_is_not_overwritten() {
        let dir = std::env::temp_dir().join(format!(
            "agentpet-jcode-utf8-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = b"\xff\xfe[hooks]\nturn_end = 'x'\n";
        std::fs::write(&path, bytes).unwrap();
        assert!(jcode::install_to_disk(JCODE_CMD, &path, JCODE_EVENTS).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jcode_windows_apostrophe_path_installs() {
        let cmd = r#""C:\Users\O'Brien\agentpet.exe" hook --agent jcode"#;
        let out = jcode::install("[hooks]\n", cmd, &["turn_end"], "config.toml").unwrap();
        assert!(out.contains(r#""\"C:\\Users\\O'Brien\\agentpet.exe\" hook --agent jcode""#));
        assert!(jcode::is_installed(&out, &["turn_end"]));
    }

    #[test]
    fn jcode_quoted_key_is_conflict() {
        let src = "[hooks]\n\"turn_end\" = \"~/bin/notify\"\n";
        let err = jcode::install(src, JCODE_CMD, &["turn_end"], "config.toml").unwrap_err();
        assert!(err.contains("turn_end"));
        assert_eq!(jcode::uninstall(src, &["turn_end"]), src);
        assert!(!jcode::is_installed(src, &["turn_end"]));
    }

    #[test]
    fn jcode_crlf_no_final_newline_missing_header() {
        let src = "[display]\r\nx = 1";
        let out = jcode::install(src, JCODE_CMD, &["turn_end"], "config.toml").unwrap();
        assert!(out.contains("[display]\r\nx = 1\r\n\r\n[hooks]\r\n"), "{out:?}");
        assert!(out.contains("turn_end = "));
        assert!(!out.ends_with('\n'), "{out:?}");
        assert!(!out.ends_with('\r'), "{out:?}");
    }

    #[test]
    fn jcode_crlf_existing_hooks_no_final_newline_no_trailing_cr() {
        let src = "[hooks]\r\npre_tool_timeout_ms = 5000";
        let out = jcode::install(src, JCODE_CMD, &["turn_end"], "config.toml").unwrap();
        assert!(out.contains("[hooks]\r\n"), "{out:?}");
        assert!(out.contains("pre_tool_timeout_ms = 5000"), "{out:?}");
        assert!(out.contains("turn_end = "));
        assert!(!out.ends_with('\n'), "{out:?}");
        assert!(!out.ends_with('\r'), "{out:?}");
        let last_ours = "[hooks]\r\nturn_end = '\"/x/agentpet\" hook --agent jcode'";
        let moved = jcode::install(last_ours, "\"/y/agentpet\" hook --agent jcode", &["turn_end"], "config.toml").unwrap();
        assert!(!moved.ends_with('\r'), "{moved:?}");
        assert!(!moved.ends_with('\n'), "{moved:?}");
        assert!(moved.contains("/y/agentpet"));
    }

    #[test]
    fn jcode_own_comment_suffix_is_preserved() {
        let src = "[hooks]\nturn_end = '\"/x/agentpet\" hook --agent jcode' # keep\n";
        let out = jcode::install(src, "\"/y/agentpet\" hook --agent jcode", &["turn_end"], "config.toml").unwrap();
        assert!(out.contains("# keep"), "{out:?}");
        assert!(out.contains("/y/agentpet"));
        assert!(!out.contains("/x/agentpet"));
    }

    #[test]
    fn jcode_array_and_quoted_hooks_headers_are_rejected() {
        let cmd = JCODE_CMD;
        for src in [
            "[[hooks]]\npre_tool = \"x\"\n",
            "[[ hooks ]]\npre_tool = \"x\"\n",
            "[ [hooks] ]\npre_tool = \"x\"\n",
            "\t[[hooks]]\t\npre_tool = \"x\"\n",
            "[\"hooks\"]\npre_tool = \"x\"\n",
            "['hooks']\npre_tool = \"x\"\n",
            "[ \"hooks\" ]\npre_tool = \"x\"\n",
            "[['hooks']]\npre_tool = \"x\"\n",
        ] {
            let err = jcode::install(src, cmd, &["turn_end"], "config.toml").unwrap_err();
            assert!(err.contains("unsupported hooks representation"), "{src:?} {err}");
            assert_eq!(jcode::uninstall(src, &["turn_end"]), src);
            assert!(!jcode::is_installed(src, &["turn_end"]));
            assert!(!src.contains("[hooks]\n[") && jcode::install(src, cmd, &["turn_end"], "config.toml").is_err());
        }
        assert!(!jcode::is_hooks_header("[[hooks]]"));
        assert!(!jcode::is_hooks_header("[ [hooks] ]"));
        assert!(!jcode::is_hooks_header("[\"hooks\"]"));
    }

    #[test]
    fn jcode_duplicate_hooks_tables_are_rejected() {
        let src = "[hooks]\nx = 1\n\n[hooks]\ny = 2\n";
        let err = jcode::install(src, JCODE_CMD, &["turn_end"], "config.toml").unwrap_err();
        assert!(err.contains("unsupported hooks representation"), "{err}");
        assert_eq!(jcode::uninstall(src, &["turn_end"]), src);
        assert!(!jcode::is_installed(src, &["turn_end"]));
    }

    // ----- Hermes Agent ---------------------------------------------------

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
