// ============================================================
// YAWREC · main.js
// Bootstrap frontend : connexion API Tauri, contrôles fenêtre,
// logique d'enregistrement, popovers audio/webcam/écran.
// ============================================================

import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { Recorder } from "./recorder.js";

const appWindow = getCurrentWindow();

// ============================================================
// VU-MÈTRE — animation rAF pilotée par recorder://vu (100 ms)
// ============================================================

let _vuTarget  = 0.0; // niveau reçu de Rust
let _vuCurrent = 0.0; // niveau affiché (lissé)

listen("recorder://vu", (e) => {
  _vuTarget = typeof e.payload === "number" ? e.payload : 0;
});

(function vuLoop() {
  // Ballistics JS : attaque rapide (~50 ms), déclin lent (~600 ms à 60 fps)
  const alpha = _vuTarget > _vuCurrent ? 0.45 : 0.055;
  _vuCurrent += (_vuTarget - _vuCurrent) * alpha;

  const lv = _vuCurrent;

  // 3 segments : b1 (bas/vert), b2 (milieu/jaune à partir de 55%), b3 (haut/rouge à partir de 80%)
  const s1 = Math.min(lv / 0.45, 1.0);
  const s2 = Math.max(0, Math.min((lv - 0.30) / 0.45, 1.0));
  const s3 = Math.max(0, Math.min((lv - 0.65) / 0.35, 1.0));

  const bars = document.querySelectorAll(".vu .bar");
  if (bars.length === 3) {
    bars[0].style.transform = `scaleY(${Math.max(s1, 0.04).toFixed(3)})`;
    bars[1].style.transform = `scaleY(${Math.max(s2, 0.04).toFixed(3)})`;
    bars[2].style.transform = `scaleY(${Math.max(s3, 0.04).toFixed(3)})`;
    // Couleurs zonées
    bars[1].style.background = s2 > 0.01 ? "#fbbf24" : "";       // jaune
    bars[2].style.background = s3 > 0.01 ? "var(--rec-500)" : ""; // rouge
  }

  requestAnimationFrame(vuLoop);
})();

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

// ---------- Arrêt automatique ----------
const _savedAutoStop = parseInt(localStorage.getItem("yawrec_autostop"), 10);
let autoStopSecs = isFinite(_savedAutoStop) && _savedAutoStop >= 0 ? _savedAutoStop : 0;

function parseElapsedSecs(elapsed) {
  const parts = elapsed.split(":").map(Number);
  if (parts.length === 3) return parts[0] * 3600 + parts[1] * 60 + parts[2];
  if (parts.length === 2) return parts[0] * 60 + parts[1];
  return 0;
}

// ---------- Logique d'enregistrement ----------
const recorder = new Recorder({
  onSaved: (filePath) => showSavedToast(filePath),
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

    // Reset autostop label quand on revient à idle
    if (phase === "idle") refreshAutoStopLabel();

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
    // Auto-stop countdown (uniquement pendant l'enregistrement, pas la pause)
    if (autoStopSecs > 0 && recorder.phase === "recording") {
      const elapsedS = parseElapsedSecs(elapsed);
      const remaining = autoStopSecs - elapsedS;
      if (remaining <= 0) {
        recorder.stop();
      } else if (remaining <= 60) {
        document.getElementById("autostop-label").textContent = `Arrêt dans ${remaining}s`;
      }
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
document.getElementById("btn-rec").addEventListener("click", async () => {
  closeAllPopovers();
  if (recorder.phase === "idle") {
    // En mode région sans zone définie → ouvrir le picker d'abord
    if (recorder.mode === "region" && !selectedRegion) {
      await recorder.openRegionPicker();
      return;
    }
    recorder.start();
  } else {
    recorder.stop();
  }
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
  const asp = document.getElementById("popover-autostop");
  if (asp) { asp.classList.remove("visible"); asp.setAttribute("aria-hidden", "true"); }
  const outp = document.getElementById("popover-output");
  if (outp) { outp.classList.remove("visible"); outp.setAttribute("aria-hidden", "true"); }
  const qualp = document.getElementById("popover-quality");
  if (qualp) { qualp.classList.remove("visible"); qualp.setAttribute("aria-hidden", "true"); }
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

let micEnabled = localStorage.getItem("yawrec_mic_enabled") !== "false"; // défaut true
let loopbackEnabled = localStorage.getItem("yawrec_loopback") === "true"; // défaut false
let selectedMicName = localStorage.getItem("yawrec_mic") || null;

const _savedGain = parseFloat(localStorage.getItem("yawrec_mic_gain"));
let micGain = isFinite(_savedGain) && _savedGain >= 0 ? _savedGain : 1.0;

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
      localStorage.setItem("yawrec_mic", rawName);
      micList.querySelectorAll(".pop-device").forEach((x) => x.classList.remove("selected"));
      el.classList.add("selected");
      await recorder.setAudioConfig(micEnabled, loopbackEnabled, rawName);
    });
    micList.appendChild(el);
  });

  micList.style.display = micEnabled && mics.length > 0 ? "" : "none";
}

function gainToDb(gain) {
  if (gain < 1e-4) return "-∞ dB";
  const db = 20 * Math.log10(gain);
  return (db >= 0 ? "+" : "") + db.toFixed(1) + " dB";
}

function updateGainLabel(gainLinear) {
  document.getElementById("mic-gain-label").textContent = gainToDb(gainLinear);
}

function setGainRowVisible(visible) {
  document.getElementById("mic-gain-row").classList.toggle("hidden", !visible);
}

document.getElementById("mic-gain-slider").addEventListener("input", async (e) => {
  micGain = parseInt(e.target.value, 10) / 100;
  localStorage.setItem("yawrec_mic_gain", String(micGain));
  updateGainLabel(micGain);
  await recorder.setMicGain(micGain);
});

document.getElementById("chk-mic").addEventListener("change", async (e) => {
  micEnabled = e.target.checked;
  localStorage.setItem("yawrec_mic_enabled", String(micEnabled));
  const micList = document.getElementById("mic-devices");
  micList.style.display = micEnabled && micList.children.length > 0 ? "" : "none";
  setGainRowVisible(micEnabled);
  await recorder.setAudioConfig(micEnabled, loopbackEnabled, selectedMicName);
});

document.getElementById("chk-loopback").addEventListener("change", async (e) => {
  loopbackEnabled = e.target.checked;
  localStorage.setItem("yawrec_loopback", String(loopbackEnabled));
  await recorder.setAudioConfig(micEnabled, loopbackEnabled, selectedMicName);
});

// ============================================================
// WEBCAM — popover
// ============================================================

let webcamEnabled = localStorage.getItem("yawrec_webcam_enabled") === "true"; // défaut false
const _savedWebcamId = localStorage.getItem("yawrec_webcam_id");
let selectedWebcamId = _savedWebcamId !== null ? parseInt(_savedWebcamId, 10) : null;
const PIP_POSITIONS = ["top_left", "top_right", "bottom_left", "bottom_right"];
const _savedPos = localStorage.getItem("yawrec_pip_pos");
let pipPosition = PIP_POSITIONS.includes(_savedPos) ? _savedPos : "bottom_right";

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
        localStorage.setItem("yawrec_webcam_id", String(devId));
        list.querySelectorAll(".pop-device").forEach((x) => x.classList.remove("selected"));
        el.classList.add("selected");
        await recorder.setWebcam(devId);
      });
      list.appendChild(el);
    });
  }

  list.style.display = webcamEnabled ? "" : "none";
  setPipControlsVisible(webcamEnabled);
}

document.getElementById("chk-webcam").addEventListener("change", async (e) => {
  webcamEnabled = e.target.checked;
  localStorage.setItem("yawrec_webcam_enabled", String(webcamEnabled));
  document.getElementById("webcam-control").classList.toggle("webcam-active", webcamEnabled);
  document.getElementById("webcam-devices").style.display = webcamEnabled ? "" : "none";
  setPipControlsVisible(webcamEnabled);
  await recorder.setWebcamEnabled(webcamEnabled);
});

// ============================================================
// POSITION PiP — sélecteur 2×2
// ============================================================

function setPipControlsVisible(visible) {
  document.querySelectorAll(".pip-pos-ctrl").forEach((el) => {
    el.style.display = visible ? "" : "none";
  });
}

function updatePipCornerActive(activePos) {
  document.querySelectorAll(".pip-corner").forEach((b) => {
    const isActive = b.dataset.pos === activePos;
    b.classList.toggle("active", isActive);
    b.setAttribute("aria-pressed", String(isActive));
  });
}

document.querySelectorAll(".pip-corner").forEach((btn) => {
  btn.addEventListener("click", async (e) => {
    e.stopPropagation();
    pipPosition = btn.dataset.pos;
    localStorage.setItem("yawrec_pip_pos", pipPosition);
    updatePipCornerActive(pipPosition);
    await recorder.setPipPosition(pipPosition);
  });
});

// ============================================================
// ÉCRAN — mode de capture + popover
// ============================================================

let selectedScreenId = null;
let selectedHwnd     = null;
let selectedRegion   = null; // { x, y, w, h } en pixels physiques, null si non défini

// Mode de capture (segmented control)
document.querySelectorAll("#mode-seg button[data-mode]").forEach((btn) => {
  btn.addEventListener("click", async () => {
    document.querySelectorAll("#mode-seg button").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    await recorder.setMode(btn.dataset.mode);
    localStorage.setItem("yawrec_capture_mode", btn.dataset.mode);

    // Pour le mode plein écran, afficher le sélecteur d'écran (sauf si 1 seul écran)
    if (btn.dataset.mode === "fullscreen") {
      const needsPicker = await populateScreenPopover();
      if (needsPicker) openPopover("popover-screen");
    } else if (btn.dataset.mode === "window") {
      await populateWindowPopover();
      openPopover("popover-window");
    } else if (btn.dataset.mode === "region") {
      await recorder.openRegionPicker();
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
    return false;
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
  return true;
}

async function populateWindowPopover() {
  const windows = await recorder.listWindows();
  const list = document.getElementById("window-devices");
  list.innerHTML = "";

  if (windows.length === 0) {
    const el = document.createElement("div");
    el.className = "pop-device";
    el.style.fontStyle = "italic";
    el.style.pointerEvents = "none";
    el.textContent = "Aucune fenêtre détectée";
    list.appendChild(el);
    return;
  }

  windows.forEach((w) => {
    const el = document.createElement("div");
    el.className = "pop-device" + (selectedHwnd === w.hwnd ? " selected" : "");
    el.textContent = `${w.name}  (${w.width}×${w.height})`;
    el.title = w.name;
    el.addEventListener("click", async (e) => {
      e.stopPropagation();
      selectedHwnd = w.hwnd;
      list.querySelectorAll(".pop-device").forEach((x) => x.classList.remove("selected"));
      el.classList.add("selected");
      await recorder.setWindowHwnd(w.hwnd);
      closeAllPopovers();
    });
    list.appendChild(el);
  });
}

// ============================================================
// QUALITÉ VIDÉO — popover encodeur
// ============================================================

const _savedFps     = parseInt(localStorage.getItem("yawrec_fps"), 10);
const _savedBitrate = parseInt(localStorage.getItem("yawrec_bitrate"), 10);
let videoFps         = isFinite(_savedFps)     && _savedFps     > 0  ? _savedFps     : 30;
let videoBitrateKbps = isFinite(_savedBitrate) && _savedBitrate > 0  ? _savedBitrate : 8000;
let _encoderName     = "Encodeur";

function updateQualityActive() {
  document.querySelectorAll(".quality-preset").forEach((btn) => {
    btn.classList.toggle("active", parseInt(btn.dataset.kbps, 10) === videoBitrateKbps);
  });
  document.querySelectorAll(".fps-btn").forEach((btn) => {
    btn.classList.toggle("active", parseInt(btn.dataset.fps, 10) === videoFps);
  });
}

function updateEncoderPillLabel(encoderName) {
  const mbps = videoBitrateKbps >= 1000
    ? (videoBitrateKbps / 1000).toFixed(0) + " Mbps"
    : videoBitrateKbps + " kbps";
  document.getElementById("encoder-text").textContent =
    `${encoderName} · ${mbps} · ${videoFps} fps`;
}

// Ouvrir/fermer le popover qualité depuis la pill encodeur
document.getElementById("encoder-pill").addEventListener("click", (e) => {
  e.stopPropagation();
  const pop = document.getElementById("popover-quality");
  if (pop.classList.contains("visible")) {
    pop.classList.remove("visible");
    pop.setAttribute("aria-hidden", "true");
  } else {
    closeAllPopovers();
    updateQualityActive();
    pop.classList.add("visible");
    pop.setAttribute("aria-hidden", "false");
  }
});

document.getElementById("quality-close-btn").addEventListener("click", (e) => {
  e.stopPropagation();
  const pop = document.getElementById("popover-quality");
  pop.classList.remove("visible");
  pop.setAttribute("aria-hidden", "true");
});

document.addEventListener("mousedown", (e) => {
  const pop  = document.getElementById("popover-quality");
  const pill = document.getElementById("encoder-pill");
  if (!pop.classList.contains("visible")) return;
  if (!pop.contains(e.target) && !pill.contains(e.target)) {
    pop.classList.remove("visible");
    pop.setAttribute("aria-hidden", "true");
  }
});

document.querySelectorAll(".quality-preset").forEach((btn) => {
  btn.addEventListener("click", async (e) => {
    e.stopPropagation();
    videoBitrateKbps = parseInt(btn.dataset.kbps, 10);
    localStorage.setItem("yawrec_bitrate", String(videoBitrateKbps));
    updateQualityActive();
    await recorder.setVideoQuality(videoFps, videoBitrateKbps);
    updateEncoderPillLabel(_encoderName);
  });
});

document.querySelectorAll(".fps-btn").forEach((btn) => {
  btn.addEventListener("click", async (e) => {
    e.stopPropagation();
    videoFps = parseInt(btn.dataset.fps, 10);
    localStorage.setItem("yawrec_fps", String(videoFps));
    updateQualityActive();
    await recorder.setVideoQuality(videoFps, videoBitrateKbps);
    updateEncoderPillLabel(_encoderName);
  });
});

// ============================================================
// TOAST — enregistrement sauvegardé
// ============================================================

let _toastTimer = null;
let _toastPath  = null;

function showSavedToast(filePath) {
  _toastPath = filePath;
  const sep = filePath.includes("\\") ? "\\" : "/";
  const filename = filePath.split(sep).pop();
  document.getElementById("toast-filename").textContent = filename;

  const toast = document.getElementById("toast-saved");
  // Reset animation en retirant puis rajoutant la classe
  toast.classList.remove("visible");
  void toast.offsetWidth; // force reflow
  toast.classList.add("visible");

  clearTimeout(_toastTimer);
  _toastTimer = setTimeout(dismissToast, 8000);
}

function dismissToast() {
  const toast = document.getElementById("toast-saved");
  toast.classList.remove("visible");
  clearTimeout(_toastTimer);
}

document.getElementById("toast-open-btn").addEventListener("click", async () => {
  if (_toastPath) await recorder.revealInExplorer(_toastPath);
  dismissToast();
});

document.getElementById("toast-dismiss-btn").addEventListener("click", () => {
  dismissToast();
});

// ============================================================
// ARRÊT AUTOMATIQUE
// ============================================================

function refreshAutoStopLabel() {
  const label = document.getElementById("autostop-label");
  const item  = document.getElementById("autostop-item");
  if (autoStopSecs > 0) {
    const h = Math.floor(autoStopSecs / 3600);
    const m = Math.floor((autoStopSecs % 3600) / 60);
    label.textContent = h > 0 ? `${h}h${m > 0 ? m + "min" : ""}` : `${m} min`;
    item.classList.add("autostop-active");
  } else {
    label.textContent = "Arrêt automatique";
    item.classList.remove("autostop-active");
  }
}

function updateAutoStopPresetActive() {
  document.querySelectorAll(".as-preset").forEach((btn) => {
    btn.classList.toggle("active", parseInt(btn.dataset.secs, 10) === autoStopSecs);
  });
}

function setAutoStop(secs) {
  autoStopSecs = secs;
  localStorage.setItem("yawrec_autostop", String(secs));
  refreshAutoStopLabel();
  updateAutoStopPresetActive();
}

// Footer item → ouvrir/fermer le popover autostop
document.getElementById("autostop-item").addEventListener("click", (e) => {
  e.stopPropagation();
  const pop = document.getElementById("popover-autostop");
  if (pop.classList.contains("visible")) {
    pop.classList.remove("visible");
    pop.setAttribute("aria-hidden", "true");
  } else {
    closeAllPopovers();
    updateAutoStopPresetActive();
    pop.classList.add("visible");
    pop.setAttribute("aria-hidden", "false");
  }
});

// Bouton ✕ dans le popover
document.getElementById("autostop-close-btn").addEventListener("click", (e) => {
  e.stopPropagation();
  const pop = document.getElementById("popover-autostop");
  pop.classList.remove("visible");
  pop.setAttribute("aria-hidden", "true");
});

// Clic en dehors → fermer (sur le document, pour capturer hors .win-body)
document.addEventListener("mousedown", (e) => {
  const pop  = document.getElementById("popover-autostop");
  const item = document.getElementById("autostop-item");
  if (!pop.classList.contains("visible")) return;
  if (!pop.contains(e.target) && !item.contains(e.target)) {
    pop.classList.remove("visible");
    pop.setAttribute("aria-hidden", "true");
  }
});

// Boutons presets
document.querySelectorAll(".as-preset").forEach((btn) => {
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    setAutoStop(parseInt(btn.dataset.secs, 10));
  });
});

// Input personnalisé — Enter ou blur
const _asInput = document.getElementById("autostop-custom-min");
_asInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    const mins = parseInt(e.target.value, 10);
    if (isFinite(mins) && mins >= 1) setAutoStop(mins * 60);
    e.target.value = "";
    e.target.blur();
  }
});
_asInput.addEventListener("blur", (e) => {
  const mins = parseInt(e.target.value, 10);
  if (isFinite(mins) && mins >= 1) setAutoStop(mins * 60);
  e.target.value = "";
});

// ============================================================
// SORTIE — dossier + préfixe du nom de fichier
// ============================================================

let filenamePrefix = localStorage.getItem("yawrec_prefix") || "YawREC";

function buildOutputPreview(prefix) {
  const now = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  const dateStr = `${now.getFullYear()}-${pad(now.getMonth()+1)}-${pad(now.getDate())}`;
  const timeStr = `${pad(now.getHours())}-${pad(now.getMinutes())}-${pad(now.getSeconds())}`;
  return `${prefix || "YawREC"}_${dateStr}_${timeStr}.mp4`;
}

function refreshOutputPopover() {
  const pathEl   = document.getElementById("output-path-text");
  document.getElementById("output-dir-display").textContent = pathEl.textContent;
  document.getElementById("output-prefix-input").value      = filenamePrefix;
  document.getElementById("output-preview").textContent     = buildOutputPreview(filenamePrefix);
}

// Ouvrir/fermer le popover depuis l'item footer
document.getElementById("output-path-item").addEventListener("click", (e) => {
  e.stopPropagation();
  const pop = document.getElementById("popover-output");
  if (pop.classList.contains("visible")) {
    pop.classList.remove("visible");
    pop.setAttribute("aria-hidden", "true");
  } else {
    closeAllPopovers();
    refreshOutputPopover();
    pop.classList.add("visible");
    pop.setAttribute("aria-hidden", "false");
  }
});

document.getElementById("output-close-btn").addEventListener("click", (e) => {
  e.stopPropagation();
  const pop = document.getElementById("popover-output");
  pop.classList.remove("visible");
  pop.setAttribute("aria-hidden", "true");
});

// Clic en dehors → fermer
document.addEventListener("mousedown", (e) => {
  const pop  = document.getElementById("popover-output");
  const item = document.getElementById("output-path-item");
  if (!pop.classList.contains("visible")) return;
  if (!pop.contains(e.target) && !item.contains(e.target)) {
    pop.classList.remove("visible");
    pop.setAttribute("aria-hidden", "true");
  }
});

// Bouton "Changer" → sélecteur de dossier
document.getElementById("output-dir-btn").addEventListener("click", async (e) => {
  e.stopPropagation();
  const path = await recorder.pickOutputDirectory();
  if (path) {
    document.getElementById("output-path-text").textContent = path;
    document.getElementById("output-dir-display").textContent = path;
  }
});

// Input préfixe — mise à jour live du preview
document.getElementById("output-prefix-input").addEventListener("input", (e) => {
  document.getElementById("output-preview").textContent = buildOutputPreview(e.target.value);
});

// Appliquer sur Enter ou blur
async function applyPrefix(value) {
  const cleaned = value.trim() || "YawREC";
  filenamePrefix = cleaned;
  localStorage.setItem("yawrec_prefix", cleaned);
  document.getElementById("output-preview").textContent = buildOutputPreview(cleaned);
  document.getElementById("output-path-text").textContent = cleaned + "…";
  await recorder.setFilenamePrefix(cleaned);
}

document.getElementById("output-prefix-input").addEventListener("keydown", async (e) => {
  if (e.key === "Enter") { await applyPrefix(e.target.value); e.target.blur(); }
});
document.getElementById("output-prefix-input").addEventListener("blur", async (e) => {
  await applyPrefix(e.target.value);
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
    _encoderName = info.hardware ? info.display_name : `${info.display_name} (CPU)`;
    document.getElementById("encoder-pill").title = info.codec;
    updateEncoderPillLabel(_encoderName);
  }

  // Appliquer la qualité et le préfixe sauvegardés au backend
  await recorder.setVideoQuality(videoFps, videoBitrateKbps);
  updateQualityActive();
  await recorder.setFilenamePrefix(filenamePrefix);

  // Le bouton région est maintenant actif (plus de classe "unimplemented")

  // Auto-sélection du microphone : si rien de sauvegardé ou device disparu,
  // prendre le premier mic disponible et l'appliquer immédiatement au backend.
  const allDevices = await recorder.listAudioDevices();
  const micNames = allDevices
    .filter((d) => d.id.startsWith("input::"))
    .map((d) => d.id.slice("input::".length));
  if (micNames.length > 0 && (selectedMicName === null || !micNames.includes(selectedMicName))) {
    selectedMicName = micNames[0];
    localStorage.setItem("yawrec_mic", selectedMicName);
  }
  await recorder.setAudioConfig(micEnabled, loopbackEnabled, selectedMicName);

  // Sync toggles avec les valeurs restaurées depuis localStorage
  document.getElementById("chk-mic").checked = micEnabled;
  document.getElementById("chk-loopback").checked = loopbackEnabled;
  document.getElementById("chk-webcam").checked = webcamEnabled;

  // Restaurer l'état webcam
  if (webcamEnabled) {
    document.getElementById("webcam-control").classList.add("webcam-active");
    setPipControlsVisible(true);
    await recorder.setWebcamEnabled(true);
    if (selectedWebcamId !== null) {
      await recorder.setWebcam(selectedWebcamId);
    }
  }

  // Restaurer le mode de capture
  const savedMode = localStorage.getItem("yawrec_capture_mode") || "fullscreen";
  const savedModeBtn = document.querySelector(`#mode-seg button[data-mode="${savedMode}"]`);
  if (savedModeBtn && !savedModeBtn.classList.contains("unimplemented")) {
    document.querySelectorAll("#mode-seg button").forEach((b) => b.classList.remove("active"));
    savedModeBtn.classList.add("active");
    await recorder.setMode(savedMode);
    if (savedMode === "fullscreen") {
      await populateScreenPopover(); // auto-sélection si 1 écran
    }
  }

  // Gain micro — restaurer la valeur sauvegardée
  const sliderPct = Math.round(micGain * 100);
  document.getElementById("mic-gain-slider").value = String(sliderPct);
  updateGainLabel(micGain);
  setGainRowVisible(micEnabled);
  await recorder.setMicGain(micGain);

  // Initialiser le sélecteur de position PiP
  updatePipCornerActive(pipPosition);
  await recorder.setPipPosition(pipPosition);
  // Les contrôles de position sont masqués si webcam désactivée
  setPipControlsVisible(webcamEnabled);

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

  // Restaurer l'état d'arrêt automatique
  refreshAutoStopLabel();
  updateAutoStopPresetActive();

  // Écouter la confirmation du picker de région
  await listen("recorder://region-set", (e) => {
    const { w, h } = e.payload;
    selectedRegion = e.payload;
    document.getElementById("capture-lbl").textContent = `${w} × ${h}`;
    // Activer le bouton région dans le segmented control si ce n'est pas déjà fait
    document.querySelectorAll("#mode-seg button").forEach((b) => b.classList.remove("active"));
    const regionBtn = document.querySelector('#mode-seg button[data-mode="region"]');
    if (regionBtn) regionBtn.classList.add("active");
    recorder.mode = "region";
  });
}

init();

console.log("[YawREC] UI prête");
