//! Protocoles personnalisés du rendu (ADR-004) :
//! - `mailview://localhost/<message>` : document assaini du message, avec
//!   la CSP qui n'autorise que l'agent ;
//! - `mailpart://localhost/<message>/<partie>` : image en ligne (`cid:`),
//!   types d'image matriciels seulement ;
//! - `remote-img://localhost/<message>/<index>` : image distante via le
//!   proxy, seulement si l'expéditeur est autorisé (vérifié à chaque
//!   requête : l'index renvoie à la liste établie par le dernier rendu,
//!   qui n'en contient que pour un expéditeur autorisé).
//!
//! Seule la fenêtre principale peut s'en servir. Aucune réponse n'est mise
//! en cache par la webview ; `nosniff` partout.

use norelly_domain::fetch_policy::PartPath;
use norelly_domain::ports::{MessageId, MessageLocator, ReadingPreferences};
use norelly_domain::reading::sender_address;
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::remote_images::accepted_type;
use crate::state::AppState;

/// Fenêtre autorisée à utiliser les protocoles.
const MAIN_WINDOW: &str = "main";

/// Segments du chemin (`/12/1.2` → `["12", "1.2"]`).
fn segments(request: &Request<Vec<u8>>) -> Vec<String> {
    request
        .uri()
        .path()
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn message_of(raw: &str) -> Option<MessageId> {
    if raw.is_empty() || raw.len() > 12 || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok().and_then(MessageId::new)
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    *response.status_mut() = code;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

fn ok(content_type: &str, csp: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    let builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer")
        .header(header::CONTENT_SECURITY_POLICY, csp)
        .header("Cross-Origin-Resource-Policy", "cross-origin");
    builder
        .body(body)
        .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR))
}

/// CSP des réponses qui ne sont pas des documents (images).
const NO_CONTENT_CSP: &str = "default-src 'none'; sandbox";

async fn mailview<R: Runtime>(app: &AppHandle<R>, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let state = app.state::<AppState>();
    let parts = segments(&request);
    let [id] = parts.as_slice() else {
        return status(StatusCode::NOT_FOUND);
    };
    let Some(id) = message_of(id) else {
        return status(StatusCode::NOT_FOUND);
    };
    match state.mail.render(id).await {
        Ok(Some(rendered)) => ok(
            "text/html; charset=utf-8",
            &norelly_render::content_security_policy(&state.mail.protocols),
            rendered.document.clone().into_bytes(),
        ),
        Ok(None) => status(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::warn!(error = %e, "rendu impossible");
            status(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn mailpart<R: Runtime>(app: &AppHandle<R>, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let state = app.state::<AppState>();
    let parts = segments(&request);
    let [id, path] = parts.as_slice() else {
        return status(StatusCode::NOT_FOUND);
    };
    let (Some(id), Some(path)) = (message_of(id), PartPath::parse(path)) else {
        return status(StatusCode::NOT_FOUND);
    };
    match state.mail.inline_part(id, &path).await {
        Ok(Some(content)) => match accepted_type(Some(&content.media_type)) {
            Some(media_type) => ok(media_type, NO_CONTENT_CSP, content.bytes),
            // Seules les images sont servies dans le rendu.
            None => status(StatusCode::FORBIDDEN),
        },
        Ok(None) => status(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::warn!(error = %e, "partie indisponible");
            status(StatusCode::NOT_FOUND)
        }
    }
}

async fn remote_image<R: Runtime>(
    app: &AppHandle<R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let state = app.state::<AppState>();
    let parts = segments(&request);
    let [id, index] = parts.as_slice() else {
        return status(StatusCode::NOT_FOUND);
    };
    let (Some(id), Ok(index)) = (message_of(id), index.parse::<usize>()) else {
        return status(StatusCode::NOT_FOUND);
    };
    // Autorisation revérifiée à chaque image (retrait possible entre le
    // rendu et la requête).
    let allowed = match state.mail.store.locate(id).await {
        Ok(Some(location)) => match sender_address(location.from.as_ref()) {
            Some(sender) => state
                .mail
                .prefs
                .remote_images_allowed(&sender)
                .await
                .unwrap_or(false),
            None => false,
        },
        _ => false,
    };
    if !allowed {
        return status(StatusCode::FORBIDDEN);
    }
    let (Some(url), Some(proxy)) = (
        state.mail.remote_image(id, index),
        state.mail.proxy.as_ref(),
    ) else {
        return status(StatusCode::NOT_FOUND);
    };
    match proxy.fetch(&url).await {
        Ok((media_type, bytes)) => ok(media_type, NO_CONTENT_CSP, bytes),
        Err(e) => {
            tracing::debug!(error = %e, "image distante indisponible");
            status(StatusCode::BAD_GATEWAY)
        }
    }
}

/// Traitement d'une requête, lancé en tâche asynchrone.
type Handler<R> =
    fn(AppHandle<R>, Request<Vec<u8>>) -> tauri::async_runtime::JoinHandle<Response<Vec<u8>>>;

/// Gestionnaire commun : fenêtre vérifiée, méthode `GET` seulement,
/// traitement asynchrone.
fn serve<R: Runtime>(
    ctx: &UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
    handler: Handler<R>,
) {
    if ctx.webview_label() != MAIN_WINDOW || request.method() != tauri::http::Method::GET {
        responder.respond(status(StatusCode::FORBIDDEN));
        return;
    }
    let pending = handler(ctx.app_handle().clone(), request);
    tauri::async_runtime::spawn(async move {
        let response = pending
            .await
            .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR));
        responder.respond(response);
    });
}

/// Vrai si la webview peut naviguer vers `url` : l'interface elle-même et
/// le rendu des messages seulement. Toute autre navigation (lien déposé
/// sur la fenêtre, redirection) est refusée ; les liens passent par
/// `open_external`.
#[must_use]
pub fn navigation_allowed(url: &url::Url) -> bool {
    match (url.scheme(), url.host_str()) {
        ("tauri", Some("localhost")) | ("mailview", Some("localhost")) => true,
        ("http" | "https", Some("tauri.localhost" | "mailview.localhost")) => true,
        // Serveur de développement Vite (pnpm tauri dev).
        ("http", Some("localhost")) => cfg!(debug_assertions) && url.port() == Some(1420),
        ("about", _) => url.as_str() == "about:blank",
        _ => false,
    }
}

/// Garde de navigation de la webview (greffon interne).
pub fn navigation_guard<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_navigation(|_webview, url| {
            let allowed = navigation_allowed(url);
            if !allowed {
                tracing::warn!(scheme = url.scheme(), "navigation refusée");
            }
            allowed
        })
        .build()
}

/// Enregistre les trois protocoles.
pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .register_asynchronous_uri_scheme_protocol("mailview", |ctx, request, responder| {
            serve(&ctx, request, responder, |app, request| {
                tauri::async_runtime::spawn(async move { mailview(&app, request).await })
            });
        })
        .register_asynchronous_uri_scheme_protocol("mailpart", |ctx, request, responder| {
            serve(&ctx, request, responder, |app, request| {
                tauri::async_runtime::spawn(async move { mailpart(&app, request).await })
            });
        })
        .register_asynchronous_uri_scheme_protocol("remote-img", |ctx, request, responder| {
            serve(&ctx, request, responder, |app, request| {
                tauri::async_runtime::spawn(async move { remote_image(&app, request).await })
            });
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_interface_and_mail_views_can_be_navigated_to() {
        let ok =
            |u: &str| navigation_allowed(&url::Url::parse(u).unwrap_or_else(|e| panic!("{e}")));
        assert!(ok("tauri://localhost/index.html"));
        assert!(ok("http://tauri.localhost/"));
        assert!(ok("mailview://localhost/12"));
        for bad in [
            "https://hameconnage.example/",
            "http://localhost:8080/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,x",
            "mailpart://localhost/1/2",
        ] {
            assert!(!ok(bad), "{bad}");
        }
    }

    #[test]
    fn message_ids_in_paths_are_strict() {
        assert!(message_of("12").is_some());
        for bad in ["", "0", "-1", "1e3", "12a", "99999999999999", "../1"] {
            assert!(message_of(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn paths_are_split_into_segments() {
        let request = Request::builder()
            .uri("mailpart://localhost/12/1.2")
            .body(Vec::new())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(segments(&request), ["12", "1.2"]);
    }
}
