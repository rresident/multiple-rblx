use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    fs,
    mem::size_of_val,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    process,
    rc::Rc,
    sync::{Arc, Once, atomic::Ordering, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use url::Url;
use webview2_com::{
    CoTaskMemPWSTR, CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, DownloadStartingEventHandler,
    GetCookiesCompletedHandler, LaunchingExternalUriSchemeEventHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_COLOR, COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC,
        COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_RELEASE_CHANNELS_STABLE,
        COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL,
        CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2_2, ICoreWebView2_4,
        ICoreWebView2_13, ICoreWebView2_14, ICoreWebView2_18, ICoreWebView2Controller,
        ICoreWebView2Controller2, ICoreWebView2Cookie, ICoreWebView2CookieList,
        ICoreWebView2CookieManager, ICoreWebView2Environment, ICoreWebView2Environment10,
        ICoreWebView2EnvironmentOptions, ICoreWebView2EnvironmentOptions2,
        ICoreWebView2EnvironmentOptions6, ICoreWebView2EnvironmentOptions7, ICoreWebView2Settings3,
        ICoreWebView2Settings4,
    },
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    PermissionRequestedEventHandler, ServerCertificateErrorDetectedEventHandler, take_pwstr,
};
use windows::{
    Win32::{
        Foundation::{
            E_INVALIDARG, E_POINTER, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
            HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM,
        },
        Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute},
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
            LibraryLoader::GetModuleHandleW,
            Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_ANY, RegGetValueW},
            Threading::GetCurrentThreadId,
        },
        UI::{
            Shell::{
                FOLDERID_LocalAppData, GetCurrentProcessExplicitAppUserModelID, KF_FLAG_DEFAULT,
                SHGetKnownFolderPath,
            },
            WindowsAndMessaging::{
                CREATESTRUCTW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
                DispatchMessageW, GWLP_USERDATA, GetClientRect, GetMessageW, GetSystemMetrics,
                HICON, IDC_ARROW, IMAGE_ICON, KillTimer, LR_SHARED, LoadCursorW, LoadImageW, MSG,
                PM_NOREMOVE, PeekMessageW, PostMessageW, PostQuitMessage, PostThreadMessageW,
                RegisterClassExW, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SW_SHOW,
                SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
                TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_NCCREATE,
                WM_NCDESTROY, WM_SIZE, WM_TIMER, WNDCLASSEXW, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW,
            },
        },
    },
    core::{BOOL, Interface, PCWSTR, PWSTR, w},
};

use crate::security::MAX_SESSION_BYTES;

use super::{CancellationState, Completion, LoginOutcome, WorkerError};

const ROBLOX_COOKIE_NAME: &str = ".ROBLOSECURITY";
const WINDOW_CLASS: PCWSTR = w!("MultipleRblx.RobloxLogin");
const WINDOW_TITLE: PCWSTR = w!("Opening Roblox sign-in | Multiple Roblox");
const APP_ICON_RESOURCE_ID: usize = 1;
const WINDOW_WIDTH: i32 = 900;
const WINDOW_HEIGHT: i32 = 720;
const COOKIE_POLL_TIMER: usize = 1;
const COOKIE_POLL_INTERVAL_MS: u32 = 500;
const WM_LOGIN_CANCEL: u32 = WM_APP + 0x31;
const WM_LOGIN_COMPLETE: u32 = WM_APP + 0x32;
const USER_DATA_PREFIX: &str = "multiple-rblx-login-";
const STALE_USER_DATA_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_STALE_USER_DATA_DIRECTORIES: usize = 32;
const WEBVIEW_ENVIRONMENT_OVERRIDES: [&str; 8] = [
    "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
    "WEBVIEW2_USER_DATA_FOLDER",
    "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
    "WEBVIEW2_CHANNEL_SEARCH_KIND",
    "WEBVIEW2_RELEASE_CHANNELS",
    "WEBVIEW2_RELEASE_CHANNEL_PREFERENCE",
    "WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER",
    "WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER",
];
const WEBVIEW_POLICY_NAMES: [&str; 6] = [
    "BrowserExecutableFolder",
    "UserDataFolder",
    "AdditionalBrowserArguments",
    "ChannelSearchKind",
    "ReleaseChannels",
    "ReleaseChannelPreference",
];
const WEBVIEW_POLICY_ROOT: &str = r"Software\Policies\Microsoft\Edge\WebView2";
const LEGACY_WEBVIEW_POLICY_ROOT: &str =
    r"Software\Policies\Microsoft\EmbeddedBrowserWebView\LoaderOverride";
const LEGACY_WEBVIEW_POLICY_VALUES: [&str; 4] = [
    "browserExecutableFolder",
    "userDataFolder",
    "additionalBrowserArguments",
    "releaseChannelPreference",
];

fn reject_external_webview_configuration() -> Result<(), WorkerError> {
    for name in WEBVIEW_ENVIRONMENT_OVERRIDES {
        if std::env::var_os(name).is_some_and(|value| !value.is_empty()) {
            return Err(WorkerError::Unavailable(format!(
                "WebView2 developer override {name} is active; secure sign-in is disabled"
            )));
        }
    }

    if std::env::args_os().any(|argument| is_edge_webview_switch(&argument)) {
        return Err(WorkerError::Unavailable(
            "a WebView2 browser-switch command line override is active; secure sign-in is disabled"
                .into(),
        ));
    }

    reject_webview_registry_policies()
}

fn is_edge_webview_switch(argument: &std::ffi::OsStr) -> bool {
    let argument = argument.to_string_lossy().to_ascii_lowercase();
    argument == "--edge-webview-switches" || argument.starts_with("--edge-webview-switches=")
}

fn reject_webview_registry_policies() -> Result<(), WorkerError> {
    let mut app_ids = vec!["multiple-rblx".to_owned(), "*".to_owned()];
    if let Some(app_id) = explicit_app_user_model_id() {
        app_ids.push(app_id);
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(name) = executable.file_name().and_then(|name| name.to_str()) {
            app_ids.push(name.to_owned());
        }
        if let Some(stem) = executable.file_stem().and_then(|name| name.to_str()) {
            app_ids.push(stem.to_owned());
        }
    }
    app_ids.sort();
    app_ids.dedup();

    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for policy in WEBVIEW_POLICY_NAMES {
            let subkey = format!(r"{WEBVIEW_POLICY_ROOT}\{policy}");
            for app_id in &app_ids {
                if registry_value_exists(root, &subkey, app_id)? {
                    return Err(WorkerError::Unavailable(format!(
                        "WebView2 policy override {policy} is active; secure sign-in is disabled"
                    )));
                }
            }
        }

        for app_id in &app_ids {
            let subkey = format!(r"{LEGACY_WEBVIEW_POLICY_ROOT}\{app_id}");
            for value_name in LEGACY_WEBVIEW_POLICY_VALUES {
                if registry_value_exists(root, &subkey, value_name)? {
                    return Err(WorkerError::Unavailable(
                        "a legacy WebView2 loader policy is active; secure sign-in is disabled"
                            .into(),
                    ));
                }
            }
        }
    }

    Ok(())
}

fn explicit_app_user_model_id() -> Option<String> {
    let raw = unsafe { GetCurrentProcessExplicitAppUserModelID() }.ok()?;
    let value = unsafe { raw.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    value.filter(|value| !value.is_empty())
}

fn registry_value_exists(
    root: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    value_name: &str,
) -> Result<bool, WorkerError> {
    let subkey = wide_null(std::ffi::OsStr::new(subkey));
    let value_name = wide_null(std::ffi::OsStr::new(value_name));
    let mut byte_count = 0_u32;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value_name.as_ptr()),
            RRF_RT_ANY,
            None,
            None,
            Some(&mut byte_count),
        )
    };

    if status == ERROR_SUCCESS {
        return Ok(true);
    }
    if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
        return Ok(false);
    }

    Err(WorkerError::Unavailable(format!(
        "Windows could not verify WebView2 security policy state (error {})",
        status.0
    )))
}

pub(super) fn spawn_login_thread(
    completion: Arc<Completion>,
    cancellation: Arc<CancellationState>,
    ready: async_channel::Sender<()>,
) -> std::io::Result<()> {
    thread::Builder::new()
        .name("roblox-login-webview".into())
        .spawn(move || {
            tracing::debug!("native WebView2 login thread started");
            let guarded = catch_unwind(AssertUnwindSafe(|| {
                run_sta_login(completion.clone(), cancellation, ready)
            }));

            match guarded {
                Ok(Ok(())) => {
                    if !completion.is_completed() {
                        completion.try_complete(LoginOutcome::Cancelled);
                    }
                }
                Ok(Err(error)) => {
                    completion.try_complete_worker_error(error);
                }
                Err(_) => {
                    tracing::error!("native WebView2 login thread panicked");
                    completion.try_complete_worker_error(WorkerError::Internal(
                        "the native login worker panicked".into(),
                    ));
                }
            }
            tracing::debug!("native WebView2 login thread stopped");
        })
        .map(|_| ())
}

pub(super) fn wake_login_thread(cancellation: &CancellationState) {
    let thread_id = cancellation
        .thread_id
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(thread_id) = *thread_id {
        let _ = unsafe {
            PostThreadMessageW(
                thread_id,
                WM_LOGIN_CANCEL,
                WPARAM(cancellation.message_token),
                LPARAM::default(),
            )
        };
    }
}

fn run_sta_login(
    completion: Arc<Completion>,
    cancellation: Arc<CancellationState>,
    ready: async_channel::Sender<()>,
) -> Result<(), WorkerError> {
    reject_external_webview_configuration()?;
    let _apartment = StaApartment::initialize()?;
    tracing::debug!("WebView2 STA initialized");
    let _thread_registration = LoginThreadRegistration::new(cancellation.clone());

    if cancellation.requested.load(Ordering::Acquire) {
        return Err(WorkerError::Cancelled);
    }

    let user_data = TemporaryUserData::create()?;
    tracing::debug!("isolated WebView2 data directory created");
    register_window_class();

    let state = Rc::new(BrowserState::new(completion.clone()));
    let window_data = Box::new(WindowData {
        state: state.clone(),
    });
    let hwnd = create_login_window((&*window_data) as *const WindowData as *const c_void)?;
    state.hwnd.set(hwnd);
    set_dark_titlebar(hwnd);

    let result = (|| {
        if cancellation.requested.load(Ordering::Acquire) {
            completion.try_complete(LoginOutcome::Cancelled);
            return Ok(());
        }

        let environment = create_environment(user_data.path(), &cancellation)?;
        tracing::debug!("stable WebView2 environment created");
        let controller = create_in_private_controller(&environment, hwnd, &cancellation)?;
        tracing::debug!("InPrivate WebView2 controller created");
        configure_controller(&state, controller, user_data.path())?;
        tracing::debug!("Roblox sign-in surface hardened and configured");

        if cancellation.requested.load(Ordering::Acquire) {
            completion.try_complete(LoginOutcome::Cancelled);
            return Ok(());
        }

        show_login_window(hwnd);
        let _ = ready.try_send(());
        tracing::debug!("Roblox sign-in window is visible");

        run_message_loop(hwnd, &completion, &cancellation)
    })();

    let _ = unsafe { KillTimer(Some(hwnd), COOKIE_POLL_TIMER) };
    let _ = unsafe { DestroyWindow(hwnd) };
    state.close_controller();

    drop(window_data);
    drop(state);
    drop(user_data);
    result
}

struct StaApartment;

impl StaApartment {
    fn initialize() -> Result<Self, WorkerError> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|error| unavailable("initializing the WebView2 STA", error))?;
        Ok(Self)
    }
}

impl Drop for StaApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct LoginThreadRegistration {
    cancellation: Arc<CancellationState>,
}

impl LoginThreadRegistration {
    fn new(cancellation: Arc<CancellationState>) -> Self {
        let mut message = MSG::default();
        unsafe {
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
        }
        *cancellation
            .thread_id
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(unsafe { GetCurrentThreadId() });
        Self { cancellation }
    }
}

impl Drop for LoginThreadRegistration {
    fn drop(&mut self) {
        *self
            .cancellation
            .thread_id
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
    }
}

fn register_window_class() {
    let instance = unsafe { GetModuleHandleW(None) }
        .ok()
        .map(|module| HINSTANCE(module.0));
    let icon = load_app_icon(instance, SM_CXICON, SM_CYICON);
    let small_icon = load_app_icon(instance, SM_CXSMICON, SM_CYSMICON);
    let class = WNDCLASSEXW {
        cbSize: size_of_val(&WNDCLASSEXW::default()) as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance.unwrap_or_default(),
        hIcon: icon,
        hIconSm: small_icon,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        lpszClassName: WINDOW_CLASS,
        ..Default::default()
    };

    let _ = unsafe { RegisterClassExW(&class) };
}

fn load_app_icon(
    instance: Option<HINSTANCE>,
    width_metric: windows::Win32::UI::WindowsAndMessaging::SYSTEM_METRICS_INDEX,
    height_metric: windows::Win32::UI::WindowsAndMessaging::SYSTEM_METRICS_INDEX,
) -> HICON {
    let Some(instance) = instance else {
        return HICON::default();
    };
    let width = unsafe { GetSystemMetrics(width_metric) };
    let height = unsafe { GetSystemMetrics(height_metric) };
    unsafe {
        LoadImageW(
            Some(instance),
            PCWSTR(APP_ICON_RESOURCE_ID as *const u16),
            IMAGE_ICON,
            width,
            height,
            LR_SHARED,
        )
    }
    .map(|icon| HICON(icon.0))
    .unwrap_or_default()
}

fn create_login_window(state: *const c_void) -> Result<HWND, WorkerError> {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            WINDOW_CLASS,
            WINDOW_TITLE,
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            None,
            None,
            GetModuleHandleW(None)
                .ok()
                .map(|module| HINSTANCE(module.0)),
            Some(state),
        )
    }
    .map_err(|error| internal("creating the Roblox login window", error))
}

fn set_dark_titlebar(hwnd: HWND) {
    let enabled = BOOL(1);
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&enabled as *const BOOL).cast(),
            size_of_val(&enabled) as u32,
        )
    };
}

fn create_environment(
    user_data_path: &Path,
    cancellation: &CancellationState,
) -> Result<ICoreWebView2Environment, WorkerError> {
    let options: ICoreWebView2EnvironmentOptions = CoreWebView2EnvironmentOptions::default().into();

    unsafe {
        options
            .cast::<ICoreWebView2EnvironmentOptions2>()
            .and_then(|options| options.SetExclusiveUserDataFolderAccess(true))
            .map_err(|error| unavailable("configuring an isolated WebView2 data folder", error))?;
        options
            .cast::<ICoreWebView2EnvironmentOptions6>()
            .and_then(|options| options.SetAreBrowserExtensionsEnabled(false))
            .map_err(|error| unavailable("disabling WebView2 extensions", error))?;
        options
            .cast::<ICoreWebView2EnvironmentOptions7>()
            .and_then(|options| options.SetReleaseChannels(COREWEBVIEW2_RELEASE_CHANNELS_STABLE))
            .map_err(|error| unavailable("selecting the stable WebView2 runtime", error))?;
    }

    let user_data_wide = wide_null(user_data_path.as_os_str());
    let (sender, receiver) = mpsc::channel();
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |status, environment| {
            let result = status
                .and_then(|()| environment.ok_or_else(|| windows::core::Error::from(E_POINTER)));
            let _ = sender.send(result);
            Ok(())
        },
    ));

    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR(user_data_wide.as_ptr()),
            &options,
            &handler,
        )
    }
    .map_err(|error| unavailable("starting the WebView2 runtime", error))?;

    wait_for_webview_callback(receiver, cancellation)
        .and_then(|result| result.map_err(|error| unavailable("creating WebView2", error)))
}

fn create_in_private_controller(
    environment: &ICoreWebView2Environment,
    hwnd: HWND,
    cancellation: &CancellationState,
) -> Result<ICoreWebView2Controller, WorkerError> {
    let environment10 = environment
        .cast::<ICoreWebView2Environment10>()
        .map_err(|error| {
            unavailable("WebView2 InPrivate profiles require a newer runtime", error)
        })?;
    let options = unsafe { environment10.CreateCoreWebView2ControllerOptions() }
        .map_err(|error| unavailable("creating WebView2 controller options", error))?;

    unsafe {
        options
            .SetProfileName(w!("MultipleRblxLogin"))
            .map_err(|error| unavailable("setting the WebView2 login profile", error))?;
        options
            .SetIsInPrivateModeEnabled(true)
            .map_err(|error| unavailable("enabling WebView2 InPrivate mode", error))?;
    }

    let (sender, receiver) = mpsc::channel();
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |status, controller| {
            let result = status
                .and_then(|()| controller.ok_or_else(|| windows::core::Error::from(E_POINTER)));
            let _ = sender.send(result);
            Ok(())
        },
    ));

    unsafe { environment10.CreateCoreWebView2ControllerWithOptions(hwnd, &options, &handler) }
        .map_err(|error| unavailable("starting the InPrivate WebView2 controller", error))?;

    wait_for_webview_callback(receiver, cancellation).and_then(|result| {
        result.map_err(|error| unavailable("creating the InPrivate WebView2 controller", error))
    })
}

fn wait_for_webview_callback<T>(
    receiver: mpsc::Receiver<windows::core::Result<T>>,
    cancellation: &CancellationState,
) -> Result<windows::core::Result<T>, WorkerError> {
    let mut message = MSG::default();

    loop {
        if cancellation.requested.load(Ordering::Acquire) {
            return Err(WorkerError::Cancelled);
        }
        if let Ok(value) = receiver.try_recv() {
            return Ok(value);
        }

        let status = unsafe { GetMessageW(&mut message, None, 0, 0).0 };
        match status {
            -1 => {
                return Err(WorkerError::Internal(
                    "the Windows message loop failed during WebView2 startup".into(),
                ));
            }
            0 => return Err(WorkerError::Cancelled),
            _ if message.message == WM_LOGIN_CANCEL
                && message.wParam.0 == cancellation.message_token =>
            {
                return Err(WorkerError::Cancelled);
            }
            _ => unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
}

fn configure_controller(
    state: &Rc<BrowserState>,
    controller: ICoreWebView2Controller,
    expected_user_data_root: &Path,
) -> Result<(), WorkerError> {
    let webview = unsafe { controller.CoreWebView2() }
        .map_err(|error| internal("opening the WebView2 content surface", error))?;

    verify_in_private_profile(&webview, expected_user_data_root)?;
    configure_settings(&webview)?;
    install_navigation_guard(&webview, state.hwnd.get())?;
    install_external_uri_guard(&webview)?;
    install_new_window_guard(&webview)?;
    install_download_guard(&webview)?;
    install_permission_guard(&webview)?;
    install_certificate_guard(&webview)?;

    let cookie_manager = unsafe {
        webview
            .cast::<ICoreWebView2_2>()
            .and_then(|webview| webview.CookieManager())
    }
    .map_err(|error| internal("opening the isolated WebView2 cookie store", error))?;

    if let Ok(controller2) = controller.cast::<ICoreWebView2Controller2>() {
        let background = COREWEBVIEW2_COLOR {
            A: 255,
            R: 20,
            G: 22,
            B: 25,
        };
        let _ = unsafe { controller2.SetDefaultBackgroundColor(background) };
    }

    *state.controller.borrow_mut() = Some(controller);
    *state.webview.borrow_mut() = Some(webview.clone());
    *state.cookie_manager.borrow_mut() = Some(cookie_manager);
    state.resize();

    unsafe {
        controller_set_visible_and_focus(state)?;
        webview
            .Navigate(w!("https://www.roblox.com/login"))
            .map_err(|error| internal("navigating to Roblox sign-in", error))?;
        if SetTimer(
            Some(state.hwnd.get()),
            COOKIE_POLL_TIMER,
            COOKIE_POLL_INTERVAL_MS,
            None,
        ) == 0
        {
            return Err(WorkerError::Internal(
                "could not start the secure-cookie poll timer".into(),
            ));
        }
    }

    Ok(())
}

unsafe fn controller_set_visible_and_focus(state: &BrowserState) -> Result<(), WorkerError> {
    let controller = state.controller.borrow();
    let controller = controller.as_ref().ok_or_else(|| {
        WorkerError::Internal("WebView2 controller was lost during initialization".into())
    })?;

    unsafe {
        controller
            .SetIsVisible(true)
            .and_then(|()| controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC))
            .map_err(|error| internal("showing the WebView2 login surface", error))
    }
}

fn verify_in_private_profile(
    webview: &ICoreWebView2,
    expected_user_data_root: &Path,
) -> Result<(), WorkerError> {
    let profile = unsafe {
        webview
            .cast::<ICoreWebView2_13>()
            .and_then(|webview| webview.Profile())
    }
    .map_err(|error| unavailable("verifying the WebView2 InPrivate profile", error))?;
    let mut in_private = BOOL(0);
    unsafe { profile.IsInPrivateModeEnabled(&mut in_private) }
        .map_err(|error| unavailable("checking WebView2 InPrivate mode", error))?;

    if !in_private.as_bool() {
        return Err(WorkerError::Unavailable(
            "WebView2 did not honor the requested InPrivate profile".into(),
        ));
    }

    let profile_path = take_com_string(|raw_path| unsafe { profile.ProfilePath(raw_path) })
        .map_err(|error| unavailable("reading the WebView2 profile path", error))?;
    let expected_root = expected_user_data_root.canonicalize().map_err(|error| {
        WorkerError::Unavailable(format!(
            "could not verify the isolated WebView2 data folder: {error}"
        ))
    })?;
    let actual_profile = PathBuf::from(profile_path)
        .canonicalize()
        .map_err(|error| {
            WorkerError::Unavailable(format!(
                "could not verify the active WebView2 profile folder: {error}"
            ))
        })?;
    if !actual_profile.starts_with(&expected_root) {
        return Err(WorkerError::Unavailable(
            "WebView2 did not honor the isolated user data folder".into(),
        ));
    }
    Ok(())
}

fn configure_settings(webview: &ICoreWebView2) -> Result<(), WorkerError> {
    let settings = unsafe { webview.Settings() }
        .map_err(|error| internal("opening WebView2 security settings", error))?;

    unsafe {
        settings
            .SetAreDevToolsEnabled(false)
            .and_then(|()| settings.SetAreDefaultContextMenusEnabled(false))
            .and_then(|()| settings.SetIsStatusBarEnabled(false))
            .and_then(|()| settings.SetAreDefaultScriptDialogsEnabled(false))
            .and_then(|()| settings.SetAreHostObjectsAllowed(false))
            .and_then(|()| settings.SetIsWebMessageEnabled(false))
            .map_err(|error| internal("hardening the WebView2 login surface", error))?;
    }

    if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
        let _ = unsafe { settings3.SetAreBrowserAcceleratorKeysEnabled(true) };
    }
    if let Ok(settings4) = settings.cast::<ICoreWebView2Settings4>() {
        unsafe {
            settings4
                .SetIsPasswordAutosaveEnabled(false)
                .and_then(|()| settings4.SetIsGeneralAutofillEnabled(false))
                .map_err(|error| internal("disabling WebView2 credential retention", error))?;
        }
    }

    Ok(())
}

fn install_navigation_guard(webview: &ICoreWebView2, hwnd: HWND) -> Result<(), WorkerError> {
    let handler = NavigationStartingEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            let verified_host = take_com_string(|raw_uri| unsafe { args.Uri(raw_uri) })
                .ok()
                .and_then(|uri| allowed_roblox_host(&uri));
            if verified_host.is_some() {
                set_login_window_title(hwnd);
            } else {
                tracing::warn!("blocked non-Roblox top-level navigation in sign-in window");
                unsafe { args.SetCancel(true) }?;
            }
        }
        Ok(())
    }));
    let mut token = 0;
    unsafe { webview.add_NavigationStarting(&handler, &mut token) }
        .map_err(|error| internal("installing the Roblox navigation guard", error))
}

fn install_new_window_guard(webview: &ICoreWebView2) -> Result<(), WorkerError> {
    let handler = NewWindowRequestedEventHandler::create(Box::new(move |sender, args| {
        if let Some(args) = args {
            let uri = take_com_string(|raw_uri| unsafe { args.Uri(raw_uri) }).unwrap_or_default();

            unsafe { args.SetHandled(true) }?;
            if is_allowed_roblox_url(&uri)
                && let Some(sender) = sender
            {
                let uri = CoTaskMemPWSTR::from(uri.as_str());
                unsafe { sender.Navigate(*uri.as_ref().as_pcwstr()) }?;
            }
        }
        Ok(())
    }));
    let mut token = 0;
    unsafe { webview.add_NewWindowRequested(&handler, &mut token) }
        .map_err(|error| internal("disabling WebView2 popup windows", error))
}

fn install_external_uri_guard(webview: &ICoreWebView2) -> Result<(), WorkerError> {
    let webview18 = webview.cast::<ICoreWebView2_18>().map_err(|error| {
        unavailable(
            "WebView2 external-protocol blocking requires a newer runtime",
            error,
        )
    })?;
    let handler = LaunchingExternalUriSchemeEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            tracing::warn!("blocked external protocol launch from sign-in window");
            unsafe { args.SetCancel(true) }?;
        }
        Ok(())
    }));
    let mut token = 0;
    unsafe { webview18.add_LaunchingExternalUriScheme(&handler, &mut token) }
        .map_err(|error| internal("installing the WebView2 external-protocol guard", error))
}

fn install_download_guard(webview: &ICoreWebView2) -> Result<(), WorkerError> {
    let webview4 = webview.cast::<ICoreWebView2_4>().map_err(|error| {
        unavailable("WebView2 download blocking requires a newer runtime", error)
    })?;
    let handler = DownloadStartingEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            unsafe { args.SetCancel(true) }?;
        }
        Ok(())
    }));
    let mut token = 0;
    unsafe { webview4.add_DownloadStarting(&handler, &mut token) }
        .map_err(|error| internal("disabling WebView2 downloads", error))
}

fn install_permission_guard(webview: &ICoreWebView2) -> Result<(), WorkerError> {
    let handler = PermissionRequestedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            unsafe { args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY) }?;
        }
        Ok(())
    }));
    let mut token = 0;
    unsafe { webview.add_PermissionRequested(&handler, &mut token) }
        .map_err(|error| internal("installing the WebView2 permission guard", error))
}

fn install_certificate_guard(webview: &ICoreWebView2) -> Result<(), WorkerError> {
    let Ok(webview14) = webview.cast::<ICoreWebView2_14>() else {
        return Ok(());
    };
    let handler =
        ServerCertificateErrorDetectedEventHandler::create(Box::new(move |_sender, args| {
            if let Some(args) = args {
                unsafe { args.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL) }?;
            }
            Ok(())
        }));
    let mut token = 0;
    unsafe { webview14.add_ServerCertificateErrorDetected(&handler, &mut token) }
        .map_err(|error| internal("installing the WebView2 certificate guard", error))
}

fn is_allowed_roblox_url(candidate: &str) -> bool {
    allowed_roblox_host(candidate).is_some()
}

fn allowed_roblox_host(candidate: &str) -> Option<String> {
    let Ok(url) = Url::parse(candidate) else {
        return None;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.port(), None | Some(443))
    {
        return None;
    }

    let host = url.host_str()?;
    let allowed = host.eq_ignore_ascii_case("roblox.com")
        || host
            .to_ascii_lowercase()
            .strip_suffix(".roblox.com")
            .is_some_and(|prefix| !prefix.is_empty());
    allowed.then(|| host.to_ascii_lowercase())
}

fn set_login_window_title(hwnd: HWND) {
    let _ = unsafe { SetWindowTextW(hwnd, w!("Sign in to Roblox | Multiple Roblox")) };
}

struct BrowserState {
    hwnd: Cell<HWND>,
    controller: RefCell<Option<ICoreWebView2Controller>>,
    webview: RefCell<Option<ICoreWebView2>>,
    cookie_manager: RefCell<Option<ICoreWebView2CookieManager>>,
    cookie_poll_in_flight: Cell<bool>,
    completion: Arc<Completion>,
}

impl BrowserState {
    fn new(completion: Arc<Completion>) -> Self {
        Self {
            hwnd: Cell::new(HWND::default()),
            controller: RefCell::new(None),
            webview: RefCell::new(None),
            cookie_manager: RefCell::new(None),
            cookie_poll_in_flight: Cell::new(false),
            completion,
        }
    }

    fn resize(&self) {
        let Some(controller) = self.controller.borrow().as_ref().cloned() else {
            return;
        };
        let mut bounds = RECT::default();
        if unsafe { GetClientRect(self.hwnd.get(), &mut bounds) }.is_ok() {
            let _ = unsafe { controller.SetBounds(bounds) };
        }
    }

    fn poll_cookie(self: &Rc<Self>) {
        if self.completion.is_completed() || self.cookie_poll_in_flight.replace(true) {
            return;
        }

        let Some(cookie_manager) = self.cookie_manager.borrow().as_ref().cloned() else {
            self.cookie_poll_in_flight.set(false);
            return;
        };
        let weak_state = Rc::downgrade(self);
        let handler = GetCookiesCompletedHandler::create(Box::new(move |status, cookies| {
            let Some(state) = weak_state.upgrade() else {
                return Ok(());
            };
            state.cookie_poll_in_flight.set(false);

            if status.is_ok()
                && let Some(cookies) = cookies
                && let Some(secret) = find_roblox_cookie(&cookies)?
                && state.completion.try_complete(LoginOutcome::Cookie(secret))
            {
                tracing::debug!("accepted secure HttpOnly Roblox session cookie; value redacted");
                let _ = unsafe {
                    PostMessageW(
                        Some(state.hwnd.get()),
                        WM_LOGIN_COMPLETE,
                        WPARAM::default(),
                        LPARAM::default(),
                    )
                };
            }
            Ok(())
        }));

        let result = unsafe { cookie_manager.GetCookies(w!("https://www.roblox.com/"), &handler) };
        if result.is_err() {
            self.cookie_poll_in_flight.set(false);
        }
    }

    fn close_controller(&self) {
        self.cookie_manager.borrow_mut().take();
        self.webview.borrow_mut().take();
        if let Some(controller) = self.controller.borrow_mut().take() {
            let _ = unsafe { controller.Close() };
        }
    }
}

fn find_roblox_cookie(
    cookies: &ICoreWebView2CookieList,
) -> windows::core::Result<Option<SecretString>> {
    let mut count = 0;
    unsafe { cookies.Count(&mut count) }?;

    for index in 0..count {
        let cookie = unsafe { cookies.GetValueAtIndex(index) }?;
        if is_target_cookie(&cookie)? {
            let secret = take_secret_com_string(|raw_value| unsafe { cookie.Value(raw_value) })?;
            if !secret.expose_secret().is_empty()
                && secret.expose_secret().len() <= MAX_SESSION_BYTES
            {
                return Ok(Some(secret));
            }
        }
    }
    Ok(None)
}

fn is_target_cookie(cookie: &ICoreWebView2Cookie) -> windows::core::Result<bool> {
    let mut secure = BOOL(0);
    let mut http_only = BOOL(0);
    unsafe {
        cookie.IsSecure(&mut secure)?;
        cookie.IsHttpOnly(&mut http_only)?;
    }

    let name = take_com_string(|raw_name| unsafe { cookie.Name(raw_name) })?;
    let domain = take_com_string(|raw_domain| unsafe { cookie.Domain(raw_domain) })?;
    Ok(name == ROBLOX_COOKIE_NAME
        && secure.as_bool()
        && http_only.as_bool()
        && domain
            .trim_start_matches('.')
            .eq_ignore_ascii_case("roblox.com"))
}

struct WindowData {
    state: Rc<BrowserState>,
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = l_param.0 as *const CREATESTRUCTW;
        if !create.is_null() {
            let state = unsafe { (*create).lpCreateParams } as *const WindowData;
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize) };
        }
    }

    let data = unsafe {
        let pointer =
            windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWLP_USERDATA)
                as *const WindowData;
        pointer.as_ref()
    };

    let Some(data) = data else {
        return unsafe { DefWindowProcW(hwnd, message, w_param, l_param) };
    };
    let state = &data.state;

    match message {
        WM_SIZE => {
            state.resize();
            LRESULT::default()
        }
        WM_TIMER if w_param.0 == COOKIE_POLL_TIMER => {
            state.poll_cookie();
            LRESULT::default()
        }
        WM_CLOSE => {
            state.completion.try_complete(LoginOutcome::Cancelled);
            let _ = unsafe { KillTimer(Some(hwnd), COOKIE_POLL_TIMER) };
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT::default()
        }
        WM_LOGIN_COMPLETE => {
            let _ = unsafe { KillTimer(Some(hwnd), COOKIE_POLL_TIMER) };
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT::default()
        }
        WM_DESTROY => {
            state.close_controller();
            unsafe { PostQuitMessage(0) };
            LRESULT::default()
        }
        WM_NCDESTROY => {
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            unsafe { DefWindowProcW(hwnd, message, w_param, l_param) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, w_param, l_param) },
    }
}

fn run_message_loop(
    hwnd: HWND,
    completion: &Completion,
    cancellation: &CancellationState,
) -> Result<(), WorkerError> {
    let mut message = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut message, None, 0, 0).0 };
        match status {
            -1 => {
                return Err(WorkerError::Internal(
                    "the Windows message loop failed".into(),
                ));
            }
            0 => return Ok(()),
            _ if (message.message == WM_LOGIN_CANCEL
                && message.wParam.0 == cancellation.message_token)
                || cancellation.requested.load(Ordering::Acquire) =>
            {
                completion.try_complete(LoginOutcome::Cancelled);
                tracing::debug!("closing Roblox sign-in window after cancellation");
                let _ = unsafe { KillTimer(Some(hwnd), COOKIE_POLL_TIMER) };
                let _ = unsafe { DestroyWindow(hwnd) };
                return Ok(());
            }
            _ => unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
}

struct TemporaryUserData {
    path: Option<PathBuf>,
    root: PathBuf,
}

impl TemporaryUserData {
    fn create() -> Result<Self, WorkerError> {
        let root = auth_browser_data_root()?;
        schedule_stale_user_data_cleanup(&root);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        for suffix in 0..16_u8 {
            let path = root.join(format!(
                "{USER_DATA_PREFIX}{}-{stamp}-{suffix}",
                process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    let path = path.canonicalize().map_err(|error| {
                        WorkerError::Unavailable(format!(
                            "could not verify the WebView2 data folder: {error}"
                        ))
                    })?;
                    if path.parent() != Some(root.as_path()) {
                        return Err(WorkerError::Unavailable(
                            "the WebView2 data folder resolved outside the application directory"
                                .into(),
                        ));
                    }
                    return Ok(Self {
                        path: Some(path),
                        root,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(WorkerError::Unavailable(format!(
                        "could not create an isolated WebView2 data folder: {error}"
                    )));
                }
            }
        }

        Err(WorkerError::Unavailable(
            "could not allocate a unique WebView2 data folder".into(),
        ))
    }

    fn path(&self) -> &Path {
        self.path
            .as_deref()
            .expect("temporary WebView2 data path should exist until drop")
    }
}

impl Drop for TemporaryUserData {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        let safe_target = path.parent() == Some(self.root.as_path())
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(USER_DATA_PREFIX));
        if !safe_target {
            tracing::error!("refused to clean an unexpected WebView2 data path");
            return;
        }

        tracing::debug!("temporary WebView2 data cleanup scheduled");
        if let Err(error) = thread::Builder::new()
            .name("roblox-login-cleanup".into())
            .spawn(move || remove_temporary_user_data(path))
        {
            tracing::warn!(
                reason = %error,
                "temporary WebView2 data cleanup could not be scheduled"
            );
        }
    }
}

fn auth_browser_data_root() -> Result<PathBuf, WorkerError> {
    let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None) }
        .map_err(|error| unavailable("finding the local application-data directory", error))?;
    let local_app_data = take_pwstr(raw);
    if local_app_data.is_empty() {
        return Err(WorkerError::Unavailable(
            "Windows returned an empty local application-data directory".into(),
        ));
    }

    let local_app_data = PathBuf::from(local_app_data)
        .canonicalize()
        .map_err(|error| {
            WorkerError::Unavailable(format!(
                "could not verify the local application-data directory: {error}"
            ))
        })?;
    let root = local_app_data.join("MultipleRoblox").join("BrowserData");
    fs::create_dir_all(&root).map_err(|error| {
        WorkerError::Unavailable(format!(
            "could not create the WebView2 data directory: {error}"
        ))
    })?;
    let root = root.canonicalize().map_err(|error| {
        WorkerError::Unavailable(format!(
            "could not verify the WebView2 data directory: {error}"
        ))
    })?;
    if !root.starts_with(&local_app_data) {
        return Err(WorkerError::Unavailable(
            "the WebView2 data directory resolved outside local application data".into(),
        ));
    }
    Ok(root)
}

fn schedule_stale_user_data_cleanup(root: &Path) {
    static SWEEP_STARTED: Once = Once::new();
    let root = root.to_path_buf();
    SWEEP_STARTED.call_once(move || {
        let Ok(entries) = fs::read_dir(&root) else {
            return;
        };
        let stale = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let safe_target = path.parent() == Some(root.as_path())
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(USER_DATA_PREFIX))
                    && path.is_dir();
                let old_enough = entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age >= STALE_USER_DATA_AGE);
                (safe_target && old_enough).then_some(path)
            })
            .take(MAX_STALE_USER_DATA_DIRECTORIES)
            .collect::<Vec<_>>();
        if stale.is_empty() {
            return;
        }

        let _ = thread::Builder::new()
            .name("roblox-login-scavenge".into())
            .spawn(move || {
                for path in stale {
                    remove_temporary_user_data_with_attempts(path, 4);
                }
            });
    });
}

fn remove_temporary_user_data(path: PathBuf) {
    remove_temporary_user_data_with_attempts(path, 60);
}

fn remove_temporary_user_data_with_attempts(path: PathBuf, attempts: u32) {
    thread::sleep(Duration::from_millis(250));
    for attempt in 1..=attempts {
        match fs::remove_dir_all(&path) {
            Ok(()) => {
                tracing::debug!("removed temporary WebView2 data directory");
                return;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) if attempt < attempts => thread::sleep(Duration::from_millis(250)),
            Err(error) => {
                tracing::warn!(
                    reason = %error,
                    "temporary WebView2 data was not removed; cleanup is deferred"
                );
            }
        }
    }
}

fn take_com_string(
    get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
) -> windows::core::Result<String> {
    let mut raw = PWSTR::null();
    let result = get(&mut raw);
    let value = take_pwstr(raw);
    result?;
    Ok(value)
}

fn take_secret_com_string(
    get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
) -> windows::core::Result<SecretString> {
    let mut raw = PWSTR::null();
    let result = get(&mut raw);
    let allocation = SecretComString(raw);
    result?;

    let value = if allocation.0.is_null() {
        String::new()
    } else {
        unsafe { allocation.0.to_string() }.map_err(|_| windows::core::Error::from(E_INVALIDARG))?
    };
    Ok(SecretString::from(value))
}

struct SecretComString(PWSTR);

impl Drop for SecretComString {
    fn drop(&mut self) {
        if self.0.is_null() {
            return;
        }

        unsafe {
            let code_units = self.0.len().saturating_add(1);
            std::ptr::write_bytes(self.0.0, 0, code_units);
            CoTaskMemFree(Some(self.0.0.cast()));
        }
        self.0 = PWSTR::null();
    }
}

fn wide_null(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(Some(0)).collect()
}

fn show_login_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

fn unavailable(context: &str, error: windows::core::Error) -> WorkerError {
    WorkerError::Unavailable(format!("{context}: {error}"))
}

fn internal(context: &str, error: windows::core::Error) -> WorkerError {
    WorkerError::Internal(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_guard_accepts_only_https_roblox_origins() {
        assert_eq!(
            allowed_roblox_host("https://www.roblox.com/login?returnUrl=%2Fhome"),
            Some("www.roblox.com".into())
        );
        assert_eq!(
            allowed_roblox_host("https://apis.roblox.com/oauth/v1/authorize"),
            Some("apis.roblox.com".into())
        );

        for rejected in [
            "http://www.roblox.com/login",
            "https://www.roblox.com:444/login",
            "https://roblox.com@evil.example/login",
            "https://roblox.com.evil.example/login",
            "https://evilroblox.com/login",
            "javascript:alert(1)",
        ] {
            assert_eq!(
                allowed_roblox_host(rejected),
                None,
                "{rejected} must be blocked"
            );
        }
    }

    #[test]
    fn webview_browser_switches_are_rejected_without_reading_their_values() {
        assert!(is_edge_webview_switch(std::ffi::OsStr::new(
            "--edge-webview-switches=--remote-debugging-port=9222"
        )));
        assert!(is_edge_webview_switch(std::ffi::OsStr::new(
            "--EDGE-WEBVIEW-SWITCHES"
        )));
        assert!(!is_edge_webview_switch(std::ffi::OsStr::new(
            "--ordinary-app-argument"
        )));
    }

    #[test]
    fn stale_sweep_does_not_select_a_fresh_login_directory() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "multiple-rblx-janitor-test-{}-{stamp}",
            std::process::id()
        ));
        let fresh = root.join(format!("{USER_DATA_PREFIX}fresh"));
        std::fs::create_dir_all(&fresh).expect("creating fresh login directory");

        schedule_stale_user_data_cleanup(&root);
        std::thread::sleep(Duration::from_secs(1));
        let survived = fresh.is_dir();
        let _ = std::fs::remove_dir_all(&root);

        assert!(survived, "a fresh login directory must never be scavenged");
    }
}
