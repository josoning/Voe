#[cfg(feature = "dynamic-plugins")]
pub mod loader;
pub mod registry;

#[cfg(feature = "dynamic-plugins")]
pub use loader::PluginLoader;
pub use registry::PluginRegistry;
