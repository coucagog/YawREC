// ============================================================
// YAWREC · capture/screen.rs
// Capture d'écran.
//
// Windows : DXGI Desktop Duplication (D3D11 + IDXGIOutputDuplication).
//   Avantages vs windows-capture / WinRT :
//   - polling synchrone AcquireNextFrame(timeout) — pas d'événements WinRT
//   - fonctionne sans dispatcher queue (fiable Win 10 / Win 11)
//   - format BGRA8 natif (direct vers FFmpeg swscale)
//   - de-striding automatique (row pitch GPU ≠ width*4)
//
// Stubs pour macOS / Linux.
// ============================================================

use crate::capture::{Capturer, Frame};
use crate::commands::ScreenInfo;

#[derive(Debug, thiserror::Error)]
pub enum ScreenError {
    #[error("Énumération : {0}")]
    Enumeration(String),
    #[error("Initialisation : {0}")]
    Init(String),
    #[error("Capture : {0}")]
    Capture(String),
    #[error("Plateforme non supportée")]
    Unsupported,
}

// ============================================================
// API publique
// ============================================================

pub fn list_screens() -> Result<Vec<ScreenInfo>, ScreenError> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::list_screens()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(vec![ScreenInfo {
            id: 0,
            name: "Écran principal (stub)".to_string(),
            width: 1920,
            height: 1080,
            primary: true,
        }])
    }
}

#[cfg(target_os = "windows")]
pub fn make_capturer(screen_id: u32) -> Result<windows_impl::WindowsScreenCapturer, ScreenError> {
    windows_impl::WindowsScreenCapturer::new(screen_id)
}

#[cfg(not(target_os = "windows"))]
pub fn make_capturer(_screen_id: u32) -> Result<NoopScreenCapturer, ScreenError> {
    Err(ScreenError::Unsupported)
}

#[cfg(not(target_os = "windows"))]
pub struct NoopScreenCapturer;
#[cfg(not(target_os = "windows"))]
impl Capturer for NoopScreenCapturer {
    type Error = ScreenError;
    fn start(&mut self) -> Result<(), ScreenError> { Err(ScreenError::Unsupported) }
    fn stop(&mut self) -> Result<(), ScreenError> { Ok(()) }
    fn next_frame(&mut self) -> Result<Option<Frame>, ScreenError> { Ok(None) }
}

// ============================================================
// Implémentation Windows — DXGI Desktop Duplication
// ============================================================
#[cfg(target_os = "windows")]
pub mod windows_impl {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use windows::{
        core::Interface,
        Win32::{
            Foundation::HMODULE,
            Graphics::{
                Direct3D::D3D_DRIVER_TYPE_HARDWARE,
                Direct3D11::{
                    D3D11CreateDevice, D3D11_CPU_ACCESS_READ,
                    D3D11_CREATE_DEVICE_FLAG, D3D11_MAP_READ,
                    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
                    ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
                    D3D11_MAPPED_SUBRESOURCE,
                },
                Dxgi::{
                    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput,
                    IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
                    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT,
                    DXGI_OUTDUPL_FRAME_INFO,
                },
            },
        },
    };

    // --------------------------------------------------------
    // Helpers internes
    // --------------------------------------------------------

    /// Construit un IDXGIFactory1 et énumère tous les outputs des adapters.
    /// Retourne le premier output avec l'index logique `screen_id`.
    fn find_output(screen_id: u32) -> Result<(IDXGIOutput, u32, u32), ScreenError> {
        let factory: IDXGIFactory1 = unsafe {
            CreateDXGIFactory1()
                .map_err(|e| ScreenError::Init(format!("CreateDXGIFactory1 : {e}")))?
        };

        let mut idx = 0u32;
        for ai in 0u32.. {
            let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(ai) } {
                Ok(a) => a,
                Err(_) => break,
            };
            for oi in 0u32.. {
                let output: IDXGIOutput = match unsafe { adapter.EnumOutputs(oi) } {
                    Ok(o) => o,
                    Err(_) => break,
                };
                if idx == screen_id {
                    // GetDesc() returns Result<DXGI_OUTPUT_DESC> in windows 0.61
                    let desc = unsafe { output.GetDesc() }.unwrap_or_default();
                    let rect = desc.DesktopCoordinates;
                    let w = (rect.right - rect.left).unsigned_abs();
                    let h = (rect.bottom - rect.top).unsigned_abs();
                    return Ok((output, w, h));
                }
                idx += 1;
            }
        }
        Err(ScreenError::Init(format!("Écran {} introuvable", screen_id)))
    }

    // --------------------------------------------------------
    // D1 — Énumération des écrans
    // --------------------------------------------------------

    pub fn list_screens() -> Result<Vec<ScreenInfo>, ScreenError> {
        let factory: IDXGIFactory1 = unsafe {
            CreateDXGIFactory1()
                .map_err(|e| ScreenError::Enumeration(format!("CreateDXGIFactory1 : {e}")))?
        };

        let mut screens = Vec::new();
        let mut screen_id = 0u32;

        for ai in 0u32.. {
            let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(ai) } {
                Ok(a) => a,
                Err(_) => break,
            };
            for oi in 0u32.. {
                let output: IDXGIOutput = match unsafe { adapter.EnumOutputs(oi) } {
                    Ok(o) => o,
                    Err(_) => break,
                };
                let desc = match unsafe { output.GetDesc() } {
                    Ok(d) => d,
                    Err(_) => { screen_id += 1; continue; }
                };

                let name = {
                    let nw: &[u16] = &desc.DeviceName;
                    let n = nw.iter().position(|&c| c == 0).unwrap_or(nw.len());
                    String::from_utf16_lossy(&nw[..n])
                };

                let rect = desc.DesktopCoordinates;
                let w = (rect.right - rect.left).unsigned_abs();
                let h = (rect.bottom - rect.top).unsigned_abs();

                screens.push(ScreenInfo {
                    id: screen_id,
                    name,
                    width: w,
                    height: h,
                    primary: rect.left == 0 && rect.top == 0,
                });
                screen_id += 1;
            }
        }

        log::debug!("list_screens (DXGI) → {} écran(s)", screens.len());
        Ok(screens)
    }

    // --------------------------------------------------------
    // D2 — Capturer (DXGI Desktop Duplication)
    // --------------------------------------------------------

    pub struct WindowsScreenCapturer {
        screen_id: u32,
        device: Option<ID3D11Device>,
        context: Option<ID3D11DeviceContext>,
        duplication: Option<IDXGIOutputDuplication>,
        width: u32,
        height: u32,
        started_at: Option<Instant>,
        stop_signal: Arc<AtomicBool>,
    }

    // Les pointeurs COM (ID3D11*) sont thread-safe pour leur cycle de vie.
    // On les crée et utilise exclusivement dans le thread video-worker.
    unsafe impl Send for WindowsScreenCapturer {}

    impl WindowsScreenCapturer {
        pub fn new(screen_id: u32) -> Result<Self, ScreenError> {
            Ok(Self {
                screen_id,
                device: None,
                context: None,
                duplication: None,
                width: 0,
                height: 0,
                started_at: None,
                stop_signal: Arc::new(AtomicBool::new(false)),
            })
        }

        pub fn stop_handle(&self) -> Arc<AtomicBool> {
            Arc::clone(&self.stop_signal)
        }
    }

    impl Capturer for WindowsScreenCapturer {
        type Error = ScreenError;

        fn start(&mut self) -> Result<(), ScreenError> {
            let (output, width, height) = find_output(self.screen_id)?;

            // Créer un device D3D11 hardware (adaptateur par défaut).
            // DuplicateOutput requiert que le device soit sur le même adapter
            // que l'output. Pour un seul GPU / écran primaire, l'adapter par
            // défaut correspond toujours à l'output 0.
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            unsafe {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE(std::ptr::null_mut()), // pas de module software renderer
                    D3D11_CREATE_DEVICE_FLAG(0),
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
                .map_err(|e| ScreenError::Init(format!("D3D11CreateDevice : {e}")))?;
            }

            let device = device.ok_or_else(|| ScreenError::Init("D3D11 device nul".into()))?;
            let context = context.ok_or_else(|| ScreenError::Init("D3D11 context nul".into()))?;

            // IDXGIOutput1::DuplicateOutput → IDXGIOutputDuplication
            let output1: IDXGIOutput1 = output
                .cast()
                .map_err(|e| ScreenError::Init(format!("IDXGIOutput1 cast : {e}")))?;

            let dup: IDXGIOutputDuplication = unsafe {
                output1
                    .DuplicateOutput(&device)
                    .map_err(|e| ScreenError::Init(format!(
                        "DuplicateOutput ({}×{}) : {} \
                         [RDP/VM non supporté, ou device sur mauvais adapter]",
                        width, height, e
                    )))?
            };

            self.device = Some(device);
            self.context = Some(context);
            self.duplication = Some(dup);
            self.width = width;
            self.height = height;
            self.started_at = Some(Instant::now());
            self.stop_signal.store(false, Ordering::Relaxed);

            log::info!(
                "DXGI Desktop Duplication démarré (screen={}, {}×{})",
                self.screen_id, width, height
            );
            Ok(())
        }

        fn stop(&mut self) -> Result<(), ScreenError> {
            self.stop_signal.store(true, Ordering::Relaxed);
            // Libérer dans l'ordre inverse de création
            self.duplication = None;
            self.context = None;
            self.device = None;
            log::info!("DXGI Desktop Duplication arrêté");
            Ok(())
        }

        fn next_frame(&mut self) -> Result<Option<Frame>, ScreenError> {
            if self.stop_signal.load(Ordering::Relaxed) {
                return Ok(None);
            }

            let dup = match &self.duplication {
                Some(d) => d,
                None => return Ok(None),
            };
            let device = match &self.device {
                Some(d) => d,
                None => return Ok(None),
            };
            let context = match &self.context {
                Some(c) => c,
                None => return Ok(None),
            };

            // ------------------------------------------------
            // Acquérir la prochaine frame (timeout 100 ms)
            // ------------------------------------------------
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;

            match unsafe { dup.AcquireNextFrame(100, &mut frame_info, &mut resource) } {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    // Pas de nouvelle frame dans le timeout — le worker re-essaiera.
                    return Ok(None);
                }
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    return Err(ScreenError::Capture(
                        "Accès DXGI perdu (changement résolution ou verrouillage) \
                         — relancer l'enregistrement"
                            .into(),
                    ));
                }
                Err(e) => {
                    return Err(ScreenError::Capture(format!("AcquireNextFrame : {e}")));
                }
            }

            let resource = match resource {
                Some(r) => r,
                None => {
                    // AcquireNextFrame Ok mais resource None — libérer et ignorer.
                    unsafe { dup.ReleaseFrame().ok() };
                    return Ok(None);
                }
            };

            // ------------------------------------------------
            // Copier vers une staging texture CPU-lisible
            // ------------------------------------------------
            let texture: ID3D11Texture2D = match resource.cast() {
                Ok(t) => t,
                Err(e) => {
                    unsafe { dup.ReleaseFrame().ok() };
                    return Err(ScreenError::Capture(format!("cast ID3D11Texture2D : {e}")));
                }
            };

            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { texture.GetDesc(&mut desc) };

            // Staging : accessible CPU, pas de bind flags.
            // Les champs BindFlags/CPUAccessFlags/MiscFlags sont u32 dans windows 0.61.
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;  // D3D11_CPU_ACCESS_FLAG(i32) → u32
            desc.MiscFlags = 0;
            desc.ArraySize = 1;
            desc.MipLevels = 1;
            desc.SampleDesc.Count = 1;
            desc.SampleDesc.Quality = 0;

            // CreateTexture2D prend 3 args en windows 0.61 (out-param style).
            let mut staging_opt: Option<ID3D11Texture2D> = None;
            if let Err(e) = unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging_opt)) } {
                unsafe { dup.ReleaseFrame().ok() };
                return Err(ScreenError::Capture(format!("CreateTexture2D staging : {e}")));
            }
            let staging = match staging_opt {
                Some(t) => t,
                None => {
                    unsafe { dup.ReleaseFrame().ok() };
                    return Err(ScreenError::Capture("staging texture nul".into()));
                }
            };

            // CopyResource texture → staging
            let staging_res: ID3D11Resource = match staging.cast() {
                Ok(r) => r,
                Err(e) => {
                    unsafe { dup.ReleaseFrame().ok() };
                    return Err(ScreenError::Capture(format!("cast staging resource : {e}")));
                }
            };
            let texture_res: ID3D11Resource = match texture.cast() {
                Ok(r) => r,
                Err(e) => {
                    unsafe { dup.ReleaseFrame().ok() };
                    return Err(ScreenError::Capture(format!("cast texture resource : {e}")));
                }
            };
            unsafe { context.CopyResource(&staging_res, &texture_res) };

            // Libérer la frame DXGI avant le Map (pour ne pas bloquer Windows)
            unsafe { dup.ReleaseFrame().ok() };

            // ------------------------------------------------
            // Map la staging texture → lecture CPU
            // ------------------------------------------------
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            if let Err(e) = unsafe {
                context.Map(&staging_res, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            } {
                return Err(ScreenError::Capture(format!("ID3D11DeviceContext::Map : {e}")));
            }

            let w = desc.Width as usize;
            let h = desc.Height as usize;
            let row_bytes = w * 4; // BGRA8 = 4 bytes/pixel
            let row_pitch = mapped.RowPitch as usize;

            let mut data = Vec::with_capacity(row_bytes * h);
            let src = mapped.pData as *const u8;
            for y in 0..h {
                // De-stridage : copier seulement les pixels utiles de chaque ligne
                // (le GPU aligne les lignes sur 256 bytes, d'où row_pitch ≥ row_bytes).
                let slice = unsafe {
                    std::slice::from_raw_parts(src.add(y * row_pitch), row_bytes)
                };
                data.extend_from_slice(slice);
            }

            unsafe { context.Unmap(&staging_res, 0) };

            Ok(Some(Frame {
                width: desc.Width,
                height: desc.Height,
                stride: desc.Width * 4,
                data,
                timestamp: self.started_at.unwrap().elapsed(),
            }))
        }
    }
}
