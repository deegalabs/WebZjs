// Minimal gRPC-web -> gRPC proxy for a public lightwalletd. localhost:PORT (default 1234) -> UPSTREAM (default https://zec.rocks:443)
const http = require("http"), http2 = require("http2");
const UP = process.env.UPSTREAM || "https://zec.rocks:443";
const cors = (req) => ({
  "access-control-allow-origin": req.headers.origin || "*",
  "access-control-allow-headers": "content-type,x-grpc-web,x-user-agent,grpc-timeout,x-accept-content-transfer-encoding",
  "access-control-expose-headers": "grpc-status,grpc-message",
  "access-control-allow-methods": "POST,OPTIONS",
});
const client = http2.connect(UP);
client.on("error", (e) => console.error("upstream", e.message));
http.createServer((req, res) => {
  if (req.method === "OPTIONS") { res.writeHead(204, cors(req)); return res.end(); }
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => {
    const body = Buffer.concat(chunks);
    const up = client.request({ ":method": "POST", ":path": req.url, "content-type": "application/grpc", te: "trailers", "user-agent": "grpc-web-proxy" });
    let headersSent = false, status = "0", message = "";
    const send = () => { if (!headersSent) { res.writeHead(200, { ...cors(req), "content-type": "application/grpc-web+proto" }); headersSent = true; } };
    up.on("response", (h) => { if (h["grpc-status"]) { status = h["grpc-status"]; message = h["grpc-message"] || ""; } });
    up.on("trailers", (t) => { if (t["grpc-status"]) { status = t["grpc-status"]; message = t["grpc-message"] || ""; } });
    up.on("data", (d) => { send(); res.write(d); });
    up.on("end", () => {
      send();
      const t = Buffer.from(`grpc-status: ${status}\r\ngrpc-message: ${message}\r\n`);
      const f = Buffer.alloc(5 + t.length); f[0] = 0x80; f.writeUInt32BE(t.length, 1); t.copy(f, 5);
      res.end(f);
    });
    up.on("error", (e) => { console.error("req", e.message); try { send(); res.end(); } catch {} });
    up.end(body);
  });
}).listen(Number(process.env.PORT || 1234), "127.0.0.1", () => console.log("grpc-web proxy on", process.env.PORT || 1234, "->", UP));
