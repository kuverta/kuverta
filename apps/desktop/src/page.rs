//! HTML mail as a page in the reading pane: the app's own URL scheme.
//!
//! The window cannot put a sender's HTML into its own document — the
//! document runs the window. It goes into an `<iframe sandbox>` instead,
//! loaded from `kuverta-mail://localhost/…`, which this file answers: a
//! page made by `core_rpc::html_view` for a message, the pictures inside
//! the message, and the one address a link in the page may go to.
//!
//! A scheme of its own rather than a `blob:` or `srcdoc` frame because a
//! document from those inherits the window's Content-Security-Policy, and
//! the window's forbids inline style — which is all a mail is. A document
//! from a scheme is served with a policy of its own, written here: nothing
//! may load but style and the pictures the policy names, and `sandbox`
//! makes it an origin of nothing, with no scripts, forms or popups. The
//! frame's own `sandbox` attribute says the same, so the page is held twice.
//!
//! ## Links
//!
//! Every link in the page points at `/open/<secret>?to=…`. A click is a
//! navigation of the frame to that address, which arrives here, is
//! answered with `204 No Content` — a response that leaves the frame where
//! it was — and is told to the window as an event, which asks the person
//! before anything is opened. The secret is made once a start, so a message
//! cannot write that address into a picture and have it fetched without a
//! click: a request without the secret is not a link.

use std::borrow::Cow;

use core_rpc::html_view;
use tauri::{http, Emitter, Manager, UriSchemeContext};

use crate::App;

/// The scheme, as `tauri.conf.json`'s `frame-src` names it too.
pub const SCHEME: &str = "kuverta-mail";

/// A secret the link addresses carry, made once a start.
pub fn link_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the system has no randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where the scheme is reached, which differs by platform: the webview on
/// Windows and Android knows custom schemes as `http://<scheme>.localhost`.
fn origin() -> String {
    if cfg!(any(windows, target_os = "android")) {
        format!("http://{SCHEME}.localhost")
    } else {
        format!("{SCHEME}://localhost")
    }
}

/// The address the window loads a message's page from.
pub fn page_url(account: i64, id: i64, remote: bool) -> String {
    format!(
        "{}/message/{account}/{id}?remote={}",
        origin(),
        u8::from(remote)
    )
}

/// What the window is told when a page was made: how many pictures from
/// the internet were left out of it.
#[derive(Debug, Clone, serde::Serialize)]
struct PageMade {
    account: i64,
    id: i64,
    remote: bool,
    remote_blocked: usize,
    links: usize,
}

/// What the window is told when a link in a page is clicked: where it
/// goes, and what it said.
#[derive(Debug, Clone, serde::Serialize)]
struct LinkAsked {
    url: String,
    text: String,
}

type Reply = http::Response<Cow<'static, [u8]>>;

fn reply(status: u16, content_type: &str, body: Vec<u8>) -> Reply {
    http::Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", "no-store")
        .header("Referrer-Policy", "no-referrer")
        .body(Cow::Owned(body))
        .expect("a response with fixed headers builds")
}

fn not_found(why: &str) -> Reply {
    reply(404, "text/plain; charset=utf-8", why.as_bytes().to_vec())
}

/// Answers the scheme. Runs on the webview's thread for every request the
/// frame makes, so it does what a command does and no more: a lock on the
/// store for the time it takes to read one message.
pub fn handle(ctx: UriSchemeContext<'_, tauri::Wry>, request: http::Request<Vec<u8>>) -> Reply {
    let app = ctx.app_handle();
    let uri = request.uri();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match segments.as_slice() {
        ["message", account, id] => match (account.parse(), id.parse()) {
            (Ok(account), Ok(id)) => {
                let remote = html_view::query_values(&query)
                    .iter()
                    .any(|(name, value)| name == "remote" && value == "1");
                message(app, account, id, remote)
            }
            _ => not_found("no such message"),
        },
        ["part", account, id, index] => match (account.parse(), id.parse(), index.parse()) {
            (Ok(account), Ok(id), Ok(index)) => picture(app, account, id, index),
            _ => not_found("no such picture"),
        },
        ["open", token] => open(app, token, &query),
        _ => not_found("nothing here"),
    }
}

fn message(app: &tauri::AppHandle, account: i64, id: i64, remote: bool) -> Reply {
    let state = app.state::<App>();
    let origin = origin();
    let open = format!("{origin}/open/{}", state.link_token);
    let parts = format!("{origin}/part/{account}/{id}/");
    let made = {
        let core = state.core.lock().unwrap();
        core.message_page(account, id, remote, &open, &parts)
    };
    match made {
        Ok(Some(page)) => {
            let _ = app.emit(
                "mail-page",
                PageMade {
                    account,
                    id,
                    remote,
                    remote_blocked: page.remote_blocked,
                    links: page.links,
                },
            );
            http::Response::builder()
                .status(200)
                .header("Content-Type", "text/html; charset=utf-8")
                .header("Content-Security-Policy", page.csp)
                .header("X-Content-Type-Options", "nosniff")
                .header("Cache-Control", "no-store")
                .header("Referrer-Policy", "no-referrer")
                .body(Cow::Owned(page.html.into_bytes()))
                .expect("a response with fixed headers builds")
        }
        Ok(None) => not_found("this message has no page"),
        Err(err) => {
            tracing::warn!(account, id, %err, "the message's page could not be made");
            reply(
                500,
                "text/plain; charset=utf-8",
                err.to_string().into_bytes(),
            )
        }
    }
}

/// The picture kinds the window shows inline. The same four the safe
/// preview draws, and none that is a document — SVG can carry script, and
/// a script in a picture is still a script.
const PICTURES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

fn picture(app: &tauri::AppHandle, account: i64, id: i64, index: usize) -> Reply {
    let found = {
        let state = app.state::<App>();
        let core = state.core.lock().unwrap();
        core.attachment(account, id, index)
    };
    match found {
        Ok(found) if PICTURES.contains(&found.view.content_type.as_str()) && !found.view.risky => {
            http::Response::builder()
                .status(200)
                .header("Content-Type", found.view.content_type)
                .header("Content-Security-Policy", "default-src 'none'; sandbox")
                .header("X-Content-Type-Options", "nosniff")
                .header("Cache-Control", "no-store")
                .body(Cow::Owned(found.bytes))
                .expect("a response with fixed headers builds")
        }
        Ok(_) => not_found("not a picture"),
        Err(err) => not_found(&err.to_string()),
    }
}

/// A link was clicked. The frame stays where it is — a `204` is not a page
/// — and the window is asked to ask.
fn open(app: &tauri::AppHandle, token: &str, query: &str) -> Reply {
    let state = app.state::<App>();
    if token != state.link_token {
        return not_found("not a link");
    }
    let values = html_view::query_values(query);
    let value = |name: &str| {
        values
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    // Checked again here, by the same rule that let it into the page: a
    // page is made of a message, and this is the one place a message's
    // words become an address the window would open.
    let Some(url) = html_view::link_target(&value("to")) else {
        return not_found("not an address to open");
    };
    let _ = app.emit(
        "mail-link",
        LinkAsked {
            url,
            text: value("text"),
        },
    );
    http::Response::builder()
        .status(204)
        .header("Content-Type", "application/x-kuverta-link")
        .header("Cache-Control", "no-store")
        .body(Cow::Owned(Vec::new()))
        .expect("a response with fixed headers builds")
}
