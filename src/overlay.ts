// fire-speak — recording HUD (overlay.html)

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { applyDom, getLang, setLang, t, tMsg } from "./i18n";
import type {
  AppStatus,
  LevelPayload,
  Settings,
  StatusChangedPayload,
} from "./types";

const BAR_COUNT = 14;
const MAX_BAR_HEIGHT = 34; // px
const MIN_BAR_HEIGHT = 4; // px

const hud = document.getElementById("hud") as HTMLDivElement;
const statusText = document.getElementById("status-text") as HTMLSpanElement;
const modeChip = document.getElementById("mode-chip") as HTMLSpanElement;
const closeBtn = document.getElementById("close-btn") as HTMLButtonElement;
const viz = document.getElementById("viz") as HTMLDivElement;

let currentStatus: AppStatus = "idle";
let settings: Settings | null = null;
/** Last payload, replayed when the UI language changes live. */
let lastStatusPayload: StatusChangedPayload = { status: "recording" };

// ---------------------------------------------------------------------------
// visualizer
// ---------------------------------------------------------------------------

const bars: HTMLDivElement[] = [];
const targets = new Array<number>(BAR_COUNT).fill(0);
const heights = new Array<number>(BAR_COUNT).fill(0);
/** per-bar weighting: taller in the middle, like a voice waveform */
const weights = Array.from({ length: BAR_COUNT }, (_, i) => {
  const t = i / (BAR_COUNT - 1);
  return 0.45 + 0.55 * Math.sin(Math.PI * t);
});

function buildBars(): void {
  for (let i = 0; i < BAR_COUNT; i++) {
    const bar = document.createElement("div");
    bar.className = "bar";
    bar.style.setProperty("--i", String(i));
    bar.style.height = `${MIN_BAR_HEIGHT}px`;
    viz.appendChild(bar);
    bars.push(bar);
  }
}

function onLevel(rms: number): void {
  const level = Math.min(1, Math.max(0, rms));
  // perceptual boost so quiet speech is still visible
  const boosted = Math.pow(level, 0.6);
  for (let i = 0; i < BAR_COUNT; i++) {
    const jitter = 0.72 + Math.random() * 0.55;
    targets[i] = boosted * weights[i] * jitter;
  }
}

function animate(): void {
  const cssDriven = currentStatus === "transcribing" || currentStatus === "polishing";
  for (let i = 0; i < BAR_COUNT; i++) {
    // targets decay so bars fall between level events / after recording stops
    targets[i] *= 0.94;
    heights[i] += (targets[i] - heights[i]) * 0.38;
    if (!cssDriven) {
      const px = MIN_BAR_HEIGHT + heights[i] * (MAX_BAR_HEIGHT - MIN_BAR_HEIGHT);
      bars[i].style.height = `${px.toFixed(1)}px`;
    }
  }
  requestAnimationFrame(animate);
}

function resetBars(): void {
  for (let i = 0; i < BAR_COUNT; i++) {
    targets[i] = 0;
    heights[i] = 0;
    bars[i].style.height = `${MIN_BAR_HEIGHT}px`;
  }
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

function applyStatus(payload: StatusChangedPayload): void {
  lastStatusPayload = payload;
  currentStatus = payload.status;
  hud.dataset.status = payload.status;
  switch (payload.status) {
    case "recording":
      resetBars();
      statusText.textContent = t("overlay.recording");
      break;
    case "transcribing":
      statusText.textContent = t("overlay.transcribing");
      break;
    case "polishing":
      statusText.textContent = t("overlay.polishing");
      break;
    case "done":
      statusText.textContent = payload.message
        ? t("overlay.doneWith", tMsg(payload.message))
        : t("overlay.done");
      break;
    case "error":
      statusText.textContent = payload.message ? tMsg(payload.message) : t("overlay.error");
      break;
    case "idle":
      statusText.textContent = t("overlay.idle");
      resetBars();
      break;
  }
  // clear CSS animation heights when leaving processing states
  if (payload.status !== "transcribing" && payload.status !== "polishing") {
    for (const bar of bars) bar.style.removeProperty("animation");
  }
}

function updateModeChip(): void {
  if (!settings) {
    modeChip.textContent = "";
    return;
  }
  const mode = settings.modes.find((m) => m.id === settings?.active_mode_id);
  modeChip.textContent = mode?.name ?? "";
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

async function init(): Promise<void> {
  buildBars();
  requestAnimationFrame(animate);

  closeBtn.addEventListener("click", async () => {
    try {
      await invoke("cancel_recording");
    } catch {
      // backend hides the overlay; nothing to surface here
    }
  });

  await listen<StatusChangedPayload>("status-changed", (event) => {
    applyStatus(event.payload);
  });

  await listen<LevelPayload>("level", (event) => {
    onLevel(event.payload.rms);
  });

  await listen<Settings>("settings-changed", (event) => {
    settings = event.payload;
    // Follow live UI-language changes made in the settings window.
    if (settings.ui_lang !== getLang()) {
      setLang(settings.ui_lang);
      applyDom();
      applyStatus(lastStatusPayload);
    }
    updateModeChip();
  });

  try {
    settings = await invoke<Settings>("get_settings");
    // ui_lang arrives already resolved to a concrete code by the backend.
    setLang(settings.ui_lang);
    updateModeChip();
  } catch {
    // non-fatal: chip stays hidden, language stays at default
  }
  applyDom();
}

window.addEventListener("DOMContentLoaded", () => {
  void init();
});
