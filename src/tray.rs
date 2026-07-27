#[cfg(target_os = "windows")]
mod platform {
    use std::{
        sync::{
            OnceLock,
            atomic::{AtomicBool, AtomicIsize, Ordering},
        },
        thread,
    };

    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Shell::{
                    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
                    Shell_NotifyIconW,
                },
                WindowsAndMessaging::{
                    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, IDI_APPLICATION,
                    LoadIconW, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
                    RegisterWindowMessageW, SW_SHOW, SetForegroundWindow, ShowWindow,
                    TPM_BOTTOMALIGN, TPM_RIGHTALIGN, TrackPopupMenu, TranslateMessage, WM_APP,
                    WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSEXW,
                    WS_EX_TOOLWINDOW, WS_OVERLAPPED,
                },
            },
        },
        core::{PCWSTR, w},
    };

    const TRAY_CALLBACK: u32 = WM_APP + 1;
    const MENU_OPEN: usize = 1;
    const MENU_QUIT: usize = 2;
    const ICON_ID: u32 = 1;

    static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);
    static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);
    static TRAY_WINDOW: AtomicIsize = AtomicIsize::new(0);

    fn main_window() -> Option<HWND> {
        match MAIN_WINDOW.load(Ordering::SeqCst) {
            0 => None,
            raw => Some(HWND(raw as *mut core::ffi::c_void)),
        }
    }

    pub(crate) fn remember_main_window(hwnd: isize) {
        MAIN_WINDOW.store(hwnd, Ordering::SeqCst);
    }

    pub(crate) fn exit_requested() -> bool {
        EXIT_REQUESTED.load(Ordering::SeqCst)
    }

    pub(crate) fn hide_main_window() {
        if let Some(hwnd) = main_window() {
            unsafe {
                let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
            }
        }
    }

    fn restore_main_window() {
        let Some(hwnd) = main_window() else {
            return;
        };
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }

    fn show_request_message() -> u32 {
        static MESSAGE: OnceLock<u32> = OnceLock::new();
        *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("MultipleRoblox.ShowWindow")) })
    }

    pub(crate) fn request_show_from_other_instance() {
        let message = show_request_message();
        if message == 0 {
            return;
        }
        unsafe {
            let _ = PostMessageW(
                Some(HWND(0xffff_isize as *mut core::ffi::c_void)),
                message,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    unsafe extern "system" fn tray_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message != 0 && message == show_request_message() {
            restore_main_window();
            return LRESULT(0);
        }

        match message {
            TRAY_CALLBACK => {
                match lparam.0 as u32 {
                    WM_LBUTTONUP => restore_main_window(),
                    WM_RBUTTONUP => show_menu(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                match wparam.0 & 0xffff {
                    MENU_OPEN => restore_main_window(),
                    MENU_QUIT => {
                        EXIT_REQUESTED.store(true, Ordering::SeqCst);

                        if let Some(main) = main_window() {
                            unsafe {
                                let _ = ShowWindow(main, SW_SHOW);
                                let _ = PostMessageW(Some(main), WM_CLOSE, WPARAM(0), LPARAM(0));
                            }
                        }
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    fn show_menu(hwnd: HWND) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN, w!("Open Multiple Roblox"));
            let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("Quit"));

            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            let _ = SetForegroundWindow(hwnd);
            let _ = TrackPopupMenu(
                menu,
                TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
                point.x,
                point.y,
                Some(0),
                hwnd,
                None,
            );
            let _ = DestroyMenu(menu);
        }
    }

    pub(crate) fn install() {
        static STARTED: OnceLock<()> = OnceLock::new();
        if STARTED.set(()).is_err() {
            return;
        }

        thread::Builder::new()
            .name("multiple-rblx-tray".into())
            .spawn(|| {
                if let Err(error) = run_tray() {
                    tracing::warn!(reason = %error, "tray icon unavailable");
                }
            })
            .map(|_| ())
            .unwrap_or_else(|error| {
                tracing::warn!(reason = %error, "tray thread could not start");
            });
    }

    fn run_tray() -> anyhow::Result<()> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class_name: PCWSTR = w!("MultipleRblx.Tray");

            let class = WNDCLASSEXW {
                cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or_default(),
                lpfnWndProc: Some(tray_proc),
                hInstance: instance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };
            let _ = RegisterClassExW(&class);

            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name,
                w!("Multiple Roblox"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            TRAY_WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);

            let icon = LoadIconW(
                Some(instance.into()),
                PCWSTR(std::ptr::without_provenance(1)),
            )
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))?;

            let mut data = NOTIFYICONDATAW {
                cbSize: u32::try_from(size_of::<NOTIFYICONDATAW>()).unwrap_or_default(),
                hWnd: hwnd,
                uID: ICON_ID,
                uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                uCallbackMessage: TRAY_CALLBACK,
                hIcon: icon,
                ..Default::default()
            };
            let tip = "Multiple Roblox";
            for (slot, character) in data.szTip.iter_mut().zip(tip.encode_utf16()) {
                *slot = character;
            }

            if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                let _ = DestroyWindow(hwnd);
                anyhow::bail!("the notification area rejected the icon");
            }
            tracing::info!("tray icon installed");

            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            let _ = DestroyWindow(hwnd);
        }
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    pub(crate) fn install() {}
    pub(crate) fn remember_main_window(_hwnd: isize) {}
    pub(crate) fn exit_requested() -> bool {
        true
    }
    pub(crate) fn hide_main_window() {}
    pub(crate) fn request_show_from_other_instance() {}
}

pub(crate) use platform::{
    exit_requested, hide_main_window, install, remember_main_window,
    request_show_from_other_instance,
};
