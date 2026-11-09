/* Vira launcher UI wiring: consumes `vira://event` messages from Rust. */
"use strict";

const stageOrder = ["paths", "data", "secrets", "runtimes", "pythonenv"];
const $ = (id) => document.getElementById(id);

const els = {
  progress: $("progress"),
  message: $("message"),
  title: $("stage-title"),
  steps: document.getElementById("steps"),
  errorBox: $("error-box"),
  errorDetail: $("error-detail"),
  start: $("start"),
  retry: $("retry"),
  installAi: $("install-ai"),
  aiBadge: $("ai-badge"),
  note: $("note"),
};

function setProgress(p) {
  const v = Math.max(0, Math.min(100, Number(p) || 0));
  els.progress.style.width = v + "%";
  document.getElementById("progress-shell").setAttribute("aria-valuenow", String(v));
}

function stageRows() {
  return Object.fromEntries(
    Array.from(els.steps.querySelectorAll("li")).map((li) => [li.dataset.stage, li]),
  );
}

function markStage(name, cls) {
  const li = stageRows()[name];
  if (!li) return;
  li.classList.remove("active", "done", "error");
  if (cls) li.classList.add(cls);
}

function setError(title, detail) {
  els.errorBox.classList.remove("hidden");
  els.errorDetail.textContent = detail || "";
  els.errorBox.querySelector("strong").textContent = title || "Something went wrong";
}

function clearError() {
  els.errorBox.classList.add("hidden");
}

async function init() {
  const tauri = window.__TAURI__;
  if (!tauri) return;

  const { listen } = tauri.event;
  const { invoke } = tauri.core;

  $("open-logs").addEventListener("click", () => invoke("open_logs"));
  $("open-logs-2").addEventListener("click", () => invoke("open_logs"));

  // Footer credits: open the author's links in the user's browser.
  for (const a of document.querySelectorAll("button[data-url]")) {
    a.addEventListener("click", () =>
      invoke("open_url", { url: a.getAttribute("data-url") }),
    );
  }
  els.retry.addEventListener("click", () => {
    clearError();
    stageOrder.forEach((s) => markStage(s, null));
    setProgress(0);
    invoke("retry_setup");
  });
  els.start.addEventListener("click", () => {
    els.start.disabled = true;
    els.start.textContent = "Starting…";
    els.note.textContent = "Starting services — the studio will open here automatically.";
    clearError();
    invoke("start_studio");
  });
  els.installAi.addEventListener("click", () => {
    els.installAi.disabled = true;
    els.installAi.textContent = "Installing…";
    clearError();
    invoke("install_ai");
  });

  await listen("vira://event", (event) => {
    const payload = event.payload || {};
    handleMessage(payload);
  });

  // Sync with whatever already happened before this page loaded.
  try {
    const snap = await invoke("get_snapshot");
    if (snap && snap.stage) {
      handleMessage({
        stage: snap.stage,
        message: snap.stage_message ?? "",
        error: snap.failed || undefined,
        progress: null,
      });
      if (snap.ai_state === "installed") setAiState("installed");
      else if (snap.ai_state === "not_installed") setAiState("not_installed");
      else setAiState("installing");
    }
  } catch (e) {
    // ignore
  }
}

const stageTitles = {
  init: "Starting Vira",
  paths: "Locating application files",
  data: "Preparing your data folder",
  secrets: "Loading settings",
  runtimes: "Verifying built-in runtimes",
  pythonenv: "Preparing Python libraries",
  ready: "Ready",
  starting: "Starting the studio",
  ports: "Allocating local ports",
  migrate: "Updating your database",
  backend: "Starting translator service",
  frontend: "Starting studio interface",
  app: "Studio running",
  failed: "Something went wrong",
  aiinstall: "Installing AI libraries",
};

function handleMessage(payload) {
  const { stage, message, detail, progress, error } = payload;
  if (message) els.message.textContent = message;
  if (progress !== null && progress !== undefined) setProgress(progress);

  switch (stage) {
    case "paths": markStage("paths", "active"); break;
    case "data": for (const s of stageBefore("data")) markStage(s, "done"); markStage("data", "active"); break;
    case "secrets": for (const s of stageBefore("secrets")) markStage(s, "done"); markStage("secrets", "active"); break;
    case "runtimes": for (const s of stageBefore("runtimes")) markStage(s, "done"); markStage("runtimes", "active"); break;
    case "pythonenv": for (const s of stageBefore("pythonenv")) markStage(s, "done"); markStage("pythonenv", "active"); break;
    case "ready": {
      stageOrder.forEach((s) => markStage(s, "done"));
      markStage("ready", "done");
      els.title.textContent = "Ready";
      els.start.disabled = false;
      els.start.textContent = "Start";
      els.message.textContent = message || "Ready — press Start to open the studio.";
      clearError();
      break;
    }
    case "ports":
    case "migrate":
    case "backend":
    case "frontend":
    case "app": {
      els.title.textContent = stageTitles[stage] || "Starting the studio";
      stageOrder.forEach((s) => markStage(s, "done"));
      markStage("ready", "done");
      break;
    }
    case "aiinstall": {
      els.title.textContent = stageTitles.aiinstall;
      if (progress !== null && progress !== undefined) setProgress(progress);
      if (error) {
        // Failure: offer a retry (requirement #7). The package cache is
        // kept, so retrying resumes instead of starting over.
        setAiState("not_installed");
        els.installAi.disabled = false;
        els.installAi.textContent = "Retry install";
      } else if (progress >= 100) {
        els.title.textContent = "AI libraries installed";
        clearError();
        setAiState("installed");
      }
      break;
    }
    case "failed": {
      stageOrder.forEach((s) => {
        if (!stageRows()[s].classList.contains("done")) markStage(s, "error");
      });
      els.title.textContent = "Something went wrong";
      els.start.disabled = true;
      els.start.textContent = "Start";
setError(titleFallback(stage), [message, detail].filter(Boolean).join("\n\n"));
      els.note.textContent = "You can retry — progress already made is kept.";
      break;
    }
  }

  if (error) {
    setError(message || "Something went wrong", detail || "");
  }
}

function titleFallback() {
  return "Something went wrong";
}

function stageBefore(stage) {
  const i = stageOrder.indexOf(stage);
  return stageOrder.slice(0, Math.max(0, i));
}

function setAiState(state) {
  if (state === "installed") {
    els.aiBadge.textContent = "installed";
    els.aiBadge.className = "badge ok";
    els.installAi.disabled = true;
    els.installAi.textContent = "AI ready";
  } else if (state === "not_installed") {
    els.aiBadge.textContent = "not installed";
    els.aiBadge.className = "badge miss";
    els.installAi.disabled = false;
    els.installAi.textContent = "Install AI";
  } else {
    els.aiBadge.textContent = "installing…";
    els.aiBadge.className = "badge";
    // A snapshot of "installing" means a run is live (state is in-memory
    // only, so this can't be a stale post-restart value) — block double-clicks.
    els.installAi.disabled = true;
    els.installAi.textContent = "Installing…";
  }
}

init();
