//! forge-core: Forge's pure logic (no I/O, no processes, no UI): settings,
//! keybindings, project state, names, and (as the port goes on) the project
//! graph, run configurations, network/memory/profile models and more.

pub mod commands;
pub mod deps;
pub mod dock;
pub mod dotenv;
pub mod fuzzy;
pub mod keybindings;
pub mod lint;
pub mod names;
pub mod output;
pub mod project_state;
pub mod run_config;
pub mod settings;
pub mod tabs;
pub mod testing;
