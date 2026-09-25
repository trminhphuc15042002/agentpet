import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const required = [
  ".care-hud",
  ".care-top",
  ".care-name",
  ".care-stage",
  ".care-hunger",
  ".care-bar",
  ".care-bar-fill",
  ".care-xprow",
  ".care-grid",
  ".care-stat",
  ".cs-label",
  ".cs-val",
  ".cs-sub",
  ".care-charthead",
  ".care-chart",
  ".cbar-wrap",
  ".cbar",
  ".cbar-lbl",
  ".ghelp",
  ".care-achhead",
  ".care-badges",
  ".care-badge",
  ".care-badge.on",
];

const cssPath = join(dirname(fileURLToPath(import.meta.url)), "..", "src", "styles.css");
const css = readFileSync(cssPath, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const missing = required.filter((sel) => {
  if (sel === ".care-badge.on") {
    return !/(^|[^\w-])\.care-badge\.on(\s*\{)/m.test(css);
  }
  // Match selector at start of a rule (optional compound like .care-badge.on handled above)
  const re = new RegExp(`(^|[\\s,}])${sel.replace(/\./g, "\\.")}(\\s*[,{])`, "m");
  return !re.test(css);
});

if (missing.length) {
  console.error("missing Care CSS selectors:", missing.join(", "));
  process.exit(1);
}
console.log("ok: all Care CSS selectors present");
