use std::path::Path;

use voe_repo_api::plugin::{Plugin, PluginInfo as CorePluginInfo};
use voe_types::error::{Result, VoeError};

#[repr(C)]
#[derive(Clone)]
pub struct PluginVTable {
    pub info_fn: unsafe extern "C" fn(*const std::ffi::c_void) -> *const PluginInfoRaw,
    pub destroy_fn: unsafe extern "C" fn(*mut std::ffi::c_void),
    pub commands_fn: unsafe extern "C" fn(*const std::ffi::c_void) -> PluginCommandsArray,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginCommandsArray {
    pub entries: *const PluginCommandEntry,
    pub count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginCommandEntry {
    pub name: *const i8,
    pub description: *const i8,
    pub exec_fn: unsafe extern "C" fn(*mut std::ffi::c_void, *const *const i8, u32) -> i32,
}

#[repr(C)]
pub struct PluginInfoRaw {
    pub name: *const i8,
    pub version: *const i8,
    pub description: *const i8,
    pub author: *const i8,
}

type PluginFactory = unsafe extern "C" fn() -> PluginExport;

#[repr(C)]
pub struct PluginExport {
    pub state: *mut std::ffi::c_void,
    pub vtable: *const PluginVTable,
}

pub struct PluginLoader {
    _priv: (),
}

impl PluginLoader {
    pub fn new() -> Self {
        Self { _priv: () }
    }

    pub unsafe fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Box<dyn Plugin>> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(VoeError::Plugin(format!(
                "Plugin file not found: {}",
                path.display()
            )));
        }

        let lib = libloading::Library::new(path)
            .map_err(|e| VoeError::Plugin(format!("Failed to load plugin library: {}", e)))?;

        let factory: libloading::Symbol<PluginFactory> = lib
            .get(b"voe_plugin_create")
            .map_err(|e| VoeError::Plugin(format!("Plugin missing 'voe_plugin_create': {}", e)))?;

        let export = factory();
        if export.state.is_null() || export.vtable.is_null() {
            return Err(VoeError::Plugin(
                "Plugin factory returned null pointers".to_string(),
            ));
        }

        let vtable = &*export.vtable;

        Ok(Box::new(PluginAdapter {
            state: export.state,
            vtable: vtable.clone(),
            _lib: lib,
        }))
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

struct PluginAdapter {
    state: *mut std::ffi::c_void,
    vtable: PluginVTable,
    _lib: libloading::Library,
}

unsafe impl Send for PluginAdapter {}
unsafe impl Sync for PluginAdapter {}

impl Plugin for PluginAdapter {
    fn info(&self) -> CorePluginInfo {
        unsafe {
            let raw = (self.vtable.info_fn)(self.state);
            if raw.is_null() {
                return CorePluginInfo {
                    name: "unknown",
                    version: "0.0.0",
                    description: "",
                    author: "",
                };
            }
            let raw = &*raw;
            CorePluginInfo {
                name: raw.name.is_null().then_some("").unwrap_or_else(|| {
                    std::ffi::CStr::from_ptr(raw.name)
                        .to_str()
                        .unwrap_or("unknown")
                }),
                version: raw.version.is_null().then_some("0.0.0").unwrap_or_else(|| {
                    std::ffi::CStr::from_ptr(raw.version)
                        .to_str()
                        .unwrap_or("0.0.0")
                }),
                description: raw.description.is_null().then_some("").unwrap_or_else(|| {
                    std::ffi::CStr::from_ptr(raw.description)
                        .to_str()
                        .unwrap_or("")
                }),
                author: raw
                    .author
                    .is_null()
                    .then_some("")
                    .unwrap_or_else(|| std::ffi::CStr::from_ptr(raw.author).to_str().unwrap_or("")),
            }
        }
    }

    fn commands(&self) -> Vec<Box<dyn voe_repo_api::command::Command>> {
        Vec::new()
    }
}

impl Drop for PluginAdapter {
    fn drop(&mut self) {
        unsafe {
            (self.vtable.destroy_fn)(self.state);
        }
    }
}
