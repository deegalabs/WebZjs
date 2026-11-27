const http = require("http"), fs = require("fs"), path = require("path");
const { chromium } = require("playwright-core");
const [variant, isolated, syncWait] = [process.argv[2], process.argv[3] === "1", process.argv[4] || "90"];
const root = process.env.SITE_DIR || path.join(__dirname, "site");
const proxy = process.env.PROXY_URL || "http://localhost:1234";
const mime = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".wasm": "application/wasm" };
const srv = http.createServer((q, r) => {
  let p = path.join(root, decodeURIComponent(q.url.split("?")[0])); if (p.endsWith(path.sep)) p += "index.html";
  if (!p.startsWith(root) || !fs.existsSync(p) || fs.statSync(p).isDirectory()) { r.writeHead(404).end(); return; }
  const h = { "content-type": mime[path.extname(p)] || "application/octet-stream" };
  if (isolated) { h["cross-origin-opener-policy"] = "same-origin"; h["cross-origin-embedder-policy"] = "require-corp"; h["cross-origin-resource-policy"] = "same-origin"; }
  r.writeHead(200, h); fs.createReadStream(p).pipe(r);
}).listen(8091, "127.0.0.1", async () => {
  const b = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH });
  const page = await b.newPage(); const con = [];
  page.on("console", (m) => con.push(m.type() + ": " + m.text().trim().slice(0, 400)));
  page.on("pageerror", (e) => con.push("pageerror: " + String(e).slice(0, 400)));
  const resp = await page.goto(`http://localhost:8091/?variant=${variant}&proxy=${encodeURIComponent(proxy)}&syncWait=${syncWait}`);
  const hh = resp.headers();
  console.log(`variant=${variant} COOP=${hh["cross-origin-opener-policy"] || "-"} COEP=${hh["cross-origin-embedder-policy"] || "-"}`);
  const deadline = Date.now() + (Number(syncWait) + 150) * 1000; let s;
  for (;;) { await new Promise((r) => setTimeout(r, 1000)); s = await page.evaluate(() => ({ ...window.state })); if (s.done || Date.now() > deadline) break; }
  console.log("page crossOriginIsolated:", s.isolated);
  for (const l of s.log) console.log("worker:", JSON.stringify(l));
  console.log("--- browser console (last 25) ---"); console.log(con.slice(-25).join("\n"));
  await b.close(); srv.close(); process.exit(0);
});
