// ============================================================
// YAWREC · main.js
// Bootstrap frontend : connexion API Tauri, contrôles fenêtre,
// logique d'enregistrement, popovers audio/webcam/écran.
// ============================================================

import { getCurrentWindow } from "@tauri-apps/api/window";
import { Recorder } from "./recorder.js";

const appWindow = getCurrentWindow();

// ---------- Boutons système (titlebar custom) ----------
document.getElementById("btn-minimize").addEventListener("click", () => {
  appWindow.minimize();
});
document.getElementById("btn-maximize").addEventListener("click", () => {
  appWindow.toggleMaximize();
});
document.getElementById("btn-close").addEventListener("click", async () => {
  await appWindow.close();
});

// ---------- Logique d'enregistrement ----------
const recorder = new Recorder({
  onPhaseChange: (phase) => {
    const win = document.querySelector(".win-window");
    win.classList.toggle("recording", phase === "recording");
    win.classList.toggle("paused", phase === "paused");

    const recBtn = document.getElementById("btn-rec");
    recBtn.setAttribute(
      "aria-label",
      phase === "recording" ? "Arrêter l'enregistrement" : "Démarrer l'enregistrement"
    );

    // Frame count : visible seulement pendant enregistrement/pause
    const fcItem = document.getElementById("frame-count-item");
    fcItem.style.display = (phase === "recording" || phase === "paused") ? "" : "none";

    // Hint pause dans le footer
    const pauseHint = document.getElementById("pause-hint");
    if (pauseHint) {
      pauseHint.style.display = (phase === "recording" || phase === "paused") ? "" : "none";
    }

    // Icône pause / play dans le bouton pause
    document.getElementById("icon-pause").style.display = phase === "paused" ? "none" : "";
    document.getElementById("icon-resume").style.display = phase === "paused" ? "" : "none";
  },
  onTick: ({ elapsed, sizeHuman, frameCount }) => {
    document.getElementById("timer-text").textContent = elapsed;
    document.getElementById("size-text").textContent = sizeHuman;
    if (frameCount !== undefined) {
      document.getElementById("frame-count-text").textContent = `${frameCount} frames`;
    }
  },
  onError: (err) => {
    console.error("[YawREC]", err);
    const timerEl = document.getElementById("timer-text");
    timerEl.textContent = "ERREUR";
    timerEl.title = typeof err === "string" ? err : JSON.stringify(err);
    setTimeout(() => {
      timerEl.textContent = "00:00:00";
      timerEl.title = "";
    }, 4000);
  },
});

// Bouton REC central
document.getElementById("btn-rec").addEventListener("click", () => {
  closeAllPopovers();
  if (recorder.phase === "idle") recorder.start();
  else recorder.stop();
});

// Bouton Pause
document.getElementById("btn-pause").addEventListener("click", () => {
  if (recorder.phase === "recording") recorder.pause();
  else if (recorder.phase === "paused") recorder.resume();
});

// ============================================================
// GESTION POPOVERS
// ============================================================

let currentPopover = null;

function openPopover(id) {
  if (currentPopover === id) return;
  if (currentPopover) closeAllPopovers();
  const el = document.getElementById(id);
  if (!el) return;
  el.classList.add("visible");
  el.setAttribute("aria-hidden", "false");
  currentPopover = id;
}

function closeAllPopovers() {
  document.querySelectorAll(".popover.visible").forEach((el) => {
    el.classList.remove("visible");
    el.setAttribute("aria-hidden", "true");
  });
  currentPopover = null;
}

// Clic en dehors d'un popover → ferme
document.querySelector(".win-body").addEventListener("mousedown", (e) => {
  if (!currentPopover) return;
  const pop = document.getElementById(currentPopover);
  if (pop && !pop.contains(e.target)) {
    // Check if click is on one of the control anchors (would re-open)
    const anchors = ["audio-control", "webcam-control"];
    const onAnchor = anchors.some((id) => {
      const a = document.getElementById(id);
      return a && a.contains(e.target);
    });
    if (!onAnchor) closeAllPopovers();
  }
});

// Boutons de fermeture
document.querySelectorAll(".pop-close").forEach((btn) => {
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    closeAllPopovers();
  });
});

// ============================================================
// AUDIO — popover
// ============================================================

let micEnabled = true;
let loopbackEnabled = false; // OFF par défaut — activer manuellement si besoin
let selectedMicName = null; // null = device par défaut

document.getElementById("audio-control").addEventListener("click", async (e) => {
  if (currentPopover === "popover-audio") {
    closeAllPopovers();
    return;
  }
  await populateAudioPopover();
  openPopover("popover-audio");
});

async function populateAudioPopover() {
  const devices = await recorder.listAudioDevices();
  const micList = document.getElementById("mic-devices");
  micList.innerHTML = "";

  const mics = devices.filter((d) => d.id.startsWith("input::"));
  mics.forEach((d) => {
    const rawName = d.id.slice("input::".length);
    const el = document.createElement("div");
    el.className = "pop-device" + (selectedMicName === rawName ? " selected" : "");
    el.textContent = rawName;
    el.addEventListener("click", async (e) => {
      e.stopPropagation();
      selectedMicName = rawName;
      micList.querySelectorAll(".pop-device").forEach((x) => x.classList.remove("selected"));
      el.classList.add("selected");
      await recorder.setAudioConfig(micEnabled, loopbackEnabled, rawName);
    });
    micList.appendChild(el);
  });

  micList.style.display = micEnabled && mics.length > 0 ? "" : "none";
}

document.getElementById("chk-mic").addEventListener("change", async (e) => {
  micEnabled = e.target.checked;
  const micList = document.getElementById("mic-devices");
  micList.style.display = micEnabled && micList.children.length > 0 ? "" : "none";
  await recorder.setAudioConfig(micEnabled, loopbackEnabled, selectedMicName);
});

document.getElementById("chk-loopback").addEventListener("change", async (e) => {
  loopbackEnabled = e.target.checked;
  await recorder.setAudioConfig(micEnabled, loopbackEnabled, selectedMicName);
});

// ============================================================
// WEBCAM — popover
// ============================================================

let webcamEnabled = false;
let selectedWebcamId = null;

document.getElementById("webcam-control").addEventListener("click", async (e) => {
  if (currentPopover === "popover-webcam") {
    closeAllPopovers();
    return;
  }
  await populateWebcamPopover();
  openPopover("popover-webcam");
});

async function populateWebcamPopover() {
  const devices = await recorder.listWebcams();
  const list = document.getElementById("webcam-devices");
  list.innerHTML = "";

  if (devices.length === 0) {
    const el = document.createElement("div");
    el.className = "pop-device";
    el.style.fontStyle = "italic";
    el.style.pointerEvents = "none";
    el.textContent = "Aucune webcam détectée";
    list.appendChild(el);
  } else {
    devices.forEach((d) => {
      const el = document.createElement("div");
      const devId = parseInt(d.id, 10);
      el.className = "pop-device" + (selectedWebcamId === devId ? " selected" : "");
      el.textContent = d.name.replace(/^📷\s*/, "");
      el.addEventListener("click", async (e) => {
        e.stopPropagation();
        selectedWebcamId = devId;
        list.querySelectorAll(".pop-device").forEach((x) => x.classList.remove("selected"));
        el.classList.add("selected");
        await recorder.setWebcam(devId);
      });
      list.appendChild(el);
    });
  }

  list.style.display = webcamEnabled ? "" : "none";
}

document.getElementById("chk-webcam").addEventListener("change", async (e) => {
  webcamEnabled = e.target.checked;
  document.getElementById("webcam-control").classList.toggle("webcam-active", webcamEnabled);
  document.getElementById("webcam-devices").style.display = webcamEnabled ? "" : "none";
  await recorder.setWebcamEnabled(webcamEnabled);
});

// ============================================================
// ÉCRAN — mode de capture + popover
// ============================================================

let selectedScreenId = null;

// Mode de capture (segmented control)
document.querySelectorAll("#mode-seg button[data-mode]").forEach((btn) => {
  btn.addEventListener("click", async () => {
    document.querySelectorAll("#mode-seg button").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    await recorder.setMode(btn.dataset.mode);

    // Pour le mode plein écran, afficher le sélecteur d'écran
    if (btn.dataset.mode === "fullscreen") {
      await populateScreenPopover();
      openPopover("popover-screen");
    }
  });
});

async function populateScreenPopover() {
  const screens = await recorder.listScreens();
  const list = document.getElementById("screen-devices");
  list.innerHTML = "";

  if (screens.length <= 1 && screens.length > 0) {
    // Un seul écran : le sélectionner automatiquement et ne pas ouvrir le popover
    selectedScreenId = screens[0].id;
    await recorder.setScreen(screens[0].id);
    return;
  }

  screens.forEach((s) => {
    const el = document.createElement("div");
    const primary = s.primary ? " · Principal" : "";
    el.className = "pop-device" + (selectedScreenId === s.id ? " selected" : "");
    el.textContent = `Écran ${s.id + 1} — ${s.width}×${s.height}${primary}`;
    el.addEventListener("click", async (e) => {
      e.stopPropagation();
      selectedScreenId = s.id;
      list.querySelectorAll(".pop-device").forEach((x) => x.classList.remove("selected"));
      el.classList.add("selected");
      await recorder.setScreen(s.id);
      closeAllPopovers();
    });
    list.appendChild(el);
  });
}

// ============================================================
// DOSSIER DE SORTIE
// ============================================================

document.getElementById("output-path-item").addEventListener("click", async () => {
  closeAllPopovers();
  const path = await recorder.pickOutputDirectory();
  if (path) {
    document.getElementById("output-path-text").textContent = path;
  }
});

// ============================================================
// INITIALISATION
// ============================================================

async function init() {
  await recorder.init();

  // Afficher le dossier de sortie par défaut
  const outDir = await recorder.getOutputDirectory();
  if (outDir) {
    document.getElementById("output-path-text").textContent = outDir;
  }

  // Encoder actif dans le footer
  const info = await recorder.getActiveEncoder();
  if (info) {
    document.getElementById("encoder-text").textContent = info.hardware
      ? `MP4 — ${info.display_name}`
      : `MP4 — ${info.display_name} (CPU)`;
    document.getElementById("encoder-pill").title = info.codec;
  }

  // Griser les boutons non implémentés (fenêtre, région)
  document.querySelectorAll(
    '#mode-seg button[data-mode="window"], #mode-seg button[data-mode="region"]'
  ).forEach((btn) => {
    btn.classList.add("unimplemented");
    btn.title = "Non disponible dans cette version";
  });

  // Sync état initial des toggles avec les valeurs par défaut
  document.getElementById("chk-mic").checked = micEnabled;        // true
  document.getElementById("chk-loopback").checked = loopbackEnabled; // false
  document.getElementById("chk-webcam").checked = webcamEnabled;  // false

  // Ajouter le raccourci pause dans le footer
  const shortcutHint = document.querySelector(".win-footer .ft-item:last-child");
  if (shortcutHint) {
    shortcutHint.insertAdjacentHTML("beforebegin",
      `<span class="ft-item" id="pause-hint" style="display:none">
        <span>Pause</span>
        <span class="kbd">Ctrl</span>
        <span class="kbd">Shift</span>
        <span class="kbd">P</span>
      </span>`
    );
  }
}

init();

console.log("[YawREC] UI prête");
