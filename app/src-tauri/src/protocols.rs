//! Protocoles personnalisés du rendu (ADR-004) :
//! - `mailview://localhost/<message>` : document assaini du message, avec
//!   la CSP qui n'autorise que l'agent ;
//! - `mailpart://localhost/<message>/<partie>` : image en ligne (`cid:`),
//!   types d'image matriciels seulement ;
//! - `remote-img://localhost/<message>/<index>` : image distante via le
//!   proxy, seulement si l'expéditeur est autorisé (vérifié à chaque
//!   requête : l'index renvoie à la liste établie par le dernier rendu,
//!   qui n'en contient que pour un expéditeur autorisé) ;
//! - `contact-photo://localhost/<contact>` : photo d'un contact du carnet,
//!   en lecture seule, telle que l'import l'a réencodée (JPEG seulement,
//!   revérifié à chaque requête).
//!
//! Seule la fenêtre principale peut s'en servir. Aucune réponse n'est mise
//! en cache par la webview ; `nosniff` partout.

use norelly_application::contact_photos::ReadContactPhoto;
use norelly_domain::contact_photo::{PHOTO_MEDIA_TYPE, is_kept_photo};
use norelly_domain::contacts::ContactId;
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

fn contact_of(raw: &str) -> Option<ContactId> {
    if raw.is_empty() || raw.len() > 12 || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok().and_then(ContactId::new)
}

/// La requête vient-elle de l'interface ? `Origin` et `Referer`, s'ils sont
/// présents, doivent désigner l'interface elle-même ([`is_interface`]).
fn from_interface(request: &Request<Vec<u8>>) -> bool {
    [header::ORIGIN, header::REFERER].iter().all(|name| {
        request.headers().get(name).is_none_or(|value| {
            value
                .to_str()
                .ok()
                .and_then(|v| url::Url::parse(v).ok())
                .is_some_and(|url| is_interface(&url))
        })
    })
}

/// Adresse de l'interface (pas d'un document de message).
fn is_interface(url: &url::Url) -> bool {
    match (url.scheme(), url.host_str()) {
        ("tauri", Some("localhost")) | ("http" | "https", Some("tauri.localhost")) => true,
        // Serveur de développement Vite (pnpm tauri dev).
        ("http", Some("localhost")) => cfg!(debug_assertions) && url.port() == Some(1420),
        _ => false,
    }
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

async fn contact_photo<R: Runtime>(
    app: &AppHandle<R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    // Demandée par l'interface seulement, jamais par un document de
    // message (cadre `mailview://`, origine opaque).
    if !from_interface(&request) {
        return status(StatusCode::FORBIDDEN);
    }
    let state = app.state::<AppState>();
    let parts = segments(&request);
    let [id] = parts.as_slice() else {
        return status(StatusCode::NOT_FOUND);
    };
    let Some(id) = contact_of(id) else {
        return status(StatusCode::NOT_FOUND);
    };
    let read = ReadContactPhoto {
        directory: &state.contacts,
        photos: &state.contact_photos,
    };
    match read.execute(id).await {
        // Même contrôle que les images du rendu (type matriciel accepté),
        // et le contenu doit être le JPEG réencodé : rien d'autre n'est
        // servi, même depuis un magasin altéré.
        Ok(Some(bytes)) => match accepted_type(Some(PHOTO_MEDIA_TYPE)) {
            Some(media_type) if is_kept_photo(&bytes) => ok(media_type, NO_CONTENT_CSP, bytes),
            _ => status(StatusCode::FORBIDDEN),
        },
        Ok(None) => status(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::warn!(error = %e, "photo de contact indisponible");
            status(StatusCode::NOT_FOUND)
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

/// Enregistre les quatre protocoles.
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
        .register_asynchronous_uri_scheme_protocol("contact-photo", |ctx, request, responder| {
            serve(&ctx, request, responder, |app, request| {
                tauri::async_runtime::spawn(async move { contact_photo(&app, request).await })
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
            "contact-photo://localhost/1",
            "http://contact-photo.localhost/1",
        ] {
            assert!(!ok(bad), "{bad}");
        }
    }

    /// Directive `name` de la CSP de la fenêtre principale.
    fn window_csp(name: &str) -> Vec<String> {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .unwrap_or_else(|e| panic!("{e}"));
        let csp = conf["app"]["security"]["csp"].as_str().unwrap_or_default();
        csp.split(';')
            .map(str::trim)
            .find_map(|d| d.strip_prefix(name).filter(|r| r.starts_with(' ')))
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_window_loads_images_from_itself_and_contact_photos_only() {
        assert_eq!(
            window_csp("img-src"),
            [
                "'self'",
                "data:",
                "contact-photo:",
                "http://contact-photo.localhost"
            ]
        );
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

    fn get(uri: &str) -> Request<Vec<u8>> {
        Request::builder()
            .uri(uri)
            .body(Vec::new())
            .unwrap_or_else(|e| panic!("{e}"))
    }

    #[tokio::test]
    async fn mail_views_carry_the_security_headers() {
        let t = crate::test_app::test_app().await;
        let handle = t.app.handle();
        let id = t.messages[0].get();
        let response = mailview(handle, get(&format!("mailview://localhost/{id}"))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let headers = response.headers();
        assert_eq!(
            headers.get(header::CONTENT_TYPE).map(|v| v.as_bytes()),
            Some(&b"text/html; charset=utf-8"[..])
        );
        assert_eq!(
            headers
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .map(|v| v.as_bytes()),
            Some(&b"nosniff"[..])
        );
        assert_eq!(
            headers.get(header::REFERRER_POLICY).map(|v| v.as_bytes()),
            Some(&b"no-referrer"[..])
        );
        let csp = headers
            .get(header::CONTENT_SECURITY_POLICY)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(csp.contains("default-src 'none'"), "{csp}");
        for bad in [
            "mailview://localhost/",
            "mailview://localhost/abc",
            "mailview://localhost/1/2",
            "mailview://localhost/999999",
        ] {
            assert_eq!(
                mailview(handle, get(bad)).await.status(),
                StatusCode::NOT_FOUND,
                "{bad}"
            );
        }
    }

    #[tokio::test]
    async fn contact_photos_are_served_read_only_as_jpeg() {
        use norelly_domain::contact_import::{PhotoTarget, PlacedPhoto};
        use norelly_domain::ports::{ContactBook, ContactPhotoStore};
        use norelly_test_support::contacts_contracts::sample_contact;

        let t = crate::test_app::test_app().await;
        let handle = t.app.handle();
        let state = t.state();
        let jpeg = b"\xFF\xD8\xFF\xE0 vignette".to_vec();
        let photo = state
            .contact_photos
            .put(jpeg.clone())
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        // Contenu enregistré qui n'est pas un JPEG (magasin altéré).
        let trapped = state
            .contact_photos
            .put(b"<svg onload=alert(1)>".to_vec())
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        state
            .contacts
            .import(
                &[
                    sample_contact("Alice", &["alice@example.com"]),
                    sample_contact("Bob", &["bob@example.com"]),
                    sample_contact("Carole", &["carole@example.com"]),
                ],
                &[],
                &[],
                &[
                    PlacedPhoto {
                        target: PhotoTarget::Created(0),
                        photo,
                    },
                    PlacedPhoto {
                        target: PhotoTarget::Created(2),
                        photo: trapped,
                    },
                ],
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let response = contact_photo(handle, get("contact-photo://localhost/1")).await;
        assert_eq!(response.status(), StatusCode::OK);
        // Demandée par l'interface : servie aussi.
        let from_ui = Request::builder()
            .uri("contact-photo://localhost/1")
            .header("Referer", "tauri://localhost/index.html")
            .body(Vec::new())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            contact_photo(handle, from_ui).await.status(),
            StatusCode::OK
        );
        let headers = response.headers();
        assert_eq!(
            headers.get(header::CONTENT_TYPE).map(|v| v.as_bytes()),
            Some(&b"image/jpeg"[..])
        );
        assert_eq!(
            headers
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .map(|v| v.as_bytes()),
            Some(&b"nosniff"[..])
        );
        assert_eq!(
            headers
                .get(header::CONTENT_SECURITY_POLICY)
                .map(|v| v.as_bytes()),
            Some(NO_CONTENT_CSP.as_bytes())
        );
        assert_eq!(response.body(), &jpeg);
        assert_eq!(
            contact_photo(handle, get("contact-photo://localhost/3"))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        for bad in [
            "contact-photo://localhost/2",
            "contact-photo://localhost/999",
            "contact-photo://localhost/",
            "contact-photo://localhost/0",
            "contact-photo://localhost/1/2",
            "contact-photo://localhost/..%2F1",
            "contact-photo://localhost/abababababababababababababababababababababababababababababababab",
        ] {
            assert_eq!(
                contact_photo(handle, get(bad)).await.status(),
                StatusCode::NOT_FOUND,
                "{bad}"
            );
        }
    }

    #[tokio::test]
    async fn photos_no_contact_carries_are_swept_once_old_enough() {
        use norelly_domain::contact_import::{PhotoTarget, PlacedPhoto};
        use norelly_domain::contact_photo::PHOTO_SWEEP_MIN_AGE;
        use norelly_domain::ports::{ContactBook, ContactPhotoStore};
        use norelly_test_support::contacts_contracts::sample_contact;

        let t = crate::test_app::test_app().await;
        let state = t.state();
        let photo = state
            .contact_photos
            .put(b"\xFF\xD8\xFF\xE0 supprimee".to_vec())
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        state
            .contacts
            .import(
                &[sample_contact("Alice", &["alice@example.com"])],
                &[],
                &[],
                &[PlacedPhoto {
                    target: PhotoTarget::Created(0),
                    photo: photo.clone(),
                }],
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let path = state.contact_photos.path_of(&photo);
        let old = std::time::SystemTime::now() - PHOTO_SWEEP_MIN_AGE * 2;
        std::fs::File::options()
            .append(true)
            .open(&path)
            .and_then(|f| f.set_modified(old))
            .unwrap_or_else(|e| panic!("{e}"));
        // Encore portée : gardée.
        crate::state::sweep_contact_photos(&state).await;
        assert!(path.exists());
        let id = ContactId::new(1).unwrap_or_else(|| panic!("id"));
        state
            .contacts
            .delete(id)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        crate::state::sweep_contact_photos(&state).await;
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn a_mail_view_cannot_load_contact_photos() {
        let t = crate::test_app::test_app().await;
        let handle = t.app.handle();
        for (name, value) in [
            ("Referer", "mailview://localhost/1"),
            ("Origin", "http://mailview.localhost"),
            ("Origin", "null"),
        ] {
            let request = Request::builder()
                .uri("contact-photo://localhost/1")
                .header(name, value)
                .body(Vec::new())
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(
                contact_photo(handle, request).await.status(),
                StatusCode::FORBIDDEN,
                "{name}: {value}"
            );
        }
    }

    #[tokio::test]
    async fn parts_and_remote_images_are_refused_by_default() {
        let t = crate::test_app::test_app().await;
        let handle = t.app.handle();
        let id = t.messages[0].get();
        for bad in [
            format!("mailpart://localhost/{id}"),
            format!("mailpart://localhost/{id}/pas-un-chemin"),
            format!("mailpart://localhost/{id}/1.9"),
            "mailpart://localhost/0/1".to_owned(),
        ] {
            assert_eq!(
                mailpart(handle, get(&bad)).await.status(),
                StatusCode::NOT_FOUND,
                "{bad}"
            );
        }
        // Expéditeur non autorisé : aucune requête vers l'extérieur.
        assert_eq!(
            remote_image(handle, get(&format!("remote-img://localhost/{id}/0")))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            remote_image(handle, get("remote-img://localhost/x/0"))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}
