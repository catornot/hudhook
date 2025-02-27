#![feature(once_cell)]
//! # hudhook
//!
//! This library implements a mechanism for hooking into the
//! render loop of applications and drawing things on screen via
//! [`imgui`](https://docs.rs/imgui/0.8.0/imgui/). It has been largely inspired
//! by [CheatEngine](https://www.cheatengine.org/).
//!
//! Currently, DirectX9, DirectX 11, DirectX 12 and OpenGL 3 are supported.
//!
//! This library **requires** Rust nightly.
//!
//! For complete, fully fledged examples of usage, check out the following
//! projects:
//!
//! - [`darksoulsiii-practice-tool`](https://github.com/veeenu/darksoulsiii-practice-tool)
//! - [`eldenring-practice-tool`](https://github.com/veeenu/eldenring-practice-tool)
//!
//! It is a good idea to refer to these projects for any doubts about the API
//! which aren't clarified by this documentation, as this project is directly
//! derived from them.
//!
//! Refer to [this post](https://veeenu.github.io/blog/sekiro-practice-tool-architecture/) for
//! in-depth information about the architecture of the library.
//!
//! [`darksoulsiii-practice-tool`]: https://github.com/veeenu/darksoulsiii-practice-tool
//! [`eldenring-practice-tool`]: https://github.com/veeenu/eldenring-practice-tool
//!
//! ## Fair warning
//!
//! [`hudhook`](crate) provides essential, crash-safe features for memory
//! manipulation and UI rendering. It does, alas, contain a hefty amount of FFI
//! and `unsafe` code which still has to be thoroughly tested, validated and
//! audited for soundness. It should be OK for small projects such as videogame
//! mods, but it may crash your application at this stage.
//!
//! ## Examples
//!
//! ### Hooking the render loop and drawing things with `imgui`
//!
//! Compile your crate with both a `cdylib` and an executable target. The
//! executable will be very minimal and used to inject the DLL into the
//! target process.
//!
//! #### Building the render loop
//!
//! Implement the render loop trait for your hook target.
//!
//! ##### Example
//!
//! Implement the [`hooks::ImguiRenderLoop`] trait:
//!
//! ```no_run
//! // lib.rs
//! use hudhook::hooks::{ImguiRenderLoop, ImguiRenderLoopFlags};
//! use hudhook::*;
//!
//! pub struct MyRenderLoop;
//!
//! impl ImguiRenderLoop for MyRenderLoop {
//!     fn render(&mut self, ui: &mut imgui::Ui, flags: &ImguiRenderLoopFlags) {
//!         ui.window("My first render loop")
//!             .position([0., 0.], imgui::Condition::FirstUseEver)
//!             .size([320., 200.], imgui::Condition::FirstUseEver)
//!             .build(|| {
//!                 ui.text("Hello, hello!");
//!             });
//!     }
//! }
//!
//! {
//!     // Use this if hooking into a DirectX 9 application.
//!     use hudhook::hooks::dx9::ImguiDx9Hooks;
//!     hudhook!(MyRenderLoop.into_hook::<ImguiDx9Hooks>());
//! }
//!
//! {
//!     // Use this if hooking into a DirectX 11 application.
//!     use hudhook::hooks::dx11::ImguiDx11Hooks;
//!     hudhook!(MyRenderLoop.into_hook::<ImguiDx11Hooks>());
//! }
//!
//! {
//!     // Use this if hooking into a DirectX 12 application.
//!     use hudhook::hooks::dx12::ImguiDx12Hooks;
//!     hudhook!(MyRenderLoop.into_hook::<ImguiDx12Hooks>());
//! }
//!
//! {
//!     // Use this if hooking into a DirectX 9 application.
//!     use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
//!     hudhook!(MyRenderLoop.into_hook::<ImguiOpenGl3Hooks>());
//! }
//! ```
//!
//! #### Injecting the DLL
//!
//! You can use the facilities in [`inject`] in your binaries to inject
//! the DLL in your target process.
//!
//! ```no_run
//! // main.rs
//! use hudhook::inject::Process;
//!
//! fn main() {
//!     let mut cur_exe = std::env::current_exe().unwrap();
//!     cur_exe.push("..");
//!     cur_exe.push("libmyhook.dll");
//!
//!     let cur_dll = cur_exe.canonicalize().unwrap();
//!
//!     Process::by_name("MyTargetApplication.exe").unwrap().inject(cur_dll).unwrap();
//! }
//! ```
#![allow(clippy::needless_doctest_main)]

mod mh;

pub mod hooks;
pub mod inject;
pub mod renderers;

/// Utility functions.
pub mod utils {
    use std::sync::atomic::{AtomicBool, Ordering};

    static CONSOLE_ALLOCATED: AtomicBool = AtomicBool::new(false);

    /// Allocate a Windows console.
    pub fn alloc_console() {
        if !CONSOLE_ALLOCATED.swap(true, Ordering::SeqCst) {
            unsafe {
                // Allocate a console
                crate::reexports::AllocConsole();
            }
        }
    }

    pub fn enable_console_colors() {
        if CONSOLE_ALLOCATED.load(Ordering::SeqCst) {
            unsafe {
                // Get the stdout handle
                let stdout_handle =
                    crate::reexports::GetStdHandle(crate::reexports::STD_OUTPUT_HANDLE).unwrap();

                // call GetConsoleMode to get the current mode of the console
                let mut current_console_mode = crate::reexports::CONSOLE_MODE(0);
                crate::reexports::GetConsoleMode(stdout_handle, &mut current_console_mode).unwrap();

                // Set the new mode to include ENABLE_VIRTUAL_TERMINAL_PROCESSING for ANSI
                // escape sequences
                current_console_mode.0 |= crate::reexports::ENABLE_VIRTUAL_TERMINAL_PROCESSING.0;

                // Call SetConsoleMode to set the new mode
                crate::reexports::SetConsoleMode(stdout_handle, current_console_mode).unwrap();
            }
        }
    }

    /// Free the previously allocated Windows console.
    pub fn free_console() {
        if CONSOLE_ALLOCATED.swap(false, Ordering::SeqCst) {
            unsafe {
                crate::reexports::FreeConsole();
            }
        }
    }
}

/// Functions that manage the lifecycle of hooks.
///
/// ## Ejecting a DLL
///
/// To eject your DLL, invoke the [`eject`] method from anywhere in your
/// render loop. This will disable the hooks, free the console (if it has
/// been created before) and invoke `FreeLibraryAndExitThread`.
///
/// Befor calling [`eject`], make sure to perform any manual cleanup (e.g.
/// dropping/resetting the contents of static mutable variables).
///
/// [`eject`]: lifecycle::eject
pub mod lifecycle {

    use std::thread;

    use windows::Win32::System::LibraryLoader::FreeLibraryAndExitThread;

    /// Disable hooks and eject the DLL.
    pub fn eject() {
        thread::spawn(|| unsafe {
            crate::utils::free_console();

            if let Some(mut hooks) = global_state::HOOKS.take() {
                hooks.unhook();
            }

            if let Some(module) = global_state::MODULE.take() {
                FreeLibraryAndExitThread(module, 0);
            }
        });
    }

    /// Exposes functions that store and manipulate global state data.
    ///
    /// The functions contained here are automatically invoked by the
    /// [`hudhook`](crate::hudhook) macro, and are needed to manage the
    /// hooks' lifetime.
    ///
    /// This module is not meant to be used by clients, but it has to be
    /// exposed as `pub` because the [`hudhook`](crate::hudhook)
    /// macro generates code in the client's library.
    pub mod global_state {

        use std::cell::OnceCell;

        use windows::Win32::Foundation::HINSTANCE;

        use crate::hooks;

        pub(super) static mut MODULE: OnceCell<HINSTANCE> = OnceCell::new();
        pub(super) static mut HOOKS: OnceCell<Box<dyn hooks::Hooks>> = OnceCell::new();

        /// Please don't use me.
        pub fn set_module(module: HINSTANCE) {
            unsafe {
                MODULE.set(module).unwrap();
            }
        }

        /// Please don't use me.
        pub fn get_module() -> HINSTANCE {
            unsafe { *MODULE.get().unwrap() }
        }

        /// Please don't use me.
        pub fn set_hooks(hooks: Box<dyn hooks::Hooks>) {
            unsafe { HOOKS.set(hooks).ok() };
        }
    }
}

pub use imgui;
pub use tracing;

/// Convenience reexports for the [macro](crate::hudhook).
pub mod reexports {
    pub use windows::Win32::Foundation::HINSTANCE;
    pub use windows::Win32::System::Console::{
        AllocConsole, FreeConsole, GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE,
    };
    pub use windows::Win32::System::SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH};
}

/// Entry point for the library.
///
/// After implementing your [render loop](crate::hooks) of choice, invoke
/// the macro to generate the `DllMain` function that will serve as entry point
/// for your hook.
///
/// Example usage:
/// ```no_run
/// use hudhook::hooks::dx12::ImguiDx12Hooks;
/// use hudhook::hooks::{ImguiRenderLoop, ImguiRenderLoopFlags};
/// use hudhook::*;
///
/// pub struct MyRenderLoop;
///
/// impl ImguiRenderLoop for MyRenderLoop {
///     fn render(&mut self, frame: &mut imgui::Ui, flags: &ImguiRenderLoopFlags) {
///         // ...
///     }
/// }
///
/// hudhook::hudhook!(MyRenderLoop.into_hook::<ImguiDx12Hooks>());
/// ```
#[macro_export]
macro_rules! hudhook {
    ($hooks:expr) => {
        use hudhook::reexports::*;
        use hudhook::tracing::*;
        use hudhook::*;

        /// Entry point created by the `hudhook` library.
        #[no_mangle]
        pub unsafe extern "stdcall" fn DllMain(
            hmodule: HINSTANCE,
            reason: u32,
            _: *mut std::ffi::c_void,
        ) {
            if reason == DLL_PROCESS_ATTACH {
                hudhook::lifecycle::global_state::set_module(hmodule);

                trace!("DllMain()");
                std::thread::spawn(move || {
                    let hooks: Box<dyn hooks::Hooks> = { $hooks };
                    hooks.hook();
                    hudhook::lifecycle::global_state::set_hooks(hooks);
                });
            }
        }
    };
}
