use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse::Parse, parse_macro_input, punctuated::Punctuated, Ident, LitStr, Token};

/// Lightweight parser for .def files
fn parse_def_exports(content: &str) -> Vec<(String, Option<u16>, bool)> {
    let mut exports = Vec::new();
    let mut in_exports_section = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        if line.eq_ignore_ascii_case("EXPORTS") {
            in_exports_section = true;
            continue;
        }

        if in_exports_section {
            if line.eq_ignore_ascii_case("LIBRARY") || line.starts_with('[') {
                break;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(name_part) = parts.first() {
                if *name_part == "DllMain" {
                    continue;
                }

                let mut ordinal = None;
                let mut noname = false;

                for part in parts.iter().skip(1) {
                    if part.starts_with('@') {
                        let ord_str: String = part[1..]
                            .chars()
                            .take_while(|c| c.is_ascii_digit())
                            .collect();
                        if let Ok(ord) = ord_str.parse::<u16>() {
                            ordinal = Some(ord);
                        }
                    } else if part.eq_ignore_ascii_case("NONAME") {
                        noname = true;
                    }
                }

                exports.push(((*name_part).to_string(), ordinal, noname));
            }
        }
    }
    exports
}

struct DllConfig {
    def_ident: Ident,
    target_name: Option<LitStr>,
    custom_path: Option<LitStr>,
}

impl Parse for DllConfig {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        syn::braced!(content in input);

        let mut def_ident = None;
        let mut target_name = None;
        let mut custom_path = None;

        while !content.is_empty() {
            let key: Ident = content.parse()?;
            content.parse::<Token![:]>()?;

            if key == "def" {
                if let Ok(lit) = content.parse::<LitStr>() {
                    def_ident = Some(syn::parse_str::<Ident>(&lit.value())?);
                } else {
                    def_ident = Some(content.parse::<Ident>()?);
                }
            } else if key == "target" {
                target_name = Some(content.parse()?);
            } else if key == "path" {
                custom_path = Some(content.parse()?);
            }

            if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
            }
        }

        let def_ident = def_ident.ok_or_else(|| {
            syn::Error::new(
                content.span(),
                "Missing required 'def' field in proxy_dll configuration",
            )
        })?;

        Ok(DllConfig {
            def_ident,
            target_name,
            custom_path,
        })
    }
}

struct Input {
    configs: Punctuated<DllConfig, Token![,]>,
    entrypoint: Ident,
}

impl Parse for Input {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        syn::bracketed!(content in input);
        let configs = content.parse_terminated(DllConfig::parse, Token![,])?;

        input.parse::<Token![,]>()?;
        let entrypoint = input.parse()?;

        Ok(Input {
            configs,
            entrypoint,
        })
    }
}

#[proc_macro]
pub fn proxy_dll(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as Input);

    let mut targets_dlls = vec![];
    let mut custom_paths = vec![];
    let mut export_fns = vec![];
    let mut setup_forwards = vec![];
    let mut forward_counts = vec![];

    for (target_n, config) in parsed.configs.iter().enumerate() {
        let def_name = config.def_ident.to_string();

        let dll_filename = match &config.target_name {
            Some(t) => t.value(),
            None => format!("{def_name}.dll"),
        };

        let path_token = match &config.custom_path {
            Some(p) => quote! { Some(#p) },
            None => quote! { None },
        };

        targets_dlls.push(dll_filename);
        custom_paths.push(path_token);

        let file_string = std::fs::read_to_string(format!(
            "{}/defs/{def_name}.def",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();

        let exports = parse_def_exports(&file_string);

        let mut count: usize = 0;
        for (name, ordinal, noname) in exports {
            let name_ident = format_ident!("{name}");

            export_fns.push(quote! {
                #[no_mangle]
                pub unsafe extern "C" fn #name_ident() {
                    core::mem::transmute::<usize, unsafe extern "C" fn()>(FORWARDS[#target_n][#count])();
                }
            });

            let cstr_name = std::ffi::CString::new(name).unwrap();
            let name_or_ord = if noname {
                let ord = ordinal.expect("NONAME export requires an ordinal");
                quote! { #ord as *const u8 }
            } else {
                quote! { #cstr_name.as_ptr() as *const u8 }
            };

            setup_forwards.push(quote! {
                if let Some(addr) = GetProcAddress(TARGET_DLL_HANDLES[#target_n], #name_or_ord) {
                    FORWARDS[#target_n][#count] = addr as usize;
                } else {
                    ExitProcess(1);
                }
            });

            count += 1;
        }
        forward_counts.push(count);
    }

    let entrypoint = parsed.entrypoint;
    let target_count = parsed.configs.len();

    let output = quote! {
        unsafe extern "system" fn __call_entrypoint(_: usize) {
            #entrypoint();
        }

        #[cfg(windows)]
        mod __proxy_dll {
            use super::*;
            const TARGETS: &[&str] = &[#(&#targets_dlls),*];
            const CUSTOM_PATHS: &[Option<&str>] = &[#(#custom_paths),*];

            static mut TARGET_DLL_HANDLES: [HMODULE; #target_count] = [std::ptr::null_mut(); #target_count];
            static mut FORWARDS: [&mut [usize]; #target_count] = [#(&mut [0; #forward_counts]),*];

            static mut INIT: bool = false;

            mod exports {
                use super::*;
                #(#export_fns)*
            }

            unsafe fn setup_forwards() {
                #(#setup_forwards)*
            }

            use ::proxy_dll::windows_sys::core::BOOL;
            use ::proxy_dll::windows_sys::{
                Win32::{
                    Foundation::{FreeLibrary, HMODULE},
                    System::{
                        LibraryLoader::{GetProcAddress, LoadLibraryW},
                        SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH},
                        Threading::{ExitProcess, GetCurrentThread, QueueUserAPC},
                    },
                },
            };

            unsafe fn load_targets() {
                let system_dir = ::proxy_dll::get_system_directory().ok();

                for (i, target) in TARGETS.iter().enumerate() {
                    use std::os::windows::ffi::OsStrExt;

                    let base_path = if let Some(path) = CUSTOM_PATHS[i] {
                        std::path::PathBuf::from(path)
                    } else if let Some(ref dir) = system_dir {
                        dir.clone()
                    } else {
                        ExitProcess(1);
                    };

                    let mut dll_path = base_path.join(target).into_os_string();
                    dll_path.push("\0");

                    TARGET_DLL_HANDLES[i] = LoadLibraryW(dll_path.encode_wide().collect::<Vec<u16>>().as_ptr());

                    if TARGET_DLL_HANDLES[i].is_null() {
                        ExitProcess(1);
                    }
                }
            }

            #[no_mangle]
            pub extern "system" fn DllMain(
                _hinst_dll: HMODULE,
                fdw_reason: u32,
                _lpv_reserved: *mut core::ffi::c_void,
            ) -> BOOL {
                match fdw_reason {
                    DLL_PROCESS_ATTACH => unsafe {
                        if !INIT {
                            INIT = true;
                            load_targets();
                            setup_forwards();
                            QueueUserAPC(Some(__call_entrypoint), GetCurrentThread(), 0);
                        }
                    },
                    DLL_PROCESS_DETACH => unsafe {
                        for handle in TARGET_DLL_HANDLES.iter() {
                            if !handle.is_null() {
                                FreeLibrary(*handle);
                            }
                        }
                    },
                    _ => {}
                }
                true.into()
            }
        }
    };

    output.into()
}
