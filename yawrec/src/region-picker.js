// ============================================================
// YAWREC · region-picker.js
// Overlay plein écran transparent — l'utilisateur dessine un
// rectangle de capture. Les coordonnées sont envoyées en pixels
// physiques (logique × devicePixelRatio) pour correspondre au
// référentiel DXGI.
// ============================================================

import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

const appWindow = getCurrentWindow();

const canvas   = document.getElementById("canvas");
const hint     = document.getElementById("hint");
const toolbar  = document.getElementById("toolbar");
const dimsLabel = document.getElementById("dims-label");

// Résolution physique du canvas = résolution logique × DPR
const dpr = window.devicePixelRatio || 1;
const W   = window.innerWidth;
const H   = window.innerHeight;

canvas.width        = W * dpr;
canvas.height       = H * dpr;
canvas.style.width  = W + "px";
canvas.style.height = H + "px";

const ctx = canvas.getContext("2d");
ctx.scale(dpr, dpr);

// ---- état du dessin ----
let startX = 0, startY = 0;
let curX   = 0, curY   = 0;
let isDragging   = false;
let hasSelection = false;

// ---- dessin ----
function draw() {
  ctx.clearRect(0, 0, W, H);

  // Overlay sombre
  ctx.fillStyle = "rgba(0,0,0,0.48)";
  ctx.fillRect(0, 0, W, H);

  if (!isDragging && !hasSelection) return;

  const rx = Math.min(startX, curX);
  const ry = Math.min(startY, curY);
  const rw = Math.abs(curX - startX);
  const rh = Math.abs(curY - startY);
  if (rw < 1 || rh < 1) return;

  // Découpe la zone sélectionnée (transparent → on voit l'écran)
  ctx.globalCompositeOperation = "destination-out";
  ctx.fillStyle = "rgba(0,0,0,1)";
  ctx.fillRect(rx, ry, rw, rh);
  ctx.globalCompositeOperation = "source-over";

  // Bordure bleue
  ctx.strokeStyle = "#60a5fa";
  ctx.lineWidth   = 1.5;
  ctx.strokeRect(rx + 0.75, ry + 0.75, rw - 1.5, rh - 1.5);

  // Poignées de coin
  const sz = 6;
  ctx.fillStyle = "#60a5fa";
  [[rx, ry], [rx+rw, ry], [rx, ry+rh], [rx+rw, ry+rh]].forEach(([cx, cy]) => {
    ctx.fillRect(cx - sz/2, cy - sz/2, sz, sz);
  });

  // Dimensions en pixels physiques
  const pw = Math.round(rw * dpr);
  const ph = Math.round(rh * dpr);
  dimsLabel.textContent = `${pw} × ${ph}`;
}

// ---- positionnement du toolbar ----
function showToolbar() {
  const rx = Math.min(startX, curX);
  const ry = Math.min(startY, curY);
  const rw = Math.abs(curX - startX);
  const rh = Math.abs(curY - startY);

  let tx = Math.max(rx, 4);
  let ty = ry + rh + 10;
  if (ty + 44 > H) ty = ry - 50;
  if (ty < 4) ty = 4;
  if (tx + 220 > W) tx = W - 224;

  toolbar.style.left = tx + "px";
  toolbar.style.top  = ty + "px";
  toolbar.classList.add("visible");
}

// ---- événements souris ----
canvas.addEventListener("mousedown", (e) => {
  startX = e.clientX; startY = e.clientY;
  curX   = e.clientX; curY   = e.clientY;
  isDragging   = true;
  hasSelection = false;
  toolbar.classList.remove("visible");
  hint.style.display = "none";
  draw();
});

canvas.addEventListener("mousemove", (e) => {
  if (!isDragging) return;
  curX = e.clientX; curY = e.clientY;
  draw();
});

canvas.addEventListener("mouseup", (e) => {
  curX = e.clientX; curY = e.clientY;
  isDragging = false;
  const rw = Math.abs(curX - startX);
  const rh = Math.abs(curY - startY);
  if (rw >= 10 && rh >= 10) {
    hasSelection = true;
    showToolbar();
  }
  draw();
});

// ---- confirmation / annulation ----
async function confirm() {
  if (!hasSelection) return;
  const rx = Math.round(Math.min(startX, curX) * dpr);
  const ry = Math.round(Math.min(startY, curY) * dpr);
  const rw = (Math.round(Math.abs(curX - startX) * dpr)) & ~1; // arrondi pair H.264
  const rh = (Math.round(Math.abs(curY - startY) * dpr)) & ~1;
  if (rw < 2 || rh < 2) return;
  await invoke("set_region", { x: rx, y: ry, w: rw, h: rh });
  await appWindow.close();
}

async function cancel() {
  await appWindow.close();
}

document.getElementById("btn-confirm").addEventListener("click", confirm);
document.getElementById("btn-cancel").addEventListener("click", cancel);

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") cancel();
  if (e.key === "Enter" && hasSelection) confirm();
});

// Dessin initial (overlay noir sans sélection)
draw();
