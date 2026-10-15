use windows::{core::*, Win32::UI::WindowsAndMessaging::MessageBoxA};
use windows::{Win32::Foundation::*, Win32::System::SystemServices::*};

#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn DllMain(module: HINSTANCE, reason: u32, _: *mut ()) -> bool {
    match reason {
        DLL_PROCESS_ATTACH => attach(),
        DLL_PROCESS_DETACH => detach(),
        _ => (),
    }

    true
}

fn attach() {
    unsafe {
        MessageBoxA(
            None,
            s!("Hello from DLL_PROCESS_ATTACH"),
            s!("Rust"),
            Default::default(),
        );
    };
}

fn detach() {
    unsafe {
        MessageBoxA(
            None,
            s!("Goodbye from DLL_PROCESS_DETACH"),
            s!("Rust"),
            Default::default(),
        );
    };
}
