//! WebView settings configuration for the embedded web browser.
//!
//! Applies per-connection WebKit settings (JavaScript policy, user-agent override,
//! and hardened defaults) before any content is loaded in the WebView.

use rustconn_core::models::WebConfig;
use webkit6::prelude::WebViewExt;

/// Applies `WebConfig` settings to a WebView's WebKitSettings instance.
///
/// Configures JavaScript execution policy, optional user-agent override,
/// and hardened defaults that disable developer extras and modal dialogs
/// for the embedded browsing context. Must be called before
/// `web_view.load_uri()` to ensure settings take effect before content loads.
pub fn apply_settings(web_view: &webkit6::WebView, config: &WebConfig) {
    let Some(settings) = web_view.settings() else {
        return;
    };

    // JavaScript control (Req 9.2, 9.3)
    settings.set_enable_javascript(config.javascript_enabled);

    // User agent override (Req 7.3)
    if let Some(ref ua) = config.user_agent {
        settings.set_user_agent(Some(ua));
    }

    // Hardened defaults for embedded context (Req 9.4)
    settings.set_enable_developer_extras(false);
    settings.set_allow_modal_dialogs(false);
}
