use std::{collections::HashMap, convert::Infallible, net::SocketAddr, sync::OnceLock};

mod types;

use askama::Template;
use axum::{
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive},
        Html, Sse,
    },
    routing::{get, post},
    Form, Router,
};
use axum_extra::extract::{cookie::Cookie, CookieJar};
use futures::Stream;
use time::macros::{format_description, offset};
use tokio::{
    net::TcpListener,
    sync::{mpsc::Sender, RwLock},
};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tower_http::trace::TraceLayer;
use tracing::Level;
use tracing_subscriber::{filter, fmt::time::OffsetTime, prelude::*};
use types::{ChatView, Index, SendMsgReq, SseView};

static TX: OnceLock<RwLock<HashMap<u16, Sender<String>>>> = OnceLock::new();

#[tokio::main]
async fn main() {
    init_logger();
    TX.set(RwLock::new(HashMap::new()))
        .expect("Unable to init TX");

    let router = Router::new()
        .route("/", get(index))
        .route("/start", post(start))
        .route("/send_msg", post(send_msg))
        .route("/sse", get(sse))
        .layer(TraceLayer::new_for_http())
        .into_make_service();

    let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 8000)))
        .await
        .expect("Unable to listen port 8000.");

    axum::serve(listener, router)
        .await
        .expect("Server startup failed.");
}

async fn index() -> Html<String> {
    Html::from(Index.render().unwrap_or_default())
}

async fn start(cookies: CookieJar) -> (CookieJar, Html<String>) {
    let user_id = rand::random::<u16>();

    (
        cookies.add(Cookie::new("UserId", format!("{user_id}"))),
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
    cookies: CookieJar,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(1);

    let Some(user_id) = cookies.get("UserId") else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let Ok(user_id) = user_id.value().parse() else {
        return Err(StatusCode::BAD_REQUEST);
    };

    match TX.get() {
        Some(global_tx) => {
            global_tx.write().await.insert(user_id, tx);
            let stream = ReceiverStream::new(rx).map(|data| Ok(Event::default().data(data)));
            Ok(Sse::new(stream).keep_alive(KeepAlive::new().text("keep-alive-text")))
        }
        None => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn send_msg(cookies: CookieJar, Form(req): Form<SendMsgReq>) -> StatusCode {
    let Some(global_tx) = TX.get() else {
        return StatusCode::INTERNAL_SERVER_ERROR;
    };

    let Some(user_id) = cookies.get("UserId") else {
        return StatusCode::UNAUTHORIZED;
    };

    let Ok(user_id) = user_id.value().parse() else {
        return StatusCode::BAD_REQUEST;
    };

    let reader = global_tx.read().await;
    if let (Some(self_tx), Some(target_tx)) = (reader.get(&user_id), reader.get(&req.target_id)) {
        match (
            self_tx
                .send(
                    SseView {
                        from_user_id: user_id,
                        data: req.message.clone(),
                    }
                    .render()
                    .unwrap_or_default(),
                )
                .await,
            target_tx
                .send(
                    SseView {
                        from_user_id: user_id,
                        data: req.message,
                    }
                    .render()
                    .unwrap_or_default(),
                )
                .await,
        ) {
            (Ok(()), Ok(())) => StatusCode::OK,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    } else {
        StatusCode::NOT_FOUND
    }
}

fn init_logger() {
    let format = tracing_subscriber::fmt::format()
            .compact()
            .with_timer(OffsetTime::new(
                offset!(+8),
                format_description!("[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute padding:zero]"),
            ))
            .with_target(cfg!(debug_assertions))
            .with_thread_names(cfg!(debug_assertions))
            .with_source_location(cfg!(debug_assertions))
            .with_line_number(cfg!(debug_assertions));

    let filter = filter::Targets::new()
        .with_target("tower_http::trace::on_response", Level::DEBUG)
        .with_target("tower_http::trace::on_request", Level::DEBUG)
        .with_target("tower_http::trace::make_span", Level::DEBUG)
        .with_default(Level::INFO);

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().event_format(format))
        .with(filter)
        .init();
}
