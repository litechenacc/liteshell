use libloading::Library as DynamicLibrary;
use std::path::PathBuf;

/// Owns the pinned fff 0.10.0 module. All unsafe loading is confined here.
pub struct Library {
    #[allow(dead_code)]
    module: DynamicLibrary,
}
impl Library {
    pub fn load() -> Result<Self, String> {
        let dll = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("fff_c.dll")))
            .unwrap_or_else(|| PathBuf::from("fff_c.dll"));
        // SAFETY: loading does not dereference symbols. Symbol declarations are
        // intentionally kept out of the domain crates.
        unsafe { DynamicLibrary::new(&dll) }
            .map(|module| Self { module })
            .map_err(|e| format!("cannot load {}: {e}", dll.display()))
    }
}
