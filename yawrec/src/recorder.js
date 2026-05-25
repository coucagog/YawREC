// ============================================================
// YAWREC · recorder.js
// Machine d'état UI + pont vers Rust (invoke + listen).
// Toutes les commandes Rust sont définies dans src-tauri/src/commands.rs
// ============================================================

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

export class Recorder {
  constructor({ onPhaseChange, onTick, onError } = {}) {
    this.phase = "idle"; // 'idle' | 'recording' | 'paused'
    this.mode  = "fullscreen";
    this.outputDir = null;
    this._unlistenTick    = null;
    this._unlistenStopped = null;

    this._onPhaseChange = onPhaseChange || (() => {});
    this._onTick        = onTick        || (() => {});
    this._onError       = onError       || (() => {});
  }

  async init() {
    // Écoute des events Rust (cf. C3 dans la roadmap)
    this._unlistenTick = await listen("recorder://tick", (e) => {
      const { phase, elapsed, size_human, frame_count } = e.payload;
      this._onTick({ elapsed, sizeHuman: size_human, frameCount: frame_count });
      if (phase !== this.phase) {
        this.phase = phase;
        this._onPhaseChange(this.phase);
      }
    });
    this._unlistenStopped = await listen("recorder://stopped", (e) => {
      console.log("[YawREC] enregistrement sauvegardé →", e.payload);
      this.phase = "idle";
      this._onPhaseChange(this.phase);
    });
    this._unlistenError = await listen("recorder://error", (e) => {
      console.error("[YawREC] Erreur :", e.payload);
      // Reset to idle so the REC button works again
      this.phase = "idle";
      this._onPhaseChange(this.phase);
      this._onError(e.payload);
    });
  }

  async destroy() {
    if (this._unlistenTick)    this._unlistenTick();
    if (this._unlistenStopped) this._unlistenStopped();
    if (this._unlistenError)   this._unlistenError();
  }

  // ---------- Commandes principales ----------
  async start() {
    try {
      await invoke("start_recording");
      this.phase = "recording";
      this._onPhaseChange(this.phase);
    } catch (err) {
      this._onError(err);
    }
  }

  async stop() {
    try {
      const path = await invoke("stop_recording");
      this.phase = "idle";
      this._onPhaseChange(this.phase);
      return path;
    } catch (err) {
      this._onError(err);
    }
  }

  async pause() {
    try { await invoke("pause_recording"); this.phase = "paused"; this._onPhaseChange(this.phase); }
    catch (err) { this._onError(err); }
  }

  async resume() {
    try { await invoke("resume_recording"); this.phase = "recording"; this._onPhaseChange(this.phase); }
    catch (err) { this._onError(err); }
  }

  // ---------- Configuration ----------
  async setMode(mode) {
    this.mode = mode;
    try { await invoke("set_capture_mode", { mode }); }
    catch (err) { this._onError(err); }
  }

  async pickOutputDirectory() {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: "Choisir le dossier de sortie YawREC",
    });
    if (selected && typeof selected === "string") {
      this.outputDir = selected;
      try { await invoke("set_output_directory", { path: selected }); }
      catch (err) { this._onError(err); }
      return selected;
    }
    return null;
  }

  // ---------- Énumération devices ----------
  async listAudioDevices() {
    try { return await invoke("list_audio_devices"); }
    catch (err) { this._onError(err); return []; }
  }

  async listWebcams() {
    try { return await invoke("list_webcams"); }
    catch (err) { this._onError(err); return []; }
  }

  async listScreens() {
    try { return await invoke("list_screens"); }
    catch (err) { this._onError(err); return []; }
  }

  async listWindows() {
    try { return await invoke("list_windows"); }
    catch (err) { this._onError(err); return []; }
  }

  async setWindowHwnd(hwnd) {
    try { await invoke("set_window_hwnd", { hwnd }); }
    catch (err) { this._onError(err); }
  }

  async setScreen(id) {
    try { await invoke("set_screen_id", { id }); }
    catch (err) { this._onError(err); }
  }

  // ---------- Webcam (D4) ----------
  async setWebcamEnabled(enabled) {
    this.webcamEnabled = enabled;
    try { await invoke("set_webcam_enabled", { enabled }); }
    catch (err) { this._onError(err); }
  }

  async setWebcam(id) {
    try { await invoke("set_webcam_id", { id }); }
    catch (err) { this._onError(err); }
  }

  async setPipPosition(position) {
    try { await invoke("set_pip_position", { position }); }
    catch (err) { this._onError(err); }
  }

  async setMicGain(gain) {
    try { await invoke("set_mic_gain", { gain }); }
    catch (err) { this._onError(err); }
  }

  // ---------- Audio config ----------
  async setAudioConfig(micEnabled, loopbackEnabled, micDevice = null) {
    try {
      await invoke("set_audio_config", {
        micEnabled,
        loopbackEnabled,
        micDevice: micDevice || null,
      });
    } catch (err) { this._onError(err); }
  }

  // ---------- Répertoire de sortie ----------
  async getOutputDirectory() {
    try { return await invoke("get_output_directory"); }
    catch (err) { this._onError(err); return null; }
  }

  // ---------- Encoder info (E2) ----------
  async getActiveEncoder() {
    try { return await invoke("get_active_encoder"); }
    catch (err) { this._onError(err); return null; }
  }
}
