// ============================================================
// YAWREC · capture/screen.rs
// Capture d'écran.
//
// Windows : DXGI Desktop Duplication (D3D11 + IDXGIOutputDuplication).
//   - polling synchrone AcquireNextFrame(timeout) — pas d'événements WinRT
//   - device D3D11 créé sur l'adaptateur qui porte l'écran (PC à double GPU)
//   - texture staging réutilisée d'une frame à l'autre
//   - curseur souris incrusté (DXGI ne le dessine pas dans l'image)
//   - reprise automatique après DXGI_ERROR_ACCESS_LOST (UAC, Ctrl+Alt+Suppr,
//     changement de résolution, jeu plein écran exclusif)
//   - format BGRA8 natif, de-striding (row pitch GPU ≠ width*4)
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
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            primary: true,
        }])
    }
}

/// Écran (id) qui contient le point (x, y) en coordonnées bureau, s'il existe.
pub fn screen_at(x: i32, y: i32) -> Option<ScreenInfo> {
    list_screens().ok()?.into_iter().find(|s| {
        x >= s.x && y >= s.y && x < s.x + s.width as i32 && y < s.y + s.height as i32
    })
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
impl NoopScreenCapturer {
    pub fn set_acquire_timeout_ms(&mut self, _ms: u32) {}
    pub fn desktop_origin(&self) -> (i32, i32) { (0, 0) }
    pub fn access_lost(&self) -> bool { false }
}
#[cfg(not(target_os = "windows"))]
impl Capturer for NoopScreenCapturer {
    type Error = ScreenError;
    fn start(&mut self) -> Result<(), ScreenError> { Err(ScreenError::Unsupported) }
    fn stop(&mut self) -> Result<(), ScreenError> { Ok(()) }
    fn next_frame(&mut self) -> Result<Option<Frame>, ScreenError> { Ok(None) }
}

// ============================================================
// Curseur — incrustation d'une forme DXGI dans une frame BGRA
// ============================================================

/// Types de forme renvoyés par GetFramePointerShape
/// (DXGI_OUTDUPL_POINTER_SHAPE_TYPE_*).
pub const POINTER_MONOCHROME: u32 = 1;
pub const POINTER_COLOR: u32 = 2;
pub const POINTER_MASKED_COLOR: u32 = 4;

/// Forme du pointeur telle que fournie par DXGI.
#[derive(Debug, Clone, Default)]
pub struct PointerShape {
    pub kind: u32,
    pub width: u32,
    /// Pour MONOCHROME, hauteur totale = 2 × hauteur visible (masque AND puis XOR).
    pub height: u32,
    pub pitch: u32,
    pub data: Vec<u8>,
}

/// Dessine `shape` avec son coin haut-gauche en (px, py) dans `frame`
/// (BGRA8 packé, largeur `fw`, hauteur `fh`). Les parties hors cadre sont
/// ignorées. Algorithme repris de l'exemple DesktopDuplication de Microsoft.
pub fn draw_pointer(frame: &mut [u8], fw: u32, fh: u32, shape: &PointerShape, px: i32, py: i32) {
    let visible_h = if shape.kind == POINTER_MONOCHROME { shape.height / 2 } else { shape.height };
    let stride = fw as usize * 4;

    for sy in 0..visible_h as i32 {
        let y = py + sy;
        if y < 0 || y >= fh as i32 {
            continue;
        }
        for sx in 0..shape.width as i32 {
            let x = px + sx;
            if x < 0 || x >= fw as i32 {
                continue;
            }
            let d = y as usize * stride + x as usize * 4;
            if d + 3 >= frame.len() {
                continue;
            }

            match shape.kind {
                POINTER_MONOCHROME => {
                    let byte = (sx / 8) as usize;
                    let bit = 0x80u8 >> (sx % 8);
                    let and_idx = sy as usize * shape.pitch as usize + byte;
                    let xor_idx = (sy as usize + visible_h as usize) * shape.pitch as usize + byte;
                    let (Some(&and_b), Some(&xor_b)) = (shape.data.get(and_idx), shape.data.get(xor_idx)) else {
                        continue;
                    };
                    let and_set = and_b & bit != 0;
                    let xor_set = xor_b & bit != 0;
                    for c in 0..3 {
                        let base = if and_set { frame[d + c] } else { 0 };
                        frame[d + c] = if xor_set { base ^ 0xFF } else { base };
                    }
                }
                POINTER_COLOR | POINTER_MASKED_COLOR => {
                    let s = sy as usize * shape.pitch as usize + sx as usize * 4;
                    let Some(src) = shape.data.get(s..s + 4) else { continue };
                    let a = src[3];
                    if shape.kind == POINTER_COLOR {
                        // Alpha blending classique.
                        let a16 = a as u16;
                        for c in 0..3 {
                            let v = (src[c] as u16 * a16 + frame[d + c] as u16 * (255 - a16)) / 255;
                            frame[d + c] = v as u8;
                        }
                    } else if a == 0 {
                        // Masked color : alpha 0 → remplacement…
                        frame[d..d + 3].copy_from_slice(&src[..3]);
                    } else {
                        // … alpha 0xFF → XOR avec l'écran.
                        for c in 0..3 {
                            frame[d + c] ^= src[c];
                        }
                    }
                }
                _ => return,
            }
        }
    }
}

// ============================================================
// Implémentation Windows — DXGI Desktop Duplication
// ============================================================
#[cfg(target_os = "windows")]
pub mod windows_impl {
    use super::*;
    use std::time::{Duration, Instant};

    use windows::{
        core::Interface,
        Win32::{
            Foundation::{GENERIC_ALL, HMODULE},
            Graphics::{
                Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
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
                    DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_POINTER_SHAPE_INFO,
                },
            },
            System::StationsAndDesktops::{
                CloseDesktop, OpenInputDesktop, SetThreadDesktop, DESKTOP_ACCESS_FLAGS,
                DESKTOP_CONTROL_FLAGS,
            },
        },
    };

    /// Durée max pendant laquelle on tente de récupérer une duplication perdue
    /// (bureau sécurisé UAC, écran verrouillé…) avant d'abandonner.
    const ACCESS_LOST_GIVE_UP: Duration = Duration::from_secs(30);

    /// Rattache le thread courant au bureau de saisie actif, comme le fait
    /// l'exemple Microsoft de Desktop Duplication avant chaque DuplicateOutput.
    /// Pendant une invite UAC ou l'écran Ctrl+Alt+Suppr, le bureau actif est
    /// le bureau sécurisé, qu'une app non élevée ne peut pas ouvrir : Err,
    /// il faut attendre. Sans ce contrôle, DuplicateOutput réussissait pendant
    /// l'invite UAC et donnait une duplication du mauvais bureau (boucle
    /// ACCESS_LOST puis image noire jusqu'à la fin de l'enregistrement).
    fn attach_to_input_desktop() -> windows::core::Result<()> {
        unsafe {
            let desk = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_ACCESS_FLAGS(GENERIC_ALL.0))?;
            let attached = SetThreadDesktop(desk);
            // Le thread garde son rattachement ; le handle n'est plus utile.
            let _ = CloseDesktop(desk);
            attached
        }
    }

    // --------------------------------------------------------
    // Helpers internes
    // --------------------------------------------------------

    struct FoundOutput {
        adapter: IDXGIAdapter1,
        output: IDXGIOutput,
        left: i32,
        top: i32,
        width: u32,
        height: u32,
    }

    /// Énumère tous les outputs des adapters et retourne celui d'index
    /// logique `screen_id`, avec l'adaptateur qui le porte.
    fn find_output(screen_id: u32) -> Result<FoundOutput, ScreenError> {
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
                    let desc = unsafe { output.GetDesc() }.unwrap_or_default();
                    let rect = desc.DesktopCoordinates;
                    return Ok(FoundOutput {
                        adapter,
                        output,
                        left: rect.left,
                        top: rect.top,
                        width: (rect.right - rect.left).unsigned_abs(),
                        height: (rect.bottom - rect.top).unsigned_abs(),
                    });
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
                screens.push(ScreenInfo {
                    id: screen_id,
                    name,
                    x: rect.left,
                    y: rect.top,
                    width: (rect.right - rect.left).unsigned_abs(),
                    height: (rect.bottom - rect.top).unsigned_abs(),
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

    #[derive(Default)]
    struct PointerState {
        visible: bool,
        x: i32,
        y: i32,
        shape: Option<PointerShape>,
    }

    pub struct WindowsScreenCapturer {
        screen_id: u32,
        device: Option<ID3D11Device>,
        context: Option<ID3D11DeviceContext>,
        output1: Option<IDXGIOutput1>,
        duplication: Option<IDXGIOutputDuplication>,
        /// Texture CPU-lisible réutilisée tant que la taille ne change pas.
        staging: Option<(ID3D11Texture2D, u32, u32)>,
        origin: (i32, i32),
        width: u32,
        height: u32,
        started_at: Option<Instant>,
        acquire_timeout_ms: u32,
        /// Instant de la première perte d'accès non encore récupérée.
        access_lost_since: Option<Instant>,
        /// Dernière récupération réussie : une nouvelle perte juste après
        /// signale une boucle (perte/récupération en rafale), qu'on ralentit.
        recovered_at: Option<Instant>,
        pointer: PointerState,
        stopped: bool,
    }

    // Les pointeurs COM (ID3D11*) sont créés et utilisés exclusivement dans
    // le thread video-worker ; le capturer n'y est que déplacé.
    unsafe impl Send for WindowsScreenCapturer {}

    impl WindowsScreenCapturer {
        pub fn new(screen_id: u32) -> Result<Self, ScreenError> {
            Ok(Self {
                screen_id,
                device: None,
                context: None,
                output1: None,
                duplication: None,
                staging: None,
                origin: (0, 0),
                width: 0,
                height: 0,
                started_at: None,
                acquire_timeout_ms: 100,
                access_lost_since: None,
                recovered_at: None,
                pointer: PointerState::default(),
                stopped: false,
            })
        }

        /// Délai d'attente d'AcquireNextFrame. Plus court = le worker reprend
        /// la main plus souvent quand l'écran est statique.
        pub fn set_acquire_timeout_ms(&mut self, ms: u32) {
            self.acquire_timeout_ms = ms.clamp(1, 500);
        }

        /// Coin haut-gauche de l'écran capturé en coordonnées bureau.
        pub fn desktop_origin(&self) -> (i32, i32) {
            self.origin
        }

        /// Accès à l'écran perdu (bureau sécurisé UAC, Ctrl+Alt+Suppr) :
        /// la dernière image est répétée en attendant la récupération.
        pub fn access_lost(&self) -> bool {
            self.access_lost_since.is_some()
        }

        fn duplicate(&mut self) -> Result<(), ScreenError> {
            let (Some(output1), Some(device)) = (&self.output1, &self.device) else {
                return Err(ScreenError::Init("capturer non démarré".into()));
            };
            let dup = unsafe {
                output1.DuplicateOutput(device).map_err(|e| ScreenError::Init(format!(
                    "DuplicateOutput ({}×{}) : {}", self.width, self.height, e
                )))?
            };
            self.duplication = Some(dup);
            Ok(())
        }

        /// Après ACCESS_LOST : retente la duplication. Ok(true) si récupérée.
        fn try_recover(&mut self) -> Result<bool, ScreenError> {
            let since = *self.access_lost_since.get_or_insert_with(Instant::now);
            self.duplication = None;
            if let Err(e) = attach_to_input_desktop() {
                // Bureau sécurisé encore affiché : on garde la dernière image.
                if since.elapsed() >= ACCESS_LOST_GIVE_UP {
                    return Err(ScreenError::Capture(format!(
                        "Accès à l'écran perdu depuis {}s : bureau de saisie inaccessible ({e})",
                        ACCESS_LOST_GIVE_UP.as_secs()
                    )));
                }
                log::debug!("DXGI · bureau sécurisé actif, attente : {e}");
                std::thread::sleep(Duration::from_millis(200));
                return Ok(false);
            }
            // La résolution a pu changer : relire la géométrie de l'écran.
            if let Ok(found) = find_output(self.screen_id) {
                self.origin = (found.left, found.top);
                self.width = found.width;
                self.height = found.height;
            }
            match self.duplicate() {
                Ok(()) => {
                    log::info!("DXGI · duplication récupérée après {:?}", since.elapsed());
                    self.access_lost_since = None;
                    self.recovered_at = Some(Instant::now());
                    Ok(true)
                }
                Err(e) if since.elapsed() < ACCESS_LOST_GIVE_UP => {
                    log::debug!("DXGI · récupération en attente : {e}");
                    std::thread::sleep(Duration::from_millis(200));
                    Ok(false)
                }
                Err(e) => Err(ScreenError::Capture(format!(
                    "Accès à l'écran perdu depuis {}s : {e}", ACCESS_LOST_GIVE_UP.as_secs()
                ))),
            }
        }

        fn update_pointer(&mut self, dup: &IDXGIOutputDuplication, info: &DXGI_OUTDUPL_FRAME_INFO) {
            if info.LastMouseUpdateTime != 0 {
                self.pointer.visible = info.PointerPosition.Visible.as_bool();
                self.pointer.x = info.PointerPosition.Position.x;
                self.pointer.y = info.PointerPosition.Position.y;
            }
            if info.PointerShapeBufferSize == 0 {
                return;
            }
            let mut buf = vec![0u8; info.PointerShapeBufferSize as usize];
            let mut required = 0u32;
            let mut shape_info = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
            let res = unsafe {
                dup.GetFramePointerShape(
                    buf.len() as u32,
                    buf.as_mut_ptr() as *mut core::ffi::c_void,
                    &mut required,
                    &mut shape_info,
                )
            };
            match res {
                Ok(()) => {
                    self.pointer.shape = Some(PointerShape {
                        kind: shape_info.Type,
                        width: shape_info.Width,
                        height: shape_info.Height,
                        pitch: shape_info.Pitch,
                        data: buf,
                    });
                }
                Err(e) => log::debug!("GetFramePointerShape : {e}"),
            }
        }

        fn staging_texture(&mut self, desc: &D3D11_TEXTURE2D_DESC) -> Result<ID3D11Texture2D, ScreenError> {
            if let Some((tex, w, h)) = &self.staging {
                if *w == desc.Width && *h == desc.Height {
                    return Ok(tex.clone());
                }
            }
            let device = self.device.as_ref().ok_or_else(|| ScreenError::Capture("device nul".into()))?;
            let mut sdesc = *desc;
            sdesc.Usage = D3D11_USAGE_STAGING;
            sdesc.BindFlags = 0;
            sdesc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            sdesc.MiscFlags = 0;
            sdesc.ArraySize = 1;
            sdesc.MipLevels = 1;
            sdesc.SampleDesc.Count = 1;
            sdesc.SampleDesc.Quality = 0;

            let mut tex: Option<ID3D11Texture2D> = None;
            unsafe { device.CreateTexture2D(&sdesc, None, Some(&mut tex)) }
                .map_err(|e| ScreenError::Capture(format!("CreateTexture2D staging : {e}")))?;
            let tex = tex.ok_or_else(|| ScreenError::Capture("staging texture nul".into()))?;
            self.staging = Some((tex.clone(), desc.Width, desc.Height));
            Ok(tex)
        }
    }

    impl Capturer for WindowsScreenCapturer {
        type Error = ScreenError;

        fn start(&mut self) -> Result<(), ScreenError> {
            let found = find_output(self.screen_id)?;

            // Le device doit être créé sur l'adaptateur qui porte l'écran,
            // sinon DuplicateOutput échoue (portables NVIDIA Optimus, eGPU…).
            // Avec un adaptateur explicite, le driver type doit être UNKNOWN.
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            unsafe {
                D3D11CreateDevice(
                    &found.adapter,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    HMODULE(std::ptr::null_mut()),
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

            let output1: IDXGIOutput1 = found.output
                .cast()
                .map_err(|e| ScreenError::Init(format!("IDXGIOutput1 cast : {e}")))?;

            self.device = Some(device);
            self.context = Some(context);
            self.output1 = Some(output1);
            self.origin = (found.left, found.top);
            self.width = found.width;
            self.height = found.height;
            if let Err(e) = attach_to_input_desktop() {
                log::debug!("DXGI · rattachement au bureau de saisie : {e}");
            }
            self.duplicate().map_err(|e| ScreenError::Init(format!("{e} [session RDP ou VM non supportée ?]")))?;
            self.started_at = Some(Instant::now());
            self.stopped = false;

            log::info!(
                "DXGI Desktop Duplication démarré (screen={}, {}×{} @ {:?})",
                self.screen_id, self.width, self.height, self.origin
            );
            Ok(())
        }

        fn stop(&mut self) -> Result<(), ScreenError> {
            self.stopped = true;
            // Libérer dans l'ordre inverse de création
            self.duplication = None;
            self.staging = None;
            self.output1 = None;
            self.context = None;
            self.device = None;
            log::info!("DXGI Desktop Duplication arrêté");
            Ok(())
        }

        fn next_frame(&mut self) -> Result<Option<Frame>, ScreenError> {
            if self.stopped {
                return Ok(None);
            }
            if self.duplication.is_none() && !self.try_recover()? {
                return Ok(None);
            }
            let dup = self.duplication.clone().expect("duplication présente");

            // ------------------------------------------------
            // Acquérir la prochaine frame
            // ------------------------------------------------
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;

            match unsafe { dup.AcquireNextFrame(self.acquire_timeout_ms, &mut frame_info, &mut resource) } {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    if self.recovered_at.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
                        // Perdue juste après récupération : on ralentit la boucle.
                        log::debug!("DXGI · accès reperdu juste après récupération");
                        std::thread::sleep(Duration::from_millis(100));
                    } else {
                        log::warn!("DXGI · accès perdu, tentative de récupération");
                    }
                    self.try_recover()?;
                    return Ok(None);
                }
                Err(e) => return Err(ScreenError::Capture(format!("AcquireNextFrame : {e}"))),
            }

            self.update_pointer(&dup, &frame_info);

            let Some(resource) = resource else {
                unsafe { dup.ReleaseFrame().ok() };
                return Ok(None);
            };

            // ------------------------------------------------
            // Copier vers la texture staging CPU-lisible
            // ------------------------------------------------
            let copy = (|| -> Result<(ID3D11Resource, D3D11_TEXTURE2D_DESC), ScreenError> {
                let texture: ID3D11Texture2D = resource
                    .cast()
                    .map_err(|e| ScreenError::Capture(format!("cast ID3D11Texture2D : {e}")))?;
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                unsafe { texture.GetDesc(&mut desc) };

                let staging = self.staging_texture(&desc)?;
                let staging_res: ID3D11Resource = staging
                    .cast()
                    .map_err(|e| ScreenError::Capture(format!("cast staging resource : {e}")))?;
                let texture_res: ID3D11Resource = texture
                    .cast()
                    .map_err(|e| ScreenError::Capture(format!("cast texture resource : {e}")))?;
                let context = self.context.as_ref().ok_or_else(|| ScreenError::Capture("context nul".into()))?;
                unsafe { context.CopyResource(&staging_res, &texture_res) };
                Ok((staging_res, desc))
            })();

            // Libérer la frame DXGI avant le Map (pour ne pas bloquer Windows)
            unsafe { dup.ReleaseFrame().ok() };
            let (staging_res, desc) = copy?;

            // ------------------------------------------------
            // Map la staging texture → lecture CPU
            // ------------------------------------------------
            let context = self.context.as_ref().ok_or_else(|| ScreenError::Capture("context nul".into()))?;
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe { context.Map(&staging_res, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
                .map_err(|e| ScreenError::Capture(format!("ID3D11DeviceContext::Map : {e}")))?;

            let w = desc.Width as usize;
            let h = desc.Height as usize;
            let row_bytes = w * 4; // BGRA8 = 4 bytes/pixel
            let row_pitch = mapped.RowPitch as usize;

            let mut data = Vec::with_capacity(row_bytes * h);
            let src = mapped.pData as *const u8;
            for y in 0..h {
                // De-stridage : le GPU aligne les lignes, row_pitch ≥ row_bytes.
                let slice = unsafe { std::slice::from_raw_parts(src.add(y * row_pitch), row_bytes) };
                data.extend_from_slice(slice);
            }
            unsafe { context.Unmap(&staging_res, 0) };

            if self.pointer.visible {
                if let Some(shape) = &self.pointer.shape {
                    draw_pointer(&mut data, desc.Width, desc.Height, shape, self.pointer.x, self.pointer.y);
                }
            }

            Ok(Some(Frame {
                width: desc.Width,
                height: desc.Height,
                stride: desc.Width * 4,
                data,
                timestamp: self.started_at.map(|t| t.elapsed()).unwrap_or_default(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray_frame(w: u32, h: u32, v: u8) -> Vec<u8> {
        vec![v; (w * h * 4) as usize]
    }

    #[test]
    fn pointer_color_alpha_blend_et_clipping() {
        let mut f = gray_frame(4, 4, 100);
        // Forme 2×2 : un pixel blanc opaque, un transparent, deux semi-opaques noirs.
        let shape = PointerShape {
            kind: POINTER_COLOR,
            width: 2,
            height: 2,
            pitch: 8,
            data: vec![
                255, 255, 255, 255,   0, 0, 0, 0,
                0, 0, 0, 128,         0, 0, 0, 128,
            ],
        };
        // Placée en (3, 3) : seul le pixel (0,0) de la forme est dans le cadre.
        draw_pointer(&mut f, 4, 4, &shape, 3, 3);
        let px = |x: usize, y: usize| &f[(y * 4 + x) * 4..(y * 4 + x) * 4 + 3];
        assert_eq!(px(3, 3), &[255, 255, 255]);
        assert_eq!(px(2, 2), &[100, 100, 100]);

        let mut f = gray_frame(4, 4, 100);
        draw_pointer(&mut f, 4, 4, &shape, 0, 0);
        let px = |x: usize, y: usize| &f[(y * 4 + x) * 4..(y * 4 + x) * 4 + 3];
        assert_eq!(px(1, 0), &[100, 100, 100], "pixel transparent inchangé");
        assert_eq!(px(0, 1), &[49, 49, 49], "alpha 128 → moitié");
    }

    #[test]
    fn pointer_monochrome_and_xor() {
        let mut f = gray_frame(8, 1, 0x40);
        // Largeur 4, hauteur visible 1 : AND = 1010, XOR = 0110.
        // px0 : and=1 xor=0 → inchangé ; px1 : and=0 xor=1 → blanc ;
        // px2 : and=1 xor=1 → inversé ; px3 : and=0 xor=0 → noir.
        let shape = PointerShape {
            kind: POINTER_MONOCHROME,
            width: 4,
            height: 2,
            pitch: 1,
            data: vec![0b1010_0000, 0b0110_0000],
        };
        draw_pointer(&mut f, 8, 1, &shape, 0, 0);
        assert_eq!(&f[0..3], &[0x40; 3]);
        assert_eq!(&f[4..7], &[0xFF; 3]);
        assert_eq!(&f[8..11], &[0xBF; 3]);
        assert_eq!(&f[12..15], &[0x00; 3]);
    }

    #[test]
    fn pointer_masked_color() {
        let mut f = gray_frame(2, 1, 0x0F);
        let shape = PointerShape {
            kind: POINTER_MASKED_COLOR,
            width: 2,
            height: 1,
            pitch: 8,
            data: vec![0x20, 0x20, 0x20, 0x00,  0xFF, 0xFF, 0xFF, 0xFF],
        };
        draw_pointer(&mut f, 2, 1, &shape, 0, 0);
        assert_eq!(&f[0..3], &[0x20; 3], "alpha 0 → remplacement");
        assert_eq!(&f[4..7], &[0xF0; 3], "alpha FF → XOR");
    }
}
