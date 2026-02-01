use crate::sidebar::ConnectionItem;
use gtk4::prelude::*;
use gtk4::{gdk, glib};
use serde::{Deserialize, Serialize};

/// Strongly typed payload for drag operations
#[derive(Serialize, Deserialize, Debug)]
pub enum DragPayload {
    Connection(String), // Connection ID
    Group(String),      // Group ID
}

/// Prepares drag data for a connection item
///
/// Returns a ContentProvider containing the serialized DragPayload
pub fn prepare_drag_data(item: &ConnectionItem) -> Option<gdk::ContentProvider> {
    let payload = if item.is_group() {
        DragPayload::Group(item.id())
    } else {
        DragPayload::Connection(item.id())
    };

    let json = serde_json::to_string(&payload).ok()?;
    Some(gdk::ContentProvider::for_value(&json.to_value()))
}

/// Parses dropped value into DragPayload
pub fn parse_drag_data(value: &glib::Value) -> Option<DragPayload> {
    let s = value.get::<String>().ok()?;
    serde_json::from_str(&s).ok()
}
