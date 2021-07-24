#![feature(async_closure)]

use crate::shortcuts::ShortcutDb;
use crate::utils::*;
use crate::visits::{Visit, VisitsLog};
use actix_service::Service;
use actix_web::body::AnyBody;
use actix_web::dev::{self, HttpServiceFactory, RequestHead, ServiceResponse};
use actix_web::middleware::{ErrorHandlerResponse, ErrorHandlers};
use actix_web::{
    delete, get, guard, post, web, App, HttpRequest, HttpResponse, HttpServer, Responder,
};
use blog::Blog;
use futures::future::FutureExt;
use http::StatusCode;
use log::{info, warn, LevelFilter};
use rustls::{NoClientAuth, ServerConfig};
use shortcuts::Shortcut;
use simplelog::{ColorChoice, TermLogger, TerminalMode};
use std::collections::HashMap;
use std::io::BufReader;
use std::net::SocketAddr;
use tokio::fs;

mod blog;
mod shortcuts;
mod utils;
mod visits;

#[derive(Clone)]
struct Config {
    address: SocketAddr,
    admin_key: String,
    tls_config: Option<TlsConfig>,
}
#[derive(Clone)]
struct TlsConfig {
    certificate: String,
    key: String,
}
impl Config {
    async fn load() -> Self {
        let config = fs::read_to_string("Config.toml")
            .await
            .unwrap()
            .parse::<toml::Value>()
            .unwrap();
        Self {
            address: config["address"].as_str().unwrap().parse().unwrap(),
            admin_key: config["admin_key"].as_str().unwrap().into(),
            tls_config: config
                .get("https")
                .and_then(|it| it.as_table())
                .map(|cert_info| TlsConfig {
                    certificate: cert_info["certificate_chain"].as_str().unwrap().into(),
                    key: cert_info["private_key"].as_str().unwrap().into(),
                }),
        }
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    TermLogger::init(
        LevelFilter::Info,
        simplelog::Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .unwrap();

    let config = web::Data::new(Config::load().await);
    let visits_log = web::Data::new(VisitsLog::new());
    let blog = web::Data::new(Blog::new().await);
    let shortcut_db = web::Data::new(ShortcutDb::new().await);
    let address = config.address.clone();

    let tls_config = config.tls_config.clone().map(|config| {
        let mut tls_config = ServerConfig::new(NoClientAuth::new());
        tls_config
            .set_single_cert(
                load_certs(&config.certificate),
                load_private_key(&config.key),
            )
            .unwrap();
        tls_config
    });

    // TODO: Enable compression?
    let cloned_log = visits_log.clone();
    let server = HttpServer::new(move || {
        let cloned_log = cloned_log.clone();
        App::new()
            .app_data(cloned_log.clone())
            .app_data(blog.clone())
            .app_data(shortcut_db.clone())
            .wrap_fn(move |req, srv| {
                let log = cloned_log.clone();
                let visit = Visit::for_request(&req);
                srv.call(req).then(async move |res| {
                    log.register(visit.finish(&res)).await;
                    res
                })
            })
            .wrap(
                ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, error_500_handler),
            )
            // .wrap(middleware::NormalizePath::default())
            .service(index)
            .service(go_shortcut)
            .service(api(&config.admin_key))
            .service(url_with_key)
            .default_service(web::route().to(default_handler))
    });

    let server = if let Some(config) = tls_config {
        info!("Binding using HTTPS.");
        server.bind_rustls(address, config)?
    } else {
        warn!("Binding using insecure HTTP.");
        server.bind(address)?
    };

    server.run().await?;

    info!("Server ended.");
    visits_log.flush().await;

    info!("Ending server executable.");
    Ok(())
}

fn load_certs(filename: &str) -> Vec<rustls::Certificate> {
    let certfile = std::fs::File::open(filename).expect("Can't open the certificate file.");
    let mut reader = BufReader::new(certfile);
    rustls_pemfile::certs(&mut reader)
        .unwrap()
        .iter()
        .map(|v| rustls::Certificate(v.clone()))
        .collect()
}

fn load_private_key(filename: &str) -> rustls::PrivateKey {
    let keyfile = std::fs::File::open(filename).expect("Can't open the private key file.");
    let mut reader = BufReader::new(keyfile);

    loop {
        match rustls_pemfile::read_one(&mut reader).expect("Can't parse the .pem file.") {
            Some(rustls_pemfile::Item::RSAKey(key)) => return rustls::PrivateKey(key),
            Some(rustls_pemfile::Item::PKCS8Key(key)) => return rustls::PrivateKey(key),
            None => break,
            _ => {}
        }
    }

    panic!("No keys found in {:?}.", filename);
}

// Visitors of mgar.us get a list of all articles.
#[get("/")]
async fn index(blog: web::Data<Blog>) -> impl Responder {
    let article_template = template::article_teaser().await;
    let articles = blog
        .list()
        .await
        .into_iter()
        .rev()
        .map(|article| article_template.fill_in_article(&article))
        .collect::<Vec<_>>();
    let page = template::page()
        .await
        .fill_in_content(&itertools::join(articles, "\n"));
    HttpResponse::Ok().html(page)
}

/// For brevity, most URLs consist of a single key.
#[get("/{key}")]
async fn url_with_key(req: HttpRequest, path: web::Path<(String,)>) -> impl Responder {
    let (key,) = path.into_inner();

    // Check if this is one of the static assets.
    let static_assets: HashMap<String, String> = vec![
        ("favicon.ico", "image/vnd.microsoft.icon"),
        ("icon.png", "image/png"),
        ("prism.css", "text/css"),
        ("prism.js", "text/javascript"),
    ]
    .into_iter()
    .map(|(key, content_type)| (key.to_owned(), content_type.to_owned()))
    .collect();
    if let Some(content_type) = static_assets.get(&key) {
        return match fs::read(&format!("assets/{}", key)).await {
            Ok(content) => HttpResponse::Ok()
                .content_type(content_type.to_owned())
                .body(content),
            Err(_) => panic!("The file is missing."),
        };
    }

    // Or maybe it's a blog article?
    let blog = req.app_data::<web::Data<Blog>>().unwrap();
    if let Some(article) = blog.get(&key).await {
        let article_html = template::full_article().await.fill_in_article(&article);
        return HttpResponse::Ok().html(template::page().await.fill_in_content(&article_html));
    }

    error_page_404().await
}

/// Shortcuts are not content of the website itself. Rather, they redirect to somewhere else.
#[get("/go/{shortcut}")]
async fn go_shortcut(
    path: web::Path<(String,)>,
    shortcut_db: web::Data<ShortcutDb>,
) -> impl Responder {
    let (shortcut,) = path.into_inner();
    if let Some(shortcut) = shortcut_db.shortcut_for(&shortcut).await {
        return HttpResponse::Found()
            .append_header(("Location", shortcut.url.clone()))
            .body("");
    }

    error_page_404().await
}

fn api(admin_key: &str) -> impl HttpServiceFactory {
    web::scope("/api")
        .guard(AuthGuard(admin_key.into()))
        .service(
            web::scope("/shortcuts")
                .service(shortcuts_api::list)
                .service(shortcuts_api::update)
                .service(shortcuts_api::remove),
        )
        .service(web::scope("/blog").service(blog_api::refresh))
        .service(web::scope("/visits").service(visits_api::tail))
}
pub struct AuthGuard(String);
impl guard::Guard for AuthGuard {
    fn check(&self, req: &RequestHead) -> bool {
        if let Some(val) = req.headers.get("admin-key") {
            return consistenttime::ct_u8_slice_eq(val.as_bytes(), self.0.as_bytes());
        }
        false
    }
}

mod shortcuts_api {
    use super::*;

    #[get("/")]
    async fn list(shortcut_db: web::Data<ShortcutDb>) -> impl Responder {
        let shortcuts = shortcut_db.list().await;
        HttpResponse::Ok().json(shortcuts)
    }

    #[post("/")]
    async fn update(
        shortcut: web::Json<Shortcut>,
        shortcut_db: web::Data<ShortcutDb>,
    ) -> impl Responder {
        shortcut_db.register(shortcut.0).await;
        HttpResponse::Ok().body("Added shortcut.")
    }

    #[delete("/{shortcut}")]
    async fn remove(
        path: web::Path<(String,)>,
        shortcut_db: web::Data<ShortcutDb>,
    ) -> impl Responder {
        let (shortcut,) = path.into_inner();
        shortcut_db.delete(&shortcut).await;
        HttpResponse::Ok().body("Deleted shortcut.")
    }
}

mod blog_api {
    use super::*;

    #[get("/refresh")]
    pub async fn refresh(blog: web::Data<Blog>) -> impl Responder {
        match blog.load().await {
            Ok(_) => HttpResponse::Ok().body("Refreshed blog articles."),
            Err(error) => HttpResponse::InternalServerError().body(error),
        }
    }
}

mod visits_api {
    use super::*;

    #[get("/tail")]
    async fn tail(visits_log: web::Data<VisitsLog>) -> impl Responder {
        HttpResponse::Ok().json(visits_log.get_tail().await)
    }
}

async fn default_handler(req: HttpRequest) -> impl Responder {
    warn!("Default handler invoked. The request was: {:?}", req);
    error_page_404().await
}

async fn error_page_404() -> HttpResponse {
    error_page(
        StatusCode::NOT_FOUND,
        "Nope-di-nope. Nothing to see here.",
        "From whatever place you dug up that URL… it's doesn't link to any content.",
    )
    .await
}

async fn error_page(status: StatusCode, title: &str, description: &str) -> HttpResponse {
    let error_html = template::error()
        .await
        .fill_in_error(status, title, description);
    HttpResponse::Ok()
        .status(status)
        .html(template::page().await.fill_in_content(&error_html))
}

fn error_500_handler(
    service_response: dev::ServiceResponse<AnyBody>,
) -> actix_web::Result<ErrorHandlerResponse<AnyBody>> {
    let req = service_response.request().clone();
    Ok(ErrorHandlerResponse::Future(Box::pin(async {
        Ok(ServiceResponse::new(
            req,
            error_page(StatusCode::INTERNAL_SERVER_ERROR, "", "").await,
        ))
    })))
}
