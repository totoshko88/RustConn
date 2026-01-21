//! Property-based tests for `RustConn` core library

// Allow common test patterns that Clippy warns about
#![allow(clippy::redundant_clone)]
#![allow(clippy::similar_names)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::items_after_statements)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::cognitive_complexity)]
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::single_component_path_imports)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::use_self)]
#![allow(clippy::redundant_closure)]
#![allow(clippy::redundant_closure_for_method_calls)]
#![allow(clippy::expect_fun_call)]
#![allow(clippy::unwrap_or_default)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::single_char_pattern)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::default_trait_access)]
#![allow(clippy::useless_format)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::collection_is_never_read)]
#![allow(clippy::used_underscore_binding)]
#![allow(clippy::stable_sort_primitive)]
#![allow(clippy::implicit_clone)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::len_zero)]
#![allow(clippy::literal_string_with_formatting_args)]
#![allow(clippy::collapsible_str_replace)]
#![allow(clippy::needless_collect)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::branches_sharing_code)]
#![allow(clippy::format_push_string)]
#![allow(clippy::clone_on_copy)]

mod async_credential_tests;
mod batch_processing_tests;
mod cli_tests;
mod clipboard_tests;
mod cluster_tests;
mod config_tests;
mod connection_tests;
mod custom_property_tests;
mod dashboard_tests;
mod detection_tests;
mod dialog_tests;
mod document_tests;
mod drag_drop_tests;
mod expect_tests;
mod export_tests;
mod ffi_tests;
mod freerdp_tests;
mod history_tests;
mod import_tests;
mod interning_tests;
mod keepass_tests;
mod key_sequence_tests;
mod lazy_loading_tests;
mod logging_tests;
mod native_export_tests;
mod progress_tests;
mod protocol_tests;
mod quick_connect_tests;
mod rdp_client_tests;
mod search_tests;
mod security_tests;
mod selection_tests;
mod serialization_tests;
mod session_restore_tests;
mod session_tests;
mod snippet_tests;
mod spice_client_tests;
mod split_view;
mod ssh_agent_tests;
mod storage_backend_tests;
mod task_tests;
mod template_tests;
mod testing_tests;
mod tracing_tests;
mod variable_tests;
mod verification_tests;
mod virtual_scrolling_tests;
mod window_mode_tests;
mod wol_tests;
