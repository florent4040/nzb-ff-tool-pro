//! Zone de notification (Windows) : icône à côté de l'horloge quand la fenêtre est fermée,
//! menu (ouvrir, surveillance, quitter), bulles d'information, fenêtre cachée / réaffichée,
//! lancement avec Windows. Ailleurs que sous Windows, tout est sans effet.

#![cfg_attr(not(windows), allow(dead_code))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Commandes venues du menu de l'icône.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrayCmd {
    Show,
    StartWatch,
    StopWatch,
    /// Téléchargements du client : pause, reprise.
    DlPause,
    DlResume,
}

/// Ce que l'icône doit savoir de l'application.
pub struct Hooks {
    /// Exécute une commande du menu (réafficher la fenêtre, surveillance…).
    pub on_cmd: Box<dyn Fn(TrayCmd) + Send + Sync>,
    /// Surveillance active ?
    pub watching: Box<dyn Fn() -> bool + Send + Sync>,
    /// Travail en cours (post, dépôt…) : texte affiché avant de quitter, sinon None.
    pub busy: Box<dyn Fn() -> Option<String> + Send + Sync>,
}

static HIDDEN: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static NOTIFY: AtomicBool = AtomicBool::new(true);
static HOOKS: OnceLock<Hooks> = OnceLock::new();
static STATUS: Mutex<String> = Mutex::new(String::new());
/// Copie ou préparation en cours (ligne à part, en plus du post).
static COPY: Mutex<String> = Mutex::new(String::new());
/// Téléchargements du client (ligne à part) et leur état : 0 aucun, 1 en cours, 2 en pause.
static DL: Mutex<String> = Mutex::new(String::new());
static DL_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// La zone de notification existe-t-elle sur ce système ?
pub fn available() -> bool {
    cfg!(windows) || std::env::var_os("NZBFF_DEMO_TRAY").is_some()
}

/// La fenêtre est-elle cachée (application dans la zone de notification) ?
pub fn is_hidden() -> bool {
    HIDDEN.load(Ordering::Relaxed)
}

static MINIMIZED: AtomicBool = AtomicBool::new(false);
static HEARTBEAT: AtomicBool = AtomicBool::new(false);

/// Fenêtre réduite dans la barre des tâches (noté à chaque image).
pub fn set_minimized(on: bool) {
    MINIMIZED.store(on, Ordering::Relaxed);
}

/// Fenêtre invisible (zone de notification ou réduite) : on ne demande alors **aucune** image.
///
/// Pourquoi : Windows n'envoie pas de WM_PAINT à une fenêtre cachée. Une image demandée à
/// eframe dans cet état n'arrive jamais, et sa boucle d'événements tourne à vide en
/// l'attendant (un cœur du processeur à 100 %, soit 12 à 13 % sur un processeur à 8 fils).
/// Les fils de fond travaillent sans l'interface ; les événements qu'ils envoient sont
/// traités par le battement ci-dessous.
pub fn in_background() -> bool {
    HIDDEN.load(Ordering::Relaxed) || MINIMIZED.load(Ordering::Relaxed)
}

/// Battement de la fenêtre invisible : toutes les `secs` secondes, un WM_PAINT lui est posté
/// directement (il arrive même fenêtre cachée), ce qui fait passer une image très légère :
/// les événements des fils de fond sont traités, l'enregistrement automatique se fait, et
/// une image restée en attente est servie (fin de la boucle à vide). Rien quand la fenêtre est
/// visible. Coût : une image toutes les 2 s, sans rien dessiner.
pub fn start_heartbeat(secs: f32) {
    if HEARTBEAT.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(windows)]
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs_f32(secs.max(0.5)));
        if in_background() {
            win::wake_main();
        }
    });
    #[cfg(not(windows))]
    let _ = secs;
}

/// Bulles d'information autorisées ?
pub fn set_notify(on: bool) {
    NOTIFY.store(on, Ordering::Relaxed);
}

/// Texte d'état (infobulle de l'icône, première ligne du menu).
pub fn set_status(text: &str) {
    if let Ok(mut s) = STATUS.lock() {
        if *s == text {
            return;
        }
        *s = text.to_string();
    }
    #[cfg(windows)]
    win::update_tip();
}

/// Ligne de la copie en cours (vide : aucune). Mise à jour depuis le fil de la copie, même
/// fenêtre cachée.
pub fn set_copy(text: &str) {
    if let Ok(mut s) = COPY.lock() {
        if *s == text {
            return;
        }
        *s = text.to_string();
    }
    #[cfg(windows)]
    win::update_tip();
}

/// Ligne des téléchargements (vide : rien en cours) ; `state` : 0 aucun, 1 en cours, 2 en
/// pause (menu « Pause des téléchargements » / « Reprendre »).
pub fn set_dl(text: &str, state: u8) {
    DL_STATE.store(state, Ordering::Relaxed);
    if let Ok(mut s) = DL.lock() {
        if *s == text {
            return;
        }
        *s = text.to_string();
    }
    #[cfg(windows)]
    win::update_tip();
}

fn status() -> String {
    let copy = COPY.lock().map(|s| s.clone()).unwrap_or_default();
    let post = STATUS.lock().map(|s| s.clone()).unwrap_or_default();
    let dl = DL.lock().map(|s| s.clone()).unwrap_or_default();
    [copy, post, dl].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n")
}

/// Bulle d'information, seulement quand la fenêtre est cachée.
pub fn notify(title: &str, text: &str) {
    if !is_hidden() || !NOTIFY.load(Ordering::Relaxed) {
        return;
    }
    #[cfg(windows)]
    win::balloon(title, text);
    #[cfg(not(windows))]
    let _ = (title, text);
}

/// Crée l'icône (une seule fois), avec l'image RGBA 128×128 de l'application.
pub fn start(icon_rgba128: &'static [u8], hooks: Hooks) {
    if !available() || STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = HOOKS.set(hooks);
    #[cfg(windows)]
    std::thread::spawn(move || win::run(icon_rgba128));
    #[cfg(not(windows))]
    let _ = icon_rgba128;
}

/// Une autre instance de l'application tourne déjà (elle vient d'être priée de se montrer) :
/// celle-ci doit s'arrêter, pour n'avoir qu'une icône dans la zone de notification.
pub fn another_instance() -> bool {
    another_instance_show(true)
}

/// Comme `another_instance`, la fenêtre de l'instance ouverte n'étant montrée que si `show`
/// (demande traitée en fond, depuis le clic droit par exemple).
pub fn another_instance_show(show: bool) -> bool {
    #[cfg(windows)]
    {
        win::already_running(show)
    }
    #[cfg(not(windows))]
    {
        let _ = show;
        false
    }
}

/// Cache la fenêtre principale (l'application continue dans la zone de notification).
pub fn hide_main() -> bool {
    #[cfg(windows)]
    {
        if let Some(h) = win::main_window() {
            win::show(h, false);
            HIDDEN.store(true, Ordering::Relaxed);
            return true;
        }
    }
    false
}

/// Réaffiche la fenêtre principale au premier plan.
pub fn show_main() {
    HIDDEN.store(false, Ordering::Relaxed);
    #[cfg(windows)]
    if let Some(h) = win::main_window() {
        win::show(h, true);
    }
}

/// Retire l'icône (avant de quitter).
pub fn remove() {
    #[cfg(windows)]
    win::remove();
}

/// Ferme l'application tout de suite (icône retirée).
pub fn exit_now() -> ! {
    remove();
    crate::backup::on_exit();
    std::process::exit(0)
}

// ------------------------------------------------------------------ lancement avec Windows

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

fn reg(args: &[&str]) -> bool {
    let mut cmd = std::process::Command::new("reg");
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// L'application est-elle lancée avec Windows ?
pub fn autostart_enabled() -> bool {
    autostart_value().is_some()
}

/// Commande enregistrée pour le lancement avec Windows (clé « Exécuter » du compte).
pub fn autostart_value() -> Option<String> {
    #[cfg(windows)]
    {
        win::run_value()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Commande attendue : cet exécutable, démarré dans la zone de notification.
fn autostart_command() -> Option<String> {
    std::env::current_exe().ok().map(|exe| format!("\"{}\" {TRAY_ARG}", exe.display()))
}

/// Argument de lancement : démarrer sans fenêtre, dans la zone de notification.
pub const TRAY_ARG: &str = "--tray";

/// Lancement avec Windows activé mais périmé (exécutable déplacé, ou ancienne version sans
/// « --tray » qui ouvrait la fenêtre) : la commande est réécrite. Renvoie true si corrigée.
pub fn refresh_autostart() -> bool {
    match (autostart_value(), autostart_command()) {
        (Some(v), Some(want)) if !v.trim().eq_ignore_ascii_case(&want) => set_autostart(true).is_ok(),
        _ => false,
    }
}

/// Démarrage dans la zone de notification : eframe rend la fenêtre visible après sa première
/// image, donc après notre `hide_main`. Pendant quelques secondes, un fil la recache aussitôt
/// qu'elle apparaît — jusqu'à ce que l'utilisateur l'ouvre lui-même (`show_main`).
pub fn keep_hidden_at_start() {
    HIDDEN.store(true, Ordering::Relaxed);
    #[cfg(windows)]
    std::thread::spawn(|| {
        // 15 s : large, même au démarrage d'un PC chargé
        for _ in 0..300 {
            if !HIDDEN.load(Ordering::Relaxed) {
                return;
            }
            if let Some(h) = win::main_window() {
                if win::visible(h) {
                    win::show(h, false);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });
}

/// Lance (ou non) l'application avec Windows, réduite dans la zone de notification.
pub fn set_autostart(on: bool) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("seulement sous Windows".into());
    }
    let ok = if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let value = format!("\"{}\" {TRAY_ARG}", exe.display());
        reg(&["add", RUN_KEY, "/v", crate::APP_NAME, "/t", "REG_SZ", "/d", &value, "/f"])
    } else {
        reg(&["delete", RUN_KEY, "/v", crate::APP_NAME, "/f"])
    };
    if ok {
        Ok(())
    } else {
        Err("modification du registre impossible".into())
    }
}

// ------------------------------------------------------------------ Win32

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::AtomicIsize;

    type Hwnd = isize;
    const WM_DESTROY: u32 = 0x0002;
    const WM_COMMAND: u32 = 0x0111;
    const WM_NULL: u32 = 0x0000;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_CONTEXTMENU: u32 = 0x007B;
    const WM_APP_TRAY: u32 = 0x8000 + 1;
    const NIM_ADD: u32 = 0;
    const NIM_MODIFY: u32 = 1;
    const NIM_DELETE: u32 = 2;
    const NIF_MESSAGE: u32 = 1;
    const NIF_ICON: u32 = 2;
    const NIF_TIP: u32 = 4;
    const NIF_INFO: u32 = 0x10;
    const NIIF_INFO: u32 = 1;
    const MF_STRING: u32 = 0;
    const MF_GRAYED: u32 = 1;
    const MF_SEPARATOR: u32 = 0x800;
    const TPM_RETURNCMD: u32 = 0x100;
    const TPM_RIGHTBUTTON: u32 = 2;
    const SW_HIDE: i32 = 0;
    const SW_SHOW: i32 = 5;
    const SW_RESTORE: i32 = 9;
    const MB_YESNO: u32 = 4;
    const MB_ICONWARNING: u32 = 0x30;
    const MB_SETFOREGROUND: u32 = 0x10000;
    const IDYES: i32 = 6;
    const ID_OPEN: usize = 1;
    const ID_START: usize = 2;
    const ID_STOP: usize = 3;
    const ID_DL_PAUSE: usize = 4;
    const ID_DL_RESUME: usize = 5;
    const ID_QUIT: usize = 9;

    #[repr(C)]
    struct WndClassExW {
        cb_size: u32,
        style: u32,
        wnd_proc: extern "system" fn(Hwnd, u32, usize, isize) -> isize,
        cls_extra: i32,
        wnd_extra: i32,
        instance: *mut c_void,
        icon: *mut c_void,
        cursor: *mut c_void,
        background: *mut c_void,
        menu_name: *const u16,
        class_name: *const u16,
        icon_sm: *mut c_void,
    }

    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: [i32; 2],
        private: u32,
    }

    #[repr(C)]
    struct Guid {
        a: u32,
        b: u16,
        c: u16,
        d: [u8; 8],
    }

    #[repr(C)]
    struct NotifyIconDataW {
        cb_size: u32,
        hwnd: Hwnd,
        id: u32,
        flags: u32,
        callback_message: u32,
        icon: *mut c_void,
        tip: [u16; 128],
        state: u32,
        state_mask: u32,
        info: [u16; 256],
        timeout_or_version: u32,
        info_title: [u16; 64],
        info_flags: u32,
        guid: Guid,
        balloon_icon: *mut c_void,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassExW(c: *const WndClassExW) -> u16;
        fn CreateWindowExW(ex: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: Hwnd, menu: *mut c_void, inst: *mut c_void, param: *mut c_void) -> Hwnd;
        fn DefWindowProcW(h: Hwnd, m: u32, w: usize, l: isize) -> isize;
        fn GetMessageW(m: *mut Msg, h: Hwnd, min: u32, max: u32) -> i32;
        fn TranslateMessage(m: *const Msg) -> i32;
        fn DispatchMessageW(m: *const Msg) -> isize;
        fn PostMessageW(h: Hwnd, m: u32, w: usize, l: isize) -> i32;
        fn CreatePopupMenu() -> *mut c_void;
        fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, text: *const u16) -> i32;
        fn TrackPopupMenu(menu: *mut c_void, flags: u32, x: i32, y: i32, r: i32, h: Hwnd, rect: *const c_void) -> i32;
        fn DestroyMenu(menu: *mut c_void) -> i32;
        fn SetForegroundWindow(h: Hwnd) -> i32;
        fn GetCursorPos(p: *mut [i32; 2]) -> i32;
        fn ShowWindow(h: Hwnd, cmd: i32) -> i32;
        fn IsIconic(h: Hwnd) -> i32;
        fn IsWindowVisible(h: Hwnd) -> i32;
        fn CreateIcon(inst: *mut c_void, w: i32, h: i32, planes: u8, bpp: u8, and: *const u8, xor: *const u8) -> *mut c_void;
        fn RegisterWindowMessageW(name: *const u16) -> u32;
        fn MessageBoxW(h: Hwnd, text: *const u16, caption: *const u16, t: u32) -> i32;
        fn EnumWindows(cb: extern "system" fn(Hwnd, isize) -> i32, l: isize) -> i32;
        fn GetWindowThreadProcessId(h: Hwnd, pid: *mut u32) -> u32;
        fn GetWindowTextW(h: Hwnd, buf: *mut u16, n: i32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetCurrentProcessId() -> u32;
        fn CreateMutexW(attr: *mut c_void, owner: i32, name: *const u16) -> *mut c_void;
        fn GetLastError() -> u32;
    }
    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> Hwnd;
        fn AllowSetForegroundWindow(pid: u32) -> i32;
    }

    /// Message demandant à l'instance déjà lancée de montrer sa fenêtre.
    const WM_APP_SHOW: u32 = 0x8000 + 2;
    static INSTANCE_MUTEX: AtomicIsize = AtomicIsize::new(0);

    /// Une instance tourne déjà ? Si oui, elle est priée de réafficher sa fenêtre.
    pub fn already_running(show_it: bool) -> bool {
        unsafe {
            let m = CreateMutexW(std::ptr::null_mut(), 0, w("Local\\NZBFFToolPro.instance").as_ptr());
            if GetLastError() != 183 {
                // première instance : le verrou vit aussi longtemps que le processus
                INSTANCE_MUTEX.store(m as isize, Ordering::Relaxed);
                return false;
            }
            let tray = FindWindowW(w("NZBFFToolProTray").as_ptr(), std::ptr::null());
            let target = if tray != 0 { tray } else { FindWindowW(std::ptr::null(), w(crate::APP_NAME).as_ptr()) };
            if target != 0 && show_it {
                let mut pid = 0u32;
                GetWindowThreadProcessId(target, &mut pid);
                AllowSetForegroundWindow(pid);
                if tray != 0 {
                    PostMessageW(tray, WM_APP_SHOW, 0, 0);
                } else {
                    show(target, true);
                }
            }
            true
        }
    }
    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(msg: u32, data: *mut NotifyIconDataW) -> i32;
    }

    static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
    static ICON: AtomicIsize = AtomicIsize::new(0);
    static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn fill(dst: &mut [u16], s: &str) {
        let v: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
        dst[..v.len()].copy_from_slice(&v);
        dst[v.len()] = 0;
    }

    fn data(flags: u32) -> NotifyIconDataW {
        NotifyIconDataW {
            cb_size: std::mem::size_of::<NotifyIconDataW>() as u32,
            hwnd: TRAY_HWND.load(Ordering::Relaxed),
            id: 1,
            flags,
            callback_message: WM_APP_TRAY,
            icon: ICON.load(Ordering::Relaxed) as *mut c_void,
            tip: [0; 128],
            state: 0,
            state_mask: 0,
            info: [0; 256],
            timeout_or_version: 0,
            info_title: [0; 64],
            info_flags: 0,
            guid: Guid { a: 0, b: 0, c: 0, d: [0; 8] },
            balloon_icon: std::ptr::null_mut(),
        }
    }

    fn tip_text() -> String {
        let s = status();
        if s.is_empty() {
            crate::APP_NAME.to_string()
        } else {
            // plusieurs lignes : nom, avancement (débit, temps restant), release
            format!("{}\n{s}", crate::APP_NAME)
        }
    }

    /// Icône 32×32 réduite depuis l'image RGBA 128×128.
    fn make_icon(rgba128: &[u8]) -> *mut c_void {
        let (src, dst) = (128usize, 32usize);
        let f = src / dst;
        let mut bgra = vec![0u8; dst * dst * 4];
        let mut and = vec![0u8; dst * dst];
        for y in 0..dst {
            for x in 0..dst {
                let mut acc = [0u32; 4];
                for dy in 0..f {
                    for dx in 0..f {
                        let i = ((y * f + dy) * src + x * f + dx) * 4;
                        for c in 0..4 {
                            acc[c] += *rgba128.get(i + c).unwrap_or(&0) as u32;
                        }
                    }
                }
                let n = (f * f) as u32;
                let o = (y * dst + x) * 4;
                bgra[o] = (acc[2] / n) as u8;
                bgra[o + 1] = (acc[1] / n) as u8;
                bgra[o + 2] = (acc[0] / n) as u8;
                bgra[o + 3] = (acc[3] / n) as u8;
                and[y * dst + x] = bgra[o + 3].wrapping_sub(u8::MAX);
            }
        }
        unsafe { CreateIcon(std::ptr::null_mut(), dst as i32, dst as i32, 1, 32, and.as_ptr(), bgra.as_ptr()) }
    }

    fn add_icon() {
        let mut d = data(NIF_MESSAGE | NIF_ICON | NIF_TIP);
        fill(&mut d.tip, &tip_text());
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &mut d);
        }
    }

    pub fn update_tip() {
        if TRAY_HWND.load(Ordering::Relaxed) == 0 {
            return;
        }
        let mut d = data(NIF_TIP);
        fill(&mut d.tip, &tip_text());
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &mut d);
        }
    }

    pub fn balloon(title: &str, text: &str) {
        if TRAY_HWND.load(Ordering::Relaxed) == 0 {
            return;
        }
        let mut d = data(NIF_INFO);
        fill(&mut d.info, text);
        fill(&mut d.info_title, title);
        d.info_flags = NIIF_INFO;
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &mut d);
        }
    }

    pub fn remove() {
        if TRAY_HWND.load(Ordering::Relaxed) == 0 {
            return;
        }
        let mut d = data(0);
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &mut d);
        }
    }

    struct Found {
        pid: u32,
        title: Vec<u16>,
        hwnd: Hwnd,
    }

    extern "system" fn enum_cb(h: Hwnd, l: isize) -> i32 {
        let f = unsafe { &mut *(l as *mut Found) };
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(h, &mut pid);
        }
        if pid != f.pid || h == TRAY_HWND.load(Ordering::Relaxed) {
            return 1;
        }
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(h, buf.as_mut_ptr(), 256) };
        if n > 0 && buf[..n as usize] == f.title[..f.title.len() - 1] {
            f.hwnd = h;
            return 0;
        }
        1
    }

    /// Fenêtre principale de l'application (même processus, même titre).
    pub fn main_window() -> Option<Hwnd> {
        let mut f = Found { pid: unsafe { GetCurrentProcessId() }, title: w(crate::APP_NAME), hwnd: 0 };
        unsafe {
            EnumWindows(enum_cb, &mut f as *mut Found as isize);
        }
        (f.hwnd != 0).then_some(f.hwnd)
    }

    static MAIN: AtomicIsize = AtomicIsize::new(0);

    /// Poste un WM_PAINT à la fenêtre principale (voir `start_heartbeat`).
    pub fn wake_main() {
        const WM_PAINT: u32 = 0x000F;
        let mut h: Hwnd = MAIN.load(Ordering::Relaxed);
        if h == 0 {
            match main_window() {
                Some(m) => {
                    h = m;
                    MAIN.store(m, Ordering::Relaxed);
                }
                None => return,
            }
        }
        unsafe {
            if PostMessageW(h, WM_PAINT, 0, 0) == 0 {
                // fenêtre recréée : on la cherchera de nouveau
                MAIN.store(0, Ordering::Relaxed);
            }
        }
    }

    pub fn visible(h: Hwnd) -> bool {
        unsafe { IsWindowVisible(h) != 0 }
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, ty: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
    }

    /// Valeur de l'application dans HKCU\…\Run (lue directement, sans passer par reg.exe :
    /// pas de souci d'encodage avec un chemin accentué).
    pub fn run_value() -> Option<String> {
        const HKCU: isize = 0x8000_0001u32 as i32 as isize;
        const RRF_RT_REG_SZ: u32 = 0x2;
        let sub = w(r"Software\Microsoft\Windows\CurrentVersion\Run");
        let name = w(crate::APP_NAME);
        let mut len: u32 = 0;
        unsafe {
            if RegGetValueW(HKCU, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut len) != 0 || len == 0 {
                return None;
            }
            let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
            let mut l2 = (buf.len() * 2) as u32;
            if RegGetValueW(HKCU, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut c_void, &mut l2) != 0 {
                return None;
            }
            let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..end]))
        }
    }

    pub fn show(h: Hwnd, visible: bool) {
        unsafe {
            if visible {
                ShowWindow(h, if IsIconic(h) != 0 { SW_RESTORE } else { SW_SHOW });
                SetForegroundWindow(h);
            } else {
                ShowWindow(h, SW_HIDE);
            }
        }
    }

    fn menu(h: Hwnd) {
        let Some(hooks) = HOOKS.get() else { return };
        unsafe {
            let m = CreatePopupMenu();
            let s = status();
            if !s.is_empty() {
                AppendMenuW(m, MF_STRING | MF_GRAYED, 0, w(&s).as_ptr());
                AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
            }
            AppendMenuW(m, MF_STRING, ID_OPEN, w(&format!("Ouvrir {}", crate::APP_NAME)).as_ptr());
            if (hooks.watching)() {
                AppendMenuW(m, MF_STRING, ID_STOP, w("Arrêter la surveillance").as_ptr());
            } else {
                AppendMenuW(m, MF_STRING, ID_START, w("Démarrer la surveillance").as_ptr());
            }
            match DL_STATE.load(Ordering::Relaxed) {
                1 => {
                    AppendMenuW(m, MF_STRING, ID_DL_PAUSE, w("Mettre les téléchargements en pause").as_ptr());
                }
                2 => {
                    AppendMenuW(m, MF_STRING, ID_DL_RESUME, w("Reprendre les téléchargements").as_ptr());
                }
                _ => {}
            }
            AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(m, MF_STRING, ID_QUIT, w("Quitter").as_ptr());
            let mut pt = [0i32; 2];
            GetCursorPos(&mut pt);
            SetForegroundWindow(h);
            let id = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt[0], pt[1], 0, h, std::ptr::null()) as usize;
            PostMessageW(h, WM_NULL, 0, 0);
            DestroyMenu(m);
            command(h, id);
        }
    }

    fn command(h: Hwnd, id: usize) {
        let Some(hooks) = HOOKS.get() else { return };
        match id {
            ID_OPEN => (hooks.on_cmd)(TrayCmd::Show),
            ID_START => (hooks.on_cmd)(TrayCmd::StartWatch),
            ID_STOP => (hooks.on_cmd)(TrayCmd::StopWatch),
            ID_DL_PAUSE => (hooks.on_cmd)(TrayCmd::DlPause),
            ID_DL_RESUME => (hooks.on_cmd)(TrayCmd::DlResume),
            ID_QUIT => {
                if let Some(work) = (hooks.busy)() {
                    let text = w(&format!("{work}\n\nQuitter maintenant interrompt ce travail. Quitter quand même ?"));
                    let r = unsafe { MessageBoxW(h, text.as_ptr(), w(crate::APP_NAME).as_ptr(), MB_YESNO | MB_ICONWARNING | MB_SETFOREGROUND) };
                    if r != IDYES {
                        return;
                    }
                }
                super::exit_now();
            }
            _ => {}
        }
    }

    extern "system" fn wnd_proc(h: Hwnd, msg: u32, wp: usize, lp: isize) -> isize {
        if msg == WM_APP_SHOW {
            // relancée alors qu'elle tournait déjà : on montre la fenêtre existante
            if let Some(hk) = HOOKS.get() {
                (hk.on_cmd)(TrayCmd::Show);
            }
            return 0;
        }
        if msg == WM_APP_TRAY {
            match (lp & 0xFFFF) as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    if let Some(hk) = HOOKS.get() {
                        (hk.on_cmd)(TrayCmd::Show);
                    }
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => menu(h),
                _ => {}
            }
            return 0;
        }
        if msg == TASKBAR_CREATED.load(Ordering::Relaxed) && msg != 0 {
            add_icon(); // l'explorateur a redémarré : on remet l'icône
            return 0;
        }
        match msg {
            WM_COMMAND => {
                command(h, wp & 0xFFFF);
                0
            }
            WM_DESTROY => {
                remove();
                0
            }
            _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
        }
    }

    pub fn run(rgba128: &'static [u8]) {
        unsafe {
            let inst = GetModuleHandleW(std::ptr::null());
            let class = w("NZBFFToolProTray");
            let wc = WndClassExW {
                cb_size: std::mem::size_of::<WndClassExW>() as u32,
                style: 0,
                wnd_proc,
                cls_extra: 0,
                wnd_extra: 0,
                instance: inst,
                icon: std::ptr::null_mut(),
                cursor: std::ptr::null_mut(),
                background: std::ptr::null_mut(),
                menu_name: std::ptr::null(),
                class_name: class.as_ptr(),
                icon_sm: std::ptr::null_mut(),
            };
            RegisterClassExW(&wc);
            // fenêtre invisible qui reçoit les clics sur l'icône
            let h = CreateWindowExW(0, class.as_ptr(), w("NZB FF Tool Pro (zone de notification)").as_ptr(), 0, 0, 0, 0, 0, 0, std::ptr::null_mut(), inst, std::ptr::null_mut());
            if h == 0 {
                return;
            }
            TRAY_HWND.store(h, Ordering::Relaxed);
            ICON.store(make_icon(rgba128) as isize, Ordering::Relaxed);
            TASKBAR_CREATED.store(RegisterWindowMessageW(w("TaskbarCreated").as_ptr()), Ordering::Relaxed);
            add_icon();
            let mut m = Msg { hwnd: 0, message: 0, wparam: 0, lparam: 0, time: 0, pt: [0; 2], private: 0 };
            while GetMessageW(&mut m, 0, 0, 0) > 0 {
                TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
    }
}
