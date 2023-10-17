use std::{collections::HashMap, convert::Infallible, net::SocketAddr, sync::OnceLock};

use askama::Template;
use axum::{
    extract::Path,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive},
        Html, Sse,
    },
    routing::{get, post},
    Form, Router, Server,
};
use axum_macros::debug_handler;
use futures::Stream;
use serde::Deserialize;
use tokio::sync::{mpsc::Sender, RwLock};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tower_http::trace::TraceLayer;
use tracing::Level;
use tracing_subscriber::{filter, prelude::*};

static TX: OnceLock<RwLock<HashMap<usize, Option<Sender<String>>>>> = OnceLock::new();

#[tokio::main]
async fn main() {
    let tracing_filter = filter::Targets::new()
        .with_target("tower_http::trace::on_response", Level::DEBUG)
        .with_target("tower_http::trace::on_request", Level::DEBUG)
        .with_target("tower_http::trace::make_span", Level::DEBUG)
        .with_default(Level::INFO);
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_filter)
        .init();

    TX.set(RwLock::new(HashMap::new()))
        .expect("Unable to init TX");

    let router = Router::new()
        .route("/", get(index))
        .route("/start", post(start))
        .route("/send_msg/:user_id", post(send_msg))
        .route("/sse/:user_id", get(sse))
        .layer(TraceLayer::new_for_http())
        .into_make_service();

    let addr = SocketAddr::from(([0, 0, 0, 0], 1370));

    Server::bind(&addr)
        .serve(router)
        .await
        .expect("Server startup failed.");
}

#[derive(Template)]
#[template(path = "index.html")]
struct Index;

#[derive(Template)]
#[template(path = "chat.html")]
struct ChatView {
    user_id: String,
}

#[derive(Template)]
#[template(path = "sse.html")]
struct SseView {
    from_user_id: usize,
    data: String,
}

async fn index() -> Html<String> {
    Html::from(Index.render().unwrap_or_default())
}

async fn start() -> (StatusCode, Html<String>) {
    let user_id = rand::random::<usize>();

    (
        StatusCode::OK,
        Html::from(
            ChatView {
                user_id: user_id.to_string(),
            }
            .render()
            .unwrap_or_default(),
        ),
    )
}

async fn sse(
    Path(user_id): Path<usize>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(1);

    match TX.get() {
        Some(global_tx) => {
            let mut map = global_tx.write().await;
            map.insert(user_id, Some(tx));
            let stream = ReceiverStream::new(rx).map(|data| Ok(Event::default().data(data)));
            Ok(Sse::new(stream).keep_alive(KeepAlive::new().text("keep-alive-text")))
        }
        None => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Deserialize)]
struct SendMsgReq {
    target_id: usize,
    message: String,
}

#[debug_handler]
async fn send_msg(Path(user_id): Path<usize>, Form(req): Form<SendMsgReq>) -> StatusCode {
    let Some(global_tx) = TX.get() else {
        return StatusCode::INTERNAL_SERVER_ERROR;
    };

    if let (Some(Some(_)), Some(Some(tx))) = (
        global_tx.read().await.get(&user_id),
        global_tx.read().await.get(&req.target_id),
    ) {
        let tx = tx.clone();
        match tx
            .send(
                SseView {
                    from_user_id: user_id,
                    data: req.message,
                }
                .render()
                .unwrap_or_default(),
            )
            .await
        {
            Ok(_) => StatusCode::OK,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    } else {
        StatusCode::NOT_FOUND
    }
}
