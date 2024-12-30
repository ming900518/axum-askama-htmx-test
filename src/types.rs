use askama::Template;
use serde::Deserialize;

#[derive(Template)]
#[template(path = "index.html")]
pub struct Index;

#[derive(Template)]
#[template(path = "chat.html")]
pub struct ChatView {
    pub user_id: String,
}

#[derive(Template)]
#[template(path = "sse.html")]
pub struct SseView {
    pub from_user_id: u16,
    pub data: String,
}

#[derive(Deserialize)]
pub struct SendMsgReq {
    pub target_id: u16,
    pub message: String,
}
