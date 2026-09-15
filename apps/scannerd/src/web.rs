//! The page on the Pi: what the camera sees, what has been sent, and setup.
//!
//! Served by scannerd itself, so a headless Pi Zero needs no desktop, no
//! browser and no second program — open `http://<pi>.local:8080` on a phone.
//! hyper, which reqwest already builds, and a page that is one HTML file
//! compiled in.
//!
//! The page never reaches the camera, the spool's queue or the uploader. It
//! reads the status the capture loop publishes and leaves commands for it (see
//! [`crate::hub`]), so a button pressed mid-photograph waits a moment rather
//! than starting a second `rpicam-still`.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderValue, AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, WWW_AUTHENTICATE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use tokio::net::TcpListener;

use crate::hub::{Command, Hub};
use crate::settings::Settings;

const PAGE: &str = include_str!("ui.html");

type Reply = Response<Full<Bytes>>;

pub struct Web {
    hub: Arc<Hub>,
    spool_dir: PathBuf,
    password: Option<String>,
}

impl Web {
    /// `password`: required of every request when set. `main` insists on one
    /// unless the page is only reachable from the Pi itself.
    pub fn new(hub: Arc<Hub>, spool_dir: PathBuf, password: Option<String>) -> Self {
        Self {
            hub,
            spool_dir,
            password: password.filter(|password| !password.is_empty()),
        }
    }

    async fn handle(&self, request: Request<Incoming>) -> Reply {
        if !self.authorised(request.headers().get(AUTHORIZATION)) {
            let mut reply = text(StatusCode::UNAUTHORIZED, "scannerd needs its password");
            reply.headers_mut().insert(
                WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"scannerd\""),
            );
            return reply;
        }

        let (parts, body) = request.into_parts();
        let path = parts.uri.path();

        if parts.method == Method::GET {
            return match path {
                "/" => typed("text/html; charset=utf-8", PAGE.as_bytes().to_vec()),
                "/api/status" => json(&self.hub.status()),
                "/frame.bmp" => self.bitmap(self.hub.frame()),
                "/full-view.bmp" => self.bitmap(self.hub.full_view()),
                _ => match path.strip_prefix("/page/") {
                    Some(name) => self.page(name),
                    None => not_found(),
                },
            };
        }
        if parts.method != Method::POST {
            return text(StatusCode::METHOD_NOT_ALLOWED, "GET or POST");
        }

        // A browser attaches a saved password to any request for this address,
        // including one a different site makes it send. It will not add a
        // custom header to such a request without asking this server first,
        // which never agrees — so requiring one keeps other pages from pressing
        // these buttons.
        if !parts.headers.contains_key("x-scannerd") {
            return text(StatusCode::FORBIDDEN, "missing the X-Scannerd header");
        }

        let command = match path {
            "/api/finish" => Command::FinishLetter,
            "/api/learn-empty" => Command::LearnEmpty,
            "/api/retry" => Command::RetryNow,
            "/api/full-view" => Command::FullView,
            "/api/check" => Command::CheckPaperless,
            "/api/settings" => match read_settings(body).await {
                Ok(settings) => Command::Settings(settings),
                Err(message) => return text(StatusCode::BAD_REQUEST, &message),
            },
            _ => return not_found(),
        };
        self.hub.send(command);
        text(StatusCode::ACCEPTED, "queued")
    }

    fn authorised(&self, header: Option<&HeaderValue>) -> bool {
        let Some(password) = &self.password else {
            return true;
        };
        header
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Basic "))
            .and_then(|encoded| base64_decode(encoded.trim()))
            .and_then(|raw| String::from_utf8(raw).ok())
            .and_then(|pair| pair.split_once(':').map(|(_, given)| given.to_string()))
            .is_some_and(|given| same(given.as_bytes(), password.as_bytes()))
    }

    fn bitmap(&self, frame: Option<Vec<u8>>) -> Reply {
        let (width, height) = self.hub.frame_size;
        match frame.and_then(|frame| bmp(&frame, width, height)) {
            Some(bytes) => typed("image/bmp", bytes),
            None => not_found(),
        }
    }

    /// A page of the letter being collected, by file name — and nothing that
    /// could climb out of the spool's `open/` directory.
    fn page(&self, name: &str) -> Reply {
        let plain = name.ends_with(".jpg")
            && !name.starts_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
        if !plain {
            return not_found();
        }
        match std::fs::read(self.spool_dir.join("open").join(name)) {
            Ok(bytes) => typed("image/jpeg", bytes),
            Err(_) => not_found(),
        }
    }
}

/// Accepts connections until the process ends.
pub async fn serve(listener: TcpListener, web: Arc<Web>) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(err) => {
                // Out of file descriptors, typically. Pause rather than spin.
                tracing::warn!(%err, "could not accept a connection to the page");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let web = web.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let web = web.clone();
                async move { Ok::<_, Infallible>(web.handle(request).await) }
            });
            if let Err(err) = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                tracing::debug!(%err, "a connection to the page ended early");
            }
        });
    }
}

async fn read_settings(body: Incoming) -> Result<Settings, String> {
    let bytes = Limited::new(body, 16 * 1024)
        .collect()
        .await
        .map_err(|err| format!("could not read the settings: {err}"))?
        .to_bytes();
    let settings: Settings =
        serde_json::from_slice(&bytes).map_err(|err| format!("those are not settings: {err}"))?;
    settings.validate().map_err(|err| err.to_string())?;
    Ok(settings)
}

/// A greyscale frame as an 8-bit BMP, which every browser shows and which
/// needs no image library to write: two headers, a grey palette, and the rows
/// bottom-up, each padded to four bytes.
pub fn bmp(luma: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || luma.len() < w * h {
        return None;
    }
    let row = w.div_ceil(4) * 4;
    let offset = 14 + 40 + 256 * 4;
    let size = offset + row * h;

    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(offset as u32).to_le_bytes());

    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    // Positive: rows run bottom-up, the form every reader supports.
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&((row * h) as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&256u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    for grey in 0..=255u8 {
        out.extend_from_slice(&[grey, grey, grey, 0]);
    }
    for y in (0..h).rev() {
        out.extend_from_slice(&luma[y * w..(y + 1) * w]);
        out.resize(out.len() + (row - w), 0);
    }
    Some(out)
}

/// Standard base64 with or without padding. Only for Basic auth, which is too
/// little to take a dependency for.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in input.trim_end_matches('=').bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Equal, taking the same time however early they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn typed(content_type: &'static str, body: Vec<u8>) -> Reply {
    let mut reply = Response::new(Full::new(Bytes::from(body)));
    let headers = reply.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    // Every answer is "now": a cached frame or status is a wrong one.
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    reply
}

fn json<T: Serialize>(value: &T) -> Reply {
    match serde_json::to_vec(value) {
        Ok(bytes) => typed("application/json", bytes),
        Err(err) => text(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
    }
}

fn text(status: StatusCode, message: &str) -> Reply {
    let mut reply = typed("text/plain; charset=utf-8", message.as_bytes().to_vec());
    *reply.status_mut() = status;
    reply
}

fn not_found() -> Reply {
    text(StatusCode::NOT_FOUND, "not here")
}
