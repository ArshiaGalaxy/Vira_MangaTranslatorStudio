/**
 * Vira LAN gateway — the ONLY component that ever listens on 0.0.0.0.
 *
 * Sits in front of the Next.js standalone server (loopback) when the user
 * opts in to LAN exposure, and requires a 6-digit PIN from every
 * non-loopback client. The desktop app itself connects from loopback and
 * is exempted at the SOCKET level (remoteAddress), which a remote client
 * cannot spoof.
 *
 * Why a custom gateway instead of Next middleware: Next 15.1 middleware
 * runs on the edge runtime — no access to the client's socket address and
 * env vars are inlined at build time, so a trustworthy loopback-vs-LAN
 * distinction is impossible there.
 *
 * Env (set by the desktop launcher):
 *   PORT           public port to bind on 0.0.0.0
 *   UPSTREAM_PORT  loopback port the Next.js server listens on
 *   VIRA_LAN_PIN   6-digit PIN (gateway refuses to start without one)
 *
 * Changes to the PIN/binding take effect on the next app start; the
 * settings UI states this explicitly.
 */

"use strict";

const http = require("http");
const crypto = require("crypto");

const PORT = parseInt(process.env.PORT || "0", 10);
const UPSTREAM_PORT = parseInt(process.env.UPSTREAM_PORT || "0", 10);
const LAN_PIN = process.env.VIRA_LAN_PIN || "";

if (!PORT || !UPSTREAM_PORT) {
  console.error("[lan-gate] PORT and UPSTREAM_PORT are required");
  process.exit(1);
}
if (!/^\d{6}$/.test(LAN_PIN)) {
  // Fail loud rather than expose the studio unauthenticated on the LAN.
  console.error("[lan-gate] VIRA_LAN_PIN must be exactly 6 digits");
  process.exit(1);
}

const COOKIE_NAME = "vira_pin";
const UNLOCK_PATH = "/__vira_pin";

/* ── naive fixed-window rate limit per client IP ─────────────────────────── */
const WINDOW_MS = 5 * 60 * 1000;
const MAX_FAILS = 10;
const fails = new Map(); // ip -> { count, windowStart }

function rateLimited(ip) {
  const now = Date.now();
  const rec = fails.get(ip);
  if (!rec || now - rec.windowStart > WINDOW_MS) return false;
  return rec.count >= MAX_FAILS;
}

function recordFail(ip) {
  const now = Date.now();
  const rec = fails.get(ip);
  if (!rec || now - rec.windowStart > WINDOW_MS) {
    fails.set(ip, { count: 1, windowStart: now });
  } else {
    rec.count += 1;
  }
}

function constantTimeEqual(a, b) {
  // Hash first so lengths never leak and inputs are fixed-size.
  const ha = crypto.createHash("sha256").update(String(a)).digest();
  const hb = crypto.createHash("sha256").update(String(b)).digest();
  return crypto.timingSafeEqual(ha, hb);
}

/* ── loopback detection (socket level, unspoofable) ──────────────────────── */
function isLoopback(remoteAddress) {
  if (!remoteAddress) return false;
  // Node reports IPv4-mapped IPv6 as ::ffff:a.b.c.d
  const norm = remoteAddress.replace(/^::ffff:/, "");
  return norm === "::1" || norm.startsWith("127.");
}

function parseCookies(header) {
  const out = {};
  for (const part of String(header || "").split(";")) {
    const idx = part.indexOf("=");
    if (idx > 0) out[part.slice(0, idx).trim()] = part.slice(idx + 1).trim();
  }
  return out;
}

/* ── proxy ────────────────────────────────────────────────────────────────── */
function proxy(req, res) {
  const upstreamReq = http.request(
    {
      host: "127.0.0.1",
      port: UPSTREAM_PORT,
      method: req.method,
      path: req.url,
      headers: { ...req.headers, host: `127.0.0.1:${UPSTREAM_PORT}` },
    },
    (upstreamRes) => {
      res.writeHead(upstreamRes.statusCode || 502, upstreamRes.headers);
      upstreamRes.pipe(res);
    },
  );
  upstreamReq.on("error", (err) => {
    console.error(`[lan-gate] upstream error: ${err.message}`);
    if (!res.headersSent) res.writeHead(502, { "Content-Type": "text/plain" });
    res.end("Studio is starting up — try again in a moment.");
  });
  req.pipe(upstreamReq);
}

/* ── built-in unlock page (no external assets, en + fa) ──────────────────── */
const GATE_PAGE = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Vira Translator Studio</title>
<style>
  :root { color-scheme: dark; }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center;
         background: #17141f; color: #efeaf7; font-family: system-ui, sans-serif; }
  .card { background: #221d30; border: 1px solid #3a3352; border-radius: 18px;
          padding: 34px 30px; width: min(360px, 88vw); text-align: center; }
  h1 { font-size: 17px; margin: 0 0 4px; }
  p { font-size: 13px; color: #a99fc2; margin: 0 0 20px; line-height: 1.6; }
  input { width: 100%; box-sizing: border-box; font-size: 26px; letter-spacing: 10px;
          text-align: center; padding: 12px 0 12px 10px; border-radius: 12px;
          border: 1px solid #4a4170; background: #17141f; color: #efeaf7; outline: none; }
  input:focus { border-color: #8b6fe8; }
  button { margin-top: 16px; width: 100%; padding: 12px; font-size: 15px; font-weight: 700;
           border: 0; border-radius: 12px; background: #8b6fe8; color: #fff; cursor: pointer; }
  button:disabled { opacity: 0.6; }
  .err { color: #ff8f8f; font-size: 12.5px; min-height: 18px; margin-top: 12px; }
  .fa { direction: rtl; }
</style>
</head>
<body>
  <div class="card">
    <h1>Vira Translator Studio</h1>
    <p>Enter the PIN shown in the desktop app's settings.<br>
       <span class="fa">کد ۶ رقمی تنظیمات برنامه را وارد کنید.</span></p>
    <form id="f">
      <input id="pin" type="text" inputmode="numeric" autocomplete="one-time-code"
             maxlength="6" pattern="[0-9]*" autofocus>
      <button type="submit">Unlock &nbsp;·&nbsp; <span class="fa">ورود</span></button>
      <div class="err" id="err"></div>
    </form>
  </div>
<script>
  const f = document.getElementById("f"), pin = document.getElementById("pin"),
        err = document.getElementById("err");
  pin.addEventListener("input", () => { pin.value = pin.value.replace(/\\D/g, "").slice(0, 6); });
  f.addEventListener("submit", async (e) => {
    e.preventDefault();
    err.textContent = "";
    if (pin.value.length !== 6) { err.textContent = "Enter all 6 digits · ۶ رقم را کامل وارد کنید"; return; }
    const btn = f.querySelector("button"); btn.disabled = true;
    try {
      const r = await fetch("${UNLOCK_PATH}", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ pin: pin.value }),
      });
      if (r.ok) { location.replace("/"); return; }
      err.textContent = r.status === 429
        ? "Too many attempts — wait a few minutes · تلاش‌های زیاد، چند دقیقه صبر کنید"
        : "Wrong PIN · کد اشتباه است";
    } catch { err.textContent = "Network error · خطای شبکه"; }
    btn.disabled = false;
  });
</script>
</body>
</html>`;

function sendUnlockPage(res, status = 200) {
  const body = Buffer.from(GATE_PAGE, "utf8");
  res.writeHead(status, {
    "Content-Type": "text/html; charset=utf-8",
    "Content-Length": body.length,
    "Cache-Control": "no-store",
  });
  res.end(body);
}

function readBody(req, limit = 4096) {
  return new Promise((resolve, reject) => {
    let size = 0;
    const chunks = [];
    req.on("data", (c) => {
      size += c.length;
      if (size > limit) {
        reject(new Error("body too large"));
        req.destroy();
        return;
      }
      chunks.push(c);
    });
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

function respondJson(res, status, obj, extraHeaders = {}) {
  const body = Buffer.from(JSON.stringify(obj), "utf8");
  res.writeHead(status, {
    "Content-Type": "application/json",
    "Content-Length": body.length,
    "Cache-Control": "no-store",
    ...extraHeaders,
  });
  res.end(body);
}

/* ── server ───────────────────────────────────────────────────────────────── */
const server = http.createServer(async (req, res) => {
  const ip = req.socket.remoteAddress || "";
  const loopback = isLoopback(ip);

  if (loopback) {
    proxy(req, res);
    return;
  }

  const cookies = parseCookies(req.headers.cookie);
  if (cookies[COOKIE_NAME] && constantTimeEqual(cookies[COOKIE_NAME], LAN_PIN)) {
    proxy(req, res);
    return;
  }

  // Unlocked requests from the LAN must never poison the browser cache of
  // other devices: no-store is set on the gate responses, proxied responses
  // pass through unchanged.
  if (req.method === "POST" && req.url === UNLOCK_PATH) {
    if (rateLimited(ip)) {
      respondJson(res, 429, { ok: false });
      return;
    }
    let body;
    try {
      body = JSON.parse((await readBody(req)) || "{}");
    } catch {
      respondJson(res, 400, { ok: false });
      return;
    }
    if (body && constantTimeEqual(body.pin || "", LAN_PIN)) {
      respondJson(res, 200, { ok: true }, { "Set-Cookie": `${COOKIE_NAME}=${LAN_PIN}; HttpOnly; Path=/; SameSite=Lax` });
      return;
    }
    recordFail(ip);
    setTimeout(() => respondJson(res, 401, { ok: false }), 400);
    return;
  }

  sendUnlockPage(res);
});

server.listen(PORT, "0.0.0.0", () => {
  console.log(`[lan-gate] listening on 0.0.0.0:${PORT} -> 127.0.0.1:${UPSTREAM_PORT}`);
});
