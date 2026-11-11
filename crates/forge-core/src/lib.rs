//! forge-core: Forge's pure logic (no I/O, no processes, no UI): settings,
//! keybindings, project state, names, and (as the port goes on) the project
//! graph, run configurations, network/memory/profile models and more.

pub mod api_fields;
pub mod blocks;
pub mod box_code;
pub mod box_layer;
pub mod box_sync;
pub mod boxes;
pub mod commands;
pub mod data_summary;
pub mod dataflow;
pub mod deps;
pub mod dock;
pub mod dotenv;
pub mod fuzzy;
pub mod git;
pub mod graph_layout;
pub mod graph_view;
pub mod history;
pub mod keybindings;
pub mod lint;
pub mod live_run;
pub mod names;
pub mod network;
pub mod openapi;
pub mod openapi_code;
pub mod output;
pub mod pipeline_edit;
pub mod profile;
pub mod project_graph;
pub mod project_state;
pub mod run_config;
pub mod settings;
pub mod suggest;
pub mod tabs;
pub mod testing;
